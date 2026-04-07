Fuzzilli-inspired Rust fuzzer for GC lifecycle bugs (UaF, heap-buffer-overflow, type confusion) in Lua VMs. Initial target: Redis embedded Lua 5.1 via `EVAL`. `Target` trait for future LuaJIT/nginx/Lua 5.4 support.

## Code guidelines

- No trivial comments
- Minimal bloat (KISS, DRY, SRP)
- No unnecessary state
- Follow Rust idioms, latest features OK
- Descriptive names, no wildcard imports
- Import types at top, use short names everywhere
- Consts at top of file, after imports
- `Result<T, E>` over panics; `color_eyre` for general errors, `thiserror` for domain errors
- Unit tests in same file (`#[cfg(test)]`), `#[test_case]`, snake_case, no tautologies
- Workspace deps in root `Cargo.toml`, `workspace=true` in crate
- Prefer existing deps; new ones OK when justified
- Mindful of allocations in hot paths
- Structured logging, helpful error messages
- No inline magic numbers/strings; const test messages, assert against shared constant
- `#[derive(Copy)]` only on structs with 1 primitive field
- Comments only for non-obvious stuff, Tone: warm, simple, concise, no em-dashes/emojis/AI tone

## Testing

- cargo clippy --all --tests -- -D warnings
- cargo nextest run --workspace

## Architecture

Rust workspace. Sync, thread-per-worker, no async.

```
cli (entry point, fuzzer loop, multi-worker)
├── gen (weighted code generators + program templates)
├── mutate (Input/Operation/Splice/CodeGen + GcInjection/MetamethodSwap/ChainDepth/TableSize/Interleave/LoadstringWrap/CallbackGc)
├── corpus (storage, scheduling: Uniform/Weighted/Focused, minimization)
├── target-redis (process mgmt, RESP client, shared-mem coverage, sandbox config)
├── target (Target trait, Execution, ExecStatus, CrashInfo, SandboxConfig)
├── ir (Op, Instruction, Program, Variable, LuaType, Lua lifter)
├── coverage (edge bitmap + GC-phase-aware bitmap: alloc_site x gc_phase x gc_debt)
└── patches/ (C patches: GC-stress allocator, edge coverage instrumentation)
```
