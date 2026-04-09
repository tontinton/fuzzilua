#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# night-evolve.sh - Autonomous fuzzer evolution loop
#
# Every 30 minutes, spawns a headless Claude to:
#   1. Read the memory file + stats to understand current/past fuzzer state
#   2. Think of novel ideas to improve CVE-finding chances
#   3. Implement the change (code, seeds, config)
#   4. Rebuild & restart the fuzzer
#   5. Record what was done in the memory file
#
# Usage:
#   ./night-evolve.sh                # start fuzzer + evolution loop
#   ./night-evolve.sh --evolve-only  # attach to already-running fuzzer
#   ./night-evolve.sh --stop         # stop everything
#
# The fuzzer is restarted after each code change to pick up improvements.
# If a change breaks the build, it is reverted automatically.
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# --- Config (override via env) ---
REDIS_BIN="${REDIS_BIN:-target/redis/redis-server}"
WORKERS="${WORKERS:-10}"
CORPUS_DIR="${CORPUS_DIR:-corpus}"
STATS_JSON="${STATS_JSON:-night-stats.jsonl}"
EVOLVE_INTERVAL="${EVOLVE_INTERVAL:-1800}"  # 30 minutes
NIGHT_DIR="${NIGHT_DIR:-night}"
MEMORY_FILE="${NIGHT_DIR}/evolve-memory.md"
RUNS_DIR="${NIGHT_DIR}/evolve-runs"
FUZZER_LOG="${NIGHT_DIR}/fuzzer.log"
FUZZER_PID_FILE="${NIGHT_DIR}/fuzzer.pid"
EVOLVE_PID_FILE="${NIGHT_DIR}/evolve.pid"
MAX_HOURS="${MAX_HOURS:-10}"
GENERATION_RATIO="${GENERATION_RATIO:-0.1}"
TIMEOUT="${TIMEOUT:-3s}"
ALLOC_FAIL_PROB="${ALLOC_FAIL_PROB:-0.02}"
LLM_SEED_DIR="${LLM_SEED_DIR:-llm-seeds}"

# --- Helpers ---
log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"; }
die() { log "ERROR: $*" >&2; exit 1; }

start_fuzzer() {
    if [[ -f "$FUZZER_PID_FILE" ]] && kill -0 "$(cat "$FUZZER_PID_FILE")" 2>/dev/null; then
        log "Fuzzer already running (pid $(cat "$FUZZER_PID_FILE")), not starting another"
        return 0
    fi

    mkdir -p "$LLM_SEED_DIR"

    log "Starting fuzzer: workers=$WORKERS gen_ratio=$GENERATION_RATIO timeout=$TIMEOUT alloc_fail=$ALLOC_FAIL_PROB"

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
    local pid=$!
    echo "$pid" > "$FUZZER_PID_FILE"
    log "Fuzzer started (pid $pid)"

    sleep 5
    if ! kill -0 "$pid" 2>/dev/null; then
        log "Fuzzer died immediately! Last output:"
        tail -20 "$FUZZER_LOG"
        return 1
    fi
    return 0
}

stop_fuzzer() {
    if [[ ! -f "$FUZZER_PID_FILE" ]]; then
        return 0
    fi
    local pid
    pid="$(cat "$FUZZER_PID_FILE")"
    if kill -0 "$pid" 2>/dev/null; then
        log "Stopping fuzzer (pid $pid)..."
        kill -INT "$pid" 2>/dev/null || true
        for _ in $(seq 1 30); do
            kill -0 "$pid" 2>/dev/null || break
            sleep 1
        done
        kill -0 "$pid" 2>/dev/null && kill -KILL "$pid" 2>/dev/null
    fi
    rm -f "$FUZZER_PID_FILE"
}

cleanup() {
    log "Cleaning up..."
    if [[ -f "$EVOLVE_PID_FILE" ]]; then
        kill "$(cat "$EVOLVE_PID_FILE")" 2>/dev/null || true
        rm -f "$EVOLVE_PID_FILE"
    fi
    stop_fuzzer
    log "Done."
}

# --- Handle --stop ---
if [[ "${1:-}" == "--stop" ]]; then
    cleanup
    exit 0
fi

EVOLVE_ONLY=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --evolve-only) EVOLVE_ONLY=true; shift ;;
        --stop)        cleanup; exit 0 ;;
        *)             echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

# --- Preflight ---
[[ -f "$REDIS_BIN" ]] || die "Redis binary not found: $REDIS_BIN"
command -v claude >/dev/null || die "claude CLI not found in PATH"
command -v cargo >/dev/null || die "cargo not found in PATH"

mkdir -p "$NIGHT_DIR" "$RUNS_DIR" "$CORPUS_DIR" "$LLM_SEED_DIR"

# --- Initialize memory file ---
if [[ ! -f "$MEMORY_FILE" ]]; then
    cat > "$MEMORY_FILE" << 'EOF'
