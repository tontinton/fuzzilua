#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# night-run.sh - Launch fuzzer + periodic maki overseer
#
# Usage:
#   ./night-run.sh                  # start fuzzer + overseer loop
#   ./night-run.sh --overseer-only  # attach overseer to already-running fuzzer
#   ./night-run.sh --stop           # gracefully stop everything
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# --- Config (override via env) ---
REDIS_BIN="${REDIS_BIN:-target/redis/redis-server}"
WORKERS="${WORKERS:-12}"
CORPUS_DIR="${CORPUS_DIR:-corpus}"
STATS_JSON="${STATS_JSON:-night-stats.jsonl}"
CHECK_INTERVAL="${CHECK_INTERVAL:-900}"   # 15 minutes
NIGHT_DIR="${NIGHT_DIR:-night}"
MEMORY_FILE="${NIGHT_DIR}/memory.md"
RUNS_DIR="${NIGHT_DIR}/runs"
FUZZER_LOG="${NIGHT_DIR}/fuzzer.log"
FUZZER_PID_FILE="${NIGHT_DIR}/fuzzer.pid"
OVERSEER_PID_FILE="${NIGHT_DIR}/overseer.pid"
MAX_HOURS="${MAX_HOURS:-10}"

# --- Helpers ---
log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"; }
die() { log "ERROR: $*" >&2; exit 1; }

cleanup() {
    log "Cleaning up..."
    # Stop overseer
    if [[ -f "$OVERSEER_PID_FILE" ]]; then
        kill "$(cat "$OVERSEER_PID_FILE")" 2>/dev/null || true
        rm -f "$OVERSEER_PID_FILE"
    fi
    # Gracefully stop fuzzer (SIGINT -> clean shutdown)
    if [[ -f "$FUZZER_PID_FILE" ]]; then
        local pid
        pid="$(cat "$FUZZER_PID_FILE")"
        if kill -0 "$pid" 2>/dev/null; then
            log "Sending SIGINT to fuzzer (pid $pid)..."
            kill -INT "$pid" 2>/dev/null || true
            # Wait up to 30s for graceful shutdown
            for _ in $(seq 1 30); do
                kill -0 "$pid" 2>/dev/null || break
                sleep 1
            done
            kill -0 "$pid" 2>/dev/null && kill -KILL "$pid" 2>/dev/null
        fi
        rm -f "$FUZZER_PID_FILE"
    fi
    log "Done."
}

stop_cmd() {
    if [[ ! -d "$NIGHT_DIR" ]]; then
        die "No night run directory found"
    fi
    cleanup
    exit 0
}

# --- Handle --stop ---
if [[ "${1:-}" == "--stop" ]]; then
    stop_cmd
fi

OVERSEER_ONLY=false
if [[ "${1:-}" == "--overseer-only" ]]; then
    OVERSEER_ONLY=true
fi

# --- Preflight checks ---
[[ -f "$REDIS_BIN" ]] || die "Redis binary not found: $REDIS_BIN (set REDIS_BIN env var)"
command -v maki >/dev/null || die "maki not found in PATH"
command -v cargo >/dev/null || die "cargo not found in PATH"

mkdir -p "$NIGHT_DIR" "$RUNS_DIR" "$CORPUS_DIR"

# Initialize memory file if it doesn't exist
if [[ ! -f "$MEMORY_FILE" ]]; then
    cat > "$MEMORY_FILE" << 'EOF'
# Fuzzilua Night Run - Overseer Memory

## Status
- Night run initialized, no checks yet.

## Crashes Found
(none yet)

## Observations
(none yet)

## Actions Taken
(none yet)
EOF
    log "Initialized $MEMORY_FILE"
fi

# --- Start fuzzer (unless --overseer-only) ---
if [[ "$OVERSEER_ONLY" == false ]]; then
    if [[ -f "$FUZZER_PID_FILE" ]] && kill -0 "$(cat "$FUZZER_PID_FILE")" 2>/dev/null; then
        die "Fuzzer already running (pid $(cat "$FUZZER_PID_FILE")). Use --overseer-only or --stop first."
    fi

    log "Starting fuzzer: $WORKERS workers, corpus=$CORPUS_DIR, stats=$STATS_JSON"
    cargo run --release -- \
        --redis-bin "$REDIS_BIN" \
        --jobs "$WORKERS" \
        --corpus "$CORPUS_DIR" \
        --stats-json "$STATS_JSON" \
        --stats-interval 30s \
        > "$FUZZER_LOG" 2>&1 &
    FUZZER_PID=$!
    echo "$FUZZER_PID" > "$FUZZER_PID_FILE"
    log "Fuzzer started (pid $FUZZER_PID), logging to $FUZZER_LOG"

    # Give it a few seconds to start up
    sleep 5
    if ! kill -0 "$FUZZER_PID" 2>/dev/null; then
        log "Fuzzer died immediately! Last output:"
        tail -20 "$FUZZER_LOG"
        die "Fuzzer failed to start"
    fi
fi

# --- Overseer loop ---
trap cleanup EXIT INT TERM

echo $$ > "$OVERSEER_PID_FILE"

START_TS=$(date +%s)
MAX_SECS=$((MAX_HOURS * 3600))
RUN_NUM=0

log "Overseer started. Checking every ${CHECK_INTERVAL}s for up to ${MAX_HOURS}h."
log "  Runs dir: $RUNS_DIR"
log "  Memory:   $MEMORY_FILE"

