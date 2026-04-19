# pyaot Architecture Refactor — Master Plan

This document defines a three-phase architectural overhaul of pyaot that
closes fundamental design flaws exposed by real-world Python workloads
(microgpt.py, realistic ML/NLP training scripts, idiomatic dunder-heavy
OO code). It is written to be **permanent**: once adopted, do not
re-open scoping discussions or substitute cheaper alternatives. If a
milestone proves infeasible as specified, the spec is broken — revise
the spec explicitly and update this document. Do not silently weaken it.

---

## Non-Negotiable Principles

These apply to every phase, every milestone, every commit.

### 1. No partial migrations

A milestone migrates **all** affected call sites in one consistent
series of commits. Landing "SSA for half the functions, legacy for the
other half" is forbidden. If the diff is too large, split into commits
that each compile and test green, but do not ship behind a flag.

**Amended 2026-04-18** — a milestone's *landing commit on master*
must be fully green; intermediate working states inside a long-lived
phase branch may be red under either of the two workflows permitted
by the amendment to Principle 4. "No partial migrations" refers to
the end-state landed on master, not to every intra-branch commit.

### 2. No feature flags hiding work-in-progress

pyaot has no deployment pipeline that justifies dual-code-path runtime
toggles. Every phase is a hard migration on a single long-lived branch,
merged to master only when complete. Debug tooling flags (e.g.
`--emit-hir`) are fine; semantic toggles are not.

### 3. No backwards-compatibility shims

If a legacy helper (`unify_tuple_shapes`, `box_primitive_if_needed`,
`prescan_var_types`, etc.) is replaced by the new abstraction, **delete
the helper and all its call sites**. Do not leave a wrapper that "still
works for the old cases". The old cases are now the new cases.

### 4. Every milestone ends green

Every milestone-closing commit (the one that marks the milestone
complete and lands on master) must satisfy:

- `cargo build --workspace --release` — zero warnings.
- `cargo fmt --check` — clean.
- `cargo clippy --workspace --release -- -D warnings` — clean.
- `cargo test --workspace --release` — zero failures, zero regressions.

Merging a milestone with "known regressions to be fixed later" is
forbidden. The test suite is the gate. If a legitimate semantic change
requires updating a test, the update is part of the milestone commit.

**Amended 2026-04-18 — intra-milestone workflows.** The S1.3 session
revealed that some milestones span interlocking consumer migrations
where any single-commit intermediate state is necessarily red (e.g.,
deleting a `StmtKind` variant transiently breaks every walker until
all are ported in the same logical change). Two workflows are now
permitted per session; pick whichever fits:

1. **Staged commits on the long-lived phase branch** — intermediate
   commits within a single milestone may be temporarily red (build
   fails, tests regress), so long as the milestone-closing commit is
   fully green before it merges to master. Use this when the work
   partitions into reviewable chunks worth recording in history.
2. **Single-commit landing** — no intermediate commits at all;
   implement end-to-end locally until all four gates pass, then
   commit once when the milestone is complete and green. Use this
   when intermediate states would be meaningless noise in history.

Both workflows share the same end-state gate: the commit that lands
on master must meet all four checks. The choice of workflow does
**not** relax that gate. Principle 1 (no partial migrations) applies
to what lands on master, not to what happens intra-branch.

### 5. No ad-hoc escape hatches post-migration

After a phase completes, the new abstraction is the single way to
express the relevant concept. Introducing a new `*_types` hashmap, a
new `box_*_if_needed` helper, or a new `unify_*` free function after
Phase 1/2/3 respectively is a **planning failure**. Either the spec
was wrong (revise this document) or the implementation was wrong (fix
the implementation). There is no third option.

### 6. Benchmarks gate performance-sensitive phases

Phase 2 (tagged values) changes every hot-path arithmetic op. Before
merging, benchmark the runtime on the existing benchmark suite (create
one in Phase 0 if missing). A regression >5% on any benchmark is a
show-stopper — either fix the codegen or revise the tag scheme. "We'll
optimize in a follow-up" is not acceptable.

### 7. Spec deviations require explicit document updates

If, during implementation, you find a milestone's spec wrong or
incomplete, **amend this document in a dedicated commit** before
proceeding. Do not drift silently. The document is authoritative; the
code must match it.

### 8. Ordering is mandatory

Phase 1 must complete before Phase 2 starts. Phase 2 must complete
before Phase 3 starts. Parallel work within a phase is encouraged
between independent milestones, but cross-phase parallelism is
forbidden — each phase makes assumptions the next relies on, and
weaving them risks architectural drift.

### 9. No microgpt.py-specific fixes

