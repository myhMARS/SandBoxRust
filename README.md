<p align="center">
  <h1 align="center">SandBoxRust</h1>
  <p align="center">A high-performance, seccomp-based code execution sandbox for Python and Node.js with 10-layer defense-in-depth.</p>
</p>

<p align="center">
  <a href="https://github.com/myhMARS/SandBoxRust/actions"><img src="https://img.shields.io/github/actions/workflow/status/myhMARS/SandBoxRust/ci.yml?branch=main" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.97%2B-orange.svg" alt="Rust"></a>
</p>

---
## Features

- **seccomp-bpf syscall filtering** — per-language whitelists (~74 syscalls Python, ~56 Node.js); `openat` allowed only without `O_CREAT` (no file creation)
- **Non-privileged mode with Landlock** — filesystem isolation via Linux Landlock LSM (ABI V6 + `Scope::Signal`), no `CAP_SYS_CHROOT` required
- **chroot filesystem isolation** (privileged mode) — sandboxed to `/usr/local/share/sandbox` with hard-linked read-only system libraries
- **Privilege dropping** — `setgroups(0)` → `setgid` → `setuid` to non-root (`65537:65537`), `PR_SET_NO_NEW_PRIVS` + `PR_SET_DUMPABLE=0`
- **Pre-warmed zygote pool** — Python interpreter and stdlib modules pre-loaded; children forked via copy-on-write for sub-ms cold starts
- **RLIMIT_AS address space cap** — 1 GiB Python / 2 GiB Node.js; runaway allocations fail with ENOMEM instead of OOMing the host
- **Per-request stdin execution** — no temp files; code XOR-encrypted and fed over a pipe, decrypted after sandbox is applied
- **Concurrency control** — configurable MPMC worker pool with FIFO queuing and structured per-request logging
- **Dual-mode deployment** — privileged (chroot) and non-privileged (Landlock) modes for Kubernetes compatibility
- **Crash recovery** — zygote single-flight auto-restart on connection loss; event loop crash isolation per-request

## Quick Start

```bash
# Privileged mode (chroot-based)
docker build -f priv.Dockerfile -t sandbox-rust .
docker run -d -p 8194:8194 sandbox-rust

# Non-privileged mode (Landlock-based, K8s-compatible)
docker build -f nopriv.Dockerfile -t sandbox-rust-nopriv .
docker run -d -p 8194:8194 sandbox-rust-nopriv

# Smoke test
curl -H "X-Api-Key: sandbox" http://127.0.0.1:8194/health
```

### Local Development

```bash
# Dependencies: Rust 1.97+, Python 3.12, Node.js, libseccomp-dev, libseccomp2
sudo apt-get install -y libseccomp-dev libseccomp2

# Build seccomp shared libraries
cargo build --release -p sandbox_seccomp --features python3 && mv target/release/libsandbox.so runtime/libpython.so
cargo build --release -p sandbox_seccomp --features nodejs  && mv target/release/libsandbox.so runtime/libnodejs.so

# Build and run server
cargo build --release -p sandbox-server
CONFIG_PATH=runtime/config.toml cargo run -p sandbox-server
```

## API

**Authentication:** All endpoints except `/health` require `X-Api-Key` header (constant-time comparison via `subtle` crate).

### `GET /health`

```bash
curl http://127.0.0.1:8194/health
```

```json
{ "ok": true, "role": "sandbox", "queue_depth": 0, "workers": 4 }
```

### `POST /v1/sandbox/run`

Execute untrusted code inside the sandbox.

```bash
# Encode "print(1 + 2)" to base64 and send
CODE=$(echo -n 'print(1 + 2)' | base64 -w0)

curl -X POST http://127.0.0.1:8194/v1/sandbox/run \
  -H "Content-Type: application/json" \
  -H "X-Api-Key: sandbox" \
  -d "{\"language\":\"python3\",\"code\":\"$CODE\"}"
```

