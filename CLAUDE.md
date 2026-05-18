# CLAUDE.md

Guidance for Claude Code when working with this repository.

## Project Overview

AOT compiler for a Python subset, implemented in Rust with Cranelift backend.

**Goals:** Native executables, static typing, CPython-compatible behavior, good error diagnostics
**Non-goals:** Full CPython compatibility, dynamic features (`eval/exec`, metaclasses), arbitrary precision integers (uses i64)

**References:** [COMPILER_STATUS.md](COMPILER_STATUS.md) (features), [CONVENTIONS.md](CONVENTIONS.md) (coding standards), [INSIGHTS.md](INSIGHTS.md) (non-obvious gotchas)

## Development Stage & Philosophy

**The compiler is at pre-alpha stage.** Architecture is the priority, not stability of any specific feature.

**Architectural soundness over tactical fixes.** When a bug exposes a design asymmetry, the right answer is almost always to fix the asymmetry — even if that requires a multi-day refactor touching dozens of files. Tactical workarounds (side-tables, ad-hoc passthroughs, special-case guards) compound into the kind of inconsistent IR that makes every subsequent pass harder. We have already paid this price once and won't pay it again.

**Break and rebuild as needed.** Backward compatibility is not a goal at this stage. If unifying two mechanisms requires invalidating callers, rewriting MIR opcodes, changing runtime ABI, or rethinking type semantics — do it. There are no external consumers; the only constraint is correctness on the test suite at the end of each landing.

**Operational rules:**
1. Diagnose the root cause before proposing a fix. If a fix introduces a new flag, side-table, or special-case path to compensate for an inconsistency elsewhere, that is a signal to refactor the inconsistency instead.
2. Prefer one large coherent change over many small incremental band-aids. Atomic landings can leave intermediate commits red — that's acceptable as long as the final state is green and the architecture is cleaner.
3. When two mechanisms exist for the same conceptual operation (e.g., `ValueFromInt` + `rt_box_float` + `emit_value_slot` doing related work), unify them into one before adding new functionality on top.
4. Document the architectural intent in plans / memory / `INSIGHTS.md` so future sessions don't re-derive the same decisions.

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

## Strong-Typed MIR Rewrite (in progress)

**Architectural axiom**: `pyaot_mir::MirType` determines physical
representation; the MIR Verifier rejects representation mismatch. HIR
continues using `pyaot_types::Type` (logical/structural). Translation
`Type → MirType` happens at the lowering boundary.

**MirType** (`crates/mir/src/types.rs`) — Level 2 representation system:
- `Raw(RawKind)` — primitive bits in slot (`I64`, `F64`, `I8`, `I32`)
- `Tagged` — tagged Value slot (any of `TAG_PTR`/`TAG_INT`/`TAG_BOOL`/`TAG_NONE`).
  MIR-layer discriminator for all tagged-Value slots (HIR uses `Type::Any`).
- `Heap(HeapShape)` — typed heap pointer (List/Dict/Tuple/Class/...)
- `FuncPtr(Signature)` — code address with typed signature
- `Closure(ClosureShape)` — closure tuple with sig + captures
- `Var(InternedString)` — TypeVar placeholder (pre-mono)
- `Never` — bottom type

**Verifier** (`crates/mir/src/verify.rs`) — checks each instruction's
operand/dest types match its typed signature. Per-stage mode via
`VerifyMirConfig` (Stage A.1): default policy is `HardError` at
`final-pre-codegen` in **both** debug and release builds (Stage G.1).
Granular overrides via `--verify-mir-stage=STAGE=MODE`. Legacy
`--verify-mir` shortcut enables Warn at every stage.

**Migration plan**: Strong-Typed MIR Rewrite v2, staged A-G. Full plan
at `.claude/plans/strong-typed-mir-v2-coordinated.md` (supersedes
`velvety-waddling-map.md`). `Local.ty: Type` (logical) and
`Local.mir_ty: Option<MirType>` (physical) are the **designed
dual-field final state** — see "Dual-Field Type System" in INSIGHTS.md.
`Option` cannot be removed from `mir_ty` without multi-session
prerequisite work (see `memory/feedback_mir_ty_option_removal_blocked.md`).
Stage F.2 deletes `Local.ty` only (blocked on C.2). Acceptance script
`scripts/assert_no_regression.sh` gates stage boundaries.

**Status** — 38/38 examples verifier-clean at `final-pre-codegen` in
both debug and release builds (HardError). Stage progression:

- **Stage A (verifier infrastructure)** — DONE
  - A.1 `VerifyMirConfig` per-stage hard-error toggle
  - A.2 comprehensive checks: CallNamed cross-module signature lookup,
    CallVirtual typed via B.1 vtable signatures, indirect Call FuncPtr
    arity, Refine src→dest cross-class detection
  - A.3 test corpus (24 verify-mod unit tests)