while true; do
    # Check if we've exceeded max runtime
    NOW_TS=$(date +%s)
    ELAPSED=$(( NOW_TS - START_TS ))
    if (( ELAPSED >= MAX_SECS )); then
        log "Max runtime (${MAX_HOURS}h) reached. Shutting down."
        break
    fi

    # Sleep until next check
    sleep "$CHECK_INTERVAL"

    # Check if fuzzer is still alive
    FUZZER_ALIVE=true
    if [[ -f "$FUZZER_PID_FILE" ]]; then
        if ! kill -0 "$(cat "$FUZZER_PID_FILE")" 2>/dev/null; then
            FUZZER_ALIVE=false
        fi
    fi

    RUN_NUM=$((RUN_NUM + 1))
    RUN_ID="$(date '+%Y%m%d_%H%M%S')_$(printf '%03d' $RUN_NUM)"
    RUN_FILE="$RUNS_DIR/${RUN_ID}.md"
    TIMESTAMP="$(date '+%Y-%m-%d %H:%M:%S')"

    log "=== Overseer check #${RUN_NUM} (${RUN_ID}) ==="

    # Gather context for maki
    STATS_TAIL=""
    if [[ -f "$STATS_JSON" ]]; then
        STATS_TAIL="$(tail -5 "$STATS_JSON" 2>/dev/null || echo '(no stats yet)')"
    fi

    FUZZER_TAIL=""
    if [[ -f "$FUZZER_LOG" ]]; then
        FUZZER_TAIL="$(tail -30 "$FUZZER_LOG" 2>/dev/null || echo '(no log yet)')"
    fi

    CRASH_FILES=""
    if [[ -d "$CORPUS_DIR/crashes" ]]; then
        CRASH_FILES="$(ls -lt "$CORPUS_DIR/crashes/" 2>/dev/null | head -30 || echo '(no crashes)')"
    fi

    DISK_USAGE="$(du -sh "$CORPUS_DIR" "$NIGHT_DIR" "$STATS_JSON" 2>/dev/null || echo 'unknown')"

    CORPUS_COUNT="$(find "$CORPUS_DIR" -maxdepth 1 -name 'entry_*.bin' 2>/dev/null | wc -l)"

    MEMORY_CONTENTS=""
    if [[ -f "$MEMORY_FILE" ]]; then
        MEMORY_CONTENTS="$(cat "$MEMORY_FILE")"
    fi

    # Build the prompt
    PROMPT="$(cat << PROMPT_EOF
You are the overnight overseer for fuzzilua, a Lua VM fuzzer targeting Redis's embedded Lua 5.1.
This is check #${RUN_NUM} at ${TIMESTAMP}. The fuzzer has been running for $((ELAPSED / 60)) minutes.
Fuzzer alive: ${FUZZER_ALIVE}

YOUR TASKS:
1. Analyze the fuzzer's health and progress.
2. Check for REAL crashes (not UBSan false positives from Redis VectorSets RM_GetApi).
3. For any new .txt crash reports in corpus/crashes/, read them and determine if they are real Lua GC bugs (UaF, heap-buffer-overflow, type confusion) or false positives.
4. If real crashes found, read the .lua reproducer and try to understand the bug. Record findings in the memory file.
5. Check disk usage - warn if corpus or stats file is growing too fast.
6. Update the memory file (${MEMORY_FILE}) with your findings, keeping a running log.

IMPORTANT RULES:
- Only READ files. Do NOT modify any fuzzer code or corpus entries.
- DO update ${MEMORY_FILE} with your analysis.
- The output of this run will be saved to ${RUN_FILE}.
- Be concise. Focus on actionable findings.
- UBSan "call through incorrect function type" from RM_GetApi in VectorSets is a KNOWN FALSE POSITIVE - skip these.
- Real crashes will have ASan reports (heap-use-after-free, heap-buffer-overflow, stack-buffer-overflow) or signals (SIGSEGV, SIGABRT with meaningful stack traces through lua* functions).

PERSISTENT MEMORY (from previous checks):
---
${MEMORY_CONTENTS}
---

RECENT STATS (last 5 lines of ${STATS_JSON}):
${STATS_TAIL}

FUZZER LOG (last 30 lines):
${FUZZER_TAIL}

CRASH FILES:
${CRASH_FILES}

DISK USAGE:
${DISK_USAGE}

CORPUS ENTRIES: ${CORPUS_COUNT}
PROMPT_EOF
)"

    # Run maki headless
    log "Running maki overseer..."
    MAKI_OUTPUT="$(maki "$PROMPT" \
        --print \
        --output-format json \
        --yolo \
        --allowed-tools Read,Glob,Grep,Bash,Edit \
        2>/dev/null || echo '{"result": "maki failed to run", "is_error": true}')"

    # Extract the result text
    RESULT_TEXT="$(echo "$MAKI_OUTPUT" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    print(data.get('result', '(no result)'))
except:
    print(sys.stdin.read() if hasattr(sys.stdin, 'read') else '(parse error)')
" 2>/dev/null || echo "$MAKI_OUTPUT")"

    COST="$(echo "$MAKI_OUTPUT" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    print(f\"\${data.get('total_cost_usd', 0):.4f}\")
except:
    print('unknown')
" 2>/dev/null || echo 'unknown')"

    # Save run report
    cat > "$RUN_FILE" << RUN_EOF
# Overseer Check #${RUN_NUM}
- **Time**: ${TIMESTAMP}
- **Elapsed**: $((ELAPSED / 60)) min
- **Fuzzer alive**: ${FUZZER_ALIVE}
- **Cost**: ${COST}

## Analysis
${RESULT_TEXT}
RUN_EOF

    log "Check #${RUN_NUM} complete. Cost: ${COST}. Saved to ${RUN_FILE}"

    # If fuzzer died, do one final check then exit
    if [[ "$FUZZER_ALIVE" == false ]]; then
        log "Fuzzer is no longer running. Exiting overseer."
        break
    fi
done

log "Night run finished after $RUN_NUM checks."
