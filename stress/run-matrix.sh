#!/usr/bin/env bash
# Run the perfbench matrix (idle/echo/mix2 × 100/500/1000/2000) against
# whichever projectrtp binary is currently at build/Release/projectrtp.node
# and emit the 12 sections to $1.
#
#   ./run-matrix.sh rust.log
#   ./run-matrix.sh cpp.log
#
# Set LD_LIBRARY_PATH in the caller's env if the binary needs it (the
# locally-built C++ does; Rust doesn't).
#
# Each perfbench run is wrapped in `timeout 12` so the C++ build's
# exit-hang (some worker thread doesn't release) doesn't stall the matrix.

set -u

out="${1:-/tmp/perfmatrix.log}"
: > "$out"

for mode in idle echo mix2; do
  for n in 100 500 1000 2000; do
    echo "=== MODE=$mode CHANNELS=$n ===" >> "$out"
    # C++ build's exit hang can take a few seconds to surface; send
    # SIGTERM after 20s to let perfbench finish cleanly, then SIGKILL
    # 3s later if it hasn't exited. The useful metrics are printed
    # long before either signal.
    timeout --kill-after=3 20 env \
      CHANNELS=$n DURATION_MS=5000 MODE=$mode \
      node "$(dirname "$0")/perfbench.js" >> "$out" 2>&1 || true
    # Small pause between scenarios so port reuse / fd cleanup settles.
    sleep 1
  done
done

echo "matrix done: $out ($(grep -c '^=== MODE' "$out") scenarios)"
