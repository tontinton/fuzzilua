#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REDIS_VERSION="${REDIS_VERSION:-7.2.4}"
REDIS_REPO="${REDIS_REPO:-https://github.com/redis/redis.git}"
BUILD_DIR="${BUILD_DIR:-/tmp/fuzzilua-redis-build}"
INSTALL_DIR="${INSTALL_DIR:-$SCRIPT_DIR/../target/redis}"

CC="${CC:-clang}"

# Single source of truth for bitmap size. Must match DEFAULT_BITMAP_SIZE in fuzzilua-coverage.
BITMAP_SIZE="${BITMAP_SIZE:-65536}"

CFLAGS="${CFLAGS:--fsanitize=address,undefined -fsanitize-coverage=trace-pc-guard -DFUZZILUA_GC_STRESS -DFUZZILUA_GC_STRESS_INTERVAL=1 -DFUZZILUA_GC_STRESS_MODE=1 -DFUZZILUA_BITMAP_SIZE=${BITMAP_SIZE} -g -O1 -fno-omit-frame-pointer}"
LDFLAGS="${LDFLAGS:--fsanitize=address,undefined -lrt}"

echo "=== fuzzilua: building instrumented Redis ==="
echo "  Redis version: $REDIS_VERSION"
echo "  CC: $CC"
echo "  BITMAP_SIZE: $BITMAP_SIZE"
echo "  CFLAGS: $CFLAGS"
echo "  Build dir: $BUILD_DIR"
echo "  Install dir: $INSTALL_DIR"

if [ ! -d "$BUILD_DIR/redis" ]; then
    echo "--- Cloning Redis $REDIS_VERSION ---"
    mkdir -p "$BUILD_DIR"
    git clone --depth 1 --branch "$REDIS_VERSION" "$REDIS_REPO" "$BUILD_DIR/redis"
else
    echo "--- Using existing Redis checkout at $BUILD_DIR/redis ---"
fi

cd "$BUILD_DIR/redis"

echo "--- Applying GC stress patch ---"
git checkout -- . 2>/dev/null || true
patch -p1 < "$SCRIPT_DIR/redis-lua51-gc-stress.patch"

echo "--- Adding edge coverage instrumentation ---"
cp "$SCRIPT_DIR/fuzzilua_coverage.c" src/fuzzilua_coverage.c

echo "--- Building Redis ---"
make distclean 2>/dev/null || true
make -j"$(nproc)" \
    CC="$CC" \
    OPTIMIZATION="" \
    CFLAGS="$CFLAGS" \
    LDFLAGS="$LDFLAGS" \
    MALLOC=libc \
    BUILD_TLS=no \
    redis-server

echo "--- Installing ---"
mkdir -p "$INSTALL_DIR"
cp src/redis-server "$INSTALL_DIR/redis-server"
cp src/redis-cli "$INSTALL_DIR/redis-cli" 2>/dev/null || true

echo ""
echo "=== Build complete ==="
echo "  Binary: $INSTALL_DIR/redis-server"
echo ""
echo "  Test with:"
echo "    export FUZZILUA_REDIS_BIN=$INSTALL_DIR/redis-server"
echo "    cargo nextest run --workspace"
