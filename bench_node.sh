#!/bin/bash
# Quick benchmark for node package only

set -e

ZB="./target/release/zb"

get_ms() {
    python3 -c 'import time; print(int(time.time() * 1000))'
}

echo "=== Node Benchmark ==="
echo ""

# Clean up first
$ZB uninstall node 2>/dev/null || true
rm -rf /opt/zerobrew/db /opt/zerobrew/cache /opt/zerobrew/store 2>/dev/null || true

# Cold install
echo "Cold install..."
COLD_START=$(get_ms)
$ZB install node >/dev/null 2>&1
COLD_END=$(get_ms)
COLD_MS=$((COLD_END - COLD_START))
echo "  Cold: ${COLD_MS}ms"

# Uninstall but keep cache
$ZB uninstall node >/dev/null 2>&1

# Warm install
echo "Warm install..."
WARM_START=$(get_ms)
$ZB install node >/dev/null 2>&1
WARM_END=$(get_ms)
WARM_MS=$((WARM_END - WARM_START))
echo "  Warm: ${WARM_MS}ms"

# Second warm install (everything cached)
$ZB uninstall node >/dev/null 2>&1
echo "Warm install (2nd)..."
WARM2_START=$(get_ms)
$ZB install node >/dev/null 2>&1
WARM2_END=$(get_ms)
WARM2_MS=$((WARM2_END - WARM2_START))
echo "  Warm (2nd): ${WARM2_MS}ms"

# Cleanup
$ZB uninstall node >/dev/null 2>&1 || true

echo ""
echo "Results:"
echo "  Cold:  ${COLD_MS}ms"
echo "  Warm:  ${WARM_MS}ms"
echo "  Warm2: ${WARM2_MS}ms"
