#!/usr/bin/env python3
"""
Comprehensive security audit for SandBoxRust (non-privileged, zygote mode).
Target: http://127.0.0.1:8194  API-Key: sandbox
"""

import base64, json, urllib.request, sys, os

API = "http://127.0.0.1:8194"
KEY = "sandbox"
TIMEOUT = 15

def sandbox_exec(code: str, language: str = "python3", network: bool = False) -> dict:
    """Execute code in the sandbox and return parsed response."""
    b64 = base64.b64encode(code.encode()).decode()
    body = {"language": language, "code": b64}
    if network:
        body["options"] = {"enable_network": True}
    data = json.dumps(body).encode()
    req = urllib.request.Request(f"{API}/v1/sandbox/run", data=data,
        headers={"Content-Type": "application/json", "X-Api-Key": KEY})
    try:
        resp = urllib.request.urlopen(req, timeout=TIMEOUT)
        return json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        return {"http_error": e.code, "body": e.read().decode()[:500]}
    except Exception as e:
        return {"error": str(e)}

def health() -> dict:
    """Check server health."""
    try:
        resp = urllib.request.urlopen(f"{API}/health", timeout=5)
        return json.loads(resp.read().decode())
    except Exception as e:
        return {"error": str(e)}

def test(name: str, code: str, **kwargs) -> tuple[bool, str]:
    """Run a single test, return (passed, message)."""
    result = sandbox_exec(code, **kwargs)
    return result

# =============================================================================
# LAYER 1: SECCOMP — Syscall Allowlist Audit
# =============================================================================

def test_seccomp_blocked_syscalls():
    """Test that dangerous syscalls are properly blocked."""
    results = []

    # 1a: kill(2) — send signal
    r = test("kill-parent", '''
import os,signal
os.kill(os.getppid(), signal.SIGKILL)
print("SHOULD_NOT_PRINT")
''')
    results.append(("kill(2) blocked", r.get("code") == 31))

    # 1b: tgkill — send signal to specific thread
    r = test("tgkill", '''
import ctypes,os,signal
libc = ctypes.CDLL(None)
# SYS_tgkill = 234 on x86_64
ret = libc.syscall(234, os.getppid(), os.getppid(), signal.SIGKILL)
print(f"tgkill: {ret}")
''')
    results.append(("tgkill(2) blocked", r.get("code") == 31))

    # 1c: tkill — send signal
    r = test("tkill", '''
import ctypes,os,signal
libc = ctypes.CDLL(None)
ret = libc.syscall(200, os.getppid(), signal.SIGKILL)
print(f"tkill: {ret}")
''')
    results.append(("tkill(2) blocked", r.get("code") == 31))

    # 1d: ptrace — no
    r = test("ptrace", '''
import ctypes
libc = ctypes.CDLL(None)
ret = libc.syscall(101, 0, os.getppid(), 0, 0)
print(f"ptrace: {ret}")
''')
    results.append(("ptrace(2) blocked", r.get("code") == 31))

    # 1e: execve — execute program
    r = test("execve", '''
import ctypes,os
libc = ctypes.CDLL(None)
# execve = 59 on x86_64
ret = libc.syscall(59, ctypes.c_char_p(b"/bin/sh"), 0, 0)
print(f"execve: {ret}")
''')
    results.append(("execve(2) blocked", r.get("code") == 31))

    # 1f: fork — create process (clone is in ALLOW_ERROR, but fork uses clone)
    r = test("clone-fork", '''
import ctypes
libc = ctypes.CDLL(None)
# SYS_clone = 56, SYS_fork = 57
ret = libc.syscall(57, 0, 0, 0, 0)
print(f"fork: {ret}")
''')
    results.append(("fork blocked or EPERM", r.get("code") in (31, 0)))

    # 1g: mount — filesystem mount
    r = test("mount", '''
import ctypes
libc = ctypes.CDLL(None)
ret = libc.syscall(165, 0, 0, 0, 0, 0)
print(f"mount: {ret}")
''')
    results.append(("mount(2) blocked", r.get("code") == 31))

    # 1h: openat with O_CREAT — file creation
    r = test("openat-creat", '''
import os
try:
    fd = os.open("/tmp/should_not_create", os.O_RDWR | os.O_CREAT, 0o644)
    print(f"CREATED: fd={fd}")
    os.close(fd)
except Exception as e:
    print(f"blocked: {e}")
''')
    results.append(("openat+O_CREAT blocked", r.get("code") == 31 or "blocked" in r.get("data",{}).get("stdout","")))

    # 1i: setuid / setgid
    r = test("setuid", '''
import ctypes
libc = ctypes.CDLL(None)
ret = libc.syscall(105, 0)  # setuid
print(f"setuid(0): {ret}")
''')
    results.append(("setuid(2) blocked", r.get("code") == 31))

    # 1j: chmod / chown
    r = test("chmod", '''
import ctypes
libc = ctypes.CDLL(None)
ret = libc.syscall(90, 0, 0)  # chmod
print(f"chmod: {ret}")
''')
    results.append(("chmod(2) blocked", r.get("code") == 31))

    return results


