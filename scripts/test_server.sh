#!/usr/bin/env bash
set -euo pipefail
cd /Users/denerfernandes/ai/rustphp

pkill -9 -f "turbine serve" 2>/dev/null || true
sleep 1

export DYLD_LIBRARY_PATH="$(pwd)/vendor/php-embed/lib"

echo "=== Test 1: test-app ==="
RUST_LOG=warn ./target/release/turbine serve \
    --workers 1 --listen 127.0.0.1:9199 --root ./test-app \
    > /tmp/turb1.log 2>&1 &
PID=$!

for i in $(seq 1 20); do
    sleep 0.5
    if ! kill -0 $PID 2>/dev/null; then
        echo "  DIED at iter $i. Log:"; tail -5 /tmp/turb1.log; break
    fi
    CODE=$(curl -s -o /tmp/turb_resp.txt -w "%{http_code}" http://127.0.0.1:9199/ 2>/dev/null || echo "000")
    if [ "$CODE" != "000" ]; then
        echo "  HTTP $CODE at iter $i"
        head -2 /tmp/turb_resp.txt
        break
    fi
done
kill $PID 2>/dev/null; wait $PID 2>/dev/null || true
sleep 1

echo ""
echo "=== Test 2: laravel-test ==="
RUST_LOG=warn ./target/release/turbine serve \
    --workers 1 --listen 127.0.0.1:9199 --root ./laravel-test \
    > /tmp/turb2.log 2>&1 &
PID=$!

for i in $(seq 1 20); do
    sleep 0.5
    if ! kill -0 $PID 2>/dev/null; then
        echo "  DIED at iter $i. Log:"; tail -5 /tmp/turb2.log; break
    fi
    CODE=$(curl -s -o /tmp/turb_resp.txt -w "%{http_code}" http://127.0.0.1:9199/ 2>/dev/null || echo "000")
    if [ "$CODE" != "000" ]; then
        echo "  HTTP $CODE at iter $i"
        head -2 /tmp/turb_resp.txt
        break
    fi
done
kill $PID 2>/dev/null; wait $PID 2>/dev/null || true

echo ""
echo "=== DONE ==="