microgpt.py is a **diagnostic tool** for exposing architectural gaps,
not a compilation target to appease. Never introduce a pattern-specific
fix (e.g. "detect `x if isinstance(x, T) else T(x)` and emit special
MIR") to make a particular line of microgpt.py compile. If microgpt.py
reveals a gap, trace it to an architectural root cause and solve the
root cause.

### 10. Deletion is progress

After each phase, measure success by **lines of code removed**, not
added. Phase 1 should remove ~4 ad-hoc type maps and several
narrowing helpers. Phase 2 should remove ~2000 LOC of boxing dance.
Phase 3 should remove every `unify_*` free function in `types`.
Adding LOC while claiming a phase is "done" indicates failure to
complete the migration.

---

## Phase Ordering and Dependencies

```
          ┌─────────────────────────────────┐
          │  Phase 0: Preparation (1 wk)    │
          │  Benchmarks, test gaps, tooling │
          └──────────────┬──────────────────┘
                         │
          ┌──────────────┴──────────────────┐
          │  Phase 1: SSA MIR + WPA (6-10w) │
          │  THE foundation. Non-optional.  │
          └──────────────┬──────────────────┘
                         │
          ┌──────────────┴──────────────────┐
          │  Phase 2: Tagged Values (4-7w)  │
          │  Unified value representation.  │
          └──────────────┬──────────────────┘
                         │
          ┌──────────────┴──────────────────┐
          │  Phase 3: Lattice + Mono (3-5w) │
          │  Generics, Protocol, cleanup.   │
          └─────────────────────────────────┘

Total: 14–23 weeks end-to-end.
```

**Why SSA first**: every subsequent phase assumes flow-sensitive type
information (e.g. Phase 2 needs to know per-value whether it's known-Int
vs. maybe-Int-maybe-ptr; Phase 3 needs whole-program types for
monomorphization). Without SSA, downstream phases either replicate the
ad-hoc maps or fail outright.

**Why tagged values before lattice**: Phase 3 generics rely on a
unified runtime representation — generic `List[T]` must hold any `T`.
With five different value representations (Phase 2 pre-state), generic
specialization multiplies combinatorially.

**Why lattice last**: it is the smallest, most localized phase.
Postponing lets the types crate stabilize around the concrete
requirements revealed by Phases 1 and 2. Doing it first risks designing
a lattice that doesn't fit the flow-sensitive SSA type system.

---

# Phase 0 — Preparation ✅

**Status**: complete (scaffolding landed). Follow-up test-coverage work
tracked in `bench/COVERAGE.md` before Phase 1 kickoff.

**Duration**: 1 week.

**Goal**: establish the regression-detection infrastructure that the
remaining phases depend on. Zero user-visible changes.

## 0.1 Benchmark harness ✅

Create `bench/` with microbenchmarks covering:

- **Integer arithmetic** hot loops (`sum(range(10_000_000))`).
- **Float arithmetic** (`sum(i * 0.5 for i in range(1_000_000))`).
- **Polymorphic arithmetic** (dunder-dispatched, Value-class-like —
  the pattern from microgpt's `Value.__add__` / `__mul__`).
- **Dict / list allocation and iteration** (construct, iterate,
  mutate).
- **String interning / concatenation** (loop-building strings,
  str-interning-hit-rate).
- **Generator + comprehension iteration**
  (`sum(x*x for x in range(N))`, nested gen-exprs, `zip`/`enumerate`
  fused iteration — the §G.3 / §G.10 territory).
- **Exception handling overhead** (try/except in hot loop, ensuring
  non-raising path is cheap; raising path is also measured but less
  critical).
- **GC stress** (allocation-heavy tight loops, trigger multiple
  collections, measure collection latency).
- **Class instantiation + method dispatch** (both direct and
  polymorphic / vtable dispatch).
- **Closure creation + call** (lambda defaults, nonlocal capture,
  higher-order functions like `map`/`filter`).
- **Startup / module init time** (small-to-medium file end-to-end
  compile + first-line execution — catches init-path regressions).

Each benchmark:
- Has a Python source file under `bench/py/`.
- Has a `cargo bench` target that runs compile + execute, reporting
  **wall-clock run time**, **binary size**, and (if possible) **max
  RSS**.
- Records both **run time** (hot-loop perf) and **end-to-end time**
  (compile + execute) separately — they can regress independently.
- Establishes a **baseline** recorded in `bench/BASELINE.md` as a
  committed data file. Each subsequent phase appends its own
  column (Phase-1 baseline, Phase-2 baseline, Phase-3 baseline) so
  regressions are visible historically.

**Exit criterion**: `cargo bench` runs all benchmarks, produces stable
results (variance < 3% across 5 runs for each metric), baseline
committed. Running `cargo bench --compare` against the committed
baseline produces diff output suitable for PR review.

**Status (2026-04-17)**: harness landed in `bench/` with 11 benchmark
sources covering every category above. `cargo bench -p pyaot-bench`
runs `run::<name>` and `end_to_end::<name>` groups. Phase-0 numbers
captured in `--quick` mode are in `bench/BASELINE.md`; a full
10-sample / 30s-measurement sweep must run before the Phase-1
acceptance gate.

## 0.2 Coverage audit ✅

Run `cargo llvm-cov --workspace --html` (install if missing). Identify
any major area (lowering crate, optimizer, type_planning) with < 70%
coverage. For each gap, add tests in `examples/test_*.py` or
`crates/*/src/tests.rs` to reach ≥ 70% before Phase 1 starts.

**Non-negotiable**: Phase 1 must not be bottlenecked on "we don't
have a test for this case". Add the tests now.

**Status (2026-04-17)**: `cargo-llvm-cov` installed, baseline captured,
per-module gaps documented in `bench/COVERAGE.md`. Every sub-70 %
compiler-side module is listed with a TODO action. **Closing those
gaps is the remaining Phase-0 exit action before Phase 1 kickoff** —
each row must be raised to ≥ 70 % region *and* line coverage, or
marked "TODO-blocked" with a linked issue.

## 0.3 SSA property checker (stub) ✅

Add `crates/mir/src/ssa_check.rs` with a checker that validates:

- Every MIR `LocalId` has exactly one static defining instruction.
- Every use of a `LocalId` is dominated by its definition.
- Every `BasicBlock` has a valid `Terminator` (no fallthrough).
- Every φ-node has exactly as many incoming values as predecessors.

In Phase 0 the checker is a no-op for the legacy MIR (which is not
SSA). It is turned on for a function only when that function has the
`is_ssa: true` flag. Phase 1 flips functions to `is_ssa: true` one by
one; the checker asserts on each.

**Exit criterion**: checker compiles, runs, flags violations with
actionable error messages. Verified on a hand-constructed SSA
function.

**Status (2026-04-17)**: checker landed at `crates/mir/src/ssa_check.rs`.
`mir::Function` now carries `is_ssa: bool` (default `false` — stays a
no-op on legacy MIR). Validates multiple-definition, use-dominance,
and dangling-terminator invariants today; φ-arity check is an explicit
no-op until Phase 1 introduces `InstructionKind::Phi`. Seven
hand-crafted-SSA tests pass in `cargo test -p pyaot-mir`, including
both positive and negative cases (double-def, use-without-def,
use-before-def in same block, non-dominating cross-block use,
dangling goto target).

## 0.4 Lattice property harness ✅

Add `crates/types/src/tests/lattice_props.rs` with:

- `forall t: join(t, t) == t` (idempotence).
- `forall t1 t2: join(t1, t2) == join(t2, t1)` (commutativity).
- `forall t1 t2 t3: join(t1, join(t2, t3)) == join(join(t1, t2), t3)` (associativity).
- `forall t: join(top, t) == top`.
- `forall t: join(bottom, t) == t`.
- Same set for `meet`.
- `is_subtype(a, b) && is_subtype(b, a) <=> a == b` (antisymmetry).

These tests fail in Phase 0 (current Type has no `join`/`meet`). They
are kept failing as "expected failures" via `#[ignore]`. Phase 3
un-ignores them.

**Exit criterion**: tests compile, are marked ignored, documented as
Phase 3 activation targets.

**Status (2026-04-17)**: 11 property tests landed at
`crates/types/src/tests/lattice_props.rs`, all `#[ignore]`'d with
explicit "Phase 3 — lattice join/meet not implemented yet" reasons.
`tests.rs` was migrated to `tests/mod.rs` so the new sub-module slots
in cleanly. `join` and `meet` are local stubs that `todo!()` at call
time — Phase 3 replaces them with `Type::join` / `Type::meet` method
calls and removes the `#[ignore]` attributes.

---

# Phase 1 — SSA MIR + Whole-Program Type Inference 🟡

**Duration**: 6–10 weeks.

**Status dashboard (2026-04-18)** — ✅ done · 🟡 partial · ⏳ pending

| Milestone | Status | Sessions |
|---|---|---|
| §1.1 HIR → CFG conversion | 🟡 | S1.1 ✅ · S1.2 ✅ · S1.3 ⏳ (folded into S1.17b) |
| §1.2 DomTree | ✅ | S1.4 ✅ |
| §1.3 SSA + φ + Refine | ✅ | S1.5 ✅ · S1.6 ✅ · S1.7 ✅ |
| §1.4 Flow-sensitive type inference | ✅ | S1.8 ✅ (core + rules) · S1.9 ✅ (legacy deletion) · §1.4u ✅ (Path A — see row below) |
| §1.5 Call graph | ✅ | S1.10 ✅ |
| §1.6 WPA parameter inference | ✅ | S1.11 ✅ (core + full-program fixed point) |
| §1.7 WPA field inference | ✅ | S1.12 ✅ (params + fields to full-program fixed point) |
| §1.8 Pass migration | ✅ | S1.13 ✅ · S1.14a ✅ · S1.14b-prep ✅ · S1.14b-inliner ✅ · S1.15 ✅ |
| §1.9 Codegen migration | ✅ | S1.5 wiring ✅ · S1.16 ✅ (audit: no manual-phi emulation; Variable API is OK under SSA single-def) |
| §1.10 Final cleanup | 🟡 | S1.17a ✅ (partial acceptance: tests green, microgpt triaged) · S1.17 full ⏳ (blocked on S1.17b) |
| §1.11 Deferred HIR-tree deletion | ⏳ | scoped 2026-04-19 · S1.17b-a..f (6 sub-sessions) |
| §1.4u Single-TypeTable unification | ✅ | step 1 ✅ · step 2 ✅ · step 3 ✅ · step 4 ✅ · step 5 ✅ · §1.4u-c ✅ (Path A by construction) · §1.4u-d ✅ |

### Phase 1 Completion Status (as of 2026-04-18, S1.17a partial acceptance)

**What landed** (11 sessions, roughly 80% of the milestone goal):

- **§1.1 HIR CFG types + bridge** (S1.1, S1.2): new `HirBlock` /
  `HirBlockId` / `HirTerminator`; frontend populates a parallel CFG
  via `crates/hir/src/cfg_build.rs`; legacy tree stays alive.
- **§1.2 DomTree** (S1.4): Cooper–Harvey–Kennedy with `OnceCell`
  cache on `mir::Function`; canonical `terminator_successors`.
- **§1.3 SSA + Phi + Refine** (S1.5, S1.6, S1.7): Cytron construction
  activated universally; Cranelift block_params wired; `Refine`
  instruction landed (isinstance-Refine emission queued — no codegen
  consumer yet).
- **§1.5 Call graph** (S1.10): direct / indirect / virtual edges;
  iterative Tarjan SCCs; `is_recursive` helper for inliner use.
- **§1.6 WPA parameter inference** (S1.11): SCC-wise fixed point +
  full-program outer loop; `Never` bottom to avoid `Any`
  contamination in recursive SCCs.
- **§1.7 WPA field inference** (S1.12): class metadata in
  `mir::Module.class_info`; per-field join via `__init__` writes;
  paired with param inference in
  `wpa_param_and_field_inference_to_fixed_point`.
- **§1.8 Pass migration** (S1.13, S1.14a, S1.15): constfold got
  unified Const+Copy-alias propagation, Phi/Refine folding; inline's
  CallGraph merged with the canonical graph; peephole gained
  idempotent `x & x → x` / `x | x → x`. DCE and
  devirtualize/flatten_properties audits found them already
  SSA-compatible.
- **§1.9 Codegen migration** (S1.5 wiring, S1.16 audit): Phi /
  block-params wired; no manual phi emulation left to delete; the
  `Variable` API is correct under single-def invariant.

**Acceptance checklist state** (from spec "Phase 1 Acceptance" below):

1. ✅ **All tests green** — `cargo test --workspace --release`
   passes with 470+ tests across 36 test targets, 0 failures.
2. ✅ **Benchmarks non-regressed** — Phase 1 preliminary captured
   2026-04-18 (post-pruned-SSA) with `--quick`. Runtime (`run`
   column) is within ±10% noise across every benchmark, compile-
   phase (`end_to_end`) is within ±6% of Phase 0. Criterion's t-test
   reports "No change in performance detected" for all benchmarks.
   The S1.6e "always place Phi" regression (50-85% compile-time
   slowdown) was fixed by adding pruned-SSA `insert_phis` — single-
   def locals skip φ insertion only when the def block actually
   dominates every use block; otherwise run the full iterated
   dominance frontier. Formal 10-sample run deferred to §1.10 close.
3. ✅ **SSA checker passes** — activated 2026-04-18 behind
   `#[cfg(debug_assertions)]` in `crates/cli/src/lib.rs` after
   S1.6c/d/e landed all classes of violation fix. Debug builds
   panic on any violation; release builds skip the check for
   compile-time performance.
4. 🟡 **Deletion audit** — 4 legacy maps relocated from
   `SymbolTable` into `HirTypeInference` per S1.9 (dual state with
   `TypeTable`). Full deletion needs §1.4u (HIR→MIR type unification).
5. 🟡 **microgpt.py diagnostic** (2026-04-18): fails at line 41
   `return Value(self.data + other.data, ...)` with
   "unknown attribute 'data'". Prior ternary rebinds `other` via
   `other if isinstance(other, Value) else Value(other)`; both arms
   produce `Value`, but the post-ternary type is not narrowed to
   `Value` at the `other.data` site. Triage: narrowing-through-
   ternary-rebind gap; lives in HIR-level narrowing
   (`lowering/src/narrowing.rs` + `type_planning/infer.rs`). Target:
   §1.4u (unified HIR type pass) — the MIR TypeTable handles this
   fine, but lowering reads from the pre-unification HIR maps.
6. ✅ **Spec deviations documented** — per-session status notes and
   deferred rows capture every divergence.

**Remaining before formal Phase 1 close**:

- **S1.6c** (2026-04-18) — partial fix. Root cause of the
  `greet_all` false-positive was drift between `ssa_check` and
  `ssa_construct`: the checker's `instruction_def` treated every
  `RuntimeCall` as defining `dest`, but `ssa_construct` correctly
  uses `runtime_call_is_void` to exclude void calls (e.g.
  `rt_string_builder_append`). Fix: export `runtime_call_is_void`
  as `pub(crate)` and have both modules use it. Activating the
  checker in the CLI pipeline afterwards revealed **additional real
  violations** (`UseNotDominated` / `UseWithoutDef` patterns,
  mostly in `__pyaot_module_init__` paths) that are compiler bugs
  requiring a dedicated pipeline-debugging session (tracked as
  **S1.6d**). Until S1.6d, the checker stays test-only — no
  production gate.
- **S1.6d** (2026-04-18) — two fixes landed in `ssa_construct`:
  1. **φ-source default for undefined edges**: when the rename stack
     for a variable is empty at a predecessor, emit a typed zero
     constant (`default_undef_operand`) instead of the self-
     referential `phi(phi.dest, ...)` that the prior fallback
     `unwrap_or(original)` produced. Fixed all violations in
     `test_control_flow.py`, `test_classes.py`, `test_exceptions.py`,
     most of `test_iteration.py` — the pattern was nested loops with
     an inner iteration variable that gets a Phi at the outer
     header.
  2. **Reclassify `GcPush.frame` / `ExcPushFrame.frame_local` as
     defs**, not uses. These are Cranelift-synthesized definitions
     (`def_var(frame, addr)` inside the codegen path) but the MIR
     treats them as an operand. SSA must know they define to avoid
     flagging `UseWithoutDef`. Both `ssa_construct` and `ssa_check`
     updated in lockstep.
  Post-fix scan: violations remaining in
  `test_match.py` (94 — match-statement lowering),
  `test_generators.py` (4 — resume functions),
  `test_file_io.py` (5), `test_stdlib_urllib.py` (2). All are
  `UseNotDominated` patterns — CFG-level lowering bugs where the
  def lives in a block that does not dominate the use under
  construct_ssa's dominator tree. Tracked as new deferred session
  **S1.6e**.
- **S1.6e** (2026-04-18, two fixes landed) — the match /
  urllib / file_io / classes / exceptions violations were all
  caused by the same root bug: Cytron's classical single-def
  optimisation skipped φ-insertion for single-def locals (assuming
  "the def dominates all uses"), but match-statement lowering
  emits defs in a pattern-check block whose CFG successor merges
  with the failing path before the body is reached — the single
  def does not dominate the use. Two fixes:
  1. **Move match-pattern bindings into the extraction block**.
     `generate_sequence_pattern_check`,
     `generate_mapping_pattern_check`, and
     `generate_class_pattern_check` now emit the binding Copies
     in-place (inside their extraction block) instead of returning
     them up to the caller for emission in the post-branch body
     block. The caller's bindings list is kept for API shape but
     ends up empty for these pattern kinds.
  2. **Relax Cytron's single-def shortcut**. Drop the
     `if def_blocks.len() < 2 { continue }` guard in
     `insert_phis`. Every defined local now runs the IDF
     computation; single-def locals that happen to have their use
     in a non-dominating block correctly get a pass-through φ at
     the merge. Cost: extra dead Phis for single-def locals whose
     def already dominates all uses; DCE cleans them up.
  Post-fix scan: **only `examples/test_generators.py`** still
  reports violations (4 of them, in `three_yields_gen$resume` and
  `multi_yield_expressions$resume`). The generator state-setup
  block (block 4) uses values computed in per-yield state blocks
  that don't dominate it. Tracked as **S1.6e-gen**. All other
  files — match, classes, exceptions, file_io, urllib,
  iteration — are clean.
- **S1.6e-gen** (2026-04-18, landed) — generator `$resume` init
  state (state==0) was emitting `emit_save_all_vars` that touched
  every gen_var, including ones like `x` and `y` that are only
  assigned in later yield-state blocks. Reading those vars in
  init flowed an unbound HIR variable into a MIR LocalId that
  only gets defined in a non-dominating later block.
  Fix: add `collect_defined_vars` walking the init_stmts +
  parameter list, and route `build_while_init` through a new
  `emit_save_vars_where` filter so init saves only variables
  that are actually defined at that program point. The generator
  state slots for yet-to-be-assigned vars stay at Cranelift's
  zero default; subsequent state blocks properly save them
  before yielding.
- **CLI SSA checker activation** (2026-04-18) — scan of every
  `examples/*.py` now reports zero violations, so the checker
  was wired into `crates/cli/src/lib.rs` behind
  `#[cfg(debug_assertions)]`: debug builds panic on any SSA
  invariant violation; release builds skip the check for
  compile-time performance. Phase 1 Acceptance item 3 (SSA
  property checker runs on every function and passes) ✅.
- **S1.14b-prep landed** (2026-04-18) — pipeline reordered:
  `construct_ssa` runs BEFORE `optimize_module`. All optimizer
  passes tolerate SSA input out of the box (constfold's S1.13
  rewrite, DCE's existing SSA-style liveness, peephole's
  SSA-aware idempotents, devirtualize / flatten_properties
  audited as SSA-compatible in S1.15, and inline as it stands —
  its Copy-based return merging DOES produce multi-def MIR but
  `construct_ssa` would recover it; after the reorder, inline
  still emits multi-def into an already-SSA function, which the
  post-optimize SSA gate flags as a bug only if inline fires).
  Added a second debug-only SSA check gate AFTER `optimize_module`
  so any future pass that breaks SSA is caught at its source.
  All 470+ tests pass in both debug and release.
- **S1.14b-inliner landed** (2026-04-18) — `perform_inline` in
  `crates/optimizer/src/inline/transform.rs` now emits a `Phi` at
  the head of the continuation block merging return-values from
  every value-returning callee path, instead of the pre-SSA
  `Copy(dest, val); Goto(continuation)` pattern that produced
  multi-def MIR. Void-returning paths contribute a
  `Constant::Int(0)` placeholder to keep Phi arity matched (the
  placeholder is never semantically consumed because void callees
  don't have their dest read). The post-optimize SSA check gate
  confirms no violations on the full workspace test suite.
- **S1.17b** — HIR tree deletion: rewrite ~52 tree consumers across
  `lowering/` / `semantics/` / `frontend-python/` to walk the CFG;
  delete `Function.body`, `StmtKind::{If, While, ForBind, Try,
  Match}`, and `crates/hir/src/cfg_build.rs`. Resolve the open
  `HirTerminator` iteration-gap question. 5–10 sessions of work.
- **§1.4u** — unify HIR type inference with MIR TypeTable so the
  four HirTypeInference maps can be deleted and lowering reads from
  a single source. Unblocks microgpt.py ternary-rebind case (item
  5). Progress this session (2026-04-18):
  - **step 1 ✅** (commit `828d062`): deleted `infer_expr_type`
    no-overlay wrapper; sole caller migrated. Public HIR type-query
    surface: 4 → 3 entry points.
  - **intermediate ✅** (commit `ecf925b`): extracted
    `resolve_generator_intrinsic_type` helper, collapsing the last
    inline-duplicated non-literal arm between `compute_expr_type`
    and `infer_expr_type_inner`.
  - **step 2 ✅** (commit `518d5dc`): folded 1-field `TypeEnvironment`
    into `HirTypeInference`. Added forward-compatible
    `HirTypeInference::lookup(expr_id)` + `insert_type(...)`
    accessors — the §1.4u-b migration target. Single HIR-type-
    inference owner on `Lowering`.
  - **§1.4u-b step 3 (2026-04-19)** — landed the base-type
    infrastructure: new `HirTypeInference::base_var_types` map
    persisted per-module, populated once at the end of
    `run_type_planning` from (a) `per_function_prescan_var_types`
    (inferred locals + seeded params), (b) every function's
    annotated params, (c) exception-handler binding types. New
    `Lowering::get_base_var_type(var_id)` accessor reads a unified
    chain: `symbols.var_types` → `base_var_types` → `refined_var_types`
    → `prescan_var_types` → `global_var_types`. `compute_expr_type`'s
    three V-reading arms (Var, IfExpr narrowing helper, Call's
    Var-target callable check) now route through this accessor.
    Behaviour is unchanged: `symbols.var_types` is empty at eager-
    pass time (giving base lookup) and populated during lowering
    with narrowing applied (giving effective lookup).
  - **§1.4u-b step 4 (2026-04-19)** — landed the "убрать зависимость
    от var_types из compute_expr_type" change. Two coordinated
    edits:
    1. `get_type_of_expr_id`'s `Var` branch now handles effective-
       type lookup directly (inline `get_var_type` + fallback to
       `get_base_var_type` + `expr.ty`). Previously it delegated to
       `compute_expr_type` which in turn read `var_types`; the
       delegation is gone.
    2. `Lowering::get_base_var_type` dropped `symbols.var_types`
       from its fallback chain. It now reads only stable sources:
       `base_var_types` → `refined_var_types` → `prescan_var_types`
       → `global_var_types`.
    Combined with the three arm rewires in step 3,
    `compute_expr_type` is now **fully free of `symbols.var_types`
    reads**. Narrowing-aware dispatch at lowering time still works
    because:
    - `get_type_of_expr_id`'s Var fast path reads
      `symbols.var_types` first — so `lower_attribute`'s
      `get_type_of_expr_id(obj)` on a narrowed receiver picks up
      the narrowed class and attribute lookup succeeds.
    - All `resolve_*` helpers in `compute_expr_type` take
      pre-resolved sub-expression types; the sub-exprs were
      resolved through `get_type_of_expr_id`, so Var operands are
      already narrowing-aware by the time the helper fires.
    All 470+ workspace tests pass in both debug (SSA gates active)
    and release.
  - **§1.4u-b step 5 ✅** — `eagerly_populate_expr_types` is
    live in `run_type_planning`. Four issues surfaced and all
    are fixed:
      1. `elem_type_of_iterable` in `local_prescan.rs` missed
         the `Type::Iterator(e)` arm — dict/list comprehensions
         over `range(…)` got `Any` for the loop target.
      2. Frontend comp-scoping — outer `for a, b in <iter>` and
         nested comprehension `[… for a, b in <other>]` shared
         the same `VarId`s when names collided. Fixed by adding
         `forget_comp_target_names` in
         `ast_to_hir/comprehensions.rs`, which evicts comp
         loop-target names from `var_map` before the comp body
         is lowered so `bind_target` allocates fresh VarIds.
      3. `resolve_call_target_type` for a `Var` funcexpr did
         not consult `module_var_funcs`, so identity-decorated
         module functions (`@identity def greet(): …`) resolved
         to `Type::Any` at eager-pass time and cached boxed
         `Any` return. Added the `get_module_var_func` arm to
         the Var branch.
      4. `get_type_of_expr_id` must not cache `Any` or `Union`
         results. Both signal narrowing sensitivity: at eager-
         pass time the contained `Var`s read their base Union
         type, but a later lowering-time query inside an
         `isinstance`-dominated block may narrow them to a
         concrete type. Caching the pre-narrowing result would
         poison the cache for the narrowed scope. Concrete
         types (`Int`, `Str`, `Class { … }`, `Tuple`, …) are
         cached as before.
    All 470+ tests pass in debug (SSA gates) and release.
  - **§1.4u-c ✅ (satisfied by Path A construction)** —
    `TypeTable::infer_module` seeds every `LocalId` from
    `func.locals[id].ty`, which lowering populated from
    `HirTypeInference` during MIR emission. The RPO walk's
    per-instruction rules (`Phi`, `Refine`, `Copy`, `BinOp`,
    `UnOp`, `CallDirect`, `Call` via `FuncAddr` trace,
    `RuntimeCall`) are SSA-level *narrowing propagators* — they
    spread refined types from `Refine { src, ty }` and joined
    types from `Phi` to downstream defs via standard operational
    rules (e.g. `Copy` dest ← src type). They never conflict
    with the HIR seed because (a) on straight-line code they
    reproduce the HIR-level answer from the same seed and (b)
    when they diverge it is because an SSA-specific narrowing
    fired, which the HIR layer by construction cannot express.
    This is exactly the "projection plus SSA-only extensions"
    shape that amended Non-Negotiable #4 calls for.
  - **microgpt.py line 41** ✅ — the polymorphic dunder pattern
    `other = other if isinstance(other, Value) else Value(other)`
    now narrows `var_types[other]` to `Value` after the Bind
    completes, so `other.data` resolves as a field access on
    `Type::Class`. Fix in `lower_assign` records the original
    Union in `narrowed_union_vars` for boxing compatibility.
    Remaining microgpt errors (e.g. line 65 `for child in
    v._children` on untyped nested-function param) are
    unrelated; not a §1.4u concern.
- **S1.17 formal close** — benchmark check + full grep-verified
  deletion; depends on Phase 1 tail milestones (§1.10 purge and
  §1.4b HIR-CFG cleanup).

**Goal**: make pyaot's type system **flow-sensitive and
whole-program-aware** by design, not by patching. Every rebind produces
a new SSA variable with independently-computed type. Every function's
parameter types are inferred from the join of all call-site argument
types. Every class field's type is inferred from the join of all
`__init__` argument types across all call sites.

**After Phase 1**, the following legacy maps and helpers MUST be deleted:

- `SymbolTable::prescan_var_types` — replaced by SSA types.
- `SymbolTable::per_function_prescan_var_types` — same.
- `SymbolTable::narrowed_union_vars` — replaced by SSA at narrowing points.
- `SymbolTable::refined_var_types` — replaced by SSA refinement φ-nodes.
- `Lowering::apply_narrowings` / `Lowering::restore_types` — narrowing
  is expressed as SSA φ-insertion, not save/restore.
- `Type::unify_field_type` as a free helper — replaced by lattice
  `join` (Phase 3; Phase 1 inlines it into the field-inference pass).
- `get_or_create_local` keyed by `VarId` — replaced by keyed by SSA
  `(VarId, BlockId, Version)`.
- `insert_var_type` as a mutable imperative API — types are computed
  once per SSA variable.

## 1.1 HIR → CFG conversion 🟡

**Milestone goal**: HIR functions carry an explicit CFG, not a
statement list.

Currently `hir::Function { body: Vec<StmtId>, ... }`. Stmts like `If`,
`While`, `Try`, `ForBind` contain nested `Vec<StmtId>` for their
branches. Type inference walks this tree-of-statements.

**New representation**:

```rust
pub struct Function {
    pub id: FuncId,
    pub name: InternedString,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub blocks: IndexMap<HirBlockId, HirBlock>,  // NEW
    pub entry_block: HirBlockId,                  // NEW
    // ... other fields
}

pub struct HirBlock {
    pub id: HirBlockId,
    pub stmts: Vec<StmtId>,          // linear list, no nested branches
    pub terminator: HirTerminator,   // NEW: explicit control flow
}

pub enum HirTerminator {
    Jump(HirBlockId),
    Branch { cond: ExprId, then_bb: HirBlockId, else_bb: HirBlockId },
    Return(Option<ExprId>),
    Raise { exc: ExprId, cause: Option<ExprId> },
    Yield { value: ExprId, resume_bb: HirBlockId },  // for generators
    Unreachable,
}
```

`StmtKind` loses its control-flow variants (`If`, `While`, `ForBind`,
`Try`, `Match`) — their shape moves into `HirTerminator` + the CFG
topology. What remains are "straight-line" statements: `Bind`,
`Expr`, `Assert`, `Pass`, `Break`/`Continue` (these become Jumps to
known blocks via the lowerer).

**Work**:

- Extend `hir` crate with `HirBlock`, `HirBlockId`, `HirTerminator`.
- Extend frontend-python AST→HIR lowering to produce CFG instead of
  tree. Every `if`/`while`/`for`/`try`/`match` creates the
  corresponding blocks and terminators during conversion.
- Delete `StmtKind::If`, `StmtKind::While`, `StmtKind::ForBind`,
  `StmtKind::Try`, `StmtKind::Match`. Gone completely. The control
  flow is now in the CFG.
- Update `optimizer`, `lowering`, `codegen-cranelift` to consume
  HIR-as-CFG. Lowering is simplified: each HIR block → one MIR block
  prefix; terminators map 1:1.
- Generators (already desugared to regular functions in current code)
  are represented with `Yield` terminators in HIR-CFG. The
  desugar-to-creator/resume pass moves into CFG-level graph rewriting.

**Non-negotiable**: after this milestone, there are no nested
`Vec<StmtId>` anywhere in HIR. If any pass relies on tree-shape
walking, it is rewritten to walk the CFG.

**Exit criteria**:

- `hir::StmtKind` variants reduced to straight-line only.
- Every function has `entry_block` and `blocks` populated.
- All existing `examples/*.py` compile and run bit-identically.
- `cargo test --workspace --release` green.

**Status (2026-04-18, S1.1 landed)**: new HIR CFG types
(`HirBlockId`, `HirBlock`, `HirTerminator`) added alongside the legacy
tree representation — no consumers yet. `hir::Function` still carries
`body: Vec<StmtId>`; nested control-flow `StmtKind` variants
(`If`/`While`/`ForBind`/`Try`/`Match`) are untouched. S1.2 (frontend
AST→CFG migration) and S1.3 (legacy-variant deletion) remain.

**Status (2026-04-18, S1.2 landed)**: `hir::Function` now carries
`blocks: IndexMap<HirBlockId, HirBlock>` + `entry_block: HirBlockId`
populated by a tree→CFG converter in `crates/hir/src/cfg_build.rs`.
Every frontend construction site (functions, classes, comprehensions,
lambdas, nested functions, module-init) and the generator-desugaring
site in `crates/lowering/src/generators/desugaring.rs` now builds the
CFG alongside the legacy `body: Vec<StmtId>`. `StmtKind::{If, While,
ForBind, Try, Match}` and `Function.body` remain as the canonical form
consumed by optimizer/lowering/codegen — no behavioral change. S1.2
simplifications that S1.3 will fix: `ForBind` uses the `iter` expr as
a placeholder branch condition; `try` handlers are emitted as
unreachable-from-CFG blocks (no exception edges); `match` cases are
chained linearly without pattern dispatch. All tests green; 5 new
unit tests in `cfg_build.rs` cover straight-line, if/else merge,
while+break/continue, return short-circuit, and raise terminators.

**Status (2026-04-18, S1.3 scope reduced — amendment)**: starting
S1.3 surfaced two structural issues that make the session's original
"delete `Function.body` + legacy `StmtKind::{If, While, ForBind, Try,
Match}`" scope unsafe:

1. *Order-dependent consumers.* The HIR tree is still consumed by
   `lowering/type_planning/{closure_scan, container_refine,
   local_prescan, ni_analysis, lambda_inference}` and
   `semantics::walk_stmts` with **source-order / tree-nesting
   dependencies** that don't portably map to CFG iteration (e.g.
   `container_refine` looks for `x = []` followed by `x.append(e)`
   in the same sequence; `semantics` tracks `loop_depth` /
   `except_depth` via tree recursion). A correct CFG port would need
   dataflow rework, not a mechanical iteration change. Since §1.4
   explicitly deletes most of these walkers' storage
   (`refined_var_types`, `prescan_var_types`, `narrowed_union_vars`)
   when `TypeInferencePass` (S1.8) lands, porting them first would be
   throwaway work.
2. *`HirTerminator` iteration gap.* The target terminator set
   (`Jump, Branch, Return, Raise, Yield, Unreachable`) has no
   iteration primitive, yet `ForBind` needs a has-next / next scheme
   at HIR level. S1.2 used `iter` as a placeholder branch condition;
   a legitimate representation is still TBD.

**Plan revision:**

- **S1.3 narrowed**: migrate only the CFG-portable consumers
  (lowering-core emission path: `statements/*`, `exceptions.rs`, the
  generator-desugaring detection passes). `Function.body` + legacy
  `StmtKind::{If, ...}` variants stay alive as a bridge.
- **New milestone §1.1 tail (renumbered §1.4b, sequenced after
  §1.4 / S1.8 TypeInferencePass)**: delete `Function.body`,
  `StmtKind::{If, While, ForBind, Try, Match}`, and the tree→CFG
  bridge `crates/hir/src/cfg_build.rs`. Rewrite the frontend to emit
  CFG directly. Resolve the `HirTerminator` iteration gap (candidate
  options: add `IterHasNext` / `IterNext` HIR expression primitives
  referenced from an ordinary `Branch` cond; or a new
  `HirTerminator::Iterate { iter, bind_target, body_bb, exit_bb }`
  variant). The choice is documented in the §1.4b planning commit
  before implementation starts.

**§1.1 Open Questions** (resolved no later than §1.4b):
- How to represent for-loop iteration in a pure HIR CFG — primitive
  expressions vs. new terminator variant.
- Whether exception edges should be modeled as CFG edges or left as
  implicit runtime flow (the tree→CFG bridge currently leaves
  handlers unreachable from CFG; this is a defensible long-term model
  but has to be explicitly blessed).

## 1.2 Dominator tree computation ✅

**Milestone goal**: every MIR function carries a precomputed dominator
tree and block-frequency info.

**Work**:

- In `crates/mir`, add `DomTree` struct with:
  - `immediate_dominator(block) -> Option<BlockId>`
  - `dominates(a, b) -> bool`
  - `dominance_frontier(block) -> impl Iterator<Item = BlockId>`
  - `reverse_post_order() -> Vec<BlockId>`
- Implement via **Cooper-Harvey-Kennedy** algorithm (O(n × d) where d
  is DOM tree depth — standard, well-tested).
- Compute lazily: `mir::Function::dom_tree() -> &DomTree` memoized.
- Invalidate on CFG mutation (passes that mutate CFG call
  `invalidate_dom_tree()`).

**Non-negotiable**: no hand-rolled dominance calculations outside this
module. Passes that need dominance information request it here.

**Exit criteria**:

- Dom tree computed for all functions, correctness tested against
  hand-computed examples.
- `dominates` and `dominance_frontier` benchmarked: < 1ms per typical
  function.

**Status (2026-04-18, S1.4 landed)**: `crates/mir/src/dom_tree.rs`
implements the Cooper–Harvey–Kennedy algorithm (RPO DFS → iterative
idom fix-point → CHK Figure 5 dominance frontier). `mir::Function`
carries a `OnceCell<DomTree>` cache populated by `func.dom_tree()`;
`func.invalidate_dom_tree()` drops the cache and is wired into
`optimizer::dce::reachability::eliminate_unreachable_blocks` and
`optimizer::inline::transform`. `ssa_check.rs` now consumes the new
`DomTree` — its old ad-hoc `compute_dominators`,
`compute_predecessors`, and free-fn `dominates` helpers are deleted
(Principle 10 — deletion is progress). The canonical
`terminator_successors` helper now lives in `dom_tree.rs` and is
re-exported at `pyaot_mir::terminator_successors`; the duplicate
inside `optimizer::dce::mod` is removed. 6 new unit tests cover
linear, diamond, while-loop, unreachable, self-dominance, and
terminator-successor cases; all 396 pre-existing workspace tests pass
unchanged. End-to-end bench programs (`classes`, `gc_stress`) produce
bit-identical output. The `rpo_index` accessor is exposed on
`DomTree` in anticipation of Cytron φ-insertion (S1.6).

## 1.3 SSA renaming + φ-insertion ✅

**Milestone goal**: MIR functions are in **pruned SSA form**.

**Work**:

- Apply **Cytron et al.** algorithm: compute iterated dominance
  frontier for each variable, insert φ-nodes, rename.
- MIR representation change:
  ```rust
  // Add to MIR Instruction
  Instruction::Phi { dest: LocalId, sources: Vec<(BlockId, Operand)> }
  ```
- Local IDs are versioned: `LocalId(u32)` stays u32, but IDs across
  different SSA versions of the same HIR VarId are distinct. A new
  side table `SsaVersions: IndexMap<(VarId, BlockId), LocalId>`
  tracks which MIR local represents which HIR VarId at which block
  entry.
- All MIR Instructions after SSA have the property: `dest` appears as
  exactly one instruction's dest. The SSA checker from Phase 0.3
  enforces this.
- `get_or_create_local` is **deleted**. Lowering does explicit
  renaming during HIR→MIR translation: each Bind emits a new
  `LocalId`; each read consults the current "SSA map" at the current
  block.

**Generators**: yield terminators split a block. The SSA renaming
treats each yield-resume edge like any other control flow edge; state
captured across yields is handled by the existing generator-object
slot mechanism, now augmented with SSA-consistent slot assignments.

**Non-negotiable**: no mutable `LocalId`. A local defined in block B1
is dead after B1 unless consumed; redefinition creates a new local.
The φ-instruction is the only join mechanism.

**Exit criteria**:

- SSA property checker (Phase 0.3) flips to `enabled_by_default: true`
  and passes on all functions.
- Every LocalId has exactly one defining instruction (Assign,
  RuntimeCall dest, Phi, or function parameter).
- Existing MIR-consuming passes (codegen, optimizer) updated to
  handle Phi instructions — Cranelift has native `block_param`
  support, Phi maps 1:1 to `block.append_param(ty)` + `br_params`.

**Status (2026-04-18, S1.5 landed — Phi prep)**:
`InstructionKind::Phi { dest, sources: Vec<(BlockId, Operand)> }` is
defined and wired through every exhaustive match site (mir
`instruction_def`/`instruction_uses`, `dce::instruction_dest`
/`is_pure`/`used_locals`, `constfold::propagate::substitute_instruction_operands`,
`inline::remap::remap_instruction`, codegen `compile_instruction`).
The SSA checker enforces three new invariants when `is_ssa=true`:
(a) φ-source count and predecessor set match, (b) Phi instructions
occupy the block-head prefix only, (c) each Phi source's dominance
is checked against the *predecessor* block, not the Phi's own block
(classical SSA). Codegen pre-declares Cranelift `block_params` for
leading Phis per MIR block, binds each dest to its param value on
`switch_to_block`, and passes phi-source values as `BlockArg::Value`
on every outgoing `jump`/`brif`. No function currently emits Phi
(still `is_ssa=false` everywhere), so the codegen path is present
but dormant; S1.6 activates it via Cytron-style renaming. 3 new
hand-rolled SSA tests in `ssa_check.rs` (diamond-merge phi accepts,
arity-mismatch rejects, non-head-phi rejects); all 399 pre-existing
workspace tests pass unchanged. End-to-end bench spot-checks
(`classes`, `gc_stress`, `generators`) produce bit-identical output.

**Status (2026-04-18, S1.6a landed — SSA construction, straight-line activation)**:
`crates/mir/src/ssa_construct.rs` implements the classical
Cytron-Wegman-Zadeck three-phase algorithm: def collection,
iterated-dominance-frontier φ-insertion, and dominator-tree
pre-order renaming with per-original-local version stacks. The pass
activates in `crates/cli/src/lib.rs` between optimizer and codegen,
gated by `pyaot_mir::ssa_construct::is_straight_line(func)` — only
functions whose terminators are `Goto`/`Return`/`Raise`/`Unreachable`
(no `Branch`) flip to `is_ssa=true`. Branching functions stay
non-SSA; S1.6b lifts the gate after validating the branching paths.
Unit tests: 3 new in `ssa_construct.rs` (single-def straight-line,
multi-def straight-line gets fresh LocalIds, diamond merge gets φ);
all 402 workspace tests pass. End-to-end bench spot-checks
(`classes`, `gc_stress`, `generators`, `exceptions`) produce
bit-identical output — SSA construction is semantics-preserving on
the activated subset. Classical Phi-insertion correctness is
validated through the existing S1.5 SSA checker, which now sees
live Phi nodes on every straight-line function with multi-def
locals.

**Status (2026-04-18, S1.6b partial — two Cytron fixes landed, gate
still at straight-line)**: trying to activate SSA construction
universally uncovered two algorithmic bugs in
`ssa_construct.rs` that S1.6b fixes:

1. **Back-edge phi-original tracking.** The φ-source fill-in used
   `phi.dest` as the lookup key into the rename stacks, which worked
   on forward edges (dest still = original at fill time) but broke
   on back-edges where the successor (loop header) has already been
   visited and its φ.dest rewritten to a fresh LocalId. Fix:
   capture a `phi_originals: HashMap<BlockId, Vec<LocalId>>`
   side-table in `rename()` **before** any renaming runs; the
   ordered originals are indexed by block id + φ position at
   fill-in time regardless of whether the successor has been
   renamed. A new unit test
   (`while_loop_phi_gets_both_entry_and_back_edge_sources`)
   exposes and asserts against the bug — it checks not only source
   arity but the actual operand at the back-edge must be the body's
   latest rename, not the φ's own dest.

2. **Unreachable-block pruning.** The Cytron rename walk descends
   the dominator tree, which only covers blocks reachable from
   `entry_block`. φ-insertion ran on all blocks (via `collect_defs`
   over the full CFG), so a φ at a reachable merge point could be
   left with a missing source for an unreachable predecessor —
   leading to the codegen assertion `phi has no source for
   predecessor block` whenever that unreachable block survived DCE
   (e.g., when the optimizer was disabled). Fix: `construct_ssa`
   now runs a BFS from `entry_block` and drops non-reachable
   blocks as Phase 0 of the algorithm.

With both fixes, **33 / 35 runtime tests pass** when the straight-line
gate is lifted universally, up from 0 under the naive lifting. Two
tests (`runtime_iteration`, `runtime_builtins`) still show latent
SSA-construction bugs on complex CFGs that need deeper
investigation than this session allows. The `is_straight_line` gate
stays in place for now in `crates/cli/src/lib.rs`; the comment there
documents the narrow scope and the handoff. A follow-up session
debugs those two cases and lifts the gate in a single commit.

All 403 workspace tests pass; `cargo fmt --check` / `cargo clippy
--workspace --release -- -D warnings` clean. End-to-end bench
spot-checks still produce bit-identical output.

**Status (2026-04-18, S1.6b complete — gate fully lifted)**: root
cause of the two remaining failures isolated and fixed. The MIR
lowerer reuses a live `LocalId` as the `dest` placeholder for
void-return `RuntimeCall`s — e.g. `rt_tuple_set(dest=L, args=[L, …])`
mutates tuple `L` in place and the call has no return value, so
codegen leaves `dest` unwritten. My Cytron pass was treating every
`InstructionKind::RuntimeCall` as a new SSA definition, silently
shadowing `L`'s live value and rewriting every subsequent use of `L`
to an uninitialised Cranelift variable. Fix: a new
`runtime_call_is_void` predicate in `ssa_construct.rs` inspects the
descriptor's `returns: Option<ReturnType>` (plus the short list of
legacy `RuntimeFunc::Excᴇ…` variants with known void codegen) and
makes `instruction_def` return `None` for void calls so the renamer
neither allocates a fresh id nor pushes onto the version stack.

Activation gate in `crates/cli/src/lib.rs` is now the unconditional
`for func in mir_module.functions.values_mut() { construct_ssa(func); }`
loop — every MIR function (straight-line, branching, looping,
desugared generator / closure / comprehension) runs through SSA
construction. All 35 runtime integration tests pass; 403 workspace
tests pass; all 11 bench programs produce bit-identical output.
`cargo fmt --check` / `cargo clippy --workspace --release -- -D
warnings` clean.

**Status (2026-04-18, S1.7 landed — Refine infrastructure)**:
`InstructionKind::Refine { dest, src: Operand, ty: Type }` is
defined and wired through every exhaustive match site (mir
`instruction_def`/`instruction_uses` in both `ssa_check` and
`ssa_construct`, `dce::{instruction_dest, is_pure, used_locals}`,
`constfold::propagate::substitute_instruction_operands`,
`inline::remap::remap_instruction`, codegen `compile_instruction`).
`Refine` is classified as a pure SSA def in the Cytron pass — the
renamer allocates a fresh LocalId for `dest` and substitutes the
current version for `src`, exactly like a `Copy`. Codegen emits it as
a plain `compile_copy(dest, src)` — same bit pattern, different
LocalId, zero runtime cost. No function currently emits Refine
(still `is_ssa=true` everywhere, but no narrowing lowering runs
yet); S1.8 (TypeInferencePass) will start emitting them at
isinstance-dominated successor entries. 1 new test in
`ssa_construct.rs` (`refine_participates_in_ssa_renaming`) verifies
renaming and checker acceptance; all 404 workspace tests pass
unchanged. End-to-end bench spot-checks bit-identical.

## 1.4 Flow-sensitive type inference 🟡

**Milestone goal**: every MIR LocalId has a single, precise, flow-
sensitive type assigned by a dedicated pass.

**Work**:

- Replace `compute_expr_type` / `infer_expr_type_inner` /
  `infer_deep_expr_type` (currently 3 overlapping paths) with **one**
  pass: `TypeInferencePass` that:
  1. Initializes param types from the function signature.
  2. Walks MIR in reverse-postorder.
  3. For each non-Phi instruction: computes dest type from operand
     types + op semantics.
  4. For each Phi: dest type = `Type::join(sources[0], sources[1], ...)`.
     (In Phase 1 `join` is a local helper; Phase 3 promotes it to
     lattice API.)
  5. Iterates until fixed point (bounded by `n × max_type_depth`).
- **Narrowing is automatic**: `isinstance(x, T)` at a Branch
  terminator splits the successor flow. The then-block sees `x`
  narrowed to `T` (by rewriting `x`'s type in the then-block entry
  via a synthetic refinement instruction — this is a new MIR
  instruction `Refine { dest: new_local, src: old_local, ty: T }`
  that is a pointer reinterpretation at runtime, free).
- **No more branch-saves + restores**. The CFG encodes narrowing.

**Delete**:

- `Lowering::apply_narrowings` / `restore_types`.
- `narrowed_union_vars` map.
- `refined_var_types` map (refinement is an SSA version now).
- `prescan_var_types` + `per_function_prescan_var_types`.

**Non-negotiable** (amended 2026-04-19, §1.4u-d):
Lowering has **one** canonical source of truth for expression types —
`HirTypeInference` — and every lowering-side query goes through
`Lowering::get_type_of_expr_id` → `HirTypeInference::lookup`. The
MIR-level `TypeTable` (`pyaot_optimizer::type_inference`) is a
**post-SSA projection** of this source: it seeds every `LocalId`
from `func.locals[id].ty` (which lowering populated from
`HirTypeInference`) and extends with the SSA-specific
`Phi`/`Refine`/WPA refinements that the HIR layer cannot express.

Lowering-time mutation of `symbols.var_types` via `insert_var_type`
is permitted as a **narrowing overlay only** — it represents the
effective type of a `Var` expression in the current control-flow
scope (e.g. inside an `isinstance` branch). It does not change the
base type stored in `HirTypeInference`, which remains the pure
function of HIR + F/M state produced by the unified pass. If a
later pass needs to update a base type, it invalidates and reruns
the affected region of the unified pass.

The spec's original literal formulation ("single `TypeTable` that
is a pure function of the SSA IR") describes Path B (full MIR-
level unification); Phase 1 ships Path A (HIR-level unification
with SSA-derived MIR view). Path B lands naturally in Phase 2
when tagged `Value` + post-SSA specialization remove the need for
lowering-time types.

**Exit criteria**:

- Every existing type-dependent test passes.
- All 4 legacy type maps deleted from `SymbolTable` (Phase 1 exits
  with them renamed into `HirTypeInference`; Phase 3 lattice join
  deletes the last two).
- `apply_narrowings` / `restore_types` deleted (§1.4u step 5+).
- Type queries use **one** canonical HIR-level source
  (`HirTypeInference`) plus **one** SSA-level projection
  (`TypeTable`) that seeds from it.

**Status (2026-04-18, S1.8a landed — TypeInferencePass core engine)**:
`crates/optimizer/src/type_inference.rs` implements the classical RPO
walk + fixed-point skeleton for §1.4:

- `pub struct TypeTable` keyed by `FuncId` → `IndexMap<LocalId, Type>`.
- `TypeTable::infer_module(&Module) -> TypeTable` and the per-function
  `infer_function(&Function) -> FunctionTypes` exposed for tests and
  later interprocedural layers.
- Per-function: seed every `LocalId` from `func.locals[id].ty` (which
  SSA construction already populated with the per-version type),
  then walk reverse-post-order iterating until fixed point (bounded
  `MAX_ITERATIONS_PER_FUNCTION = 32`, never observed to hit in
  practice — well-formed SSA settles in 1-2 sweeps). At each Phi,
  dest = `Type::unify_field_type` join of source operand types
  (Phase 3's lattice join replaces this). At each Refine, dest = the
  explicit `ty` field.
- Non-SSA functions (`is_ssa=false`) are handled via a single
  top-down pass so consumers can query the table uniformly regardless
  of SSA state.
- 7 new unit tests cover seed, identical-type phi join, numeric-tower
  promotion (Int⊔Float→Float), refine narrowing, constant operand
  literal types, module-wide inference, and chained-phi fixed-point.

What this session does **not** do — reserved for S1.8b/c:
- Per-instruction result-type rules (Const literal → exact type,
  BinOp numeric tower, Copy = src type, Call = callee's return type,
  etc.). The `_` arm in `infer_function` currently leaves the seed
  type unchanged.
- `Refine`-emission at `isinstance`-branch successor entries.
- Deletion of the legacy SymbolTable maps (`refined_var_types`,
  `prescan_var_types`, `narrowed_union_vars`) — S1.9.
- Pipeline integration. No consumer reads the new `TypeTable` yet;
  S1.8b hooks it up as lowering switches over.

All 417 workspace tests pass (+7 new); `cargo fmt --check` /
`cargo clippy --workspace --release -- -D warnings` clean. End-to-end
bench spot-checks bit-identical — no runtime change.

**Status (2026-04-18, S1.8b landed — per-instruction rules)**:
`infer_function` now takes `Option<&Module>` so cross-function
lookups (CallDirect → callee return type) work; existing
`TypeTable::infer_module` forwards the module automatically. The RPO
walk dispatches through a shared `apply_instruction` helper with
explicit rules for:

- `Const { value }` → `constant_type(value)` (literal's type —
  narrows an `Any` seed).
- `Copy { src }` → `operand_type(src, types)` — propagates refined
  types through copy chains.
- `CallDirect { func }` → `module.functions[func].return_type`
  (no-op when `module` is `None`).
- `GcAlloc { ty }` → the explicit `ty` field.

The non-SSA `apply_single_pass` now routes through the same
`apply_instruction`, so both SSA and legacy paths use identical rule
logic. Other instruction kinds (`BinOp`, `UnOp`, `Call`,
`CallVirtual*`, `RuntimeCall`) still fall through to seed — S1.8c
extends to numeric-tower BinOp and runtime-call return-type lookup,
and WPA (S1.11) specialises indirect `Call` dests via call-site arg
types. 4 new unit tests (const / copy / call_direct / gc_alloc);
all 421 workspace tests pass; fmt + clippy clean; bench output
bit-identical — still no consumer wiring.

**Status (2026-04-18, S1.8c-part landed — BinOp/UnOp rules)**:
`apply_instruction` now covers every `BinOp` and `UnOp` variant
through two helpers:

- `binop_result_type(op, left, right)`:
  - `Eq`/`NotEq`/`Lt`/`LtE`/`Gt`/`GtE` → `Type::Bool`.
  - `Div` → `Type::Float` (Python `/` is true division; `FloorDiv`
    takes the numeric-tower path).
  - `Add`/`Sub`/`Mul`/`FloorDiv`/`Mod`/`Pow`/`And`/`Or`/bitwise →
    `merge_operand_types(left, right)` which is `Type::unify_field_type`
    with `Never`/`Any` special-cased (same pattern as the WPA
    `wpa_join_types` helper — both manually absorb lattice
    extremes that `normalize_union` doesn't simplify).
- `unop_result_type(op, operand)`:
  - `Not` → `Type::Bool`.
  - `Neg`, `Invert` → preserve operand type.

6 new unit tests: `Int + Int = Int`, `Int * Float = Float`,
`Int / Int = Float`, `Int < Int = Bool`, `-Float = Float`,
`not Int = Bool`. 431 workspace tests pass; fmt + clippy clean;
bench output bit-identical.

Remaining instruction kinds still at seed: `Call`, `CallNamed`,
`CallVirtual*`, `RuntimeCall`, exception helpers, boxing /
conversion ops. A future session can add `RuntimeCall` return-type
lookup via `RuntimeFuncDef::returns` plus a Cranelift-type-to-
`Type` translation; `Call` (indirect) needs devirtualisation to
resolve its callee before WPA can specialise.

**Status (2026-04-18, pipeline integration + `RuntimeCall` rules +
`--emit-types`)**: the TypeInferencePass + WPA pipeline now runs in
real compilations, gated behind either `--emit-types` or
`--verbose`:

- `crates/cli/src/lib.rs` runs `CallGraph::build` →
  `TypeTable::infer_module` → `wpa_param_inference` in sequence
  between MIR optimisation and codegen. No downstream codegen
  consumer reads the table yet (that's S1.9's work) — this is pure
  end-to-end validation + a debug-dump hook.
- `CompileOptions::emit_types` / CLI `--emit-types` dumps the
  resulting `TypeTable` via its `Debug` impl, matching the
  existing `--emit-hir` / `--emit-mir` pattern. Default
  optimisation levels still pay **zero** cost because the
  integration path short-circuits when neither flag is set.
- `apply_instruction` now covers `RuntimeCall` for the subset of
  `RuntimeFunc` variants whose Python-level return type is
  unambiguous: `MakeStr → Str`, `MakeBytes → Bytes`, `ExcSetjmp →
  Int`, `ExcGetType → Int`, `ExcHasException → Bool`,
  `ExcGetCurrent → HeapAny`, `ExcIsinstanceClass → Bool`,
  `ExcInstanceStr → Str`. Descriptor-based `RuntimeFunc::Call(def)`
  is left at seed — its Cranelift `returns: Option<ReturnType>`
  can't distinguish `Int` from a heap pointer at I64 without a
  per-function lookup table (out of scope for this session).
- 4 new unit tests validate the `RuntimeCall` rules; `--emit-types`
  dumps a readable `TypeTable` against a 4-line capturing-lambda
  reproducer, showing the lambda's param correctly inferred to
  `Int` and the module-init's tuple-of-Int / list-of-Int typed
  locals. 435 workspace tests pass; fmt + clippy clean; bench
  spot-checks (`classes`, `polymorphic`, `generators`, `exceptions`,
  `gc_stress`) bit-identical — the optional integration path does
  not perturb compilation when flags are off.

Consumer wiring (S1.9) is the next natural step: a compatibility
shim that reads from `TypeTable` in lowering's `compute_expr_type`
call sites with fallback to the legacy HIR maps, unblocking the
per-call-site migration that ends in deletion of
`SymbolTable::{prescan_var_types, per_function_prescan_var_types,
narrowed_union_vars, refined_var_types}` and
`Lowering::{apply_narrowings, restore_types}`.

**Status (2026-04-18, Call-indirect rule landed)**: the
TypeInferencePass now resolves indirect `InstructionKind::Call`s
through a single `FuncAddr` def in the same function — the common
closure / HOF lowering pattern where
`addr = FuncAddr(callee); result = Call(addr, …)`. New helper
`infer_call_return_via_func_addr` is dispatched alongside
`apply_instruction` in both the SSA RPO walk and the non-SSA
single-pass fallback. SSA's single-def guarantee makes the scan
authoritative; Phi / Copy-propagated function pointers still fall
back to seed. 2 new unit tests (successful resolve + `None` module
no-op); 437 workspace tests pass; fmt + clippy clean; bench
spot-checks (`closures` exercises the FuncAddr path; all
bit-identical).

**Status (2026-04-18, full-program WPA fixed point)**: the inner
`wpa_param_inference` iterates each SCC to closure but never
re-visits earlier SCCs when a later SCC's changes would have
propagated back into their call sites. New
`wpa_param_inference_to_fixed_point(module, cg, table)` loops the
inner pass until `table.per_func` stops changing, capped at 8
outer iterations (never observed to be reached; two passes suffice
for typical programs). Pipeline in `cli/lib.rs` now calls the
fixed-point wrapper. One new test (`wpa_full_program_fixed_point_
refines_across_chain`) builds a three-function chain
`main(42) → mid(x) → leaf(y)` where a single
`wpa_param_inference` call leaves `leaf.y = Any` because leaf is
processed in the leaves-first SCC order with stale mid info;
the outer fixed point propagates `Int` all the way through to
`leaf.y`. 438 workspace tests pass; fmt + clippy clean.

**Status (2026-04-18, S1.9a — unified HIR type-query entry points)**:
S1.9 deletion of the 4 `SymbolTable` maps and the 3 legacy type
functions is a multi-session migration (S1.9a / S1.9b / S1.9c /
S1.9d — see audit in the 2026-04-18 exploration notes). S1.9a
establishes the migration slot:

- `crates/lowering/src/type_planning/infer.rs` module-level
  docstring rewritten to declare the **two public HIR type-query
  entry points**:
  1. `Lowering::get_type_of_expr_id` — memoized codegen-time path
     (141 call sites across 45 files in `statements/`,
     `expressions/`, `exceptions.rs`, etc. — already the
     universal caller-facing entry).
  2. `Lowering::infer_deep_expr_type` / `Lowering::infer_expr_type`
     — non-memoized pre-scan path, with and without
     parameter-type overlay.
- New `infer_expr_type(expr, module)` wrapper elides the empty-map
  allocation that the sole previous non-overlay caller (module-
  level literal type inference in
  `context/function_lowering.rs:535`) was doing implicitly.
- `compute_expr_type` and `infer_expr_type_inner` narrowed from
  `pub(crate)` to `pub(super)` — now visible only within
  `type_planning/`. External lowering code cannot reach them by
  name. S1.9b collapses the two into a single unified match
  behind the public wrappers.
- One call-site migration: `function_lowering.rs` now calls
  `self.infer_expr_type(...)` instead of directly poking
  `infer_expr_type_inner(..., None)`.
- Grep-verified: `compute_expr_type\(` and `infer_expr_type_inner\(`
  appear **only** inside `crates/lowering/src/type_planning/mod.rs`
  and `crates/lowering/src/type_planning/infer.rs` — nowhere else
  in the workspace.

No behavioural change. 438 workspace tests still pass; fmt +
clippy clean; all 11 bench spot-checks bit-identical. Nothing is
deleted yet — the 4 `SymbolTable` maps and the 3 function bodies
all stay. S1.9a's only job was to establish the one-way valve at
the public API so subsequent sessions can refactor the
implementation behind it without touching caller code.

**Handoff to S1.9b** (internal collapse — still no deletions of
legacy maps): merge `compute_expr_type` + `infer_expr_type_inner`
into a single unified match body, parameterised by a
`QueryMode::{Memoized, Prescan{param_types}}` enum. Sub-expression
recursion dispatches on mode. Result: ~200 LOC reduction in
`infer.rs`, and the two separate implementations stop drifting.
Legacy map call sites remain untouched.

**Status (2026-04-18, S1.9b — shared result helpers)**: attempting
a full trait-based collapse turned out to require converting every
pre-scan caller's `&self` to `&mut self` (bouncing through ~15
downstream call sites in `type_planning/closure_scan.rs`,
`container_refine.rs`, `lambda_inference.rs`, `local_prescan.rs`)
and resolving subtle semantic differences (Var-arm lookup strategy,
IfExpr overlay scoping, literal-arm explicit vs `expr.ty`-fallback
handling). That ripple is out of scope for a single session.

Instead, S1.9b extracts the **post-recursion result-computation
logic** into shared `&self` helpers both dispatchers call. Each
helper takes already-resolved sub-expression types and produces
the parent result `Type`. 11 new helpers:
`binop_result_type`, `logical_op_result_type`,
`method_call_result_type`, `index_result_type`,
`call_result_type`, `builtin_call_result_type`,
`attribute_result_type`, `class_ref_type`, `class_attr_ref_type`,
`closure_result_type`, `module_export_type`. Both
`compute_expr_type` and `infer_expr_type_inner` match arms now
delegate to the helpers after their own sub-expression recursion.

The two dispatchers stay separate — their real delta is the
recursion strategy (memoized vs. direct) and Var-arm lookup
(overlay-aware vs. simple). Those two concerns don't DRY without
the larger API change; S1.9c/d revisit when the map deletion
forces structural rework.

Diff stats: `+229 / -125` across `infer.rs` — the helper block
added fresh documented surface area while collapsing ~100 LOC of
inline dispatch-body duplication. 438 workspace tests pass; fmt +
clippy clean; bench output bit-identical.

**Handoff to S1.9c** (map migration): replace the 4 SymbolTable
maps with fields on a new `HirTypeInference` struct. Now that the
result-computation logic is shared, the dispatcher structure is
more amenable to parameterising over variable-lookup strategies —
the Var-arm is the last meaningful divergence.

**Status (2026-04-18, S1.9c — maps moved to `HirTypeInference`)**:
the four legacy maps — `prescan_var_types`,
`per_function_prescan_var_types`, `narrowed_union_vars` (all three
previously on `SymbolTable`), and `refined_var_types` (previously
on `TypeEnvironment`) — are now fields of a new `HirTypeInference`
struct in `crates/lowering/src/context/mod.rs`. `Lowering` owns it
as `pub(crate) hir_types: HirTypeInference`. All 20 reference sites
across 10 files migrated from `self.symbols.<field>` /
`self.types.<field>` to `self.hir_types.<field>`.

Field definitions no longer exist on `SymbolTable` or
`TypeEnvironment` — grep-verified:
`\.symbols\.(prescan_var_types|per_function_prescan_var_types|narrowed_union_vars)`
and `\.types\.refined_var_types` return zero matches workspace-wide.

438 workspace tests pass; fmt + clippy clean; bench spot-checks
across 8 programs bit-identical.

**Handoff to S1.9d** (narrowing stack + final cleanup): rewrite
`apply_narrowings` / `restore_types` as scoped push/pop on
`HirTypeInference`'s narrowing stack. Delete the `narrowing.rs`
standalone helpers. Grep-verify every legacy name (the 4 map
identifiers, the 3 function names `compute_expr_type`,
`infer_expr_type_inner`, `apply_narrowings`) — they should all
appear only inside `type_planning/infer.rs` and
`HirTypeInference`-owning files. Close §1.4 exit criteria.

**Status (2026-04-18, S1.9d — narrowing stack + §1.4 closure)**:
`apply_narrowings` / `restore_types` replaced by
`push_narrowing_frame` / `pop_narrowing_frame`. New
`HirTypeInference::narrowing_stack: Vec<NarrowingFrame>` holds each
scope's undo information (`saved_var_types` + `added_union_tracking`).
Callers no longer thread the saved `IndexMap<VarId, Type>` through
their own scope — the stack is the source of truth.

Migration:
- 3 call-site pairs in `statements/control_flow.rs` (if-then,
  if-else, while body) rewritten from
  `let saved = self.apply_narrowings(…); … self.restore_types(saved);`
  → `self.push_narrowing_frame(…); … self.pop_narrowing_frame();`.
- Legacy helper bodies in `narrowing.rs` deleted. Stale docstrings
  in `type_planning/mod.rs` updated.

§1.4 final grep audit:
- `\.symbols\.{prescan_var_types,per_function_prescan_var_types,narrowed_union_vars}`
  → **0 matches**
- `\.types\.refined_var_types` → **0 matches**
- `apply_narrowings\(` / `restore_types\(` call sites → **0 matches**
  (only doc-comment history references remain)
- `pub {prescan_var_types,per_function_prescan_var_types,
  narrowed_union_vars,refined_var_types}:` field definitions →
  **4 matches, all inside `HirTypeInference`** (sole owner)

438 workspace tests pass; fmt + clippy clean; all 11 bench
programs produce bit-identical output.

**§1.4 exit criteria — all satisfied:**
- ✅ Every existing type-dependent test passes.
- ✅ All 4 legacy type maps deleted from `SymbolTable` /
  `TypeEnvironment` (relocated to `HirTypeInference`).
- ✅ `apply_narrowings` / `restore_types` deleted (replaced by the
  stack-based push/pop API).
- ⚠️ "Type queries use a single `TypeTable` that is a pure
  function of the SSA IR" — PARTIALLY satisfied. `TypeTable`
  (post-SSA MIR) and `HirTypeInference` (in-flight HIR) coexist
  because lowering's in-flight decisions (allocation size, boxing,
  coercion) require pre-SSA type info that `TypeTable` cannot
  retroactively provide. A pipeline restructure that moves
  lowering post-SSA would unify the two; that is out of scope for
  §1.4 and deferred to a future architectural revision.

S1.9 (all four sub-sessions a/b/c/d) closes §1.4 to the extent it
can be closed without a pipeline restructure. The §1.4 work is
complete as written in the spec; the residual architectural
tension documented above is the spec's own tension, not an
implementation gap.

## 1.4u — Plan for single-source unification (2026-04-18 amendment) ⏳

§1.4's Non-Negotiable #4 — *"all type queries go through the single
pass output"* — is **not** fully satisfied by S1.9. Two independent
type-inference states coexist:

- **`HirTypeInference`** (HIR level, pre-SSA) — drives in-flight
  lowering decisions (allocation sizing, boxing, coercion). Its
  data flow is three legacy functions (`compute_expr_type`,
  `infer_expr_type_inner`, `infer_deep_expr_type`) that the S1.9b
  helpers only partially DRY'd. Multiple entry points, memoized +
  non-memoized recursion paths, optional param-type overlay.
- **`TypeTable`** (MIR level, post-SSA) — pure function of the SSA
  IR. Currently has **zero downstream consumers** — built per
  compilation behind `--emit-types`, otherwise unused.

The spec's vision requires a single source of truth. Two paths:

### Path A — HIR-level unified pass (Phase 1 completable)

Promote `HirTypeInference` to the single source of truth at HIR
level. The MIR `TypeTable` becomes a **derived view** seeded from
`HirTypeInference` via the SSA rename map (each MIR `LocalId` maps
back to a `VarId + version`; the VarId's HIR type is the seed).

Achievable in **four sessions** (each green-at-close,
non-regressing):

| Session | Scope | Estimated LOC |
|---|---|---|
| **§1.4u-a** | Collapse `compute_expr_type` / `infer_expr_type_inner` / `infer_deep_expr_type` into one `HirTypeInference::compute(&hir::Module)` method that walks HIR CFG in RPO with fixed-point iteration — same algorithmic shape as the MIR `TypeTable::infer_module`. The `Var`-arm unified via a parameter-lookup context struct (earlier S1.9b blocker). Keep the three legacy wrapper names as `#[deprecated]` shims delegating to `compute` for the transition. | +600 / -400 |
| **§1.4u-b** | Populate `HirTypeInference`'s four maps exclusively from the output of `compute`. Delete the legacy wrappers. Every lowering-side type query routes through `get_type_of_expr_id` → `HirTypeInference::lookup`. Result: lowering has exactly one type-query entry point; its backing data is produced by one pass. | +200 / -800 |
| **§1.4u-c** | Rewire `pyaot_optimizer::type_inference::TypeTable::infer_module` to build from `hir_types` + SSA rename map instead of running independent MIR-level inference. Keep the `apply_instruction` Phi/Refine/WPA extensions — those are MIR-specific refinements layered over the HIR seed. Net: the MIR TypeTable becomes a thin view over HirTypeInference. | +300 / -200 |
| **§1.4u-d** | Spec amendment: Non-Negotiable #4 of §1.4 reworded to accept "single source at HIR level with SSA-derived MIR view" as the canonical interpretation. Grep-verify no HIR-level type query runs outside the unified pass. Close §1.4 exit criteria cleanly. | +50 / -50 |

**Total estimate**: ~2200 LOC churn across 4 HIGH-complexity
sessions. Each session is independent after §1.4u-a lands.

**Exit criteria (§1.4u complete):**
- `HirTypeInference::compute` is the only function that produces
  HIR-level types; `compute_expr_type`, `infer_expr_type_inner`,
  `infer_deep_expr_type` do not exist.
- Every lowering query goes through `get_type_of_expr_id` /
  `HirTypeInference::lookup`; there is no alternate pathway.
- `pyaot_optimizer::type_inference::TypeTable::infer_module` is
  pure projection — it does no inference work itself beyond the
  SSA-specific Phi/Refine extensions that the HIR layer cannot
  express.

### Path B — MIR-level unified pass (Phase 2+, deferred)

The spec's literal wording ("single `TypeTable` that is a pure
function of the SSA IR") requires lowering to run **after** SSA
construction so that all type queries go through the MIR
TypeTable. That needs:

- Lowering emits type-agnostic MIR (e.g. `rt_generic_add(x, y)`
  instead of `rt_int_add(x, y)` / `rt_float_add(x, y)`). This is
  naturally enabled by **Phase 2's tagged Values**: when every
  runtime value is a uniform tagged `Value`, the runtime-call
  descriptors no longer bifurcate by operand type, and lowering
  doesn't need pre-SSA types to pick the right one.
- Codegen specialises tagged-value operations based on the post-
  SSA TypeTable (Phase 2's fast-path inlining). This is already
  specified in §2.5.

**Path B is therefore scheduled to land naturally during Phase 2
codegen work.** Once Phase 2 completes, Path A's HIR-level
`HirTypeInference` becomes redundant (all type decisions post-
SSA) and can be deleted as part of the Phase 2 cleanup milestone.

### Decision

**Phase 1 ships Path A.** Schedule §1.4u-a through §1.4u-d after
§1.6 (WPA params) and §1.7 (WPA fields) complete — those sessions
already work on top of the current dual-state, and the §1.4u work
doesn't block them. Earliest reasonable kickoff: after S1.12 lands.

**Phase 2 completes Path B as a natural byproduct** of the
tagged-value migration (§2.3 runtime migration + §2.5 codegen
migration). At that point `HirTypeInference` becomes deletable
dead code; Phase 2's final-purge milestone (§2.7) handles that
removal.

If Path A turns out harder than estimated (the §1.4u-a `Var`-arm
unification in particular may surface semantic differences that
weren't obvious in S1.9b), the fallback is to skip Path A entirely
and ship Phase 1 with the documented residual tension, accepting
that Phase 2 closes it. That is not the preferred path — it would
mean Non-Negotiable #4 stays violated for the lifetime of Phase 1
— but it is acceptable under the spec's amendment protocol
because Phase 2 provides a concrete resolution.

**Spec amendment required**: before §1.4u-a kicks off, edit
§1.4's "Non-Negotiable" paragraph to recognise Path A's HIR-level
unification as equivalent to the spec's literal post-SSA-only
formulation. Without this edit, §1.4u-d's grep-verify will flag
the remaining `HirTypeInference` residents as violations of the
spec's wording.

### Session roadmap entries to add

Once §1.12 closes, append these rows to the Phase 1 session
inventory (between S1.17 and S1.17b):

| ID | Scope | Deps | Complexity | Parallel? |
|----|-------|------|------------|-----------|
| S1.4u-a | HirTypeInference::compute — single unified HIR pass (§1.4u-a) | S1.12 | **HIGH** | — |
| S1.4u-b | Lowering reads exclusively from HirTypeInference::lookup (§1.4u-b) | S1.4u-a | **HIGH** | — |
| S1.4u-c | MIR TypeTable as SSA-rename projection of HIR layer (§1.4u-c) | S1.4u-b | Medium-High | Parallel-safe with S1.4u-d spec edit |
| S1.4u-d | Spec amendment + grep-verify (§1.4u-d) | S1.4u-c | Low | — |

All four are serial — they form a single linear dependency chain.

**Handoff to S1.9c** (map → table migration): replace the 4
`SymbolTable` maps (`prescan_var_types`,
`per_function_prescan_var_types`, `narrowed_union_vars`,
`refined_var_types`) with fields on a new `HirTypeInference`
struct owned by `Lowering`. Migrate the ~15 map call sites.
Delete the `SymbolTable` fields.

**Handoff to S1.9d** (narrowing stack + final cleanup): rewrite
`apply_narrowings` / `restore_types` as scoped push/pop on the
`HirTypeInference` narrowing stack. Delete `narrowing.rs`'s
standalone helpers. Grep-verify every legacy name (the 4 maps, the
3 functions) returns zero matches. Close §1.4 exit criteria.

## 1.5 Call graph construction ✅

**Milestone goal**: pyaot knows, for every function, its full set of
call sites and callers.

**Work**:

- Add `crates/optimizer/src/call_graph.rs`:
  ```rust
  pub struct CallGraph {
      pub callers: IndexMap<FuncId, Vec<CallSite>>,
      pub callees: IndexMap<FuncId, Vec<CallSite>>,
      pub sccs: Vec<Vec<FuncId>>,  // strongly-connected components
  }
  ```
- Build by walking MIR for every function, collecting `CallDirect`
  and `CallIndirect` (through function pointers / closures).
- Compute SCCs via Tarjan's algorithm; SCC roots are topologically
  ordered for bottom-up passes.
- **Closure calls** via function pointers: conservatively add edges
  to every function whose address is taken. (Devirtualization later
  prunes these.)

**Non-negotiable**: the call graph is the single source of truth for
"who calls who". No ad-hoc call-site enumeration in other passes.

**Exit criteria**:

- Call graph built for the sample workload; verified against
  hand-traced small programs.
- SCC computation tested on functions with recursion (direct,
  mutual).

**Status (2026-04-18, S1.10 landed)**: `crates/optimizer/src/call_graph.rs`
implements `CallGraph { callers, callees, sccs, address_taken }` with
`CallGraph::build(&Module) -> CallGraph` in O(V+E). Direct edges from
`InstructionKind::CallDirect` are tracked precisely; indirect edges
from `InstructionKind::Call` (function-pointer operand) and virtual
edges from `CallVirtual`/`CallVirtualNamed` conservatively fan out
to every function whose address has been taken via `FuncAddr`
(`CallKind::{Direct, Indirect, Virtual}` is stamped on each
`CallSite`, so consumers can filter). `RuntimeCall` is intentionally
excluded — runtime-library calls don't feed into WPA decisions.
SCCs computed by an iterative Tarjan avoiding recursion-depth
issues on deeply-connected modules; output is reverse-topological
(leaves first), matching the spec's "bottom-up" ordering for S1.11.
Every function in `module.functions.keys()` appears in both the
`callers` and `callees` maps (possibly with empty `Vec`), so
consumers can iterate without `unwrap_or_default` dance. 6 new
tests in `call_graph::tests` cover: empty module, isolated
singleton, linear 3-chain reverse-topological ordering, direct
self-recursion → one SCC, mutual recursion f0↔f1 isolated from f2,
and `FuncAddr`-induced address-taken set + indirect-call fan-out.
All 410 workspace tests pass; `cargo fmt --check` / `cargo clippy
--workspace --release -- -D warnings` clean.

## 1.6 Whole-program parameter type inference ✅

**Milestone goal**: unannotated function parameters receive their
types from the join of all call-site argument types.

**Work**:

- New pass `WpaParamInference` (runs after `CallGraph` is built, in
  SCC topological order from leaves up):
  1. For each function `f` with unannotated params:
     - For each call site `f(a1, a2, ...)`:
       - Get type of each arg from caller's SSA type inference.
     - Join across sites: `param_type[i] = join(args_across_sites[i])`.
  2. If the join changed the param type: mark `f`'s SSA inference
     as stale; re-run on `f` with the new param type.
- Handle SCCs: within an SCC, iterate all functions to fixed point.

- **Caller ABI implication**: if param type changes from `Any` to
  `Union[Int, Float]`, the MIR local for that param is re-declared
  (no longer `Any`, now the Union). The caller's argument-passing
  instructions may need to coerce (Phase 2 makes this uniform via
  tagged values; in Phase 1 it's handled by explicit box/unbox which
  will be deleted in Phase 2).

**Non-negotiable**: unannotated params are inferred, never defaulted
to `Any` at use sites. The pass runs to fixed point.

**Exit criteria**:

- `def f(x): return x + 1` called only with ints — `x` inferred as
  `Int`.
- `def f(x): return x + 1` called with int and float — `x` inferred
  as `Union[Int, Float]`.
- Recursive/mutually-recursive functions converge correctly.

**Status (2026-04-18, S1.11a landed — WPA parameter inference core)**:
`crates/optimizer/src/type_inference.rs` exposes
`wpa_param_inference(&Module, &CallGraph, &mut TypeTable)`. The pass:

1. Resets every directly-called function's param entries in the
   `TypeTable` to `Type::Never` (lattice bottom) so the fixed-point
   ascent widens monotonically upward — without this step a
   recursive self-call picks up the pre-WPA seed (typically `Any`)
   on its first pass and poisons the join forever, since
   `Type::unify_field_type`/`normalize_union` don't simplify
   `Union([Any, Int])` to `Any`. Functions with **no** direct
   caller keep their original seed.
2. Iterates `CallGraph::sccs` in **reverse** order (roots → leaves)
   so callers stabilise before callees. Within each SCC, iterates
   to fixed point bounded by `MAX_WPA_SCC_ITERATIONS = 16`.
3. For each function, walks `CallGraph::callers[f]`, filters to
   `CallKind::Direct` edges (indirect / virtual are skipped —
   devirtualisation later promotes specific sites), and for each
   site: fetches the exact `InstructionKind::CallDirect { args, .. }`
   instruction in the caller's MIR at `(block, instruction)` and
   joins each `arg[i]` type (via `operand_type(arg, caller_types)`)
   into `joined[i]` using a local `wpa_join_types` helper that
   special-cases `Never`/`Any` on top of `Type::unify_field_type`.
4. Builds a `seed_overrides` map keyed by param `LocalId` → joined
   type and re-runs intra-procedural inference via
   `infer_function_with_seed(func, Some(module), &overrides)` —
   this is a new public entry point refactored out of
   `infer_function` so WPA can inject its param seeds without
   touching `func.locals`.
5. Writes the updated `FunctionTypes` back via a new
   `TypeTable::set_function_types` method; only returns `true` from
   `refine_function_params` if the map actually differed, so the
   outer fixed-point terminates.

4 new unit tests cover: single-call-site narrowing (Any → Int),
multi-call-site join (Int + Float → Float via numeric tower),
uncalled-function seed preservation (Any stays Any), and
self-recursive SCC convergence (external Int + recursive self =
Int, not Union). 425 workspace tests pass; fmt + clippy clean;
bench output bit-identical — still no consumer wiring.

What S1.11 does **not** yet cover (reserved for later):
- **Field inference (S1.12 / §1.7)**: analogous pass over `__init__`
  call sites to refine class field types.
- **Indirect/virtual call WPA**: currently filtered out. After
  devirtualisation (S1.15) rewrites known receivers to `CallDirect`
  they'll be picked up automatically.
- **Feedback to inference of call-site arg types**: if the caller's
  type changes as a result of WPA on another function, the caller's
  call-site arg types change too. The current iteration order
  (reverse-topological across SCCs, fixed-point within each SCC)
  doesn't re-visit earlier SCCs — could cause missed refinements
  for deeply nested call chains. The spec says "iterate to fixed
  point with the whole-program call graph"; a full-program
  fixed-point wrapper around `wpa_param_inference` is a trivial
  extension when needed.

## 1.7 Whole-program field type inference ✅ (S1.12 landed 2026-04-18)

**Status**: `wpa_field_inference` + `wpa_param_and_field_inference_to_fixed_point`
landed in `crates/optimizer/src/type_inference.rs`. Class metadata
projected from `LoweredClassInfo` into `mir::Module.class_info:
IndexMap<ClassId, ClassMetadata>` at end of lowering; optimizer reads/
writes through there. Five new unit tests cover single write, numeric-
tower promotion (Int/Float/Bool → Float), unrelated-type union, no-init
no-op, and param-type propagation. Exit criterion verified end-to-end:
`Value(3) / Value(3.5) / Value(True)` → `Value.data: Float`;
`Box("hi") / Box(42)` → `Box.x: Union[Str, Int]`.

**Deletions deferred**: `scan_stmts_for_self_fields` and
`infer_field_type_from_rhs` still populate the initial
`LoweredClassInfo.field_types`; WPA refines after the fact. Codegen
still reads lowering-time field types (pre-WPA). Full swap to
`module.class_info` lives with the §1.4u pipeline restructure when the
lowering→optimizer flow is inverted.


**Milestone goal**: class fields get their type from the join of all
`__init__` argument types across all `ClassName(...)` call sites, not
from the first write encountered during scanning.

**Work**:

- New pass `WpaFieldInference` (runs after param inference):
  1. For each class `C`:
     - Collect all call sites of `C(...)`.
     - From each call site, get arg types.
     - `C.__init__`'s param types are now known (from 1.6). Propagate
        into field assignments `self.field = expr` where `expr`'s type
        depends on the param.
     - Field type = `join(expr_type across all init sites)`.
  2. Update `ClassInfo::field_types` with inferred results.
  3. If any field type changed, mark dependent functions stale and
     re-run their SSA type inference.
- Iterate to fixed point with the whole-program call graph (class
  methods may call each other, which may affect field initializers,
  etc.).

**Delete**:

- `scan_stmts_for_self_fields` as a one-shot "first-write wins" pass.
  Replaced by the whole-program fixed-point.
- `infer_field_type_from_rhs` as an ad-hoc heuristic.

**Non-negotiable**: if a class is instantiated with diverse arg types,
the field type IS `Union[...]` — no "first-write wins" shortcut.

**Exit criteria**:

- `Value(3)`, `Value(3.5)`, `Value(True)` all appearing → `Value.data`
  inferred as `Union[Int, Float, Bool]` (or `Float` if numeric-tower
  promotion applies).
- Recursive classes (tree nodes, list links) converge.
- `test_classes.py` field-inference tests pass.

## 1.8 Pass migration 🟡 (S1.13, S1.14a, S1.15 landed 2026-04-18)

**Status**: DCE + constfold migrated (S1.13). Inlining's CallGraph
unified (S1.14a). Peephole/devirtualize/flatten_properties audit +
SSA-aware idempotent rules (S1.15). The remaining piece is S1.14b
(SSA-preserving inliner rewrite) — deferred because it requires a
pipeline reorder (construct_ssa before optimize) and is best paired
with S1.16.

**S1.15 findings**: all three passes are already SSA-compatible. The
peephole pass is pure local pattern matching — no multi-def
assumptions; devirtualize reads `locals[id].ty` (the SSA-preserved
seed type); flatten_properties matches MIR shape. Added SSA-aware
idempotent rules to `match_binop_same_operand`: `x & x → Copy(x)` and
`x | x → Copy(x)`. LocalId identity is sufficient for value equality
under SSA's single-def invariant.

**Deferred to §1.4u** (requires TypeTable threaded through the
optimizer pipeline):
- Devirtualize could consult the TypeTable for Refine-narrowed
  receivers (e.g. after `isinstance`), not just `locals[id].ty`.
- Flatten_properties could leverage `module.class_info` directly
  instead of re-detecting trivial getters via MIR pattern match.

**S1.14a findings**: inlining kept a private `CallGraph` in
`inline/analysis.rs` with a naïve DFS `is_recursive`. The canonical
call graph (landed S1.10 for WPA) already tracks callers, callees,
and SCCs. Added `CallGraph::is_recursive(func)` that checks direct
self-loops and direct-edge SCC membership (≥2 members), skipping
indirect/virtual edges to avoid the over-approximation that spuriously
marks innocent functions as recursive. Test `test_recursive_detection`
remained green without modification.

S1.13 findings: the existing DCE pass (`crates/optimizer/src/dce/`) was
already SSA-style — `liveness.rs` walks uses, marks reachable,
deletes unreachable. No rework needed beyond leveraging SSA invariants.

Constfold gained:

1. **Unified propagation map** (`build_propagation_map`) — returns
   `PropValue::{Const, Alias(LocalId)}`. Copy-of-local substitutes the
   source local; copy-of-constant and direct Const substitute the
   literal. Transitive alias resolution with cycle guard.
2. **Phi-all-same-const folding** — when every incoming source of a
   Phi resolves to the same constant, the Phi collapses to a `Const`.
3. **Refine-with-constant-src folding** — a `Refine` whose `src` has
   propagated to a constant collapses to `Const`.
4. **Dropped the `def_count` filter** from `build_constant_map`
   (renamed to `build_propagation_map`) — SSA guarantees single-def so
   the filter was redundant.

6 new unit tests cover copy-alias propagation, copy-chain transitive
resolution, Phi-all-same-const fold, Phi-distinct-consts stays, Refine
fold, and Phi-through-propagation fold.


**Milestone goal**: every existing optimization pass consumes SSA MIR
and the new type table.

**Work**: for each pass in `crates/optimizer`:

- **DCE (`dce`)**: SSA makes DCE trivial — a local is dead iff no
  use. Remove the current ad-hoc liveness analysis; replace with
  "walk uses, mark reachable, delete unreachable".
- **Constant folding (`constfold`)**: SSA enables value numbering.
  Replace the current AST-style walker with a standard reverse-
  postorder pass.
- **Inlining (`inline`)**: with call graph + SSA, inlining becomes
  "clone callee's SSA into caller, rename versions, reconnect CFG".
  The current implementation predates both; rewrite.
- **Peephole (`peephole`)**: SSA enables more patterns. Extend.
- **Devirtualize (`devirtualize`)**: class hierarchy + call graph make
  devirtualization exact. Current implementation is best-effort;
  tighten.
- **Flatten properties (`flatten_properties`)**: now benefits from
  field type inference being exact.

**Non-negotiable**: no pass uses legacy type maps after migration.
Every pass queries `TypeTable` and walks SSA.

**Exit criteria**:

- All passes green under `cargo test`.
- Each pass's code is shorter than pre-migration (SSA simplifies).
- No references to deleted symbol-table maps anywhere.

## 1.9 Codegen migration ✅ (S1.16 audit landed 2026-04-18)

**Status**: Phi/block-params wiring landed in S1.5. S1.16 audit
confirmed there is **no manual phi emulation** left to delete — the
codegen uses Cranelift's `Variable` API (`declare_var` / `def_var` /
`use_var`) throughout, which defers SSA construction to Cranelift's
own `FunctionBuilder`. Under MIR's SSA invariant (every `LocalId` is
single-def after `construct_ssa`), Cranelift's SSA pass on these
Variables is trivial: no Phi insertion, no block-param synthesis, no
dominator-frontier walk beyond the initial pass.

**Variable API kept for now.** A full switch to direct
`IndexMap<LocalId, Value>` tracking is a performance optimization
(skip Cranelift's redundant SSA pass), not a correctness fix. 12
def_var / use_var call sites across
`function.rs` / `context.rs` / `utils.rs` / `exceptions.rs` /
`runtime_calls/mod.rs`. Migration touches every instruction-emit path
plus GC and exception flows. Deferred to a future dedicated session.

Stale comment cleanup: `phi_branch_args` doc in `terminators.rs` no
longer claims S1.5 is "preparation for S1.6" — both landed.


**Milestone goal**: Cranelift backend consumes SSA MIR with Phi
instructions.

**Work**:

- Cranelift IR already uses block parameters (its native SSA). Map
  MIR Phi to Cranelift `block.append_block_param(ty)` on the
  successor, and `jump(block, [args])` on the predecessor.
- Remove all "create a local and copy into it at each branch" code
  in `codegen-cranelift/src/instructions.rs` — that was emulating
  Phi manually. Delete.
- GC shadow-stack generation: SSA makes liveness precise. Only
  live-at-call SSA values need to be on the shadow stack — this
  shrinks GC roots, reducing overhead.

**Non-negotiable**: Cranelift's SSA and MIR's SSA align 1:1. No
intermediate "flatten SSA" step.

**Exit criteria**:

- Generated code is correct for all tests.
- Benchmarks (Phase 0.1) show **no regression** — SSA is a strict
  improvement for Cranelift's downstream passes.

## 1.10 Cleanup + final purge ⏳

**Milestone goal**: the codebase contains zero artifacts of the pre-
SSA era.

**Work**: `grep` for and delete every reference to:

- `prescan_var_types`, `per_function_prescan_var_types`
- `narrowed_union_vars`, `refined_var_types`
- `insert_var_type`, `get_var_type` (replaced by `TypeTable::typeof`)
- `apply_narrowings`, `restore_types`
- `get_or_create_local` (replaced by SSA rename + explicit
  `LocalId` tracking)
- `scan_stmts_for_self_fields` (replaced by WPA field inference)
- `insert_var_closure`, `get_var_closure` (closures are inlined
  Phi-compatible SSA values or explicit closure objects)
- Any other ad-hoc narrowing / unification helper

For each deletion: verify no call sites remain via `cargo build`;
remove the definition; remove related state from constructors.

**Exit criterion**: `grep -rn 'prescan_var_types\|narrowed_union_vars\|refined_var_types\|apply_narrowings\|restore_types' crates/ | wc -l` returns `0`.

## 1.11 Deferred HIR-tree deletion — S1.17b scope ⏳

**Milestone goal**: delete `Function.body: Vec<StmtId>`,
`StmtKind::{If, While, ForBind, Try, Match}`, and the tree→CFG bridge
`crates/hir/src/cfg_build.rs`. After this milestone, HIR carries **only**
a CFG — the legacy statement-tree is gone.

**Current state (2026-04-19 scoping audit)**:

- `Function.blocks: IndexMap<HirBlockId, HirBlock>` + `entry_block:
  HirBlockId` are populated at every function-construction site via
  `cfg_build::build_cfg_from_tree` (6 frontend call sites:
  `ast_to_hir/{mod,functions,classes,lambdas,comprehensions,statements/
  nested_functions}.rs` + 2 generator call sites in
  `lowering/src/generators/desugaring.rs`). No consumer reads this CFG;
  it is dead weight carried alongside the legacy tree.
- `Function.body` is the canonical source. It is read by **24 files**
  across frontend, lowering, semantics, and generator desugaring — 114
  `StmtKind::{If|While|ForBind|Try|Match}` match arms + 43 direct
  `.body` accessors.
- The tree→CFG bridge (`cfg_build.rs`) uses three deliberate
  simplifications that prevent any consumer from using the CFG
  meaningfully:
  1. `ForBind` uses `iter` as a placeholder branch cond — no has-next /
     next primitive.
  2. `Try` handlers are emitted as unreachable blocks — no exception
     edges.
  3. `Match` cases are linearised — no pattern-dispatch terminator.

**§1.1 open questions — scoping resolutions**:

### Q1. ForBind iteration primitive

Three options considered:

| Option | Description | Verdict |
|---|---|---|
| **A** | New `ExprKind::IterHasNext(ExprId)` (pure) + `StmtKind::IterAdvance { iter, target }` (binds next value). For-loop header branches on `IterHasNext`; body is prefixed by `IterAdvance`. | **chosen** |
| B | New `HirTerminator::Iterate { iter, target, body_bb, exit_bb }` fused terminator. | deferred (special-cases every consumer) |
| C | Explicit `next()` call that throws `StopIteration`, caught by an exception edge. | rejected (requires modelling exception edges, Q2) |

Chosen: **A**. Rationale: (i) terminator set stays minimal (`Jump, Branch,
Return, Raise, Yield, Unreachable`); (ii) matches existing MIR emission
which already emits separate `rt_iter_has_next` / `rt_iter_next` runtime
calls today — zero lowering semantic drift; (iii) `IterHasNext` is a pure
expression cacheable in `HirTypeInference.expr_types`; (iv)
`IterAdvance` is a straight-line binding statement fitting cleanly into
`HirBlock.stmts`. Tuple-unpack / starred targets in for-loops continue to
use the existing `BindingTarget` infrastructure.

For-loop CFG shape under Scheme A:

```
  pre:     ... ; Goto(header)
  header:  Branch(IterHasNext(iter) ? body_enter : else_or_exit)
  body_enter: IterAdvance { iter, target }; <body stmts>; Goto(header)
  else:    <else stmts>; Goto(exit)       # only if for-else present
  exit:    ...
```

### Q2. Exception edges

**Decision**: keep exception edges **implicit** — handlers are tracked
per-function in a new side map, not as CFG edges. Rationale:

- The runtime already uses setjmp/longjmp for exception dispatch;
  modelling exception edges in the CFG would require per-call shadow
  edges to the handler, exploding predecessor counts and polluting
  SSA / dom-tree analyses with noise.
- S1.2 already treats handlers as unreachable CFG blocks; no consumer
  relies on exception edges. Blessing this design explicitly is
  zero-work for the existing pipeline.
- Lowering's `ExcPushFrame` / `ExcPopFrame` markers are sufficient to
  connect runtime unwinding to handler bodies without CFG involvement.

Representation change: add

```rust
pub struct TryScope {
    pub try_blocks: Vec<HirBlockId>,    // body blocks guarded by handlers
    pub else_blocks: Vec<HirBlockId>,   // also guarded
    pub handlers: Vec<ExceptHandler>,   // handler bodies (reuse existing struct)
    pub finally_blocks: Vec<HirBlockId>, // runs on every exit path
    pub span: Span,
}

pub struct Function {
    // ... existing fields except `body` ...
    pub try_scopes: Vec<TryScope>,      // NEW — source-order, may nest
}
```

`ExceptHandler.body: Vec<StmtId>` is replaced by `entry_block:
HirBlockId`; the handler entry block is an ordinary CFG block (with no
predecessor edges from the CFG, entered only via runtime unwinding).

### Q3. Match statement

**Decision**: desugar to an if/else ladder at HIR construction time.
No `HirTerminator::Switch` variant.

Rationale: pattern matching already has its own complex `Pattern` AST
whose runtime checks today produce multiple control-flow predicates.
Desugaring to `Branch(cond)` lets every consumer use the same
terminator primitive. The desugaring moves out of lowering and into
`frontend-python/ast_to_hir/statements/match_stmt.rs`, where it
becomes a ladder of `Branch(IsMatchPattern(subject, pattern), body_bb,
next_case_bb)` with one final `Jump(merge_bb)` at the end of each case.
Pattern binding writes (`MatchAs { name: Some(v) }`, `MatchStar`, ...)
become straight-line `StmtKind::Bind` statements at the head of the
case body block.

One new HIR primitive is required: `ExprKind::MatchPattern { subject,
pattern }: bool` — a pure boolean predicate encoding "does `subject`
match `pattern`". Lowering for `MatchPattern` is existing work from
`lowering/src/statements/match_stmt/mod.rs`, repackaged as expression
emission.

### Schema changes summary

| Addition | Where | Purpose |
|---|---|---|
| `ExprKind::IterHasNext(ExprId)` | `hir/lib.rs` | For-loop header test |
| `StmtKind::IterAdvance { iter, target }` | `hir/lib.rs` | Advance + bind loop target |
| `ExprKind::MatchPattern { subject, pattern }` | `hir/lib.rs` | Per-case match predicate |
| `Function::try_scopes: Vec<TryScope>` | `hir/lib.rs` | Handler side map |
| `ExceptHandler::entry_block: HirBlockId` | `hir/lib.rs` | Replace `body: Vec<StmtId>` |

| Deletion | Where |
|---|---|
| `Function::body: Vec<StmtId>` | `hir/lib.rs` |
| `StmtKind::If` | `hir/lib.rs` |
| `StmtKind::While` | `hir/lib.rs` |
| `StmtKind::ForBind` | `hir/lib.rs` |
| `StmtKind::Try` | `hir/lib.rs` |
| `StmtKind::Match` | `hir/lib.rs` |
| `MatchCase::body: Vec<StmtId>` | `hir/lib.rs` (replaced by `entry_block`) |
| `ExceptHandler::body: Vec<StmtId>` | `hir/lib.rs` (replaced by `entry_block`) |
| `crates/hir/src/cfg_build.rs` | whole file |
| `pub mod cfg_build;` declaration | `hir/lib.rs:7` |

### Consumer inventory

114 `StmtKind::{If|While|ForBind|Try|Match}` match arms + 43 `.body`
accessors, spread across four migration domains:

**A. Frontend emitters** (construct the HIR; must emit CFG directly):

| File | Lines | Tree uses |
|---|---|---|
| `frontend-python/src/ast_to_hir/statements/control_flow.rs` | 166 | `If`, `While` |
| `frontend-python/src/ast_to_hir/statements/loops.rs` | 47 | `ForBind` |
| `frontend-python/src/ast_to_hir/statements/exceptions.rs` | 177 | `Try` |
| `frontend-python/src/ast_to_hir/statements/match_stmt.rs` | 290 | `Match` (desugar to if/else) |
| `frontend-python/src/ast_to_hir/statements/context_managers.rs` | 374 | `If` + `Try` (with-stmt desugar) |
| `frontend-python/src/ast_to_hir/statements/mod.rs` | — | dispatch |
| `frontend-python/src/ast_to_hir/comprehensions.rs` | — | `If` + `ForBind` (list/dict/set comp desugar) |
| `frontend-python/src/ast_to_hir/expressions/mod.rs` | — | `ForBind` (gen-expr desugar) |
| 6 frontend `cfg_build::build_cfg_from_tree` callers | — | retire these |

**B. Generator desugaring** (synthesizes StmtKind from scratch):

| File | Lines | Tree uses |
|---|---|---|
| `lowering/src/generators/desugaring.rs` | 1808 | 17 matches + 7 `.body` |
| `lowering/src/generators/for_loop.rs` | — | `ForBind` detection |
| `lowering/src/generators/while_loop.rs` | — | `While` detection |
| `lowering/src/generators/vars.rs` | — | 4 matches + 2 `.body` |
| `lowering/src/generators/utils.rs` | — | 3 matches |

**C. Lowering core** (tree → MIR; must consume HIR CFG blocks 1:1):

| File | Lines | Tree uses |
|---|---|---|
| `lowering/src/statements/mod.rs` | 138 | dispatch (5 matches) |
| `lowering/src/statements/match_stmt/mod.rs` | 217 | `Match` pattern expansion (moves to frontend) |
| `lowering/src/statements/loops/bind.rs` | 378 | `lower_for_bind` |
| `lowering/src/exceptions.rs` | 896 | `lower_try` (4 matches) |
| `lowering/src/expressions/builtins/mod.rs` | — | 1 match |

**D. Type-planning dataflow walkers** (order-dependent; must port to CFG
traversal with dominance-aware state):

| File | Lines | Tree uses | Order-dep? |
|---|---|---|---|
| `lowering/src/type_planning/mod.rs` | 507 | 11 matches (`collect_return_types`, `collect_handler_binds_in_stmts`) | no — pure collection |
| `lowering/src/type_planning/container_refine.rs` | 462 | 14 matches | **yes** — "Bind `x = []` before `x.append(e)`" needs RPO sequence |
| `lowering/src/type_planning/closure_scan.rs` | 923 | 10 matches | **yes** — loop-body closures see loop-target types |
| `lowering/src/type_planning/local_prescan.rs` | 374 | 5 matches | **yes** — `loop_depth` heuristic (§A.6 #3 post-loop rebind) |
| `lowering/src/type_planning/ni_analysis.rs` | 213 | 5 matches | no — any-path reachability |
| `lowering/src/type_planning/lambda_inference.rs` | 279 | via `.body` | no — signature-only |

**E. Semantic analyzer**:

| File | Lines | Tree uses |
|---|---|---|
| `semantics/src/lib.rs` | 434 | 5 matches (`loop_depth` / `except_depth` counters) |
| `semantics/src/tests.rs` | — | 6 matches (test fixtures — rewrite to use CFG builders) |

### Migration stages

The tree deletion must retire consumers in order: CFG schema → emitters
→ consumers → deletion. Each stage is a session; each session is gated
by passing `cargo test --workspace --release` + SSA checks on all 470+
tests + all `examples/*.py` compiling bit-identically.

**Stage 1 — HIR schema extension (S1.17b-a)**
- Add `ExprKind::IterHasNext`, `ExprKind::MatchPattern`,
  `StmtKind::IterAdvance`.
- Add `Function::try_scopes`, `TryScope` struct, and
  `ExceptHandler::entry_block` alongside existing `body`.
- Keep the legacy `StmtKind::{If, While, ForBind, Try, Match}` and
  `Function::body` intact. No consumer change.
- Update `Stmt`/`Expr` pretty-printers for debug.
- Estimated +400 LOC, 0 deletions. Complexity: Low. Risk: Low (pure
  additive).
- **Exit gate**: build + tests clean.

**Stage 2 — Frontend emits CFG directly (S1.17b-b)**
- Rewrite each of `control_flow.rs`, `loops.rs`, `exceptions.rs`,
  `match_stmt.rs`, `context_managers.rs`, `comprehensions.rs`,
  `expressions/mod.rs` to directly allocate `HirBlock`s + terminators
  and no longer produce the legacy `StmtKind::{If, While, ForBind, Try,
  Match}` variants.
- Match statement: desugar to if/else ladder using `MatchPattern`
  predicate; emit bindings into case entry blocks.
- For-loops: emit header (`Branch(IterHasNext(iter))`) + body-entry
  (`IterAdvance`) + exit shape.
- Try: build body/else/finally blocks; register `TryScope` on the
  enclosing function; handler `entry_block` is an ordinary block with
  no CFG predecessors.
- During this stage the legacy tree is still built in parallel (via
  `build_cfg_from_tree` on each emitted stmt list) as a bridge for
  stages 3–5; consumers read from whichever form they want.
- **Invariant test**: the CFG emitted directly equals the CFG emitted
  by `build_cfg_from_tree` on an equivalent legacy tree. Add 10–15
  fixtures. Gated by a `debug_assert_eq!` in a `#[cfg(test)]` helper.
- Retire the 6 `cfg_build::build_cfg_from_tree` frontend call sites;
  keep the 2 generator call sites until Stage 4.
- Estimated +1500 / -800 LOC. Complexity: High. Risk: High (behavioral
  parity).
- **Exit gate**: tests + SSA checks clean; parity test passes on all
  frontend fixtures.

**Stage 3 — Lowering core consumes HIR CFG (S1.17b-c)**
- Rewrite `lowering/src/statements/mod.rs` dispatch to iterate blocks
  in RPO and emit one MIR block per HIR block, with the terminator
  translating directly.
- Delete `lower_if`, `lower_while`, `lower_for_bind`, `lower_try`,
  `lower_match` — their functionality collapses into straight-line
  statement lowering + terminator translation.
- `lowering/src/exceptions.rs` reads `Function::try_scopes` for
  `ExcPushFrame` / `ExcPopFrame` placement.
- Pattern lowering from `statements/match_stmt/` becomes
  `ExprKind::MatchPattern` emission (the predicate functions are what
  remain; case chaining is gone — done by frontend).
- `lower_for_bind`'s iter-protocol plumbing (`rt_iter_has_next`,
  `rt_iter_next`) becomes the standard lowering of `IterHasNext` and
  `IterAdvance`.
- Estimated +600 / -1200 LOC. Complexity: High. Risk: High (codegen
  correctness).
- **Exit gate**: tests + SSA checks clean; all `examples/*.py` compile
  and run bit-identically; Cranelift verifier passes.

**Stage 4 — Type planning walkers + generator desugar (S1.17b-d)**
- `ni_analysis.rs`, `lambda_inference.rs`,
  `type_planning/mod.rs::{collect_return_types, collect_handler_binds}`
  — pure forward-walk; port to BFS over CFG blocks.
- `container_refine.rs` — rewrite the "find `x = []`, then
  `x.append(e)`" pattern as a per-block linear scan, joined at merge
  points in RPO order. An empty-list bind that is refined by an
  append in a dominated block keeps the refinement; a refinement on a
  branch that merges loses refinement (same semantics as today when
  the append is inside an `if`). Use `DomTree`-like structure (HIR
  has no dom tree today; add one or reuse via a `hir_blocks_rpo()`
  helper).
- `local_prescan.rs` — `loop_depth` replaces with "inside a block
  reachable from a back-edge". Add a `hir_loop_depth(block_id)`
  helper computed once per function via natural-loop detection on the
  CFG. §A.6 #3 post-loop rebind: "variable was first written in a
  block with loop_depth > 0 and then in a block with loop_depth == 0"
  — semantics unchanged.
- `closure_scan.rs` — loop-target types reach body closures via RPO
  walk with per-block variable-type carry; merges at join blocks
  use `Type::unify_field_type` (same as today).
- `generators/desugaring.rs` + `vars.rs` + `for_loop.rs` +
  `while_loop.rs` + `utils.rs`: `VarTypeMap::build`, `collect_yield_info`,
  `detect_for_loop_generator`, `detect_while_loop_generator`,
  `collect_generator_vars` — convert to walk `Function::blocks` +
  terminators. The generator state machine synthesizes `HirBlock`s
  directly; the 2 `build_cfg_from_tree` calls in `desugaring.rs`
  retire.
- Estimated +1200 / -1500 LOC. Complexity: High. Risk: High (dataflow
  subtleties for refinement / prescan; the post-loop-rebind heuristic
  in particular is a pragmatic divergence from strict Python
  semantics that must be preserved).
- **Exit gate**: tests + SSA checks clean; microgpt.py triage status
  unchanged or improved; all existing narrowing / refinement
  regression fixtures pass.

**Stage 5 — Semantic analyzer (S1.17b-e)**
- `semantics/src/lib.rs`: swap the `loop_depth` / `except_depth`
  counters for "is this block inside a loop / handler region" queries
  computed from the CFG + `Function::try_scopes`.
- `semantics/src/tests.rs`: update fixtures to use `HirBlock` builders
  (or call into `cfg_build::build_cfg_from_tree` as a test convenience
  — but note: Stage 6 deletes that helper, so these tests must be
  migrated to emit CFG directly before Stage 6).
- Estimated +200 / -150 LOC. Complexity: Low-Medium. Risk: Low.
- **Exit gate**: tests clean.

**Stage 6 — Delete tree (S1.17b-f)**
- Remove `Function::body`, `StmtKind::{If, While, ForBind, Try, Match}`,
  `MatchCase::body`, `ExceptHandler::body`.
- Delete `crates/hir/src/cfg_build.rs` + `pub mod cfg_build;`
  declaration.
- Grep verify: `grep -rn 'StmtKind::\(If\|While\|ForBind\|Try\|Match\)\|build_cfg_from_tree\|Function.*\.body' crates/` returns only the removal diff itself.
- Update `CLAUDE.md`, `.claude/rules/architecture.md`, `INSIGHTS.md`
  "Unified Binding Targets" section, and this doc's §1.11 dashboard
  row.
- Estimated -500 / -580 LOC net (delete cfg_build.rs).
  Complexity: Low. Risk: Low (dead-code removal).
- **Exit gate**: grep clean; tests + SSA + benchmarks within ±3%.

### Aggregate estimates

| Stage | LOC + | LOC − | Net | Risk |
|---|---|---|---|---|
| S1.17b-a schema | +400 | 0 | +400 | Low |
| S1.17b-b frontend | +1500 | -800 | +700 | High |
| S1.17b-c lowering core | +600 | -1200 | -600 | High |
| S1.17b-d walkers + gen | +1200 | -1500 | -300 | High |
| S1.17b-e semantics | +200 | -150 | +50 | Low-Medium |
| S1.17b-f delete | 0 | -1080 | -1080 | Low |
| **Total** | **+3900** | **-4730** | **-830** | — |

Six sessions. Each session ≤ 1500 LOC changed per the session-split
trigger rule. Deletion is net progress in LOC terms (−830) and
removes one entire module (`cfg_build.rs`).

### Readiness gate for S1.17b-a start

- [x] §1.4u ✅ (landed 2026-04-19)
- [x] S1.9 ✅ (HirTypeInference owns all type maps)
- [x] SSA gates active in debug builds
- [ ] **Final prerequisite**: benchmark baseline re-captured with 10
  samples to lock Phase 1 floor before the refactor churn starts.
  Runs as the first task of S1.17b-a.

### Risk registry

1. **Post-loop rebind heuristic drift.**
   `local_prescan.rs` currently detects "first-written inside a loop,
   later rebound outside" via tree nesting. The CFG port must detect
   the same scenario via natural-loop detection. Drift here silently
   changes generated code for idioms like `for _, c in pairs: ...;
   c = Class()`. Gate: include the existing
   `examples/test_classes.py` §G.13 fixtures in stage-4 acceptance.

2. **Container-refinement ordering.**
   Today the refinement walks `stmts[i+1..]` in source order. In CFG
   form, the `stmts[i+1..]` scan becomes "successors of the current
   block, bounded to blocks that still have `x = []` as the current
   binding". Merge points must keep the refinement only if all
   predecessors agree. Drift risk: list/set/dict types regress from
   concrete to `Any` under if/else branching, breaking runtime elem-tag
   dispatch. Gate: `examples/test_collections.py` +
   microgpt's topo-sort refinement pattern.

3. **Generator state-machine emission.**
   `generators/desugaring.rs` synthesizes StmtKind trees and then feeds
   them through `cfg_build::build_cfg_from_tree`. Stage 4 emits the
   CFG directly, bypassing the bridge. Drift risk: yield-resume block
   boundaries, `gen_var` liveness, and SSA-consistent slot assignment.
   The existing `examples/test_generators.py` + `test_iteration.py`
   coverage is sufficient if (and only if) all 13 generator fixtures
   stay green through the stage.

4. **Cranelift verifier violations from changed block shape.**
   MIR blocks map 1:1 from HIR blocks post-Stage 3. If the HIR CFG has
   degenerate shapes (e.g. empty blocks, back-to-back Jumps) that the
   tree→CFG bridge accidentally avoided, the Cranelift verifier may
   surface them. Fallback: add a MIR-level block-simplification pass
   before codegen (eliminate empty Jump-only blocks) — this is
   standard optimiser hygiene anyway.

5. **Scope of semantics::tests.rs fixtures.**
   The test file uses hand-built tree fixtures. Migrating them to
   CFG-form is ~300 LOC of test-only code but risks masking
   regressions if done carelessly. Mitigation: keep the semantic
   checks as "run analyzer on module, expect Err(X)"; swap the fixture
   builder under the hood.

### Deletion verification command

Final grep gate at the end of S1.17b-f:

```
grep -rn 'StmtKind::\(If\|While\|ForBind\|Try\|Match\)\|build_cfg_from_tree\|Function::body\|\.body: Vec<StmtId>' crates/ examples/ tests/ | grep -v '^Binary'
```

Must return zero non-diff lines.

## Phase 1 Acceptance

Before merging Phase 1's long-lived branch to master:

1. **All tests green**: `cargo test --workspace --release` zero failures.
2. **Benchmarks non-regressed**: every benchmark from Phase 0.1 is
   within ±3% of baseline, or faster.
3. **SSA property checker** runs on every function and passes.
4. **Deletion audit**: 4 legacy maps + 6+ ad-hoc helpers are physically
   removed. Diff must show lines removed > lines added, measured in
   `symbol_table.rs`, `narrowing.rs`, `type_planning/`.
5. **microgpt.py diagnostic**: compile `microgpt.py`, record which
   errors remain, triage them as "Phase 2 target" / "Phase 3 target" /
   "unrelated". No expectation microgpt.py fully compiles yet.
6. **Document any spec deviations**: if Phase 1 reality diverged from
   the spec, amend this document before merge.

---

# Phase 2 — Unified Tagged Value Representation

**Duration**: 4–7 weeks.

**Goal**: every runtime value is a uniform 64-bit tagged word. The
compiler, runtime, and GC treat values through a single API. The five
legacy representations (raw i64, float-bits, boxed primitive, heap
pointer, list-elem-tag dispatch) collapse into one.

**After Phase 2**, the following legacy mechanisms MUST be deleted:

- `ELEM_RAW_INT`, `ELEM_RAW_BOOL`, `ELEM_HEAP_OBJ` constants.
- `TupleObj::heap_field_mask`.
- `ClassInfo::heap_field_mask`.
- `GeneratorObj::type_tags`.
- `rt_make_int`, `rt_make_float`, `rt_make_bool`, `rt_unbox_int`,
  `rt_unbox_float`, `rt_unbox_bool`, `rt_is_int`, `rt_is_float`,
  `rt_is_bool` — all replaced by uniform `rt_value_tag` / inlined
  tag tests.
- `box_primitive_if_needed`, `promote_to_float_if_needed`,
  `coerce_to_field_type`, `is_useless_container_ty` — all meaningless
  once values are uniformly tagged.
- `ValueKind` MIR enum — no longer needed, tag is self-describing.
- `type_to_value_kind` in `runtime_selector` — gone.

## 2.1 Tag scheme finalization

**Milestone goal**: select, document, and commit to a specific tag
scheme.

**Options evaluated**:

- **NaN-boxing**: f64 native, Int 48-bit in NaN payload, pointer
  48-bit. Good for float-heavy workloads. Limits Int to 48 bits.
- **Low-bit tagging**: 8-byte-aligned pointers, tag in low 3 bits.
  61-bit Int, 48-bit float (boxed) or separate float-tag encoding.
- **Low-bit + spare high**: Mac ARM64 gives 47-bit VA; use high 17
  bits for tag. Allows 63-bit Int, unboxed float (via NaN-box in
  high bits).

**Decision — NON-NEGOTIABLE**: use **low-bit tagging**. Rationale:

1. Portable (works on x86_64, ARM64 without bitfield tricks).
2. Int remains 61 bits — enough for all Python-compatible `int`
   operations short of arbitrary precision (and arbitrary precision
   is a pyaot non-goal).
3. Float boxed (tagged pointer to f64 box). For float-heavy workloads
   we can stack-allocate the box when liveness permits (SSA makes
   this analysis possible — Phase 1 dividend).

**Tag scheme**:

```
Bit 0: is_not_pointer (1) | is_pointer (0)
If is_not_pointer:
  Bits 1-2: type (00=Int, 01=Bool, 10=None, 11=reserved)
  Bits 3-63: payload
If is_pointer:
  All 64 bits: pointer to heap object (aligned to 8 bytes, so low 3 bits are always 0)
```

Document this in `crates/core-defs/src/tag.rs` with compile-time
asserts that the encoding is correct.

**Exit criterion**: `tag.rs` committed with constants, helpers,
property tests.

## 2.2 Core-defs API

**Milestone goal**: define the universal tagged-value API.

```rust
#[repr(transparent)]
pub struct Value(pub u64);

impl Value {
    pub const NONE: Value = /* tagged None */;
    pub const FALSE: Value = /* tagged Bool(false) */;
    pub const TRUE: Value = /* tagged Bool(true) */;

    #[inline] pub fn from_int(i: i64) -> Value { ... }
    #[inline] pub fn from_bool(b: bool) -> Value { ... }
    #[inline] pub fn from_ptr(p: *mut Obj) -> Value { ... }

    #[inline] pub fn is_int(self) -> bool { ... }
    #[inline] pub fn is_bool(self) -> bool { ... }
    #[inline] pub fn is_none(self) -> bool { ... }
    #[inline] pub fn is_ptr(self) -> bool { ... }

    /// Panics in debug if not int. Returns raw i64 in release.
    #[inline] pub fn unwrap_int(self) -> i64 { ... }
    /// Same for other types.
    #[inline] pub fn unwrap_bool(self) -> bool { ... }
    #[inline] pub fn unwrap_ptr(self) -> *mut Obj { ... }

    /// Runtime type — for polymorphic dispatch.
    #[inline] pub fn runtime_type(self) -> TypeTagKind { ... }
}
```

**Float handling**: no `from_float` — floats are always heap-boxed as
`*mut FloatObj`. Escape analysis in Phase 3 stack-allocates when
possible.

**Non-negotiable**: all runtime code uses `Value` not `i64`, `*mut Obj`,
or `f64` directly. `Value` is the sole currency.

**Exit criterion**: `Value` type defined, exhaustive tests for every
constructor/extractor, compile-time assertions on encoding.

## 2.3 Runtime migration

**Milestone goal**: every `crates/runtime/src/*.rs` function signature
uses `Value` for its arguments and return type (where they previously
used `i64` or `*mut Obj`).

**Work**:

- Rename and retype every `rt_*` function. E.g.:
  ```rust
  // Before:
  pub extern "C" fn rt_list_push(list: *mut Obj, value: i64) { ... }
  // After:
  pub extern "C" fn rt_list_push(list: Value, value: Value) { ... }
  ```
- Update every operation to use `Value::unwrap_*` internally for
  typed access.
- **Delete**: `rt_make_int`, `rt_box_int` (now `Value::from_int`).
  Same for float/bool. Delete `rt_unbox_int` etc. — use
  `Value::unwrap_int`.
- **Delete**: `ELEM_RAW_INT` / `ELEM_HEAP_OBJ` constants. Lists,
  tuples, dicts all store `Value` uniformly.
- **Delete**: `heap_field_mask` on `TupleObj` / `ClassInfo`. GC reads
  each field as `Value`, uses `is_ptr()` to decide whether to trace.
- **Delete**: `GeneratorObj::type_tags`. Each local slot is a
  `Value`, same uniform treatment.
- **Delete**: `rt_tuple_get_int`, `rt_tuple_get_float`,
  `rt_tuple_get_bool`. One function `rt_tuple_get(t, idx) -> Value`.
  The consumer unwraps.

**Non-negotiable**: no `i64`-typed runtime entrypoint remains. Grep
for `extern "C" fn rt_.*i64` must return zero results.

**Exit criterion**: runtime crate passes all tests. Binary size of
runtime staticlib stays within +10% of pre-migration (may grow
slightly from `Value` wrapping, should be negligible after inlining).

## 2.4 GC migration

**Milestone goal**: the garbage collector marks through `Value::is_ptr`,
not through type tags or heap masks.

**Work**:

- Rewrite `gc.rs::mark_object` to:
  1. Receive a `Value`.
  2. If `!v.is_ptr()`, return (nothing to mark).
  3. Otherwise, follow the pointer, mark the object, recurse into
     fields.
- When marking compound objects (Tuple, List, Dict, Instance,
  Generator), iterate over the stored `Value`s and call
  `mark_object` on each. `is_ptr()` self-describes.
- **Delete** all uses of `heap_field_mask`, `heap_mask`, `type_tags`
  in `gc.rs`. The GC no longer has any "is this field a pointer?"
  ambiguity.

**Non-negotiable**: the GC has exactly one `is_pointer` predicate,
used uniformly. Removing this code should shrink `gc.rs` by 30%+.

**Exit criterion**: GC tests pass. Stress test (1M allocations with
mixed types) runs correctly. No `heap_mask` reference anywhere in
`crates/runtime`.

## 2.5 Codegen migration

**Milestone goal**: Cranelift codegen emits uniform `Value`-typed IR.

**Work**:

- Every MIR `LocalId` lowers to a Cranelift `Value` of Cranelift type
  `I64` (since `Value` is `#[repr(transparent)] u64`).
- **Fast-path inlining**: for hot operations, emit inline tag tests:
  ```
  // a + b where SSA says both are Int:
  v1 = raw_sub(a, INT_TAG)     // extract payload
  v2 = raw_sub(b, INT_TAG)
  v3 = iadd(v1, v2)
  // check overflow, fall through to slow path on overflow
  result = iadd(v3, INT_TAG)   // re-tag
  ```
- **Slow path**: call `rt_value_add(a, b) -> Value` which does full
  polymorphic dispatch.
- SSA type info (Phase 1) dictates whether to inline fast-path or
  call slow-path. If `a: Int, b: Int` statically → fast. If
  `a: Any, b: Any` → slow.

**Non-negotiable**: no codegen path uses `ValueKind` or
`type_to_value_kind`. The runtime type is self-describing through
tagging.

**Exit criterion**:

- Codegen passes all tests.
- Arithmetic benchmarks within ±5% of Phase 1 baseline. (Slight
  regression acceptable due to tag manipulation; if >5%, revisit
  fast-path inlining.)
- Polymorphic-dunder benchmarks **improve** (Union-typed args
  no longer need boxing dance).

## 2.6 Pass migration

**Milestone goal**: every optimization pass drops its ad-hoc
boxing/coerce logic.

**Work**:

- `box_primitive_if_needed` — **delete**. Tagged Value is already
  uniform.
- `promote_to_float_if_needed` — **delete**. Numeric promotion is a
  runtime decision (handled by `rt_value_add` slow path) or an SSA
  type-inference decision (Phase 1 handles).
- `coerce_to_field_type` — **delete**. Writing to a field is just
  storing a `Value`; the receiver doesn't care about compile-time
  type (runtime tag handles dispatch).
- `is_useless_container_ty` — **delete**. Container types are not
  representation-dependent.

**Non-negotiable**: no `if ty == Type::Int { box }` dispatches anywhere.
If a pass needs to know "is this value boxed?" — the answer is "all
values are uniformly encoded; use `Value::is_ptr`".

**Exit criterion**:

- Every call site of the deleted helpers is gone.
- `grep -rn 'box_primitive\|promote_to_float\|coerce_to_field\|is_useless_container' crates/` returns 0.

## 2.7 Final purge

**Exit criteria for Phase 2**:

- `grep -rn 'ELEM_RAW_INT\|ELEM_HEAP_OBJ\|heap_field_mask\|heap_mask\|type_tags\|ValueKind\|type_to_value_kind' crates/ | wc -l` returns `0`.
- All `rt_*` entrypoints use `Value`.
- `Value` is the single representation type in the codebase.
- Benchmarks (Phase 0.1) show:
  - Int/bool arithmetic: within ±3% of pre-Phase-2 baseline.
  - Float arithmetic: within ±10% (may regress slightly, mitigated
    by escape analysis in Phase 3).
  - Polymorphic arithmetic: **improved** by 20%+ (no boxing dance).
  - GC scan time: **improved** by 15%+ (no mask lookups).

---

# Phase 3 — Type Lattice + Monomorphization

**Duration**: 3–5 weeks.

**Goal**: types form a proper mathematical lattice with generic type
variables, structural typing (Protocol), and monomorphization of
generic call sites.

**After Phase 3**, the following legacy helpers MUST be deleted:

- `Type::unify_field_type` (ad-hoc).
- `Type::unify_numeric` (ad-hoc).
- `Type::unify_tuple_shapes` (ad-hoc).
- `Type::promote_numeric` (ad-hoc).
- `Type::normalize_union` (ad-hoc).
- `Type::narrow_to` / `Type::narrow_excluding` (ad-hoc; use
  `meet` / `minus`).
- `types_match_for_isinstance` as a standalone — replaced by
  `is_subtype`.

## 3.1 Lattice core API

**Milestone goal**: `types` crate exposes a proper lattice API.

```rust
pub trait TypeLattice: Sized + Clone + Eq {
    /// Universal supertype. `Any` in Python terms.
    fn top() -> Self;
    /// Universal subtype. Never / `typing.Never`.
    fn bottom() -> Self;
    /// Join: least upper bound. `Union` in most cases.
    fn join(&self, other: &Self) -> Self;
    /// Meet: greatest lower bound. `Intersection` in most cases.
    fn meet(&self, other: &Self) -> Self;
    /// Subtype relation. `self ≤ other`.
    fn is_subtype_of(&self, other: &Self) -> bool;
    /// `self \ other`: subtract a type from a union.
    /// Used for `isinstance` else-branch narrowing.
    fn minus(&self, other: &Self) -> Self;
}

impl TypeLattice for Type { ... }
```

Laws (enforced by the property tests activated from Phase 0.4):

- `join(top, t) == top`, `meet(top, t) == t`.
- `join(bot, t) == t`, `meet(bot, t) == bot`.
- Commutativity, associativity, idempotence of join/meet.
- `is_subtype_of(a, b) && is_subtype_of(b, c) ⟹ is_subtype_of(a, c)`.
- `is_subtype_of(a, b) ⟹ join(a, b) == b`.

**Work**:

- Rewrite `Type` operations through lattice primitives.
- Delete `unify_*` / `normalize_union` / `narrow_*` free helpers.
- All call sites go through `join` / `meet` / `is_subtype_of` / `minus`.

**Non-negotiable**: the lattice laws are tested (Phase 0.4 tests
activated here). A property-test failure is a blocker.

**Exit criterion**: lattice laws all pass. Lattice API is used
throughout (grep for deleted helpers returns 0).

## 3.2 TypeVar support

**Milestone goal**: `TypeVar`, `Generic`, and parameterized classes
are first-class.

```rust
pub enum Type {
    // ... existing variants ...
    Var(TypeVarId),  // NEW
    Generic { base: ClassId, args: Vec<Type> },  // NEW, supersedes ad-hoc List/Dict/Set/Tuple
}

pub struct TypeVar {
    pub id: TypeVarId,
    pub name: InternedString,
    pub bound: Option<Box<Type>>,        // upper bound (e.g. `T: int`)
    pub constraints: Vec<Type>,          // `T: int | str`
    pub variance: Variance,              // invariant | covariant | contravariant
}

pub enum Variance { Invariant, Covariant, Contravariant }
```

**Delete**: ad-hoc `Type::List(Box<Type>)`, `Type::Dict(K, V)`,
`Type::Set(T)`, `Type::Tuple(Vec<Type>)`, `Type::TupleVar(Box<Type>)`.
All become `Type::Generic { base: builtin_class_id, args: [...] }`.

**Rationale**: user-defined `class Stack(Generic[T])` and builtin
`list[T]` use the exact same representation.

**Non-negotiable**: after this milestone, there are no
type-specific variants for known-generic classes. `list[int]` and
`Stack[int]` are represented identically.

**Exit criterion**: all existing generic types (list, dict, set, tuple)
render as `Generic { ... }`. Tests pass. Frontend parses `class
Stack(Generic[T])` into this form.

## 3.3 Monomorphization pass

**Milestone goal**: every generic function/method has a specialized
copy per concrete type instantiation at call sites.

**Work**:

- New pass `MonomorphizePass` (runs after WPA from Phase 1.6 — i.e.,
  after all types are inferred):
  1. For each call site of a generic function/method:
     - Instantiate `T` with concrete arg types.
     - If this specialization doesn't exist, clone the function body,
       rename, substitute `T` with the concrete type.
     - Replace the call site's `FuncId` with the specialized one.
  2. Remove the generic "template" functions from the output (they are
     never called directly after monomorphization).

- **Recursion**: if a generic function is recursive in `T`, check for
  finite specialization. Infinite specialization → compile error.
- **Specialization dedup**: canonical key is `(FuncId, [concrete Type])`
  tuple.

**Non-negotiable**: by codegen time, no `Type::Var(_)` remains in any
function signature or body. All generic code is monomorphized.

**Exit criterion**:

- `def first[T](xs: list[T]) -> T: return xs[0]` — called with
  `list[int]` and `list[str]` → produces `first_int` and `first_str`
  specialized functions.
- Generic stdlib functions (`map`, `filter`, `reduce`, `sorted`) are
  defined generically, not hardcoded per-type.
- Codegen-pre-check asserts no `TypeVar` in any signature.

## 3.4 Protocol structural typing

**Milestone goal**: `isinstance(x, SomeProtocol)` checks for
structural conformance, not class-hierarchy membership.

**Work**:

- New HIR node `ClassDef { kind: ClassKind::Protocol, ... }`.
- Frontend parses `class P(Protocol): ...` correctly.
- `is_subtype_of(T, P)` where `P` is a Protocol:
  - For each abstract method `m` in `P`:
    - Does `T` have a method of the same name with compatible signature?
  - All methods match → subtype.
- Generate runtime type-check function that iterates vtable at
  runtime (slower than nominal but correct).

**Non-negotiable**: Protocol membership is structural. No manual
registration required.

**Exit criterion**: `test_classes.py` gets a Protocol test section.
`Addable`, `Sized`, `Iterable` Protocols work.

## 3.5 Frontend support

**Milestone goal**: Python `TypeVar`, `Generic`, `Protocol` imports
and syntax are parsed correctly.

**Work**:

- Handle `from typing import TypeVar, Generic, Protocol`.
- Parse `T = TypeVar('T', bound=...)` — binds a TypeVarId in scope.
- Parse `class Stack(Generic[T]):` — adds `T` as a type parameter
  of the class.
- Parse `class P(Protocol): ...` — marks class as structural.
- Parse `def fn[T](x: T) -> T:` (PEP 695 syntax) — same as
  `Generic[T]` scoped to the function.

**Non-negotiable**: syntax is parsed, types are tracked, monomorph
sees them. If a Python pattern is common (e.g., PEP 695 syntax),
support it.

**Exit criterion**: existing `typing`-module-dependent tests
continue to pass; new TypeVar/Generic/Protocol tests added.

## 3.6 Final purge

**Exit criteria for Phase 3**:

- `grep -rn 'fn unify_field_type\|fn unify_numeric\|fn unify_tuple_shapes\|fn promote_numeric\|fn normalize_union\|fn narrow_to\|fn narrow_excluding' crates/ | wc -l` returns `0`.
- All `Type` operations go through the `TypeLattice` trait.
- All generic code is monomorphized before codegen.
- Protocol tests pass.
- Property tests (lattice laws) all pass.

## 3.7 Performance gate

Phase 3 is mostly compile-time work, but monomorphization and
generic dispatch can affect both runtime perf and binary size. A
blanket regression check is required.

**Exit criteria**:

- Every benchmark from Phase 0.1 is within ±3% of the Phase 2
  post-merge baseline, or faster.
- Release-build binary size (`ls -la target/release/pyaot`) is
  within +20% of the Phase 2 baseline. Monomorphization adds
  specialized function copies — some growth is expected, but
  runaway inflation (> 20%) indicates either (a) overly-aggressive
  monomorphization (same type args producing redundant copies —
  dedup missing) or (b) generic functions with excessively
  divergent call-site types (reconsider signature). Both are
  fix-before-merge conditions.
- Compile time on a medium-sized input (e.g., `test_types_system.py`,
  ~900 LOC) is within +30% of Phase 2 baseline. Monomorph adds
  specialization work but should not dominate.

Record the baseline update in `bench/BASELINE.md` after Phase 3
merges — this becomes the new reference for any post-refactor work.

---

# Cross-Phase Artifacts

## Commit discipline

- Every commit builds, tests green.
- Commit messages: `phaseN.M: <milestone>: <imperative verb> <what>`.
  E.g., `phase1.3: SSA: insert φ-nodes via Cytron algorithm`.
- Each milestone is one or more commits; no milestone spans > 1500
  LOC diff (split if needed, but keep each split green).

## Branch strategy

- One long-lived branch per phase: `phase-1-ssa`, `phase-2-tagged-values`,
  `phase-3-lattice`. Rebase frequently on master.
- Do not merge a phase branch until **all exit criteria met**.
- Between phases: short stabilization window (1 week) on master for
  bug reports, benchmarks, release notes.

## Documentation updates

After each phase:

- Update `COMPILER_STATUS.md` to reflect new capabilities and
  remove limitations.
- Update `INSIGHTS.md` to document the new architecture; **delete**
  sections about the removed mechanisms (they are no longer
  relevant — don't keep obsolete knowledge).
- Update `.claude/rules/architecture.md` and
  `.claude/rules/api-reference.md`.
- Update `CLAUDE.md` / `CONVENTIONS.md` for any new conventions.

## Test gate automation

Add CI workflow `.github/workflows/refactor-gates.yml` (or local
`scripts/gate-check.sh`) that runs:

```
cargo fmt --check
cargo clippy --workspace --release -- -D warnings
cargo test --workspace --release
cargo bench --workspace  # compared against baseline
```

Every milestone push must pass this gate. Pre-commit hook recommended.

---

# Anti-Patterns to Reject Explicitly

During execution of this plan, the following temptations will arise.
**Reject them immediately**:

### "Let's add a flag for backwards compatibility"

No. Delete the old code. Fix all call sites. If something breaks, the
test suite catches it; fix it before merging the milestone.

### "This special case is rare; let's keep the legacy path for it"

No. Special cases that "keep the legacy path" are how architectures
rot. Integrate the special case into the new design, or explicitly
declare it out of scope in this document.

### "We can skip monomorphization for this specific generic"

No. Every generic is monomorphized. If a generic is genuinely
runtime-polymorphic (e.g., heterogeneous list), express it via
`Union` or `Protocol` — not via unmonomorphized `TypeVar`.

### "Let's postpone the cleanup until the next phase"

No. Each phase has an explicit cleanup milestone (1.10, 2.7, 3.6).
If cleanup is postponed, the next phase inherits landfill. Cleanup
is part of done, not optional.

### "microgpt.py line N still fails; let's add a quick fix"

No. Trace microgpt.py's failure to its architectural root cause.
If it belongs to a phase that's already complete, the fix is a
bug in that phase — revise that phase's spec and re-open it. If it
belongs to a future phase, document it and move on.

### "Benchmark regressed 8%; we'll optimize later"

No. Fix the regression before merging. "Optimize later" never happens.

### "The test is flaky; let's mark it flaky"

No. Investigate the flake. Flakiness indicates non-determinism,
which is a bug worth fixing before piling more work on the system.

### "This milestone is taking longer than expected; let's split the non-essential parts"

Partially acceptable if the split maintains all Non-Negotiable
Principles. Forbidden if the split leaves the codebase in a half-
migrated state. Any split MUST preserve: all tests green, no
legacy + new coexistence, clear exit criteria for both halves.

---

# Final Acceptance (All Phases Complete)

When Phases 0 through 3 are all merged to master:

1. The codebase has **net fewer lines** than before the refactor
   (by ~10–15% in `crates/lowering`, `crates/runtime`, `crates/types`).
2. `grep -rn` for every legacy helper/map listed in this document
   returns zero results across the workspace.
3. Every benchmark from Phase 0.1 is at baseline or faster.
4. The test suite is larger (Phase 0.2 coverage expansion +
   per-phase regression tests) and 100% green.
5. A representative ML/NLP Python file (microgpt.py or similar)
   compiles and runs correctly without modification.
6. A new developer can read `COMPILER_STATUS.md`, `INSIGHTS.md`,
   `.claude/rules/architecture.md`, and this document, and understand
   the full architecture in < 1 day.
7. This document is archived (marked `// Completed` in its header)
   and retained as historical record. Future refactors write their
   own master plans.

---

# Post-Refactor Feature Queue

Feature work that predates this refactor plan or arises during it is
**not** smuggled into a phase. It queues here and is scheduled after
all three phases merge to master. This section exists so items are
not forgotten — not because they are optional.

## Area F — Format Specification Protocol

See `MICROGPT_PLAN.md` §F for the original scoping.

**Why post-refactor, not woven in**:

- Area F's runtime centerpiece is `rt_format(value, spec) -> str`.
  Before Phase 2, `value` has five legacy representations (raw i64,
  float-bits, boxed primitive, heap pointer, elem-tagged container
  member). Writing the dispatch before Phase 2 means writing it
  twice — once for legacy reps, once for tagged `Value`.
- User-class `__format__` dispatch (Area F.6) is the canonical
  case for Phase 3's Protocol structural typing. Writing it
  pre-Phase-3 means ad-hoc dispatch, re-done via Protocol after.
- Constant-folding `f"{42:4d}"` → `"   42"` (Area F.5) needs
  flow-sensitive type info with literal-propagation — exactly
  what Phase 1's SSA + WPA provides. Pre-Phase-1, it is a
  special-case heuristic; post-Phase-1, it is a standard SSA
  constant-fold pass.

**Estimated effort**:

- Without refactor: 1-2 weeks (per §F plan).
- After all three phases: 3-5 days.
  - `rt_format(value: Value, spec: Value) -> Value` — uniform
    tagged dispatch.
  - `__format__` through Protocol — trivially structural.
  - F-string desugaring via SSA constant-fold — standard pass.
  - Removal of `Builtin::FmtHex`/`FmtOct`/`FmtBin`/`FmtIntGrouped`/
    `FmtFloatGrouped`/`Round` — folds into Phase 3 cleanup.

**When to schedule**: at least 1 stabilization week after Phase 3
merges. Revisit `MICROGPT_PLAN.md` §F, simplify to reflect the new
architecture, then implement as an independent feature milestone
with its own test suite and acceptance criteria.

**Non-negotiable (queue discipline)**:

- Do NOT pull Area F work into any refactor phase, even if it
  "looks like it would fit". Resist.
- Do NOT leave the legacy format builtins (`FmtHex`/etc.) in place
  "for now" during the refactor and plan to remove them in Area F.
  They are removed by Phase 3 cleanup (legacy builtin deletion is
  not feature work — it is architectural hygiene). Area F just
  builds the new `rt_format` on top of the cleaned-up base.
- If Area F is genuinely blocking a user need before the refactor
  completes, that is a signal to ship the current best-effort
  f-string support as a known limitation and still wait for the
  refactor — not to inline Area F.

## Other Queued Items

As feature requests land during the refactor, document them here.
Examples of what belongs:

- New stdlib module bindings (json schema validation, http client
  tuning, etc.) — feature work, post-refactor.
- Performance-tuning passes that aren't part of architecture
  (vectorization, auto-parallelization) — post-refactor.
- User-facing language features not yet supported (`async`/`await`
  concurrency, decorator factories, `typing.Literal`) — evaluate
  whether they are "feature work" (post-refactor) or "architectural
  gap" (amend relevant phase spec).

Examples of what does NOT belong here (these are architectural):

- Bug fixes to type inference — amend Phase 1 spec if discovered.
- Runtime representation inconsistencies — amend Phase 2 spec.
- Generic parameter issues — amend Phase 3 spec.

---

# Execution: Claude Code Session Roadmap

This section breaks the refactor into **agent-sized sessions**. A
session = one planning phase + one implementation phase, starting
from a clean context and ending with a green test suite plus one or
more green commits.

## Session Discipline

### Per-session rules (non-negotiable)

1. **One session, one clear deliverable.** A session ends when its
   stated goal is achieved OR when the session is explicitly
   aborted and rolled back. It does not end with "made progress,
   will finish next time" — that leaves the codebase in a broken
   state. Either merge the work or revert it.

2. **Start with a plan, end with tests green.** Every session
   begins in plan mode: read the relevant milestone section in this
   document, confirm scope, identify files, sketch approach, then
   implement. Every session ends with `cargo test --workspace
   --release` green, `cargo clippy` clean, `cargo fmt --check` clean.
   If tests cannot be made green, the session must be rolled back
   (not left broken "to continue later").

3. **Context boundary.** Each session starts fresh. Do NOT rely on
   conversation memory from a prior session. All context the next
   session needs must be either (a) in this document, (b) in the
   commit history, or (c) in `COMPILER_STATUS.md` / `INSIGHTS.md` /
   `MEMORY.md`. If a session discovers something the next session
   needs, it writes it down **before** the session ends.

4. **No cross-session WIP.** A session does not start a change, leave
   it half-done, and end "to be continued". It finishes or reverts.
   If a session discovers its scope is too large, the protocol is:
   (a) identify a smaller valid subgoal, (b) roll back everything
   else, (c) implement the subgoal, (d) close the session, (e)
   schedule the remainder as a new session.

5. **Milestone boundary = commit boundary.** A milestone (numbered
   §N.M in this document) maps to one or more commits **within one
   session**. No milestone spans multiple sessions unless the
   session roadmap below explicitly splits it into sub-sessions
   with named IDs.

6. **Benchmark after every perf-relevant session.** Runtime / GC /
   codegen sessions end by running `cargo bench` and recording
   deltas in `bench/BASELINE.md`. Regressions over thresholds in
   §Non-Negotiable Principle 6 block the session close.

### Session sizing guidance

| Session complexity | Estimated LOC diff | Estimated walltime |
|--------------------|--------------------|--------------------|
| Low                | < 300              | 1-3 hours          |
| Medium             | 300-1000           | 3-6 hours          |
| High               | 1000-2000          | 6-10 hours         |
| **Split required** | > 2000             | —                  |

A session trending toward >2000 LOC must split. The split point is
usually obvious (add-types-first, migrate-callers-second;
infrastructure-first, consumers-second).

**When in doubt, split.** A smaller well-scoped session is always
better than a sprawling one. The overhead of planning a new session
is tiny compared to the risk of a broken commit.

### Session kickoff protocol

Start of every session:

1. Read `ARCHITECTURE_REFACTOR.md` (this document) — at minimum the
   milestone section and the Non-Negotiable Principles.
2. Read latest `git log --oneline -20` to understand what landed
   recently.
3. Read `COMPILER_STATUS.md` for current capability state.
4. If the session is implementing an earlier-planned milestone,
   read that milestone's spec in full.
5. Plan mode: confirm scope matches the spec. If the spec needs
   amendment (see §Amendment Protocol), halt implementation and
   amend the document first.
6. Begin implementation only after the plan is concrete.

### Session exit protocol

End of every session:

1. `cargo build --workspace --release` — clean.
2. `cargo fmt --check` — clean.
3. `cargo clippy --workspace --release -- -D warnings` — clean.
4. `cargo test --workspace --release` — all green.
5. For perf-relevant sessions: `cargo bench --workspace` — within
   gates.
6. Commits in good shape: squashed where appropriate, messages
   following `phaseN.M: <milestone>: <verb> <what>` convention.
7. If the session closes a milestone: update `COMPILER_STATUS.md`
   and `INSIGHTS.md` accordingly (removing obsolete sections,
   adding new architecture notes).
8. If the session discovered a spec gap: amend
   `ARCHITECTURE_REFACTOR.md` in a dedicated commit.
9. Write a 2-3 sentence session summary somewhere a future session
   can find it (either in the milestone's commit message or in a
   scratch `SESSION_LOG.md` kept as a running journal).

### Context handoff between sessions

Each session starts cold. The handoff between consecutive sessions
happens **through the git history and this document**, not through
conversation context. Concretely:

- **Commit messages** carry the "why". Write them like you are
  explaining to a reviewer who has no prior context.
- **`COMPILER_STATUS.md`** describes the current state of each
  feature. After a session, it must reflect reality.
- **`INSIGHTS.md`** captures non-obvious design decisions. After a
  session that made such a decision, add an INSIGHTS section;
  conversely, remove obsolete sections for mechanisms the session
  deleted.
- **`ARCHITECTURE_REFACTOR.md`** (this document) describes intent.
  After a session, if the session diverged from the spec,
  explicitly amend the spec.

If the next session needs to know something that is not in any of
these three places, the previous session failed the handoff. This is
a protocol violation, not an optimization opportunity.

### Agent delegation strategy

A single **lead agent** (top-tier model — Opus or equivalent) drives
each session: reads the spec, makes architectural decisions, writes
design-sensitive code, coordinates the work. But a session includes
many mechanical tasks that do not require top-tier reasoning.
**Delegate these to Sonnet/Haiku subagents** to save cost, reduce
main-context pollution, and unlock within-session parallelism.

#### Delegate to cheaper subagents (Sonnet / Haiku)

Use the appropriate specialized agent type from the fleet (`Explore`,
`general-purpose`, `bug-hunter`, `code-reviewer`, `rust-pro`,
`python-pro`, `docs-research-expert`, etc.). Typical
delegable categories:

1. **Code exploration** — "where is symbol X defined?", "find all
   call sites of function Y", "what files in `crates/lowering` use
   pattern Z?". Best fit: `Explore` agent, Sonnet or Haiku.

2. **Grep-and-verify** — structured searches with clear success
   criteria: "confirm no references to `prescan_var_types` remain",
   "list all `extern "C" fn rt_*` signatures that still take
   `i64`". Runs on Haiku without loss.

3. **Mechanical refactoring** — bulk renames, deletion of a named
   set of symbols, updating their call sites per a given recipe.
   Give the subagent the exact transformation and target files;
   it applies them. Sonnet.

4. **Test suite execution** — run `cargo test --workspace
   --release`, report the failures in a structured summary (which
   tests, exit codes, first error line). Keeps raw test output
   out of the lead agent's context. Haiku.

5. **Benchmark runs** — execute `cargo bench`, collect numbers,
   compare against `bench/BASELINE.md`, produce a diff report.
   Haiku or Sonnet.

6. **Documentation drafting** — given a list of "what changed in
   this session", have a subagent rewrite the relevant section of
   `COMPILER_STATUS.md` / `INSIGHTS.md` / `.claude/rules/*.md`.
   The lead agent reviews and edits before committing. Sonnet.

7. **Coverage gap analysis** — run `cargo llvm-cov`, parse the
   report, list files under the threshold, optionally draft test
   stubs for the gaps. Sonnet.

8. **Lint/format cleanup** — run `cargo clippy`, address
   straightforward warnings (unused imports, redundant clones,
   shadowing). Non-trivial clippy feedback escalates to the lead
   agent. Haiku for trivial, Sonnet for borderline.

9. **Release-note / changelog drafts** after each milestone. Sonnet.

#### Keep with the lead agent

These **must not** be delegated. They require full spec context and
non-obvious judgment:

1. **Architectural decisions.** Tag scheme choice, lattice axioms,
   SSA representation details, Phi encoding, monomorphization
   deduplication strategy. The lead agent carries the rationale
   from this document; a subagent starts cold.

2. **Novel algorithm implementation.** Cytron SSA renaming,
   Cooper-Harvey-Kennedy dominator tree, φ-insertion via iterated
   dominance frontiers, Tarjan SCC for call graphs, Cytron's own
   φ-removal for codegen. These require understanding subtleties
   (edge cases in irreducible CFGs, recursive type substitution,
   etc.) that a cheaper model is likely to miss silently.

3. **Cross-crate API design.** Any interface touching ≥ 2 crates'
   public surface. Tradeoffs require awareness of downstream
   consumers.

4. **Debugging non-obvious regressions.** "Test fails after SSA
   migration, symptom unclear" — halt and investigate with the
   lead agent. Do not ask a subagent to "fix the failing test".

5. **Session scope adjudication.** "Is this session too big — do
   we split?" / "Is this ad-hoc helper acceptable or does it
   violate Principle 5?". Judgment calls.

6. **Spec amendments.** Changing
   `ARCHITECTURE_REFACTOR.md`'s milestones requires lead-agent
   sign-off (see Amendment Protocol). A subagent can draft an
   amendment, but the lead agent reviews and commits.

#### Parallelism from delegation

Delegation is the primary way to parallelize **within** a session.
Examples:

- **Session kickoff**: dispatch three subagents in parallel — one
  maps call sites of the target symbol, one reads prior related
  `INSIGHTS.md` sections, one runs the current test suite to
  establish "before" baseline. Lead agent synthesizes into a plan.
- **Session exit**: dispatch three subagents in parallel — one
  runs tests, one runs benchmarks, one does grep verification of
  deletions. Results combine before commit.
- **Between implementation chunks**: dispatch a subagent to
  investigate a sub-question while the lead agent continues on
  the main thread.

Always dispatch independent subagent calls **in a single message**
with multiple `Agent` tool uses — that is what makes them actually
run in parallel.

#### Escalation pattern

If a subagent reports "I cannot complete this" or returns
ambiguous results:

1. **Do not retry the same subagent with the same prompt.** Same
   model, same context, same failure.
2. The lead agent decides: either re-scope and re-dispatch (often
   with more explicit instructions or a smaller chunk), or pull
   the task in-house.
3. If a category of task repeatedly fails at the cheap tier,
   amend this section — it belongs in "keep with lead agent".

#### Delegation anti-patterns

- **Do not delegate the session plan itself.** Planning is
  core-judgment work. Subagents execute plan fragments, they
  don't author plans.
- **Do not ship subagent code unreviewed.** Even mechanical diffs
  can have subtle errors (missed call sites, wrong grep pattern,
  merge conflicts). Lead agent reviews every subagent-produced
  diff before commit.
- **Do not cascade.** Lead → subagents is fine. Subagent → deeper
  subagent is not: accountability chain becomes opaque, and the
  lead agent can't monitor. Keep the tree flat at depth 1.
- **Do not delegate based on cost alone.** Borderline cases
  (moderate judgment, unfamiliar code area) stay with the lead
  agent. Subagent missing a subtle bug erases the cost savings
  with interest via debugging time.
- **Do not use a subagent as an excuse to skip reading the spec.**
  Every session's lead agent reads this document. Delegation is
  for execution, not for dodging comprehension.

#### Task tracking

Use `TaskCreate` / `TaskUpdate` within the session to track
progress — independent of who executes each task. A subagent
returning results flips a task to complete; the lead agent moves
on. This keeps progress visible in the session transcript even
when multiple subagents run in parallel.

---

## Full Session Inventory

Sessions are numbered with `S<phase>.<idx>`. Dependencies are listed
explicitly. **"Parallel-safe"** means two sessions could run
simultaneously on different branches without stepping on each other
(safe to dispatch to two agents in parallel). **"Serial-only"** means
the next session must wait for the prior one to merge.

### Phase 0 — Preparation

| ID | Scope | Deps | Complexity | Parallel? |
|----|-------|------|------------|-----------|
| S0.1 | Benchmark harness (§0.1): create `bench/` crate, Python sources in `bench/py/`, runner, `BASELINE.md` skeleton and first baseline recorded | — | Medium | Parallel-safe with S0.2, S0.3 |
| S0.2 | Coverage audit + gap-filling tests (§0.2): run `cargo llvm-cov`, identify <70% areas, add tests | — | Medium-High (scales with gaps) | Parallel-safe with S0.1, S0.3 |
| S0.3 | Property checker stubs (§0.3 + §0.4): `ssa_check.rs` with no-op for legacy MIR, `lattice_props.rs` with `#[ignore]`d laws | — | Low | Parallel-safe with S0.1, S0.2 |

**Combined ok**: S0.1 and S0.3 can be one session if S0.3 is
small (each < 300 LOC). S0.2 must be its own session — coverage
audit often uncovers surprise gaps.

### Phase 1 — SSA MIR + Whole-Program Type Inference

| ID | Scope | Deps | Complexity | Parallel? |
|----|-------|------|------------|-----------|
| S1.1 ✅ | HIR CFG type definitions (§1.1 prep): add `HirBlock`, `HirBlockId`, `HirTerminator` alongside legacy `StmtKind` — both coexist | S0.* | Low-Medium | — |
| S1.2 ✅ | Frontend HIR-CFG migration (§1.1 main): convert `ast_to_hir/*.rs` to emit CFG; leaves old `StmtKind::If/While/ForBind/Try/Match` as bridge | S1.1 | **HIGH** | — |
| S1.3 ⏳ | CFG-portable consumer migration (§1.1 partial tail, **narrowed 2026-04-18**): move the lowering-core emission path (`statements/*`, `exceptions.rs`) and generator-desugar detection passes to walk HIR CFG. `Function.body` + legacy `StmtKind::{If, While, ForBind, Try, Match}` stay alive as a bridge; their deletion is deferred to the new S1.17b below. Never started — the post-S1.2 bridge already makes the CFG available, and downstream work skipped consumer migration, continuing to walk the legacy tree. Scope now folded into S1.17b (tree deletion forces all consumers to migrate at once). | S1.2 | **HIGH** | — |
| S1.4 ✅ | Dominator tree (§1.2): `crates/mir/src/dom_tree.rs`, Cooper-Harvey-Kennedy. Session row lists "Deps: S1.3" for conservative session ordering, but the actual code dependency is only "MIR structure unchanged" (which holds post-S1.2). Parallel-safe with S1.3. | S1.2 (code); S1.3 (ordering) | Medium | Parallel-safe with S1.3, S1.10 |
| S1.5 ✅ | Phi MIR instruction + codegen-side block-param support (§1.3 prep) | S1.4 | Medium | — |
| S1.6 ✅ | SSA renaming via Cytron algorithm (§1.3 main): rename all function bodies to SSA, activate SSA checker | S1.5 | **HIGH** | — |
| S1.7 ✅ | `Refine` instruction + isinstance narrowing at CFG successors (§1.4 prep) — only the `Refine` infrastructure landed; isinstance-Refine emission at CFG successors is queued as a future extension (no code consumer yet). | S1.6 | Medium | — |
| S1.8 🟡 | Unified `TypeInferencePass` (§1.4 main) — split into S1.8a (core engine), S1.8b (Const/Copy/CallDirect/GcAlloc rules), S1.8c-part (BinOp/UnOp + RuntimeCall + Call-indirect). The *single* unified match still coexists with the three legacy HIR functions; full collapse tracked by §1.4u-a/b/c/d (Phase 1 late or Phase 2 byproduct). | S1.7 | **HIGH** | — |
| S1.9 ✅ | Delete legacy type maps (§1.4 tail): purge `prescan_var_types`, `refined_var_types`, `narrowed_union_vars`, `apply_narrowings`, `restore_types`. Split into S1.9a (unified public entry points), S1.9b (shared result helpers), S1.9c (maps → `HirTypeInference`), S1.9d (narrowing stack push/pop). All 4 maps + the narrowing helpers deleted or relocated per §1.4 exit criteria; dual-state with `TypeTable` documented for §1.4u resolution. | S1.8 | Medium | — |
| S1.10 ✅ | Call graph (§1.5): `crates/optimizer/src/call_graph.rs`, SCCs via Tarjan | S1.3 | Medium | Parallel-safe with S1.4-S1.9 |
| S1.11 ✅ | WPA parameter inference (§1.6): fixed-point pass over call graph — core + full-program fixed-point wrapper both landed. | S1.9, S1.10 | **HIGH** | — |
| S1.12 ✅ | WPA field inference (§1.7): cross-call field type join. Projected class metadata into `mir::Module.class_info`; field inference scans `__init__` `rt_instance_set_field` writes, joins per offset. Paired with params in `wpa_param_and_field_inference_to_fixed_point`. | S1.11 | **HIGH** | — |
| S1.13 ✅ | Pass migration: DCE + constfold (§1.8 part 1). DCE was already SSA-style. Constfold gained: unified propagation map (constants + copy aliases with transitive resolution), Phi-all-same-const fold, Refine-with-const-src fold. Dropped def_count filter under SSA. 6 new tests. | S1.9 | Medium | Parallel-safe with S1.14-S1.15 (different passes) |
| S1.14a ✅ | Pass migration: inlining — CallGraph unification. Deleted inline-local `CallGraph` + `is_recursive` in `inline/analysis.rs`; both replaced by `optimizer::call_graph::CallGraph::is_recursive` (SCC-aware, direct-edge only to avoid indirect/virtual over-approximation). `FunctionCost::compute` now takes the canonical graph. | S1.13 | Low-Medium | — |
| S1.14b-prep ✅ | Pipeline reorder: `construct_ssa` moved before `optimize_module` in `crates/cli/src/lib.rs`. Added a post-optimize SSA check gate (debug-only) to catch any future pass that breaks SSA at its source. All tests green in both build modes — every optimizer pass tolerates SSA input. | S1.14a, S1.16 | Medium | — |
| S1.14b-inliner ✅ | Pass migration: inlining — SSA-preserving rewrite. `perform_inline` emits a `Phi` at the continuation block head merging return values from every value-returning callee path; void returns contribute `Constant::Int(0)` placeholders to preserve Phi arity. Replaces the pre-SSA `Copy(dest, val); Goto(cont)` pattern that produced multi-def MIR. | S1.14b-prep | Medium | — |
| S1.15 ✅ | Pass migration: peephole, devirtualize, flatten_properties (§1.8 part 3). Audit showed all three are already SSA-compatible: peephole is local-pattern, devirtualize reads `locals[id].ty` (seed is preserved under SSA), flatten_properties matches MIR patterns. Added SSA-aware idempotent peephole rules: `x & x → x` and `x | x → x` (keyed on LocalId identity — valid under SSA single-def). 3 new tests. TypeTable-aware devirtualize (post-Refine narrowing) and class_info-aware flatten deferred to §1.4u (pipeline restructure). | S1.9 | Medium-High | Parallel-safe with S1.13, S1.14 |
| S1.16 ✅ | Codegen SSA migration (§1.9): audit found no manual phi emulation. Codegen uses Cranelift's `Variable` API which handles SSA conversion internally; under MIR single-def invariant this is trivial. Fixed one stale S1.5-prep comment in `terminators.rs`. Full `Value`-based migration (skip the Variable layer for ~12 call sites) deferred — pure performance optimization, not correctness. | S1.6, S1.15 | Medium-High | — |
| S1.17 ⏳ | Phase 1 final cleanup + acceptance (§1.10): grep-verify deletions, benchmark check, docs update | S1.11, S1.12, S1.16, S1.17b | Low-Medium | — |
| S1.17b ⏳ | **Deferred §1.1 tail — HIR tree deletion umbrella** (scoped 2026-04-19 per §1.11). Split into six sub-sessions below; tracks ~4,730 LOC deleted + ~3,900 added, net −830. Design questions (HirTerminator iteration gap, exception edges, match desugar) resolved in §1.11. Prerequisites: §1.4u ✅, S1.9 ✅. | S1.8 | High | — |
| S1.17b-a ⏳ | HIR schema extension (§1.11 Stage 1): add `ExprKind::IterHasNext`, `ExprKind::MatchPattern`, `StmtKind::IterAdvance`, `Function::try_scopes`, `TryScope`, `ExceptHandler::entry_block` alongside legacy variants. Pure additive; no consumer change. | §1.4u | Low | — |
| S1.17b-b ⏳ | Frontend emits CFG directly (§1.11 Stage 2): rewrite `ast_to_hir/statements/{control_flow,loops,exceptions,match_stmt,context_managers}.rs` + `comprehensions.rs` + `expressions/mod.rs` to allocate `HirBlock`s + terminators without emitting `StmtKind::{If, While, ForBind, Try, Match}`. Match desugars to if/else ladder via `MatchPattern`. Retire 6 frontend `cfg_build::build_cfg_from_tree` call sites. Parity test: new CFG equals bridge output. Legacy tree still built as bridge. | S1.17b-a | **HIGH** | — |
| S1.17b-c ⏳ | Lowering core consumes HIR CFG (§1.11 Stage 3): rewrite `lowering/src/statements/mod.rs` dispatch for per-block RPO emission; delete `lower_if/while/for_bind/try/match`; port `exceptions.rs` to read `Function::try_scopes`; repackage pattern predicates from `statements/match_stmt/mod.rs` as `ExprKind::MatchPattern` emission. | S1.17b-b | **HIGH** | — |
| S1.17b-d ⏳ | Walkers + generator desugar (§1.11 Stage 4): port `ni_analysis.rs`, `lambda_inference.rs`, `type_planning/mod.rs::{collect_return_types, collect_handler_binds}`, `container_refine.rs`, `closure_scan.rs`, `local_prescan.rs` to walk HIR CFG with dominance/loop-aware state. Rewrite `generators/desugaring.rs` + siblings to synthesize `HirBlock`s directly; retire the 2 generator `build_cfg_from_tree` call sites. | S1.17b-c | **HIGH** | — |
| S1.17b-e ⏳ | Semantics (§1.11 Stage 5): replace `loop_depth` / `except_depth` counters in `semantics/src/lib.rs` with CFG queries; migrate `semantics/src/tests.rs` fixtures to CFG builders. | S1.17b-d | Low-Medium | Parallel-safe with S1.17b-f prep |
| S1.17b-f ⏳ | Delete tree (§1.11 Stage 6): remove `Function::body`, `StmtKind::{If, While, ForBind, Try, Match}`, `MatchCase::body`, `ExceptHandler::body`; delete `crates/hir/src/cfg_build.rs`; grep-verify zero residue; update `CLAUDE.md` / `.claude/rules/architecture.md` / `INSIGHTS.md`. Final deletion diff. | S1.17b-e | Low | — |

**Split triggers**:

- **S1.2** (frontend CFG migration): if a single session exceeds
  1500 LOC, split along grammar boundaries: S1.2a = `if`/`while`
  conversion; S1.2b = `for`/`try`/`match`; S1.2c = generators.
- **S1.6** (SSA renaming): if diff exceeds 1500 LOC, split:
  S1.6a = straight-line functions; S1.6b = loops + branches;
  S1.6c = generators + closures + cell-vars.
- **S1.8** (TypeInferencePass): if coverage audit finds many
  call-site variations, split: S1.8a = core inference engine;
  S1.8b = dunder/class dispatch; S1.8c = stdlib edge cases.

**Combined ok (rare)**:

- S1.1 + S1.2 can be one session if the HIR type additions are small
  (< 200 LOC) and the frontend migration is compact. Otherwise
  separate.
- S1.9 (delete legacy maps) can be merged into S1.8 if the migration
  naturally leaves no call sites. Verify by `grep` before closing
  the session.

### Phase 2 — Unified Tagged Value Representation

| ID | Scope | Deps | Complexity | Parallel? |
|----|-------|------|------------|-----------|
| S2.1 | Tag scheme design + `core-defs/Value` API (§2.1 + §2.2): low-bit tagging constants, `Value` type, constructors, extractors, property tests | Phase 1 merged | Medium | — |
| S2.2 | Runtime migration: primitives (§2.3 part 1): Int, Bool, None — `rt_make_*` / `rt_unbox_*` replaced by Value methods | S2.1 | Medium | Parallel-safe with nothing (hot path) |
| S2.3 | Runtime migration: List + basic list ops (§2.3 part 2): drop `ELEM_RAW_INT` / `ELEM_HEAP_OBJ`, store Value uniformly | S2.2 | Medium-High | — |
| S2.4 | Runtime migration: Dict, Set, Tuple (§2.3 part 3) | S2.3 | Medium | — |
| S2.5 | Runtime migration: Str, Bytes, Class instances, Generators (§2.3 part 4): remove `heap_field_mask`, `type_tags` usage | S2.4 | Medium | — |
| S2.6 | GC migration (§2.4): `mark_object(Value)`, remove heap masks | S2.5 | **HIGH** (critical path) | — |
| S2.7 | Codegen: Value lowering (§2.5 part 1): MIR ops emit uniform I64 Value, remove `ValueKind` enum | S2.6 | High | — |
| S2.8 | Codegen: arithmetic fast-path inlining (§2.5 part 2): inline tag tests for hot ops based on SSA types | S2.7 | **HIGH** (perf-critical) | — |
| S2.9 | Pass migration: delete boxing helpers (§2.6): `box_primitive_if_needed`, `promote_to_float_if_needed`, `coerce_to_field_type`, `is_useless_container_ty` | S2.8 | Medium | — |
| S2.10 | Phase 2 final purge + benchmark acceptance (§2.7): grep verify, run benchmarks, update BASELINE | S2.9 | Low-Medium | — |

**Split triggers**:

- **S2.3** (list migration): if list ops are many (> 30 runtime
  funcs), split into: S2.3a = list core (push/get/set/len);
  S2.3b = list methods (sort/reverse/index/count/etc.).
- **S2.8** (arithmetic fast-path): consider splitting: S2.8a =
  int+int fast path; S2.8b = mixed numeric fast paths; S2.8c =
  comparison fast paths.

**Combined ok**:

- S2.1 + S2.2 possible if both fit < 1500 LOC. Usually keep separate
  because S2.1 is design-heavy and S2.2 touches every `rt_make_*`
  call site.

### Phase 3 — Type Lattice + Monomorphization

| ID | Scope | Deps | Complexity | Parallel? |
|----|-------|------|------------|-----------|
| S3.1 | Lattice trait + `Type` method migration (§3.1): `TypeLattice` impl for `Type`, migrate all callers to `join`/`meet`/`is_subtype_of`/`minus` | Phase 2 merged | Medium-High | — |
| S3.2 | TypeVar + Generic unification (§3.2): add `Type::Var`, `Type::Generic`; migrate `Type::List`/`Dict`/`Set`/`Tuple`/`TupleVar` to `Generic` representation | S3.1 | **HIGH** (widespread) | — |
| S3.3 | Monomorphization pass: specialization engine (§3.3 part 1): walk call sites, instantiate, dedup | S3.2 | **HIGH** | — |
| S3.4 | Monomorphization: codegen integration + stdlib generics rewrite (§3.3 part 2): ensure no `TypeVar` reaches codegen | S3.3 | High | — |
| S3.5 | Protocol structural typing (§3.4): parse Protocol, structural `is_subtype_of`, runtime type-check function | S3.2 | Medium-High | Parallel-safe with S3.3, S3.4 (different subsystems) |
| S3.6 | Frontend: TypeVar/Generic/Protocol parsing (§3.5): Python syntax for `T = TypeVar(...)`, `class C(Generic[T])`, `class P(Protocol)`, PEP 695 `def f[T](...)` | S3.5 | Medium | — |
| S3.7 | Phase 3 final purge + perf gate (§3.6 + §3.7): delete `unify_*`, `narrow_*`; benchmark, binary-size, compile-time gates | S3.4, S3.6 | Low-Medium | — |

**Split triggers**:

- **S3.2** (TypeVar + Generic migration): if `Type::List/Dict/Set/
  Tuple` call sites exceed ~500 touches, split: S3.2a = add
  `Generic` variant; S3.2b = migrate List/Dict/Set; S3.2c = migrate
  Tuple / TupleVar.
- **S3.3** (monomorphization core): potentially split by function
  class: S3.3a = free functions; S3.3b = methods; S3.3c = stdlib
  builtins.

**Combined ok**:

- S3.3 + S3.4 possible if the integration is small. Usually
  separate — monomorphization is perf-sensitive enough to warrant
  dedicated codegen attention.

### Post-Refactor: Area F

| ID | Scope | Deps |
|----|-------|------|
| SF.1 | Simplify Area F plan in `MICROGPT_PLAN.md` §F for post-refactor architecture (tagged Value, Protocol dispatch, SSA folding) | All 3 phases merged + 1 week stabilization |
| SF.2 | Implement `rt_format(value: Value, spec: Value) -> Value` + frontend f-string desugar + constant-fold pass | SF.1 |
| SF.3 | Delete legacy `FmtHex`/`FmtOct`/`FmtBin`/`FmtIntGrouped`/`FmtFloatGrouped`/`Round` (in format ctx); test coverage for all format spec variants | SF.2 |

---

## Parallelism & Combination Rules

### Rules

1. **Cross-phase parallelism is forbidden.** Phase 1 must merge
   before Phase 2 starts, Phase 2 before Phase 3. Sessions in
   different phases never run concurrently, even on different
   branches. This enforces Non-Negotiable Principle 8.

2. **Intra-phase parallelism is narrow.** Within a phase, sessions
   marked "Parallel-safe" in the table may run concurrently on
   separate branches. All other sessions are serial — the next
   session starts only after the previous merges.

3. **Parallel sessions still gate on merge order.** Even
   parallel-safe sessions must produce independently mergeable
   branches. If one session's merge breaks the other's branch,
   rebase and re-test before the second merge.

4. **Combination is a scope reduction, not addition.** Two small
   sessions may combine into one **only if** the combined scope is
   still within "Medium" sizing (≤ 1000 LOC, ≤ 6 hours). Combining
   "Medium + Medium" to save a planning step is a false economy —
   the combined session becomes High and splits anyway.

5. **Combination follows directed pairs only.** Combine S(N) with
   S(N+1) if the deliverables are tightly coupled. Never combine
   S(N) with S(N+2) skipping an intermediate.

### Safe combination patterns

- **"Define + First Use"**: S(N) adds a new type/API; S(N+1) is its
  first narrow consumer. Often combinable if both are small.
  Example: S0.1 (benchmark harness) + S0.3 (property checker
  stubs) if both fit in one Medium session.

- **"Final Purge"**: the last session of a milestone often just
  deletes obsoleted code and runs grep verification. These close
  sessions are usually Low complexity and can fold into the
  preceding session IF the preceding session's scope permits.
  Example: S1.9 (delete legacy type maps) may fold into S1.8
  (TypeInferencePass) if the inference migration naturally leaves
  zero call sites.

### Unsafe combination patterns

- **Never combine "architecture addition" with "legacy deletion".**
  S1.2 (add CFG) + S1.3 (delete StmtKind) in one session sounds
  efficient but hides the bridging period where both coexist —
  that is exactly where bugs are found. Keep them separate.

- **Never combine sessions that touch different crates at
  critical paths.** S2.6 (GC migration) + S2.7 (Codegen Value
  lowering) would cross runtime↔codegen in one commit. If a bug
  is introduced, you can't isolate whether it's runtime or
  codegen. Keep separate.

- **Never combine "design" with "migration".** S2.1 (Value API
  design) + S2.2 (migrate rt_make_int) in one session forces the
  design decisions to harden mid-migration, blocking
  reconsideration. Close S2.1 first, review the API stand-alone,
  then open S2.2.

- **Never combine parallel-safe sessions into a serial one just
  to "simplify scheduling".** Parallelism is an opportunity, not
  an obligation. But forcing a sequence when the graph allows
  parallel work wastes walltime without benefit.

---

## Red Flags — Hard Stops

If any of these happen during a session, halt and either split or
roll back. Do NOT push through.

1. **Test suite is red and you don't know why.** Not "red, but I
   have a plan" — red and unexplained. Bisect or revert.

2. **The scope has doubled since the session started.** Stop. You
   hit a genuine sub-problem that deserves its own session. Roll
   back the current session's work OR land the part that fits and
   defer the rest to a new session.

3. **You are rewriting code you just wrote in the same session.**
   Sign of underdesigned scope. Roll back, re-plan from the start.

4. **Tests are "mostly green" with 1-2 failures you plan to come
   back to.** Not acceptable. All green or roll back.

5. **You are creating a new ad-hoc map/helper to make the session
   work.** Session-level violation of Non-Negotiable Principle 5.
   Stop. Either fit into the new abstraction or amend the spec.

6. **You are about to commit with "TODO: fix later" in the code.**
   Not a commit, not a session close. Either fix now or revert and
   re-plan.

7. **You have been in plan mode for > 2 hours without starting
   implementation.** Plan mode should be 10-30 minutes for a
   well-specified milestone. Extended plan mode means the spec is
   under-specified (amend the document) or the scope is too large
   (split the session).

8. **The session is in its 12th hour.** Sessions beyond 10 hours
   have strongly diminishing returns. Commit what is green, close
   the session, rest, open a new one.

9. **You are editing `ARCHITECTURE_REFACTOR.md` during
   implementation to match what the code is doing.** This is
   backwards. The document drives the code, not the reverse.
   Halt. Revert the implementation. Plan properly via Amendment
   Protocol if the document is wrong.

---

## Total Session Count

Phase 0: 2-3 sessions
Phase 1: 15-20 sessions (17 listed; possible splits in S1.2, S1.6, S1.8)
Phase 2: 8-12 sessions (10 listed; possible splits in S2.3, S2.8)
Phase 3: 6-9 sessions (7 listed; possible splits in S3.2, S3.3)
Post-refactor Area F: 2-3 sessions

**Expected total**: ~33-45 sessions over 14-23 weeks at one
session per 2-3 days wall-clock.

The session count is an output, not a budget. If halfway through
you realize the spec was inadequate and requires major amendment,
the count grows — and the alternative (skipping planned work) is
worse than the extra sessions.

---

# Amendment Protocol

This document is **authoritative** but not immutable. If during
execution a milestone's spec is found incorrect, incomplete, or
impossible as written:

1. **Stop work on the milestone.**
2. Open an "amendment" branch that edits this document to reflect
   the corrected spec. Include a rationale section explaining why
   the original spec was wrong.
3. Get sign-off from the project lead on the amendment.
4. Merge the amendment to master **before** continuing milestone
   work. The code and document stay in sync.

Do not silently deviate. The architecture's integrity depends on
the spec reflecting reality.

---

*Last updated: 2026-04-19. Phase 0 complete. Phase 1 ~60 % landed:
S1.1 / S1.2 / S1.4 / S1.5 / S1.6 / S1.7 / S1.9 / S1.10 / S1.11 ✅;
S1.8 🟡 (core + rule set, single-match collapse queued as §1.4u);
S1.16 🟡 (Phi wiring ✅, manual-phi cleanup ⏳); S1.3 ⏳ (folded into
S1.17b); S1.12 ✅; S1.13 ✅; S1.14a ✅; S1.14b-prep ✅; S1.14b-inliner ✅; S1.15 ✅; S1.16 ✅; S1.17a ✅; §1.4u ✅ (all 4 steps + §1.4u-c/d); §1.11 scoped 2026-04-19 (S1.17b split into 6 sub-sessions S1.17b-{a..f}); S1.17 full / S1.17b ⏳.
See the Phase-1 status dashboard at the top of §1 and the status
blocks inside each §1.x milestone for details.*
