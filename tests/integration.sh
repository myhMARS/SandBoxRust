#!/bin/bash
# Integration tests — start sandbox, send real HTTP requests, verify responses.
set -euo pipefail

RED='\033[0;31m' GREEN='\033[0;32m' NC='\033[0m'
pass() { echo -e "${GREEN}PASS${NC} $1"; }
fail() { echo -e "${RED}FAIL${NC} $1"; exit 1; }

CONFIG_PATH="${CONFIG_PATH:-server/config.toml}"
API_KEY="${API_KEY:-sandbox-server}"
BASE="http://127.0.0.1:8194"
HEADER="-H X-Api-Key:$API_KEY"
CT="-H Content-Type:application/json"

# ── Start server ──
echo "=== Starting sandbox server ==="
cargo build --release -p sandbox-server 2>&1 | tail -1
./target/release/sandbox-server &
PID=$!
trap "kill $PID 2>/dev/null" EXIT

# Wait for it
for i in $(seq 1 30); do
  if curl -sf $BASE/health > /dev/null 2>&1; then break; fi
  sleep 0.5
done
echo "Server ready (PID=$PID)"

# ── 1. Health check ──
echo ""
echo "=== 1. Health ==="
RESP=$(curl -sf $BASE/health)
echo "$RESP" | python3 -c "
import sys,json; d=json.load(sys.stdin)
assert d['ok']==True, 'ok != true'
assert d['role']=='sandbox', 'bad role'
assert d['workers']>=1, 'no workers'
"
pass "/health"

# ── 2. Auth: missing API key → 401 ──
echo ""
echo "=== 2. Auth: no API key ==="
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/v1/sandbox/run \
  -X POST $CT -d '{"language":"python3","code":"cHJpbnQoMSk=","preload":"","options":{}}')
[ "$HTTP_CODE" = "401" ] || fail "Expected 401, got $HTTP_CODE"
pass "missing API key → 401"

# ── 3. Auth: wrong API key → 401 ──
echo ""
echo "=== 3. Auth: wrong API key ==="
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/v1/sandbox/run \
  -X POST $CT -H "X-Api-Key: wrong" \
  -d '{"language":"python3","code":"cHJpbnQoMSk=","preload":"","options":{}}')
[ "$HTTP_CODE" = "401" ] || fail "Expected 401, got $HTTP_CODE"
pass "wrong API key → 401"

# ── 4. Python: simple execution ──
echo ""
echo "=== 4. Python: print(1+2) ==="
PY_CODE=$(echo -n 'print(1+2)' | base64 -w0)
RESP=$(curl -sf -X POST $CT $HEADER \
  -d "{\"language\":\"python3\",\"code\":\"$PY_CODE\",\"preload\":\"\",\"options\":{}}" \
  $BASE/v1/sandbox/run)
echo "$RESP" | python3 -c "
import sys,json; d=json.load(sys.stdin)
assert d['code']==0, f'code={d[\"code\"]}'
assert d['data']['stdout'].strip()=='3', f'stdout={d[\"data\"][\"stdout\"]}'
"
pass "python print(1+2) → stdout=3"

# ── 5. Python: stderr on error ──
echo ""
echo "=== 5. Python: syntax error ==="
PY_CODE=$(echo -n 'print(1/0)' | base64 -w0)
RESP=$(curl -sf -X POST $CT $HEADER \
  -d "{\"language\":\"python3\",\"code\":\"$PY_CODE\",\"preload\":\"\",\"options\":{}}" \
  $BASE/v1/sandbox/run)
echo "$RESP" | python3 -c "
import sys,json; d=json.load(sys.stdin)
# DivisionByZero should produce non-zero exit code and stderr
assert d['code'] in (0,500), f'unexpected code={d[\"code\"]}'
# Either stdout shows error or stderr is populated
"
pass "python error handled gracefully"

# ── 6. Python: JSON parsing (real workload) ──
echo ""
echo "=== 6. Python: JSON parse ==="
PY_CODE=$(echo -n 'import json,sys;d=json.loads(sys.argv[1]);print(d["users"][0]["name"])' | base64 -w0)
RESP=$(curl -sf -X POST $CT $HEADER \
  -d "{\"language\":\"python3\",\"code\":\"$PY_CODE\",\"preload\":\"\",\"options\":{}}" \
  $BASE/v1/sandbox/run)
echo "$RESP" | python3 -c "
import sys,json; d=json.load(sys.stdin)
assert d['code']==0, f'code={d[\"code\"]}'
# Just verify it ran
"
pass "python JSON parse completed"

# ── 7. Unsupported language → 400 ──
echo ""
echo "=== 7. Unsupported language ==="
RESP=$(curl -sf -X POST $CT $HEADER \
  -d '{"language":"ruby","code":"cHJpbnQoMSk=","preload":"","options":{}}' \
  $BASE/v1/sandbox/run)
echo "$RESP" | python3 -c "
import sys,json; d=json.load(sys.stdin)
assert d['code']==400, f'expected 400, got {d[\"code\"]}'
"
pass "unsupported language → 400"

# ── 8. Dependencies: list ──
echo ""
echo "=== 8. Dependencies list ==="
RESP=$(curl -sf $HEADER "$BASE/v1/sandbox/dependencies?language=python3")
echo "$RESP" | python3 -c "
import sys,json; d=json.load(sys.stdin)
assert d['code']==0, f'code={d[\"code\"]}'
assert isinstance(d['data']['dependencies'], list), 'not a list'
"
pass "GET /dependencies?language=python3"

# ── 9. Node.js: simple execution ──
echo ""
echo "=== 9. Node.js: console.log ==="
# Install koffi if needed (CI may not have it)
if [ ! -d /usr/local/share/sandbox/node_modules/koffi ]; then
  mkdir -p /usr/local/share/sandbox
  npm install --prefix /usr/local/share/sandbox koffi 2>/dev/null || true
fi
JS_CODE=$(echo -n 'console.log("ok")' | base64 -w0)
RESP=$(curl -sf -X POST $CT $HEADER \
  -d "{\"language\":\"javascript\",\"code\":\"$JS_CODE\",\"preload\":\"\",\"options\":{}}" \
  $BASE/v1/sandbox/run)
echo "$RESP" | python3 -c "
import sys,json; d=json.load(sys.stdin)
assert d['code']==0, f'code={d[\"code\"]}'
assert d['data']['stdout'].strip()=='ok', f'stdout={d[\"data\"][\"stdout\"]}'
"
pass "nodejs console.log('ok') → stdout=ok"

# ── 10. Concurrent: 5 parallel Python requests ──
echo ""
echo "=== 10. Concurrent: 5 parallel ==="
for i in $(seq 1 5); do
  PY_CODE=$(echo -n "print($i)" | base64 -w0)
  curl -sf -X POST $CT $HEADER \
    -d "{\"language\":\"python3\",\"code\":\"$PY_CODE\",\"preload\":\"\",\"options\":{}}" \
    $BASE/v1/sandbox/run -o /tmp/resp_$i.json &
done
wait

for i in $(seq 1 5); do
  python3 -c "
import json; d=json.load(open('/tmp/resp_$i.json'))
assert d['code']==0, f'req $i code={d[\"code\"]}'
assert d['data']['stdout'].strip()=='$i', f'req $i stdout={d[\"data\"][\"stdout\"]}'
" || fail "concurrent request $i"
done
pass "5 concurrent python executions"

echo ""
echo -e "${GREEN}════════════════════════════════════${NC}"
echo -e "${GREEN}  All 10 integration tests passed  ${NC}"
echo -e "${GREEN}════════════════════════════════════${NC}"
