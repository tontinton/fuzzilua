#!/usr/bin/env bash
set -euo pipefail

# llm-seed.sh - Generate targeted Lua seeds via Claude CLI and inject into corpus
#
# Usage:
#   ./llm-seed.sh                    # generate + inject seeds
#   ./llm-seed.sh --generate-only    # just generate, don't inject
#   ./llm-seed.sh --inject-only      # inject existing seeds from llm-seeds/

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

SEED_DIR="${SEED_DIR:-llm-seeds}"
CORPUS_DIR="${CORPUS_DIR:-corpus}"
BATCH_SIZE="${BATCH_SIZE:-15}"

log() { echo "[$(date '+%H:%M:%S')] $*"; }

generate_seeds() {
    mkdir -p "$SEED_DIR"

    log "Generating $BATCH_SIZE targeted Lua seed scripts via Claude CLI..."

    local PROMPT
    PROMPT="$(cat <<'PROMPT_EOF'
You are generating seed Lua 5.1 scripts for a fuzzer targeting GC lifecycle bugs in Redis's embedded Lua VM.
The fuzzer instruments every allocation with GC phase tracking and forces GC at every allocation (GC stress mode).
It also injects random allocation failures (2% chance) to exercise error recovery paths.

Generate BATCH_SIZE separate Lua 5.1 scripts, each targeting a DIFFERENT dangerous GC interaction pattern.
Each script should be self-contained and executable via Redis EVAL (no require, no io, no os, no debug library).

TARGET BUG CLASSES (in order of CVE severity):
1. Use-after-free in GC sweep: object freed during sweep phase but still referenced via metatable chain
2. Type confusion: metamethod returns wrong type, accessed as original type after GC cycle
3. Double-free: allocation failure during error recovery in __gc finalizer
4. Buffer overflow: table rehash triggered during GC propagate phase
5. Stack corruption: coroutine yield inside __gc or __newindex during GC

SPECIFIC PATTERNS TO GENERATE (one script per pattern):
- __gc metamethod that resurrects a dead object by storing it in a global/upvalue table
- Weak table where values have __gc that reference the weak table itself
- Deep metatable chain (5+ levels) with __index triggering GC via table creation
- coroutine that yields inside a __newindex metamethod, then force GC before resume
- String interning pressure: create many identical strings, force GC, access interned refs
- Table with both array and hash parts at rehash boundary (exactly 8 entries), force GC during insert of 9th
- pcall + error inside __gc + collectgarbage("collect") in the error message tostring
- Nested metatables where __eq/__lt/__le triggers allocation that triggers GC
- Weak-keyed table with string keys that may be collected during iteration (pairs/next)
- __gc finalizer that calls collectgarbage("collect") recursively
- Table freezing pattern: setmetatable during __newindex while GC is in propagate
- Multiple coroutines sharing upvalues, one yields during GC of the other's stack
- loadstring inside __gc (allocation-heavy operation during finalization)
- rawset inside __newindex inside __gc chain
- setfenv on a function during its __gc finalizer, then call the function

Output ONLY the Lua scripts, separated by a line containing exactly "---SPLIT---".
No explanations, no markdown, no comments about what each script does.
Each script should be 10-40 lines of actual Lua code.
Use collectgarbage("collect") and collectgarbage("step") liberally to force GC at critical moments.
PROMPT_EOF
)"

    # Replace BATCH_SIZE in prompt
    PROMPT="${PROMPT//BATCH_SIZE/$BATCH_SIZE}"

    local RAW_OUTPUT
    RAW_OUTPUT="$(claude -p "$PROMPT" --output-format json 2>/dev/null)" || {
        log "ERROR: claude CLI failed. Is it installed and authenticated?"
        exit 1
    }

    # Extract the result text from JSON
    local TEXT
    TEXT="$(echo "$RAW_OUTPUT" | python3 -c "
import sys, json
data = json.load(sys.stdin)
# Handle both {result: ...} and raw string formats
if isinstance(data, dict):
    print(data.get('result', ''))
else:
    print(str(data))
" 2>/dev/null)" || {
        TEXT="$RAW_OUTPUT"
    }

    # Split on ---SPLIT--- and save each script
    local COUNT=0
    local CURRENT=""

    while IFS= read -r line; do
        if [[ "$line" == *"---SPLIT---"* ]]; then
            if [[ -n "${CURRENT// /}" ]]; then
                COUNT=$((COUNT + 1))
                local FNAME
                FNAME="$(printf "seed_%03d.lua" "$COUNT")"
                # Strip leading/trailing blank lines and markdown fences
                echo "$CURRENT" | sed '/^```/d' | sed '/^$/{ N; /^\n$/d; }' > "$SEED_DIR/$FNAME"
                log "  saved $FNAME ($(wc -l < "$SEED_DIR/$FNAME") lines)"
            fi
            CURRENT=""
        else
            CURRENT="${CURRENT}${line}
"
        fi
    done <<< "$TEXT"

    # Save the last script
    if [[ -n "${CURRENT// /}" ]]; then
        COUNT=$((COUNT + 1))
        local FNAME
        FNAME="$(printf "seed_%03d.lua" "$COUNT")"
        echo "$CURRENT" | sed '/^```/d' | sed '/^$/{ N; /^\n$/d; }' > "$SEED_DIR/$FNAME"
        log "  saved $FNAME ($(wc -l < "$SEED_DIR/$FNAME") lines)"
    fi

    log "Generated $COUNT seed scripts in $SEED_DIR/"
}

# --- Main ---
generate_seeds
log "Seeds written to $SEED_DIR/. Running fuzzer will hot-load them within 5 seconds."
