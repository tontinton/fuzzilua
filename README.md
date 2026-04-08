# fuzzilua

Fuzzilli-inspired fuzzer targeting GC lifecycle bugs in Lua VMs. Current target: Redis embedded Lua 5.1.

## Prerequisites

- Rust toolchain (stable)
- Clang (for sanitizer instrumentation)
- `cargo-nextest` (for tests)

## Build instrumented Redis

```bash
# Clones Redis 7.2.4 by default, patches + builds with ASan/UBSan/coverage.
./patches/build-redis.sh

# Or use an existing checkout:
BUILD_DIR=/path/to/parent REDIS_VERSION=8.6.2 ./patches/build-redis.sh
# Expects the redis source at $BUILD_DIR/redis
```

Binary lands at `target/redis/redis-server`.

| Variable | Default |
|---|---|
| `BUILD_DIR` | `/tmp/fuzzilua-redis-build` |
| `REDIS_VERSION` | `7.2.4` |
| `CC` | `clang` |
| `BITMAP_SIZE` | `65536` |
| `ENABLE_ALLOC_FAIL` | `1` |

## Build the fuzzer

```bash
cargo build --release
```

## Run

```bash
./target/release/fuzzilua-cli --redis-bin target/redis/redis-server

# More examples
./target/release/fuzzilua-cli --redis-bin target/redis/redis-server --jobs 8 --corpus my_corpus
./target/release/fuzzilua-cli --redis-bin target/redis/redis-server --alloc-fail-prob 0.05
./target/release/fuzzilua-cli --redis-bin target/redis/redis-server --reproduce crashes/crash_001.bin
./target/release/fuzzilua-cli --redis-bin target/redis/redis-server --minimize-crash crashes/crash_001.bin
```

### Options

| Flag | Default | Description |
|---|---|---|
| `--redis-bin` | -- | Path to instrumented `redis-server` |
| `--jobs` | CPU count | Worker threads |
| `--corpus` | `corpus` | Corpus directory |
| `--timeout` | `5s` | Per-execution timeout |
| `--max-iters` | inf | Stop after N iterations |
| `--generation-ratio` | `0.3` | Generated vs mutated program ratio |
| `--edge-size` | `65536` | Edge coverage bitmap size |
| `--gc-size` | `65536` | GC coverage bitmap size |
| `--no-minimize` | off | Skip corpus entry minimization |
| `--alloc-fail-prob` | off | Allocation failure injection probability (0.0-1.0) |
| `--stats-json` | -- | Append JSON stats per interval to file |
| `--stats-interval` | `5s` | Stats display interval |
| `--verbose` | off | Per-execution logging |

## Tests

```bash
export FUZZILUA_REDIS_BIN=target/redis/redis-server
cargo nextest run --workspace
```
