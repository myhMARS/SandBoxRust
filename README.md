<p align="center">
  <h1 align="center">SandBoxRust</h1>
  <p align="center">A high-performance, seccomp-based code execution sandbox for Python and Node.js.</p>
</p>

<p align="center">
  <a href="https://github.com/myhMARS/SandBoxRust/actions"><img src="https://img.shields.io/github/actions/workflow/status/myhMARS/SandBoxRust/ci.yml?branch=main" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.97%2B-orange.svg" alt="Rust"></a>
</p>

---

## Features

- **seccomp-bpf syscall filtering** — per-language whitelists (~70 syscalls for Python, ~120 for Node.js)
- **chroot filesystem isolation** — sandboxed to `/usr/local/share/sandbox`
- **Privilege dropping** — runs as non-root user (`65537:65537`)
- **Pre-warmed zygote pool** — Python interpreter pre-loaded, children forked via copy-on-write for sub-ms cold starts
- **RLIMIT_AS address space cap** — runaway allocations fail cleanly instead of OOMing the host
- **Per-request stdin execution** — no temp files, code fed over a pipe before sandbox is applied
- **Concurrency control** — configurable worker pool with FIFO queuing
- **Simple HTTP API** — REST interface with API key authentication

## Quick Start

```bash
# Build and run with Docker
docker build -t sandbox-server .
docker run -d -p 8194:8194 sandbox-server

# Smoke test
curl -H "X-Api-Key: sandbox" http://127.0.0.1:8194/health
```

### Local Development

```bash
# Dependencies: Rust 1.97+, Python 3.12, Node.js, libseccomp-dev
sudo apt-get install -y libseccomp-dev

cargo build --release -p sandbox-server
CONFIG_PATH=runtime/config.toml cargo run -p sandbox-server
```

## API

**Authentication:** All endpoints except `/health` require `X-Api-Key` header.

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
| `preload` | `string` | *(optional)* Code injected before user code |
| `options.enable_network` | `bool` | *(optional)* Opt-in network access per request |

**Error responses:**

| Code | Meaning |
|------|---------|
| `0` | Success |
| `31` | seccomp violation (blocked syscall) |
| `400` | Unsupported language |
| `500` | Execution error |

## Configuration

Defaults are in [`runtime/config.toml`](runtime/config.toml). Every value can be overridden by an environment variable at runtime.

| Key | Env Variable | Default | Description |
|-----|-------------|---------|-------------|
| `app.port` | `SANDBOX_PORT` | `8194` | HTTP listen port |
| `app.key` | `API_KEY` | `sandbox` | API key for request auth |
| `max_workers` | `MAX_WORKERS` | `4` | Max concurrent sandbox executions |
| `worker_timeout` | `WORKER_TIMEOUT` | `30` | Per-request timeout (seconds) |
| `enable_network` | `ENABLE_NETWORK` | `true` | Allow per-request network opt-in |
| `python_zygote` | `PYTHON_ZYGOTE` | `true` | Pre-warmed zygote (Linux-only) |
| `python_max_as_bytes` | `PYTHON_MAX_AS_BYTES` | `1073741824` | Python address space cap (1 GiB) |
| `nodejs_max_as_bytes` | `NODEJS_MAX_AS_BYTES` | `2147483648` | Node.js address space cap (2 GiB) |

## Architecture

```
HTTP Request
    │
    ▼
┌─────────────────────────────────┐
│  actix-web server (Rust)        │
│  ├─ API key middleware           │
│  ├─ Job queue (max_workers cap) │
│  └─ Timeout / kill_on_drop      │
└────────────┬────────────────────┘
             │
    ┌────────┴────────┐
    ▼                 ▼
┌──────────┐   ┌──────────────┐
│ Python   │   │ Node.js      │
│ ┌──────┐ │   │              │
│ │zygote│ │   │ node -       │
│ │pool  │ │   │ (stdin)      │
│ │COW   │ │   │              │
│ │fork  │ │   └──────┬───────┘
│ └──┬───┘ │          │
│    │     │          │
└────┼─────┘          │
     │                │
     ▼                ▼
┌──────────────────────────────┐
│  init_seccomp()              │
│  ├─ chroot(".")              │
│  ├─ setuid / setgid          │
│  ├─ seccomp-bpf filter       │
│  └─ exec(user code)          │
└──────────────────────────────┘
```

## Security Model

| Layer | Mechanism |
|-------|-----------|
| **Syscall filtering** | seccomp-bpf with per-language whitelists. SIGSYS kills the process on any disallowed syscall. |
| **Filesystem isolation** | chroot to `/usr/local/share/sandbox`. Runtime libraries hard-linked into the jail at startup. |
| **Privilege drop** | Non-root UID/GID `65537:65537` before executing user code. |
| **Memory bounds** | RLIMIT_AS per request (1 GiB Python, 2 GiB Node.js). Runaway allocations fail with ENOMEM. |
| **Output bounds** | 10 MiB stdout+stderr cap per request. Exceeded → process killed, error returned. |
| **FD cleanup** | Forked children enumerate `/proc/self/fd` and close every inherited descriptor except stdio. |
| **No temp files** | Code is fed over stdin. The interpreter reads and compiles it before seccomp is applied. |

## Project Layout

```
SandBoxRust/
├── server/src/
│   ├── main.rs              # Entry point, bootstrap
│   ├── config.rs            # .toml + env-var config
│   ├── handlers.rs          # HTTP route handlers
│   ├── middleware.rs         # API key auth
│   ├── models.rs            # Request / response DTOs
│   ├── queue.rs             # Concurrent worker pool
│   ├── services/            # Sandbox execution engine
│   │   ├── python.rs        # Python runner (stdin + zygote fast-path)
│   │   ├── nodejs.rs        # Node.js runner (stdin)
│   │   └── zygote.rs        # Pre-warmed zygote process pool
│   └── setup/               # One-time initialisation
│       ├── env.rs           # chroot jail setup
│       └── dependencies.rs  # pip install / list
├── lib/sandbox_seccomp/     # seccomp .so (cdylib)
├── runtime/                 # Assets copied into the container
│   ├── config.toml
│   ├── prescript.py / .js   # Bootstrap scripts (apply sandbox, then exec)
│   ├── pool/                # Zygote worker + wire protocol
│   └── script/              # env.sh (hard-link setup)
└── tests/                   # Stress tests & benchmarks
```

## Testing

```bash
# Unit tests
cargo test -p sandbox-server

# Lint
cargo clippy -p sandbox-server -- -D warnings
```

**Stress tests** (start the server first):

```bash
# Full code-execution stress test (50 requests, 10 concurrent)
python tests/stress_test.py -n 50 -c 10

# RPS benchmark — finds max sustainable throughput
python tests/bench_health.py
```

Both scripts read `tests/.env` for target URL and API key. Copy [`.env.example`](tests/.env.example) to `.env` and edit as needed.

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feat/your-feature`)
3. Make changes, add tests, ensure `cargo test` and `cargo clippy` pass
4. Open a pull request against `main`

CI runs on every push to `main` / `develop`:
- Build, unit tests, clippy lint, seccomp .so compilation
- Docker build + container smoke test (Python & Node.js) + stress test

## License

[MIT](LICENSE)