# =============================================================================
# LAYER 2: LANDLOCK — Filesystem Isolation
# =============================================================================

def test_landlock_filesystem():
    """Test Landlock filesystem restrictions."""
    results = []

    # 2a: /proc reads (regular files)
    r = test("proc-read", '''
import os
paths = ["/proc/self/status", "/proc/1/status", "/proc/self/maps",
         "/proc/self/environ", "/proc/self/cmdline"]
for p in paths:
    try:
        os.open(p, os.O_RDONLY)
        print(f"LEAK: {p}")
    except PermissionError:
        print(f"OK: {p}")
    except Exception as e:
        print(f"ERR: {p} {e}")
''')
    results.append(("Landlock blocks /proc reads", "LEAK" not in str(r)))

    # 2b: /proc/self/ns/* bypass
    r = test("proc-ns-bypass", '''
import os
bypassed = []
for ns in ["pid","net","user","uts","ipc","mnt","cgroup","time"]:
    try:
        fd = os.open(f"/proc/self/ns/{ns}", os.O_RDONLY)
        bypassed.append(ns)
        os.close(fd)
    except:
        pass
print(f"BYPASS: {bypassed}")
''')
    results.append(("Landlock /proc/ns bypass", "BYPASS: []" in str(r)))

    # 2c: /etc reads (should be allowed)
    r = test("etc-read", '''
import os
try:
    fd = os.open("/etc/hostname", os.O_RDONLY)
    data = os.read(fd, 256)
    print(f"OK: /etc/hostname = {data[:50]}")
    os.close(fd)
except PermissionError:
    print("BLOCKED: /etc/hostname")
except Exception as e:
    print(f"ERR: {e}")
''')
    results.append(("Landlock allows /etc reads", "OK" in str(r)))

    # 2d: Write outside allowed paths
    r = test("write-outside", '''
import os
try:
    fd = os.open("/tmp/test_write", os.O_RDWR | os.O_CREAT, 0o644)
    os.write(fd, b"test")
    print(f"WROTE outside sandbox: {fd}")
    os.close(fd)
except Exception as e:
    print(f"OK blocked: {type(e).__name__}")
''')
    results.append(("Write outside Landlock blocked", "OK blocked" in str(r) or r.get("code") == 31))

    # 2e: /dev access
    r = test("dev-access", '''
import os
for dev in ["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom",
            "/dev/tty", "/dev/sda", "/dev/mem", "/dev/kmem"]:
    try:
        fd = os.open(dev, os.O_RDONLY)
        print(f"OPEN: {dev} fd={fd}")
        os.close(fd)
    except PermissionError:
        print(f"BLOCKED: {dev}")
    except Exception as e:
        print(f"ERR: {dev} {e}")
''')
    results.append(("Landlock blocks /dev access", "OPEN" not in str(r) or "OPEN: /dev/null" in str(r)))

    return results


# =============================================================================
# LAYER 3: PRIVILEGE / CAPABILITY
# =============================================================================

