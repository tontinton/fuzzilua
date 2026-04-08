#!/usr/bin/env bash
set -euo pipefail

# Quick status check for night-run.sh
NIGHT_DIR="${NIGHT_DIR:-night}"
CORPUS_DIR="${CORPUS_DIR:-corpus}"
STATS_JSON="${STATS_JSON:-night-stats.jsonl}"

echo "=== Fuzzilua Night Run Status ==="
echo

# Fuzzer status
if [[ -f "$NIGHT_DIR/fuzzer.pid" ]] && kill -0 "$(cat "$NIGHT_DIR/fuzzer.pid")" 2>/dev/null; then
    echo "Fuzzer:   RUNNING (pid $(cat "$NIGHT_DIR/fuzzer.pid"))"
else
    echo "Fuzzer:   STOPPED"
fi

# Overseer status
if [[ -f "$NIGHT_DIR/overseer.pid" ]] && kill -0 "$(cat "$NIGHT_DIR/overseer.pid")" 2>/dev/null; then
    echo "Overseer: RUNNING (pid $(cat "$NIGHT_DIR/overseer.pid"))"
else
    echo "Overseer: STOPPED"
fi

echo

# Latest stats
if [[ -f "$STATS_JSON" ]]; then
    echo "--- Latest Stats ---"
    tail -1 "$STATS_JSON" | python3 -c "
import sys, json
d = json.load(sys.stdin)
h = int(d['elapsed_secs']) // 3600
m = (int(d['elapsed_secs']) % 3600) // 60
print(f\"  Elapsed:      {h}h {m}m\")
print(f\"  Executions:   {d['total_execs']}\")
print(f\"  Exec/sec:     {d['execs_per_sec']:.1f}\")
print(f\"  Corpus:       {d['corpus_size']}\")
print(f\"  Edge cov:     {d['edge_bits']}/{d['edge_total']} ({100*d['edge_bits']/d['edge_total']:.1f}%)\")
print(f\"  GC cov:       {d['gc_bits']}/{d['gc_total']} ({100*d['gc_bits']/d['gc_total']:.1f}%)\")
print(f\"  Crashes:      {d['crashes']} total, {d['unique_crashes']} unique\")
" 2>/dev/null || echo "  (could not parse stats)"
    echo
fi

# Disk usage
echo "--- Disk Usage ---"
du -sh "$CORPUS_DIR" "$NIGHT_DIR" "$STATS_JSON" 2>/dev/null || true
echo

# Crash files
CRASH_COUNT=$(find "$CORPUS_DIR/crashes" -name '*.txt' 2>/dev/null | wc -l)
echo "--- Crashes: $CRASH_COUNT unique ---"
if (( CRASH_COUNT > 0 )); then
    ls -lt "$CORPUS_DIR/crashes/"*.txt 2>/dev/null | head -10
fi
echo

# Overseer runs
RUN_COUNT=$(find "$NIGHT_DIR/runs" -name '*.md' 2>/dev/null | wc -l)
echo "--- Overseer Runs: $RUN_COUNT ---"
if (( RUN_COUNT > 0 )); then
    echo "Latest:"
    ls -t "$NIGHT_DIR/runs/"*.md 2>/dev/null | head -3 | while read -r f; do
        echo "  $(basename "$f")"
    done
fi
echo

# Memory file summary
if [[ -f "$NIGHT_DIR/memory.md" ]]; then
    echo "--- Memory (first 10 lines) ---"
    head -10 "$NIGHT_DIR/memory.md"
fi