```json
{
  "code": 0,
  "message": "success",
  "data": {
    "stdout": "3\n",
    "stderr": ""
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `language` | `string` | `"python3"` or `"javascript"` |
| `code` | `string` | Base64-encoded source |
| `preload` | `string` | *(optional)* Code injected before user code *(requires `enable_preload: true`)* |
| `options.enable_network` | `bool` | *(optional)* Opt-in network access per request *(requires global `enable_network: true`)* |

**Error responses:**

| Code | Meaning |
|------|---------|
| `0` | Success |
| `31` | Seccomp violation — blocked syscall killed process with SIGSYS |
| `401` | Invalid or missing API key |
| `400` | Unsupported language |
| `500` | Execution error or timeout |

## Configuration

Defaults are in [`runtime/config.toml`](runtime/config.toml). Every value is overridable by environment variable.

| Key | Env Variable | Default | Description |
|-----|-------------|---------|-------------|
| `app.port` | `SANDBOX_PORT` | `8194` | HTTP listen port |
| `app.key` | `SANDBOX_API_KEY` | `sandbox` | API key for request auth |
| `max_workers` | `MAX_WORKERS` | `4` | Max concurrent sandbox executions |
| `worker_timeout` | `WORKER_TIMEOUT` | `30` | Per-request timeout (seconds) |
| `enable_network` | `ENABLE_NETWORK` | `true` | Global network opt-in gate |
| `enable_preload` | `ENABLE_PRELOAD` | `false` | Allow per-request `preload` code injection |
| `privilege` | `PRIVILEGE` | `true` | Privileged mode (`false` = Landlock instead of chroot) |
| `python_zygote` | `PYTHON_ZYGOTE` | `true` | Pre-warmed zygote pool (Linux-only) |
| `python_path` | `PYTHON_PATH` | `/usr/local/bin/python3` | Python interpreter path |
| `nodejs_path` | `NODEJS_PATH` | `/usr/bin/node` | Node.js interpreter path |
| `sandbox_uid` | — | `65537` | UID after privilege drop |
| `sandbox_gid` | — | `65537` | GID after privilege drop (must not be 0) |
| `python_max_as_bytes` | `PYTHON_MAX_AS_BYTES` | `1073741824` | Python RLIMIT_AS cap (1 GiB) |
| `nodejs_max_as_bytes` | `NODEJS_MAX_AS_BYTES` | `2147483648` | Node.js RLIMIT_AS cap (2 GiB) |
| `python_lib_paths` | `PYTHON_LIB_PATH` | *(see config.toml)* | Comma-separated paths to copy into chroot jail |
| `nodejs_lib_paths` | `NODEJS_LIB_PATH` | *(see config.toml)* | Comma-separated paths to copy into chroot jail |
| `proxy.socks5` | `SOCKS5_PROXY` | — | SOCKS5 proxy (takes precedence over HTTP/HTTPS) |
| `proxy.http` | `HTTP_PROXY` | — | HTTP proxy URL |
| `proxy.https` | `HTTPS_PROXY` | — | HTTPS proxy URL |

**Runtime tuning (env-var only):**

| Env Variable | Default | Description |
|-------------|---------|-------------|
| `CONFIG_PATH` | `runtime/config.toml` | Path to TOML config file |
| `ALLOWED_SYSCALLS` | *(built-in whitelist)* | Comma-separated syscall numbers — **replaces** the default whitelist |
| `NODE_MAX_OLD_SPACE_MB` | `768` | V8 old-space heap cap (MiB), below RLIMIT_AS |
| `ZYGOTE_MAX_OUTPUT_BYTES` | `10485760` (10 MiB) | Per-request stdout+stderr cap in zygote path |

## Architecture

```
HTTP Request
    │
    ▼
┌──────────────────────────────────────┐
│  actix-web server (Rust)             │
│  ├─ API key middleware (constant-time)│
│  ├─ MPMC worker pool (max_workers)   │
│  └─ Timeout / kill_on_drop           │
└────────────┬─────────────────────────┘
             │
    ┌────────┴──────────┐
    ▼                   ▼
┌──────────────┐   ┌──────────────┐
│ Python       │   │ Node.js      │
│ ┌──────────┐ │   │              │
│ │ zygote   │ │   │ node -       │
│ │ pool     │ │   │ (stdin)      │
│ │ COW fork │ │   │              │
│ │ + socket │ │   └──────┬───────┘
│ └────┬─────┘ │          │
│      │       │          │
│  slow path:  │          │
│  python3 -B -│          │
│  (stdin)     │          │
└──────┼───────┘          │
       │                  │
       ▼                  ▼