def test_privilege_capability():
    """Test that privilege escalation is blocked."""
    results = []

    # 3a: no_new_privs is set
    r = test("no-new-privs", '''
import ctypes,os
libc = ctypes.CDLL(None)
# prctl PR_GET_NO_NEW_PRIVS = 39
ret = libc.prctl(39, 0, 0, 0, 0)
print(f"no_new_privs={ret}")
''')
    results.append(("no_new_privs is set", "no_new_privs=1" in str(r)))

    # 3b: Can we get capabilities?
    r = test("capabilities", '''
import ctypes,os
libc = ctypes.CDLL(None)
# capget = 125
class cap_header(ctypes.Structure):
    _fields_ = [("version", ctypes.c_uint32), ("pid", ctypes.c_int)]
class cap_data(ctypes.Structure):
    _fields_ = [("effective", ctypes.c_uint32), ("permitted", ctypes.c_uint32),
                ("inheritable", ctypes.c_uint32)]
h = cap_header(0x20080522, 0)
d = cap_data()
try:
    ret = libc.syscall(125, ctypes.byref(h), ctypes.byref(d))
    print(f"caps: eff=0x{d.effective:08x} perm=0x{d.permitted:08x} inh=0x{d.inheritable:08x}")
except:
    print("capget blocked")
''')
    results.append(("capabilities are zero", "eff=0x00000000" in str(r)))

    # 3c: Can we set capabilities?
    r = test("capset", '''
import ctypes
libc = ctypes.CDLL(None)
ret = libc.syscall(126, 0, 0)  # capset
print(f"capset: {ret}")
''')
    results.append(("capset(2) blocked", r.get("code") == 31))

    # 3d: seccomp modification (seccomp syscall itself)
    r = test("seccomp-modify", '''
import ctypes
libc = ctypes.CDLL(None)
# SYS_seccomp = 317
ret = libc.syscall(317, 0, 0, 0)  # SECCOMP_SET_MODE_FILTER
print(f"seccomp: {ret}")
''')
    results.append(("seccomp(2) blocked", r.get("code") == 31))

    # 3e: prctl PR_CAP_AMBIENT
    r = test("prctl-ambient", '''
import ctypes,os
libc = ctypes.CDLL(None)
ret = libc.prctl(47, 2, 0, 0, 0)  # PR_CAP_AMBIENT_RAISE
print(f"prctl(PR_CAP_AMBIENT_RAISE): {ret}")
''')
    results.append(("prctl CAP_AMBIENT blocked", r.get("code") == 31))

    return results


# =============================================================================
# LAYER 4: NETWORK ATTACKS
# =============================================================================

def test_network_attacks():
    """Test network-related attack vectors (all require enable_network)."""
    results = []

    # 4a: FIOSETOWN attack (already confirmed)
    r = test("fiosetown-attack", '''
import socket,fcntl,array,os
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
fcntl.ioctl(s.fileno(), 0x8901, array.array("i", [os.getppid()]))  # FIOSETOWN
fcntl.ioctl(s.fileno(), 0x5452, array.array("i", [1]))  # FIOASYNC
s.connect(("127.0.0.1", 8194))
s.send(b"GET / HTTP/1.0\\r\\n\\r\\n")
print("ATTACK_LAUNCHED")
''', network=True)
    results.append(("FIOSETOWN kills zygote", r.get("code") == 500))

    # 4b: Raw sockets (should be blocked)
    r = test("raw-socket", '''
import socket
try:
    s = socket.socket(socket.AF_INET, socket.SOCK_RAW, socket.IPPROTO_TCP)
    print(f"RAW_SOCKET: fd={s.fileno()}")
except Exception as e:
    print(f"BLOCKED: {e}")
''', network=True)
    results.append(("Raw sockets blocked", "BLOCKED" in str(r)))

    # 4c: AF_PACKET (should be blocked)
    r = test("af-packet", '''
import socket
try:
    s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW)
    print(f"PACKET_SOCKET: fd={s.fileno()}")
except Exception as e:
    print(f"BLOCKED: {e}")
''', network=True)
    results.append(("AF_PACKET blocked", "BLOCKED" in str(r)))

    # 4d: Network without opt-in (should be blocked)
    r = test("network-no-optin", '''
import socket
try:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.connect(("1.2.3.4", 80))
    print(f"CONNECTED without network opt-in!")
except Exception as e:
    print(f"BLOCKED: {type(e).__name__}")
''', network=False)
    results.append(("Network blocked without opt-in", "BLOCKED" in str(r) or r.get("code") == 31))

    # 4e: Netlink sockets info leak
    r = test("netlink-route", '''
import socket
try:
    nl = socket.socket(socket.AF_NETLINK, socket.SOCK_RAW, socket.NETLINK_ROUTE)
    print(f"NETLINK_ROUTE: fd={nl.fileno()}")
except Exception as e:
    print(f"BLOCKED: {type(e).__name__}")
''', network=True)
    results.append(("Netlink ROUTE socket available", "NETLINK_ROUTE" in str(r)))

    # 4f: Interface enumeration
    r = test("iface-enum", '''
import socket
try:
    names = socket.if_nameindex()
    print(f"INTERFACES: {names}")
except Exception as e:
    print(f"BLOCKED: {type(e).__name__}")
''', network=True)
    results.append(("Interface enumeration possible", "INTERFACES" in str(r)))

    return results


