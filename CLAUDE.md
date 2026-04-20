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
./target/release/pyaot input.py -o output -O              # All optimizations (devirtualize + flatten-properties + inline + constfold + dce)
./target/release/pyaot input.py -o output --inline --dce  # Individual passes: inlining + dead code elimination
./target/release/pyaot input.py -o output --constfold    # Individual pass: constant folding & propagation
./target/release/pyaot input.py -o output --devirtualize --flatten-properties  # Object model optimizations
./target/release/pyaot input.py -o output --debug  # DWARF debug info, symbols preserved, no optimizations
```

## Architecture

```
Python → AST → HIR → [generator desugaring] → MIR → Cranelift → Object → Executable
```

HIR is CFG-only: functions carry `blocks`, `entry_block`, and `try_scopes`,
with structured control flow represented by `HirTerminator` rather than
nested statement trees. Generator functions are desugared at HIR level
(before lowering) into regular functions using `GeneratorIntrinsic`
expressions. Detailed structure in `.claude/rules/architecture.md`. Key
APIs in `.claude/rules/api-reference.md`.

## Third-Party Packages

Optional packages live under `crates/pkg/<name>/` as standalone `staticlib+rlib` crates with their own dependencies. `pyaot-pkg-defs` registers them declaratively (feature-gated). The frontend records each `import <pkg>` into `hir::Module::used_packages`; the CLI resolves the set into `libpyaot_pkg_<name>.a` archives next to the runtime lib and passes them to the linker only when the source actually imports the package. See `.claude/rules/pkg-dev.md` for the authoring recipe.

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