# Fuzzilua Evolution Memory

## Goal
Find CVSS 9+ GC lifecycle bugs (UaF, heap-buffer-overflow, type confusion, double-free)
in Redis's embedded Lua 5.1 VM, triggerable via EVAL.

## Run History
(no evolution cycles yet)

## Ideas Tried
(none yet)

## Ideas To Try
- Novel GC stress patterns not yet covered by existing mutators
- New metamethod interaction patterns
- Coroutine + GC interleaving strategies
- Memory pressure patterns that trigger rehash during GC
- String interning edge cases
- Environment manipulation during finalization
- Nested pcall/error chains with GC triggers
- Weak table resurrection patterns

## Crashes Found
(none yet)

## Current Fuzzer Config
- workers=10, generation_ratio=0.1, timeout=3s, alloc_fail_prob=0.02
- no-minimize, gc-stress mode, seed-dir hot-loading

## Key Insights
(none yet)
EOF
    log "Initialized $MEMORY_FILE"
fi

# --- Start fuzzer (unless --evolve-only) ---
if [[ "$EVOLVE_ONLY" == false ]]; then
    start_fuzzer || die "Failed to start fuzzer"
fi

# --- Evolution loop ---
trap cleanup EXIT INT TERM
echo $$ > "$EVOLVE_PID_FILE"

START_TS=$(date +%s)
MAX_SECS=$((MAX_HOURS * 3600))
RUN_NUM=0

log "Evolution loop started. Interval: ${EVOLVE_INTERVAL}s, max: ${MAX_HOURS}h."

while true; do
    NOW_TS=$(date +%s)
    ELAPSED=$(( NOW_TS - START_TS ))
    if (( ELAPSED >= MAX_SECS )); then
        log "Max runtime (${MAX_HOURS}h) reached."
        break
    fi

    # Sleep for the interval
    sleep "$EVOLVE_INTERVAL" &
    wait $! 2>/dev/null || true

    RUN_NUM=$((RUN_NUM + 1))
    RUN_ID="$(date '+%Y%m%d_%H%M%S')_$(printf '%03d' $RUN_NUM)"
    RUN_FILE="$RUNS_DIR/${RUN_ID}.md"
    TIMESTAMP="$(date '+%Y-%m-%d %H:%M:%S')"

    log "=== Evolution cycle #${RUN_NUM} (${RUN_ID}) ==="

    # Gather context
    STATS_TAIL=""
    [[ -f "$STATS_JSON" ]] && STATS_TAIL="$(tail -10 "$STATS_JSON" 2>/dev/null || echo '(no stats)')"

    FUZZER_TAIL=""
    [[ -f "$FUZZER_LOG" ]] && FUZZER_TAIL="$(tail -40 "$FUZZER_LOG" 2>/dev/null || echo '(no log)')"

    CRASH_INFO=""
    if [[ -d "$CORPUS_DIR/crashes" ]]; then
        CRASH_COUNT=$(find "$CORPUS_DIR/crashes" -name '*.txt' 2>/dev/null | wc -l)
        CRASH_INFO="Total crash reports: $CRASH_COUNT"
        if (( CRASH_COUNT > 0 )); then
            CRASH_INFO="$CRASH_INFO
Recent crash files:
$(ls -lt "$CORPUS_DIR/crashes/"*.txt 2>/dev/null | head -10 || true)"
        fi
    fi

    CORPUS_COUNT="$(find "$CORPUS_DIR" -maxdepth 1 -name 'entry_*.bin' 2>/dev/null | wc -l)"
    DISK_USAGE="$(du -sh "$CORPUS_DIR" "$NIGHT_DIR" 2>/dev/null || echo 'unknown')"

    MEMORY_CONTENTS=""
    [[ -f "$MEMORY_FILE" ]] && MEMORY_CONTENTS="$(cat "$MEMORY_FILE")"

    # Recent git log to see what was changed
    GIT_LOG="$(git log --oneline -10 2>/dev/null || echo '(no git)')"

    PROMPT="$(cat << PROMPT_EOF
You are the autonomous evolution engine for fuzzilua, a Lua 5.1 GC fuzzer targeting Redis.
This is evolution cycle #${RUN_NUM} at ${TIMESTAMP}. Fuzzer running for $((ELAPSED / 60)) minutes.

YOUR MISSION: Find CVSS 9+ GC lifecycle bugs (UaF, heap-buffer-overflow, type confusion,
double-free) in Redis's embedded Lua 5.1 VM. Make ONE improvement per cycle that maximizes
the chance of triggering a real crash.

Read the memory below to understand what's been tried. Read the stats to see if coverage
is growing or plateauing. Then do whatever you think is most likely to help.