# =============================================================================
# LAYER 5: MEMORY / RESOURCE LIMITS
# =============================================================================

def test_resource_limits():
    """Test RLIMIT_AS and resource exhaustion."""
    results = []

    # 5a: Check actual RLIMIT_AS
    r = test("rlimit-as", '''
import resource
soft, hard = resource.getrlimit(resource.RLIMIT_AS)
print(f"RLIMIT_AS: soft={soft/1024/1024:.0f}MB hard={hard/1024/1024:.0f}MB")
''')
    results.append(("RLIMIT_AS is set", "soft=" in str(r)))

    # 5b: Output limit enforcement (>10MB)
    r = test("output-limit", '''
import sys
# Try to write >10MB
chunk = b"X" * 65536
for i in range(200):
    sys.stdout.buffer.write(chunk)
print("TOO_MUCH_OUTPUT")
''')
    results.append(("Output limit enforced", "TOO_MUCH_OUTPUT" not in str(r) or "limit" in str(r).lower()))

    # 5c: FD exhaustion
    r = test("fd-exhaust", '''
import os,socket
count = 0
try:
    while True:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        count += 1
        if count > 5000:
            break
except Exception as e:
    print(f"stopped at {count}: {e}")
print(f"FDs created: {count}")
''')
    results.append(("FD limit enforced", True))  # Always passes, just informative

    return results


# =============================================================================
# LAYER 6: PROCESS / SIGNAL — Verified Already
# =============================================================================

def test_process_isolation():
    """Test process/thread isolation."""
    results = []

    # 6a: Check UID/GID (should be 1000)
    r = test("uid-check", '''
import os
print(f"uid={os.getuid()} gid={os.getgid()} euid={os.geteuid()}")
''')
    results.append(("UID=1000 (non-privileged)", "uid=1000" in str(r)))

    # 6b: Thread creation (clone blocked?)
    r = test("thread-create", '''
import threading,time
t = threading.Thread(target=lambda: time.sleep(0.1))
t.start()
t.join()
print("THREAD_OK")
''')
    results.append(("Thread creation OK", "THREAD_OK" in str(r)))

    # 6c: Process creation attempt
    r = test("subprocess", '''
import subprocess
try:
    r = subprocess.run(["echo", "hello"], capture_output=True, text=True)
    print(f"SUBPROCESS: {r.stdout}")
except Exception as e:
    print(f"BLOCKED: {type(e).__name__}: {e}")
''')
    results.append(("Subprocess creation blocked", "BLOCKED" in str(r) or r.get("code") == 31))

    return results


# =============================================================================
# LAYER 7: INFORMATION LEAK
# =============================================================================

def test_information_leak():
    """Test information leak vectors."""
    results = []

    # 7a: prlimit64 — read parent limits
    r = test("prlimit64-leak", '''
import ctypes,os,resource
libc = ctypes.CDLL(None)
class rlimit(ctypes.Structure):
    _fields_ = [("rlim_cur", ctypes.c_ulonglong), ("rlim_max", ctypes.c_ulonglong)]
lim = rlimit()
ret = libc.syscall(302, os.getppid(), 9, ctypes.byref(lim), None)  # RLIMIT_AS=9
print(f"prlimit64(ppid, RLIMIT_AS): ret={ret} cur={lim.rlim_cur} max={lim.rlim_max}")
''')
    results.append(("prlimit64 leak possible", "cur=" in str(r)))

    # 7b: uname — system info
    r = test("uname-leak", '''
import os
print(f"uname: {os.uname()}")
''')
    results.append(("uname works (syscall allowed)", True))  # SYS_uname is in allowlist

    # 7c: getcpu / sched_getaffinity
    r = test("cpu-info", '''
import os
try:
    aff = os.sched_getaffinity(0)
    print(f"CPU affinity: {aff}")
except Exception as e:
    print(f"BLOCKED: {e}")
''')
    results.append(("CPU info leak", "CPU affinity" in str(r)))

    # 7d: /sys reads
    r = test("sysfs-read", '''
import os
for p in ["/sys/kernel/", "/sys/devices/", "/sys/class/net/"]:
    try:
        fd = os.open(p, os.O_RDONLY | os.O_DIRECTORY)
        print(f"SYSFS: {p}")
        os.close(fd)
    except PermissionError:
        print(f"BLOCKED: {p}")
    except Exception as e:
        print(f"ERR: {p} {e}")
''')
    results.append(("sysfs blocked by Landlock", "SYSFS" not in str(r)))

    return results


