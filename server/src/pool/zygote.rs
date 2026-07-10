//! Pre-warmed Python zygote pool — spawns a Python worker process that
//! pre-imports common modules and fork()s children per request, avoiding
//! per-request interpreter cold start.
//!
//! Modeled after `app/core/pool/zygote_manager.py` from MemoryBear sandbox.

use std::collections::HashMap;
use std::io;
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

use crate::runners::LIB_PATH;

fn socketpair() -> io::Result<(i32, i32)> {
    let mut fds = [-1i32; 2];
    let rc = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((fds[0], fds[1]))
}

// ── Wire protocol (shared with pool/protocol.py) ──

const HEADER_SIZE: usize = 9;
const MSG_RUN: u8 = 1;
const MSG_KILL: u8 = 2;
const MSG_STDOUT: u8 = 3;
const MSG_STDERR: u8 = 4;
const MSG_DONE: u8 = 5;

fn encode_frame(msg_type: u8, req_id: u32, payload: &[u8]) -> Vec<u8> {
    let plen = payload.len() as u32;
    let mut frame = Vec::with_capacity(HEADER_SIZE + payload.len());
    frame.extend_from_slice(&plen.to_be_bytes());
    frame.push(msg_type);
    frame.extend_from_slice(&req_id.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

struct Pending {
    out: Vec<u8>,
    err: Vec<u8>,
    tx: Option<oneshot::Sender<(String, String, i32)>>,
}

// ── Zygote manager ──

pub struct ZygoteManager {
    /// Only the write half is shared/locked. The read half is owned
    /// exclusively by the background reader task, so reads never hold a
    /// lock that writers need (avoids the read-across-await deadlock).
    write_half: Arc<Mutex<OwnedWriteHalf>>,
    _child: std::process::Child,
    _read_task: JoinHandle<()>,
    running: Arc<AtomicBool>,
    next_id: Mutex<u32>,
    pending: Arc<Mutex<HashMap<u32, Pending>>>,
}

impl ZygoteManager {
    pub fn new(
        python_path: &str,
        lib_so: &str,
        lib_dir: &str,
        warm_modules: &[String],
    ) -> io::Result<Self> {
        let (server_fd, worker_fd) = socketpair()?;

        let warm_str = warm_modules.join(",");
        let mut cmd = std::process::Command::new(python_path);
        cmd.env_clear();
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", &path);
        }
        let child = cmd
            .args([
                "-B", "pool/zygote_worker.py",
                &worker_fd.to_string(), lib_so, lib_dir, &warm_str,
            ])
            .current_dir(LIB_PATH)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .spawn()?;

        unsafe { libc::close(worker_fd); }

        let s_server = unsafe { StdUnixStream::from_raw_fd(server_fd) };
        s_server.set_nonblocking(true)?;
        let stream = UnixStream::from_std(s_server)?;
        // Split into independent read/write halves. The reader task owns the
        // read half outright; writers share only the write half. This is what
        // prevents a read().await from blocking concurrent writes.
        let (read_half, write_half) = stream.into_split();
        let write_half = Arc::new(Mutex::new(write_half));

        let running = Arc::new(AtomicBool::new(true));
        let pending = Arc::new(Mutex::new(HashMap::new()));

        let p = pending.clone();
        let r = running.clone();
        let read_task = tokio::spawn(async move { read_loop(read_half, p, r).await });

        Ok(Self {
            write_half,
            _child: child,
            _read_task: read_task,
            running,
            next_id: Mutex::new(1),
            pending,
        })
    }

    /// Returns true if the zygote worker is alive and connected.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Execute code in the pre-warmed zygote.
    ///
    /// Returns `(stdout, stderr, exit_code)`. If the worker has died,
    /// returns an error string in stderr with exit_code = -1 so the
    /// caller can fall back to the slow path.
    pub async fn run(
        &self,
        code_b64: &str,
        key_b64: &str,
        uid: u32,
        gid: u32,
        net: bool,
        timeout: Duration,
    ) -> (String, String, i32) {
        if !self.running.load(Ordering::SeqCst) {
            return ("".into(), "zygote worker not running".into(), -1);
        }

        let (tx, rx) = oneshot::channel();

        let mut next = self.next_id.lock().await;
        let req_id = *next;
        *next = (*next % 0xFFFF_FFFF) + 1;
        drop(next);

        self.pending.lock().await.insert(
            req_id,
            Pending { out: Vec::new(), err: Vec::new(), tx: Some(tx) },
        );

        let payload = serde_json::json!({
            "code": code_b64, "key": key_b64,
            "uid": uid, "gid": gid, "net": net,
        }).to_string();
        let frame = encode_frame(MSG_RUN, req_id, payload.as_bytes());

        {
            let mut w = self.write_half.lock().await;
            if w.write_all(&frame).await.is_err() {
                self.pending.lock().await.remove(&req_id);
                return ("".into(), "zygote write failed".into(), -1);
            }
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => {
                self.pending.lock().await.remove(&req_id);
                result
            }
            _ => {
                let kill_frame = encode_frame(MSG_KILL, req_id, &[]);
                let mut w = self.write_half.lock().await;
                let _ = w.write_all(&kill_frame).await;
                self.pending.lock().await.remove(&req_id);
                ("".into(), "Execution timeout".into(), -1)
            }
        }
    }

    /// Gracefully stop the zygote worker, failing all pending requests.
    #[allow(dead_code)] // lifecycle hook; not yet wired to a shutdown signal
    pub async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self._read_task.abort();
        // Child is killed on drop.
        self._fail_pending("zygote stopped").await;
    }

    async fn _fail_pending(&self, msg: &str) {
        let mut map = self.pending.lock().await;
        let msg = msg.to_string();
        for (_, entry) in map.drain() {
            if let Some(tx) = entry.tx {
                let _ = tx.send(("".into(), msg.clone(), -1));
            }
        }
    }
}

