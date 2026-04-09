#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# night-run.sh - Launch fuzzer + periodic maki overseer
#
# Usage:
#   ./night-run.sh                  # start fuzzer only (no overseer)
#   ./night-run.sh --overseer       # start fuzzer + overseer loop
#   ./night-run.sh --overseer-only  # attach overseer to already-running fuzzer
#   ./night-run.sh --stop           # gracefully stop everything
#
# Fuzzer tuning rationale (targeting CVSS 9+ Lua GC bugs):
#
#   --jobs 10           Leave 2 cores free for Redis children + OS overhead.
#                       12 workers on 12 cores causes thrashing when each
#                       worker has its own Redis child process.
#
#   --generation-ratio 0.1
#                       Only 10% fresh generation, 90% mutation from corpus.
#                       The corpus is seeded; overnight we need exploitation
#                       depth, not breadth. Fresh programs rarely beat
#                       evolved corpus entries for coverage.
#
#   --timeout 3s        Most Lua scripts finish in <100ms. A 5s timeout
#                       wastes cycles on infinite loops. 3s catches complex
#                       GC chains while dropping obvious hangs faster.
#
#   --alloc-fail-prob 0.02
#                       2% chance of allocation failure in Lua's allocator.
#                       This exercises error recovery paths (luaD_throw on
#                       LUA_ERRMEM) which are prime UaF/double-free territory.
#                       Higher values cause too many benign OOM crashes.
#
#   --no-minimize       Minimization runs 3x stable_coverage + instruction
#                       removal per new-coverage input. At ~5ms/EVAL that's
#                       20-100ms per minimization. With 10 workers finding
#                       new coverage constantly in early phases, this is
#                       ~30% of total CPU. Disable for throughput; the
#                       overnight corpus will be compacted anyway.
#
#   --stats-interval 30s
#                       Low enough to track progress, high enough to not
#                       pollute the stats file (2880 lines over 24h).
#
# The GC stress patch (FUZZILUA_GC_STRESS_INTERVAL=1) forces GC threshold
# to totalbytes on every allocation, meaning collectgarbage() and allocation
# pressure hit the GC at maximum frequency. Combined with alloc-fail
# injection, this maximizes the chance of hitting GC-lifecycle bugs:
# use-after-free in sweep, type confusion in propagate, double-free on
# error recovery, buffer overflow during table rehash under memory pressure.
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# --- Config (override via env) ---
REDIS_BIN="${REDIS_BIN:-target/redis/redis-server}"
WORKERS="${WORKERS:-10}"
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
GENERATION_RATIO="${GENERATION_RATIO:-0.1}"
TIMEOUT="${TIMEOUT:-3s}"
ALLOC_FAIL_PROB="${ALLOC_FAIL_PROB:-0.02}"

# --- Helpers ---
log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"; }
die() { log "ERROR: $*" >&2; exit 1; }

cleanup() {
    log "Cleaning up..."
    if [[ -f "$OVERSEER_PID_FILE" ]]; then
        kill "$(cat "$OVERSEER_PID_FILE")" 2>/dev/null || true
        rm -f "$OVERSEER_PID_FILE"
    fi
    if [[ -f "$FUZZER_PID_FILE" ]]; then
        local pid
        pid="$(cat "$FUZZER_PID_FILE")"
        if kill -0 "$pid" 2>/dev/null; then
            log "Sending SIGINT to fuzzer (pid $pid)..."
            kill -INT "$pid" 2>/dev/null || true
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
OVERSEER_ENABLED=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --overseer-only) OVERSEER_ONLY=true; OVERSEER_ENABLED=true; shift ;;
        --overseer)      OVERSEER_ENABLED=true; shift ;;
        *)               echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

# --- Preflight checks ---
[[ -f "$REDIS_BIN" ]] || die "Redis binary not found: $REDIS_BIN (set REDIS_BIN env var)"
[[ "$OVERSEER_ENABLED" == true ]] && { command -v maki >/dev/null || die "maki not found in PATH"; }
command -v cargo >/dev/null || die "cargo not found in PATH"

