#!/usr/bin/env bash
# Tabulate perfbench runs into a comparison table.
#
#   ./tabulate.sh rust.log cpp.log
#
# Each input is the concatenated output from a full perfbench matrix —
# see run-matrix.sh. Expects 12 `=== MODE=... CHANNELS=... ===` sections
# per file (idle/echo/mix2 × 100/500/1000/2000). Sections that got
# killed before they finished (no `per channel:` line) are omitted.

set -u

rust="${1:-/tmp/rust.log}"
cpp="${2:-/tmp/cpp.log}"

[ -f "$rust" ] || { echo "missing: $rust"; exit 1; }
[ -f "$cpp" ] || { echo "missing: $cpp"; exit 1; }

# Extract one TSV row per complete section. Emit on `per channel:`
# which is the LAST metric printed per section — guarantees all
# earlier fields are populated before the row lands.
extract() {
  awk '
    /^===/ {
      mode = ""; chans = ""
      for (i = 1; i <= NF; i++) {
        if ($i ~ /^MODE=/) { split($i, a, "="); mode = a[2] }
        if ($i ~ /^CHANNELS=/) { split($i, a, "="); chans = a[2] }
      }
      cpu = ""; p99 = ""; drop = ""; peak = ""
      next
    }
    /cpu \/ wall:/ { cpu = $4 }
    /^  p99:/ { p99 = $2 }
    /drop rate:/ { drop = $3 }
    /^  peak:/ { peak = $2 }
    /per channel:/ {
      if (mode && chans) {
        printf "%s|%s|%s|%s|%s|%s|%s\n", mode, chans, $3, cpu, p99, drop, peak
      }
    }
  ' "$1"
}

r=$(mktemp); c=$(mktemp)
extract "$rust" > "$r"
extract "$cpp" > "$c"

printf "%-6s %-5s | %-11s | %-11s | %-10s | %-10s | %-9s | %-9s\n" \
  "Mode" "Chan" "Rust KiB/ch" "C++ KiB/ch" "Rust CPU" "C++ CPU" "Rust p99" "C++ p99"
printf "%-6s %-5s | %-11s | %-11s | %-10s | %-10s | %-9s | %-9s\n" \
  "------" "----" "-----------" "-----------" "---------" "---------" "--------" "--------"

# Join on "mode|chans" so missing rows on either side are visible.
join -t'|' -a1 -a2 -e "--" -o '0,1.2,2.2,1.3,2.3,1.4,2.4' \
  <(awk -F'|' '{printf "%s_%s|%s|%s|%s|%s\n", $1, $2, $3, $4, $5, $6}' "$r" | sort) \
  <(awk -F'|' '{printf "%s_%s|%s|%s|%s|%s\n", $1, $2, $3, $4, $5, $6}' "$c" | sort) \
  | while IFS='|' read -r key rper cper rcpu ccpu rp99 cp99; do
      mode="${key%_*}"; chans="${key#*_}"
      printf "%-6s %-5s | %-11s | %-11s | %-10s | %-10s | %-9s | %-9s\n" \
        "$mode" "$chans" "$rper" "$cper" "$rcpu" "$ccpu" "$rp99" "$cp99"
    done

rm -f "$r" "$c"