impl Drop for ZygoteManager {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self._read_task.abort();
        // Child is killed when its handle drops.
    }
}

// ── Background reader ──

async fn read_loop(
    mut rd: OwnedReadHalf,
    pending: Arc<Mutex<HashMap<u32, Pending>>>,
    running: Arc<AtomicBool>,
) {
    let mut buf = vec![0u8; 65536];
    let mut frame_buf: Vec<u8> = Vec::new();

    loop {
        let n = match rd.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };

        frame_buf.extend_from_slice(&buf[..n]);

        while frame_buf.len() >= HEADER_SIZE {
            let plen = u32::from_be_bytes([
                frame_buf[0], frame_buf[1], frame_buf[2], frame_buf[3],
            ]) as usize;
            let mtype = frame_buf[4];
            let req_id = u32::from_be_bytes([
                frame_buf[5], frame_buf[6], frame_buf[7], frame_buf[8],
            ]);

            if frame_buf.len() < HEADER_SIZE + plen {
                break;
            }

            let payload = frame_buf[HEADER_SIZE..HEADER_SIZE + plen].to_vec();
            frame_buf.drain(..HEADER_SIZE + plen);

            let mut pending_map = pending.lock().await;
            if let Some(entry) = pending_map.get_mut(&req_id) {
                match mtype {
                    MSG_STDOUT => entry.out.extend_from_slice(&payload),
                    MSG_STDERR => entry.err.extend_from_slice(&payload),
                    MSG_DONE => {
                        let exit_code = if payload.len() >= 4 {
                            i32::from_be_bytes([
                                payload[0], payload[1], payload[2], payload[3],
                            ])
                        } else {
                            -1
                        };
                        let out = String::from_utf8_lossy(&entry.out).into_owned();
                        let err = String::from_utf8_lossy(&entry.err).into_owned();
                        if let Some(tx) = entry.tx.take() {
                            let _ = tx.send((out, err, exit_code));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Connection lost — mark as not running and fail all pending.
    running.store(false, Ordering::SeqCst);
    tracing::error!("Python zygote connection lost; will restart on next request");

    let mut pending_map = pending.lock().await;
    for (_, entry) in pending_map.drain() {
        if let Some(tx) = entry.tx {
            let _ = tx.send(("".into(), "zygote connection lost".into(), -1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Regression test for C1 (zygote deadlock).
    ///
    /// The original design wrapped the whole `UnixStream` in a single
    /// `Mutex` and held the guard across `read().await` in the reader loop.
    /// That prevented `run()` from ever acquiring the lock to send a RUN
    /// frame while the reader was parked waiting for data — a hard deadlock
    /// on the very first request.
    ///
    /// This test reproduces that exact I/O shape with the fixed `into_split`
    /// design: a reader parked in `read().await` on the read half must NOT
    /// block a concurrent writer on the write half. It also exercises a full
    /// round trip. Needs no root / chroot, so it runs anywhere.
    #[tokio::test]
    async fn split_stream_read_does_not_block_write() {
        let (a, b) = UnixStream::pair().unwrap();
        let (mut a_rd, mut a_wr) = a.into_split();
        let (mut b_rd, mut b_wr) = b.into_split();

        // Reader parks in read().await — no data available yet.
        let reader = tokio::spawn(async move {
            let mut buf = [0u8; 16];
            let n = a_rd.read(&mut buf).await.unwrap();
            buf[..n].to_vec()
        });

        // Let the reader actually enter read().await before we write.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // While the reader is parked, the write half must remain usable.
        // (Under the old shared-Mutex design this write could never proceed.)
        a_wr.write_all(b"ping").await.unwrap();
        let mut pbuf = [0u8; 4];
        b_rd.read_exact(&mut pbuf).await.unwrap();
        assert_eq!(&pbuf, b"ping");

        // Peer replies; the parked reader must wake and receive it promptly.
        b_wr.write_all(b"pong").await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(2), reader)
            .await
            .expect("reader hung — C1 deadlock regression")
            .unwrap();
        assert_eq!(&got, b"pong");
    }
}
