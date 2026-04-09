#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BITMAP_SIZE="${BITMAP_SIZE:-65536}"
ENABLE_ALLOC_FAIL="${ENABLE_ALLOC_FAIL:-1}"
REDIS_DIR=""
INSTALL_DIR="${INSTALL_DIR:-$SCRIPT_DIR/../target/redis}"

usage() {
    cat <<EOF
Build an instrumented Redis for fuzzilua.

Usage: $0 [OPTIONS] <redis-dir>

  <redis-dir>     Path to an existing Redis source checkout.

Options:
  --install-dir DIR   Where to copy the built binary (default: target/redis/)
  --bitmap-size N     Shared-memory bitmap size (default: 65536)
  --no-alloc-fail     Disable allocation-failure injection
  -h, --help          Show this help

Examples:
  $0 ~/c/redis
  $0 --install-dir /opt/fuzzilua ~/c/redis
EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --install-dir)  INSTALL_DIR="$2"; shift 2 ;;
        --bitmap-size)  BITMAP_SIZE="$2"; shift 2 ;;
        --no-alloc-fail) ENABLE_ALLOC_FAIL=0; shift ;;
        -h|--help)      usage ;;
        -*)             echo "Error: unknown option: $1" >&2; exit 1 ;;
        *)              REDIS_DIR="$1"; shift ;;
    esac
done

if [ -z "$REDIS_DIR" ]; then
    echo "Error: redis source directory required. Run '$0 --help' for usage." >&2
    exit 1
fi

REDIS_DIR="$(cd "$REDIS_DIR" && pwd)"
if [ ! -f "$REDIS_DIR/src/Makefile" ]; then
    echo "Error: $REDIS_DIR doesn't look like a Redis source tree." >&2
    exit 1
fi

command -v clang  >/dev/null || { echo "Error: clang not found"; exit 1; }
command -v clang++ >/dev/null || { echo "Error: clang++ not found"; exit 1; }

# clang++ needs gcc's C++ headers; find the install dir automatically
GCC_LIB_DIR="$(gcc -print-search-dirs 2>/dev/null | sed -n 's/^install: //p' || true)"
GCC_FLAG=""
if [ -n "$GCC_LIB_DIR" ] && [ -d "$GCC_LIB_DIR" ]; then
    GCC_FLAG="--gcc-install-dir=$GCC_LIB_DIR"
fi

CFLAGS="-fsanitize=address -fno-sanitize-recover=address -fsanitize-coverage=trace-pc-guard"
CFLAGS="$CFLAGS -DFUZZILUA_GC_STRESS -DFUZZILUA_GC_STRESS_INTERVAL=32 -DFUZZILUA_GC_STRESS_MODE=1"
CFLAGS="$CFLAGS -DFUZZILUA_GC_ASSERT"
CFLAGS="$CFLAGS -DFUZZILUA_BITMAP_SIZE=${BITMAP_SIZE}"
[ "$ENABLE_ALLOC_FAIL" = "1" ] && CFLAGS="$CFLAGS -DFUZZILUA_ALLOC_FAIL"
CFLAGS="$CFLAGS -g -O1 -fno-omit-frame-pointer"
[ -n "$GCC_FLAG" ] && CFLAGS="$CFLAGS $GCC_FLAG"

LDFLAGS="-fsanitize=address -lrt"
[ -n "$GCC_LIB_DIR" ] && LDFLAGS="$LDFLAGS -L$GCC_LIB_DIR"

echo "=== fuzzilua: building instrumented Redis ==="
echo "  Source:  $REDIS_DIR"
echo "  Bitmap:  $BITMAP_SIZE"
echo "  Install: $INSTALL_DIR"

cd "$REDIS_DIR"

echo "--- Patching ---"
git checkout -- . 2>/dev/null || true
patch -p1 < "$SCRIPT_DIR/redis-lua51-combined.patch"
cp "$SCRIPT_DIR/fuzzilua_coverage.c" src/fuzzilua_coverage.c

echo "--- Building ---"
make distclean 2>/dev/null || true
make -j"$(nproc)" \
    CC=clang CXX=clang++ \
    OPTIMIZATION="" \
    CFLAGS="$CFLAGS" \
    LDFLAGS="$LDFLAGS" \
    MALLOC=libc \
    BUILD_TLS=no \
    redis-server

echo "--- Installing ---"
mkdir -p "$INSTALL_DIR"
cp src/redis-server "$INSTALL_DIR/redis-server"

echo ""
echo "=== Done: $INSTALL_DIR/redis-server ==="
echo "  cargo run --release -- --redis-bin $INSTALL_DIR/redis-server"
