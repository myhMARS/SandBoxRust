use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot};

use crate::models::ApiResponse;

type Job = Box<
    dyn FnOnce() -> Pin<Box<dyn Future<Output = ApiResponse> + Send>> + Send + 'static,
>;

pub(crate) struct QueueMessage {
    job: Job,
    respond: oneshot::Sender<ApiResponse>,
    enqueued_at: Instant,
    language: String,
}

#[derive(Debug)]
pub struct QueueStats {
    pub queue_depth: AtomicUsize,
    pub workers: usize,
}

#[derive(Clone)]
pub struct QueueController {
    pub(crate) sender: mpsc::UnboundedSender<QueueMessage>,
    pub stats: Arc<QueueStats>,
}

impl QueueController {
    pub fn start(max_workers: usize) -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<QueueMessage>();
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let stats = Arc::new(QueueStats {
            queue_depth: AtomicUsize::new(0),
            workers: max_workers,
        });

        for worker_id in 0..max_workers {
            let stats = Arc::clone(&stats);
            let rx = Arc::clone(&rx);
            tokio::spawn(async move {
                loop {
                    let msg = {
                        let mut guard = rx.lock().await;
                        guard.recv().await
                    };
                    match msg {
                        None => break,
                        Some(msg) => {
                            let depth =
                                stats.queue_depth.fetch_sub(1, Ordering::Relaxed);
                            let t_exec = Instant::now();
                            let queue_wait_ms =
                                (t_exec - msg.enqueued_at).as_millis();

                            let result = (msg.job)().await;

                            let exec_ms = t_exec.elapsed().as_millis();
                            let total_ms = queue_wait_ms + exec_ms;
                            tracing::info!(
                                worker = worker_id,
                                lang = %msg.language,
                                queue_wait_ms = queue_wait_ms,
                                exec_ms = exec_ms,
                                total_ms = total_ms,
                                queue_depth = depth,
                                "request done"
                            );

                            let _ = msg.respond.send(result);
                        }
                    }
                }
            });
        }

        Self { sender: tx, stats }
    }

    pub async fn submit<F, Fut>(&self, language: String, f: F) -> ApiResponse
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ApiResponse> + Send + 'static,
    {
        let enqueued_at = Instant::now();
        let (tx, rx) = oneshot::channel();

        self.stats.queue_depth.fetch_add(1, Ordering::Relaxed);

        let job: Job = Box::new(move || Box::pin(f()));
        let _ = self.sender.send(QueueMessage {
            job,
            respond: tx,
            enqueued_at,
            language,
        });

        rx.await.unwrap_or_else(|_| ApiResponse::error(500, "worker shut down"))
    }
}