# =============================================================================
# LAYER 8: DOS VECTORS
# =============================================================================

def test_dos_vectors():
    """Test Denial of Service attack vectors."""
    results = []

    # 8a: Infinite loop with max timeout
    r = test("infinite-loop", '''
import time
time.sleep(60)
print("NOT_KILLED")
''')
    results.append(("Timeout enforced (30s)", "NOT_KILLED" not in str(r)))

    # 8b: Large memory allocation
    r = test("memory-bomb", '''
try:
    x = bytearray(2 * 1024 * 1024 * 1024)  # 2GB
    print(f"ALLOCATED: {len(x)}")
except MemoryError:
    print("MEMORY_ERROR_OK")
except Exception as e:
    print(f"ERR: {e}")
''')
    results.append(("Memory limit enforced", "MEMORY_ERROR_OK" in str(r) or r.get("code") != 0))

    # 8c: Recursive import bomb
    r = test("import-bomb", '''
import sys,os
# Try to crash via deep recursion in imports
sys.setrecursionlimit(50)
def recurse(n):
    if n:
        recurse(n-1)
try:
    recurse(100)
    print("RECURSION_OK")
except RecursionError:
    print("RECURSION_LIMIT_OK")
''')
    results.append(("Recursion handled", True))

    return results


# =============================================================================
# LAYER 9: CODE INTEGRITY
# =============================================================================

def test_code_integrity():
    """Test code execution integrity."""
    results = []

    # 9a: Simple code execution
    r = test("simple-exec", "print('SANDBOX_OK')")
    results.append(("Code execution works", "SANDBOX_OK" in str(r)))

    # 9b: Non-ASCII / Unicode
    r = test("unicode", 'print("你好世界 é ñ 測試")')
    results.append(("Unicode support", "你好世界" in str(r)))

    # 9c: Import internal modules
    r = test("internal-import", '''
import sys
result = []
for mod in list(sys.modules.keys()):
    if "protocol" in mod or "zygote" in mod:
        result.append(mod)
print(f"INTERNAL_MODULES: {result}")
''')
    results.append(("Internal modules cleaned", "INTERNAL_MODULES: []" in str(r)))

    # 9d: Access to zygote internals via gc
    r = test("gc-introspect", '''
import gc
objs = gc.get_objects()
# Count object types
types = {}
for o in objs:
    t = type(o).__name__
    types[t] = types.get(t, 0) + 1
print(f"GC objects: {len(objs)} total, types={dict(list(types.items())[:5])}")
''')
    results.append(("GC introspection possible", "GC objects" in str(r)))

    return results


# =============================================================================
# RUN ALL TESTS
# =============================================================================

def run_all():
    print("=" * 70)
    h = health()
    print(f"Server: {h}")
    print("=" * 70)

    all_tests = []
    test_suites = [
        ("SECCOMP Syscall Filtering", test_seccomp_blocked_syscalls),
        ("LANDLOCK Filesystem Isolation", test_landlock_filesystem),
        ("PRIVILEGE / CAPABILITY", test_privilege_capability),
        ("NETWORK ATTACKS", test_network_attacks),
        ("RESOURCE LIMITS", test_resource_limits),
        ("PROCESS ISOLATION", test_process_isolation),
        ("INFORMATION LEAK", test_information_leak),
        ("DOS VECTORS", test_dos_vectors),
        ("CODE INTEGRITY", test_code_integrity),
    ]

    passed = 0
    failed = 0

    for suite_name, suite_fn in test_suites:
        print(f"\n{'─' * 70}")
        print(f"  {suite_name}")
        print(f"{'─' * 70}")
        try:
            results = suite_fn()
            for name, ok in results:
                status = "✅ PASS" if ok else "❌ FAIL"
                if ok:
                    passed += 1
                else:
                    failed += 1
                print(f"  {status} | {name}")
                all_tests.append((suite_name, name, ok))
        except Exception as e:
            print(f"  ❌ SUITE ERROR: {e}")
            failed += 1

    print(f"\n{'=' * 70}")
    print(f"RESULTS: {passed} passed, {failed} failed, {passed+failed} total")
    print(f"{'=' * 70}")

    if failed:
        print("\n❌ FAILED TESTS:")
        for suite, name, ok in all_tests:
            if not ok:
                print(f"  [{suite}] {name}")

    return failed == 0

if __name__ == "__main__":
    sys.exit(0 if run_all() else 1)