You have full freedom. Some ideas, but don't limit yourself to these:
- Write new seed scripts to llm-seeds/ (hot-loaded by running fuzzer within 5s)
- Add/modify mutators, generators, templates, IR ops, coverage tracking
- Tune fuzzer parameters (generation_ratio, alloc_fail_prob, timeout, etc.)
- Read the actual Redis Lua source at ~/c/redis/src/lua* and ~/c/redis/deps/lua/src/
  to find specific code patterns that look vulnerable, then target them
- Study past CVEs in Lua 5.1 GC and craft inputs that hit similar paths
- Anything else you can think of

You can explore the full fuzzilua codebase and the Redis source freely. Read AGENTS.md
for code guidelines before making Rust changes. If you modify Rust code, verify it compiles
(cargo build --release). If it doesn't compile, fix it or revert (git checkout -- <file>).
After code changes, the harness will rebuild and restart the fuzzer automatically.

Do NOT touch corpus/ files, crash data, night-evolve.sh, or night-run.sh.
Commit your changes with a short descriptive message.
Update ${MEMORY_FILE} with what you did, why, and what you observed.

PERSISTENT MEMORY (from previous cycles):
---
${MEMORY_CONTENTS}
---

RECENT STATS (last 10 lines of ${STATS_JSON}):
${STATS_TAIL}

FUZZER LOG (last 40 lines):
${FUZZER_TAIL}

CRASH INFO:
${CRASH_INFO}

CORPUS ENTRIES: ${CORPUS_COUNT}
DISK USAGE:
${DISK_USAGE}

RECENT GIT HISTORY:
${GIT_LOG}
PROMPT_EOF
)"

    log "Running Claude evolution cycle..."
    CLAUDE_OUTPUT="$(claude -p "$PROMPT" \
        --output-format json \
        --max-turns 30 \
        --dangerously-skip-permissions \
        2>/dev/null || echo '{"result": "claude failed", "is_error": true}')"

    RESULT_TEXT="$(echo "$CLAUDE_OUTPUT" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    print(data.get('result', '(no result)'))
except:
    print('(parse error)')
" 2>/dev/null || echo "$CLAUDE_OUTPUT")"

    COST="$(echo "$CLAUDE_OUTPUT" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    print(f\"\${data.get('total_cost_usd', 0):.4f}\")
except:
    print('unknown')
" 2>/dev/null || echo 'unknown')"

    # Save run report
    cat > "$RUN_FILE" << RUN_EOF
# Evolution Cycle #${RUN_NUM}
- **Time**: ${TIMESTAMP}
- **Elapsed**: $((ELAPSED / 60)) min
- **Cost**: \$${COST}
- **Corpus**: ${CORPUS_COUNT} entries

## Claude Output
${RESULT_TEXT}
RUN_EOF

    log "Cycle #${RUN_NUM} complete. Cost: \$${COST}. Saved to ${RUN_FILE}"

    # Check if code was modified (not just seeds) - if so, rebuild and restart fuzzer
    if git diff --quiet HEAD 2>/dev/null; then
        # Check for new commits (Claude may have committed)
        COMMITTED_CHANGES="$(git log --oneline -1 --since='10 minutes ago' 2>/dev/null || true)"
        if [[ -n "$COMMITTED_CHANGES" ]]; then
            log "New commit detected: $COMMITTED_CHANGES"
            log "Rebuilding and restarting fuzzer..."
            if cargo build --release 2>&1 | tail -5; then
                stop_fuzzer
                sleep 2
                start_fuzzer || log "WARNING: Failed to restart fuzzer after code change"
            else
                log "WARNING: Build failed after evolution cycle. Reverting last commit..."
                git revert --no-edit HEAD 2>/dev/null || true
                cargo build --release 2>/dev/null || true
                start_fuzzer || log "WARNING: Failed to restart fuzzer after revert"
            fi
        else
            log "No code changes this cycle (seeds only or analysis only)"
        fi
    else
        # Unstaged changes exist - try to build
        log "Unstaged code changes detected. Building..."
        if cargo build --release 2>&1 | tail -5; then
            git add -A && git commit -m "evolve cycle #${RUN_NUM}: auto-improvement" 2>/dev/null || true
            stop_fuzzer
            sleep 2
            start_fuzzer || log "WARNING: Failed to restart fuzzer"
        else
            log "WARNING: Build failed. Reverting unstaged changes..."
            git checkout -- . 2>/dev/null || true
            start_fuzzer || log "WARNING: Failed to restart fuzzer after revert"
        fi
    fi

    # Check if fuzzer is still alive
    if [[ -f "$FUZZER_PID_FILE" ]] && ! kill -0 "$(cat "$FUZZER_PID_FILE")" 2>/dev/null; then
        log "Fuzzer died. Attempting restart..."
        rm -f "$FUZZER_PID_FILE"
        start_fuzzer || log "WARNING: Failed to restart dead fuzzer"
    fi
done

log "Evolution loop finished after $RUN_NUM cycles."
