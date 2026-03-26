# CLAUDE.md

Guidance for Claude Code when working with this repository.

## Project Overview

AOT compiler for a Python subset, implemented in Rust with Cranelift backend.

**Goals:** Native executables, static typing, CPython-compatible behavior, good error diagnostics
**Non-goals:** Full CPython compatibility, dynamic features (`eval/exec`, metaclasses), arbitrary precision integers (uses i64)

**References:** [COMPILER_STATUS.md](COMPILER_STATUS.md) (features), [CONVENTIONS.md](CONVENTIONS.md) (coding standards), [INSIGHTS.md](INSIGHTS.md) (non-obvious gotchas)

## Key Principles

1. **CPython compatibility** — Compiled code must run identically in CPython
2. **Safety first** — `#![forbid(unsafe_code)]` in compiler crates (only `runtime` uses unsafe)
3. **Single Source of Truth** — Shared data in leaf crates (`core-defs`, `stdlib-defs`); never duplicate definitions
4. **Generic over specific** — Unified functions for multiple types
5. **No backward compatibility** — Freely refactor without compatibility shims
6. **Leverage Rust ecosystem** — Prefer Rust's std library and well-established crates over custom implementations

## Build & Run

```bash
cargo build --workspace --release      # Build all (release)
cargo build -p pyaot-runtime --release # Runtime library (required for linking)
cargo build -p pyaot-runtime --release --no-default-features  # Minimal runtime (no json/regex/crypto/network)
cargo test --workspace                 # Run all tests (including runtime integration tests)
cargo test -p pyaot --test runtime     # Run only runtime integration tests
cargo fmt && cargo clippy --workspace  # Format and lint

# Compile Python
./target/release/pyaot input.py -o output       # Compile
./target/release/pyaot input.py -o output --run # Compile and run
./target/release/pyaot input.py --emit-hir      # Debug: show HIR
./target/release/pyaot input.py --emit-mir      # Debug: show MIR
./target/release/pyaot input.py -o output --inline --dce  # Optimizations: inlining + dead code elimination
./target/release/pyaot input.py -o output --debug  # DWARF debug info, symbols preserved, no optimizations
```

## Architecture

```
Python → AST → HIR → MIR → Cranelift → Object → Executable
```

Detailed structure in `.claude/rules/architecture.md`. Key APIs in `.claude/rules/api-reference.md`.

## Error Handling Patterns

```rust
// Project-specific .expect() patterns:
var_map.get(&id).expect("internal error: local not in var_map");
Layout::from_size_align(...).expect("Allocation size overflow");
```

## Documentation

When implementing features:
1. Update `COMPILER_STATUS.md` — feature status
2. Update `README.md` — if significant user-facing changes
3. Add tests to appropriate existing file in `examples/`
4. Record non-obvious insights in `INSIGHTS.md`

## Dependencies

- Parsing: `rustpython-parser`
- Backend: `cranelift-codegen`, `cranelift-frontend`, `cranelift-module`, `cranelift-object`
- Debug info: `gimli` (DWARF generation), `object` (binary section manipulation)
- Data: `indexmap`, `hashbrown`, `smallvec`