mkdir -p "$NIGHT_DIR" "$RUNS_DIR" "$CORPUS_DIR"

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

    LLM_SEED_DIR="${LLM_SEED_DIR:-llm-seeds}"
    mkdir -p "$LLM_SEED_DIR"

    log "Starting fuzzer:"
    log "  workers=$WORKERS gen_ratio=$GENERATION_RATIO timeout=$TIMEOUT"
    log "  alloc_fail=$ALLOC_FAIL_PROB minimize=off seed_dir=$LLM_SEED_DIR"
    log "  corpus=$CORPUS_DIR stats=$STATS_JSON"

    cargo run --release -- \
        --redis-bin "$REDIS_BIN" \
        --jobs "$WORKERS" \
        --corpus "$CORPUS_DIR" \
        --stats-json "$STATS_JSON" \
        --stats-interval 30s \
        --generation-ratio "$GENERATION_RATIO" \
        --timeout "$TIMEOUT" \
        --alloc-fail-prob "$ALLOC_FAIL_PROB" \
        --no-minimize \
        --seed-dir "$LLM_SEED_DIR" \
        > "$FUZZER_LOG" 2>&1 &
    FUZZER_PID=$!
    echo "$FUZZER_PID" > "$FUZZER_PID_FILE"
    log "Fuzzer started (pid $FUZZER_PID), logging to $FUZZER_LOG"

    sleep 5
    if ! kill -0 "$FUZZER_PID" 2>/dev/null; then
        log "Fuzzer died immediately! Last output:"
        tail -20 "$FUZZER_LOG"
        die "Fuzzer failed to start"
    fi
fi

# --- Wait loop ---
trap cleanup EXIT INT TERM

START_TS=$(date +%s)
MAX_SECS=$((MAX_HOURS * 3600))

if [[ "$OVERSEER_ENABLED" == true ]]; then
echo $$ > "$OVERSEER_PID_FILE"
RUN_NUM=0

log "Overseer started. Checking every ${CHECK_INTERVAL}s for up to ${MAX_HOURS}h."
log "  Runs dir: $RUNS_DIR"
log "  Memory:   $MEMORY_FILE"

while true; do
    NOW_TS=$(date +%s)
    ELAPSED=$(( NOW_TS - START_TS ))
    if (( ELAPSED >= MAX_SECS )); then
        log "Max runtime (${MAX_HOURS}h) reached. Shutting down."
        break
    fi

    sleep "$CHECK_INTERVAL" &
    wait $! 2>/dev/null || true &
    wait $! 2>/dev/null || true

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

WHAT MAKES A REAL CVSS 9+ CRASH:
- ASan: heap-use-after-free, heap-buffer-overflow, stack-buffer-overflow with stack traces through lua* / luaC_* / luaV_* / luaH_* functions
- Signals: SIGSEGV, SIGABRT with lua stack frames (not Redis module frames)
- The bug must be triggerable via EVAL (unauthenticated in default Redis config = network-accessible = CVSS 9+)
- Double-free in GC sweep, type confusion in luaV_execute, buffer overflow in table rehash
- NOT: UBSan "incorrect function type" from RM_GetApi/VectorSets (known false positive)
- NOT: OOM crashes from alloc-fail injection (signal=0, no ASan report, just LUA_ERRMEM)
- NOT: Timeouts or connection lost

IMPORTANT RULES:
- Only READ files. Do NOT modify any fuzzer code or corpus entries.
- DO update ${MEMORY_FILE} with your analysis.
- The output of this run will be saved to ${RUN_FILE}.
- Be concise. Focus on actionable findings.

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

    log "Running maki overseer..."
    MAKI_OUTPUT="$(maki "$PROMPT" \
        --print \
        --output-format json \
        --yolo \
        --allowed-tools Read,Glob,Grep,Bash,Edit \
        2>/dev/null || echo '{"result": "maki failed to run", "is_error": true}')"

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

    if [[ "$FUZZER_ALIVE" == false ]]; then
        log "Fuzzer is no longer running. Exiting overseer."
        break
    fi
done

log "Night run finished after $RUN_NUM checks."
else
    # No overseer — just wait for fuzzer or max runtime
    log "Fuzzer running (no overseer). Max runtime: ${MAX_HOURS}h. Ctrl-C to stop."

    while true; do
        NOW_TS=$(date +%s)
        ELAPSED=$(( NOW_TS - START_TS ))
        if (( ELAPSED >= MAX_SECS )); then
            log "Max runtime (${MAX_HOURS}h) reached. Shutting down."
            break
        fi

        if [[ -f "$FUZZER_PID_FILE" ]] && ! kill -0 "$(cat "$FUZZER_PID_FILE")" 2>/dev/null; then
            log "Fuzzer exited."
            break
        fi

        sleep 30 &
        wait $! 2>/dev/null || true
    done
fi