- **Stage B (pre-coordination)** — DONE
  - B.1 `Module::vtable_slot_signature` + `vtable_slot_func_id` helpers
    (walk single-inheritance chain via `class_info.base_class`)
  - B.2 `MirSemantic` enum (Raw/Tagged/Heap) + optional
    `mir_param_semantics` / `mir_return_semantic` fields on
    `RuntimeFuncDef`; helper constructors (`ptr_unary`, `ptr_binary`,
    `unary_to_i64` etc.) auto-populate Tagged semantics for ~150
    helper-built defs; verifier per-arg validation
  - B.3 pattern-capture documentation (already auto-typed via
    `alloc_and_add_local`)
  - B.4 deleted `rebox_tagged_any_copies` defensive sweep (subsumed by
    Phase 3a-* monomorph mir_ty syncs + box_fusion)
- **Stage C (codegen migration)** — DONE (C.1/C.3/C.4), C.2 deferred
  - C.1 `Local::computed_is_gc_root() -> bool` helper + codegen-only
    reader (no field union)
  - C.2 body-local mir_ty sync DEFERRED — even after C.3 codegen
    migration, UnboxValue verifier breaks because narrowing leaves
    redundant Tagged→Raw bridges. Needs an additional box_fusion
    sweep to drop them.
  - C.3 atomic codegen switchover LANDED across 3 sub-commits:
    declare_var + Phi + store_result guard (step 1), declare_function
    + define_function signatures (step 2), instruction handlers +
    terminator (step 3). 37/38 examples runtime-passing; the single
    failure (test_future_annotations) is pre-existing.
  - C.4 legacy `type_to_cranelift(&Type)` deleted; `mir_type_to_cranelift`
    is the sole Cranelift register-class mapper. Var/Never gracefully
    fall back to I64.
- **Stage D (reverse HeapAny migration)** — DONE
  - All producer sites migrated across 6 batches (~22 sites total).
    `Type::HeapAny` was fully eliminated — deleted in Stage F.1.
- **Stage E (compensation cleanup)** — MOSTLY DONE
  - E.1 Source-1 (closure dispatch) + Source-2 (post-devirt BoxValue)
    fixed; orphaned `RT_CALL_WITH_CAPTURES_AND_TAGGED_ARGS` deleted.
  - E.2 DEFERRED — rt_*_tagged HOF variants load-bearing.
  - E.3 AUDITED LOAD-BEARING — phase4_return_abi_flipped mechanism.
  - E.4 `flippable_method_funcs` + `flippable_methods.rs` deleted
    (-280 lines); `phase4_unsafe_funcs` confirmed load-bearing.
  - E.5 `FunctionKind` enum replaces string heuristics.
  - E.6 `abi_immutable` flag audited as load-bearing (6 sites).
- **Stage F (HeapAny + Local.ty deletion)** — F.1 DONE
  - F.1 LANDED (commit 21b05aa): `Type::HeapAny` enum variant deleted.
    All ~344 occurrences replaced with `Type::Any` + `mir_ty`.
    38/38 examples pass.
  - F.2 PARTIAL — 5 `local.ty.is_heap()` → `computed_is_gc_root()`
    migrations done; narrowing/propagation sites blocked on C.2.
    `Local.mir_ty: Option<MirType>` — Option removal BLOCKED (37/40
    runtime failures); requires C.2 codegen migration AND is_var_local
    flag OR MirType::Unknown sentinel first (see
    `memory/feedback_mir_ty_option_removal_blocked.md`).
- **Stages G** — G.1 + G.2 + G.3 DONE
  - G.1 LANDED: verifier `HardError` at `final-pre-codegen` now
    unconditional in both debug and release builds. 0 regressions.
  - G.2 WIDENING AUDIT: all 6 Phase-2 widenings confirmed load-bearing.
  - G.3 DONE: CLAUDE.md + MEMORY.md + INSIGHTS.md + COMPILER_STATUS.md
    updated; stale `Type::HeapAny` doc references cleaned up in
    INSIGHTS.md (historical sections updated to reflect F.1 deletion);
    `flippable_method_funcs` comment refs confirmed historical-only.

**Helpers** added during Stage A-D foundation:
- `Local::computed_is_gc_root()` — derive from MirType
- `Module::vtable_slot_signature(class_id, slot)` — B.1 typed lookup
- `MirSemantic::infer_param` / `infer_return` — B.2 fallback inference
- `RuntimeFuncDef::param_semantic(idx)` — B.2 helper with fallback

**Core type translation helpers**:
- `pyaot_mir::type_to_mir_type_storage(&Type)` — storage interpretation
  (primitives → `Tagged`)
- `pyaot_mir::type_to_mir_type_register(&Type)` — register
  interpretation (primitives → `Raw(K)`)
- `Local::resolved_mir_type()` — returns `mir_ty` if set, else
  translates `ty` at register level

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