┌─────────────────────────────────────┐
│  prescript.py / prescript.js        │
│  ├─ apply_landlock() [non-privileged]│
│  └─ init_seccomp()                  │
│       ├─ RLIMIT_AS                  │
│       ├─ chroot(".") [privileged]   │
│       ├─ PR_SET_NO_NEW_PRIVS        │
│       ├─ PR_SET_DUMPABLE=0          │
│       ├─ setgroups + setgid + setuid│
│       └─ seccomp BPF load           │
│  ├─ XOR decrypt / base64 decode     │
│  └─ exec / eval (user code)         │
└─────────────────────────────────────┘
```

## Security Model

10-layer defense-in-depth, executed in order inside each sandboxed process:

| Layer | Mechanism |
|-------|-----------|
| **API auth** | Constant-time `X-Api-Key` comparison via `subtle` crate to prevent timing side-channels |
| **Worker pool** | MPMC channel-based concurrency control, bounded by `max_workers` |
| **Timeout** | `tokio::time::timeout` + `kill_on_drop` → SIGKILL on expiry |
| **RLIMIT_AS** | Virtual address space cap (1 GiB Python / 2 GiB Node.js); 0 disables |
| **chroot / Landlock** | Privileged: `chroot(.)` + `chdir(/)`. Non-privileged: Landlock ABI V6 `PATH_BENEATH` rules with `Scope::Signal` |
| **no_new_privs + dumpable** | `PR_SET_NO_NEW_PRIVS` (block SUID escalation) + `PR_SET_DUMPABLE=0` (hide `/proc/<pid>/` from same-UID peers) |
| **Privilege drop** | `setgroups(0)` → `setgid` → `setuid` to non-root `65537:65537` |
| **Seccomp BPF** | Per-language syscall whitelists; `openat` gated on `O_CREAT=0`; `clone`/`clone3`/`prlimit64`/`ioctl`/`tgkill`/`mkdir` return `EPERM` instead of killing |
| **Code encryption** | XOR with per-request 64-byte random key + base64; code never touches disk |
| **Information hiding** | `PR_SET_DUMPABLE=0` at 3 levels (manager → zygote → child); `/proc/<pid>/maps`, `/proc/<pid>/fd/`, `/proc/<pid>/environ` hidden from same-UID peers |

### Additional hardening

- **FD cleanup**: Forked children enumerate `/proc/self/fd` and close every inherited descriptor except stdio (0/1/2)
- **Output bounds**: 10 MiB stdout+stderr cap per request in zygote path; exceeded → SIGKILL
- **No temp files**: Code fed over stdin pipe, compiled in memory before sandbox is applied
- **Node.js injection prevention**: Base64 validated and re-encoded before splicing into `eval()` template
- **Zygote crash isolation**: Per-fd `try/except BaseException` — one broken request cannot crash the entire worker
- **Zygote auto-restart**: Single-flight `AtomicBool` CAS gate prevents thundering-herd restarts when zygote dies

## Project Layout

```
SandBoxRust/
├── server/src/
│   ├── main.rs              # Entry point, config loading, zygote lifecycle
│   ├── config.rs            # TOML + env-var configuration
│   ├── handlers.rs          # HTTP route handlers, seccomp-violation detection
│   ├── middleware.rs         # API key auth (constant-time compare)
│   ├── models.rs            # Request / response DTOs
│   ├── queue.rs             # MPMC worker pool with structured logging
│   ├── crypto.rs            # XOR encryption + random key generation
│   ├── services/
│   │   ├── python.rs        # Python runner (stdin slow-path + zygote fast-path)
│   │   ├── nodejs.rs        # Node.js runner (stdin, base64 validation, V8 heap cap)
│   │   └── zygote.rs        # Pre-warmed zygote process pool (Unix socket pair)
│   └── setup/
│       ├── env.rs           # chroot jail preparation via env.sh
│       └── dependencies.rs  # pip install / list management
├── lib/sandbox_seccomp/     # Security shared library (cdylib)
│   └── src/
│       ├── lib.rs           # init_seccomp, apply_landlock, seccomp BPF, privilege drop
│       ├── python_syscalls.rs
│       └── nodejs_syscalls.rs
├── runtime/                 # Assets copied into the container image
│   ├── config.toml
│   ├── prescript.py         # Python bootstrap: Landlock → seccomp → decrypt → exec
│   ├── prescript.js         # Node.js bootstrap: Landlock → seccomp → eval
│   ├── requirements.txt     # Python packages (requests, jinja2, orjson)
│   ├── pool/
│   │   ├── zygote_worker.py # Pre-warmed interpreter, COW fork, select() event loop
│   │   └── protocol.py      # 9-byte binary wire protocol
│   └── script/
│       └── env.sh           # Hard-link helper for chroot jail setup
├── tests/
│   ├── security_audit.py    # 9-layer, 52+ test security audit
│   └── stress_test.py       # Concurrent load/stress test
├── priv.Dockerfile          # Privileged mode image (chroot)
├── nopriv.Dockerfile        # Non-privileged mode image (Landlock)
└── sandbox.yaml             # Kubernetes Pod manifest
```

## Testing

```bash
# Unit tests
cargo test -p sandbox-server

# Lint
cargo clippy -p sandbox-server -- -D warnings
```

**Smoke tests** (start the server first):

```bash
# Python
CODE=$(echo -n 'print("hello")' | base64 -w0)
curl -s -X POST http://127.0.0.1:8194/v1/sandbox/run \
  -H "Content-Type: application/json" \
  -H "X-Api-Key: sandbox" \
  -d "{\"language\":\"python3\",\"code\":\"$CODE\"}"

# Node.js
CODE=$(echo -n 'console.log("hello")' | base64 -w0)
curl -s -X POST http://127.0.0.1:8194/v1/sandbox/run \
  -H "Content-Type: application/json" \
  -H "X-Api-Key: sandbox" \
  -d "{\"language\":\"javascript\",\"code\":\"$CODE\"}"
```

**Stress tests:**

```bash
# Concurrent load test (50 requests, 10 concurrent)
python tests/stress_test.py -n 50 -c 10
```

**Security audit:**

```bash
# Comprehensive 9-layer security test (52+ cases)
python tests/security_audit.py
```

Both Python scripts read `tests/.env` for target URL and API key. Copy [`.env.example`](tests/.env.example) to `.env` and edit as needed.

## License

[MIT](LICENSE)