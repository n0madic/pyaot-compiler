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

Every commit in every milestone must satisfy:

- `cargo build --workspace --release` — zero warnings.
- `cargo fmt --check` — clean.
- `cargo clippy --workspace --release -- -D warnings` — clean.
- `cargo test --workspace --release` — zero failures, zero regressions.

Merging a milestone with "known regressions to be fixed later" is
forbidden. The test suite is the gate. If a legitimate semantic change
requires updating a test, the update is part of the milestone commit.

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

# Phase 0 — Preparation

**Duration**: 1 week.

**Goal**: establish the regression-detection infrastructure that the
remaining phases depend on. Zero user-visible changes.

## 0.1 Benchmark harness

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

## 0.2 Coverage audit

Run `cargo llvm-cov --workspace --html` (install if missing). Identify
any major area (lowering crate, optimizer, type_planning) with < 70%
coverage. For each gap, add tests in `examples/test_*.py` or
`crates/*/src/tests.rs` to reach ≥ 70% before Phase 1 starts.

**Non-negotiable**: Phase 1 must not be bottlenecked on "we don't
have a test for this case". Add the tests now.

## 0.3 SSA property checker (stub)

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

## 0.4 Lattice property harness

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

---

# Phase 1 — SSA MIR + Whole-Program Type Inference

**Duration**: 6–10 weeks.

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

## 1.1 HIR → CFG conversion

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

## 1.2 Dominator tree computation

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

## 1.3 SSA renaming + φ-insertion

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

## 1.4 Flow-sensitive type inference

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

**Non-negotiable**: all type queries go through the single pass output.
No imperative `insert_var_type` during lowering. If a later pass needs
to update a type, it invalidates and reruns the pass on the affected
region.

**Exit criteria**:

- Every existing type-dependent test passes.
- All 4 legacy type maps deleted from `SymbolTable`.
- `apply_narrowings` / `restore_types` deleted.
- Type queries use a single `TypeTable` that is a pure function of
  the SSA IR.

## 1.5 Call graph construction

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

## 1.6 Whole-program parameter type inference

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

## 1.7 Whole-program field type inference

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

## 1.8 Pass migration

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

## 1.9 Codegen migration

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

## 1.10 Cleanup + final purge

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
| S1.1 | HIR CFG type definitions (§1.1 prep): add `HirBlock`, `HirBlockId`, `HirTerminator` alongside legacy `StmtKind` — both coexist | S0.* | Low-Medium | — |
| S1.2 | Frontend HIR-CFG migration (§1.1 main): convert `ast_to_hir/*.rs` to emit CFG; leaves old `StmtKind::If/While/ForBind/Try/Match` as bridge | S1.1 | **HIGH** | — |
| S1.3 | Downstream consumer migration + legacy StmtKind deletion (§1.1 tail): optimizer, lowering, codegen consume CFG; delete old StmtKind variants | S1.2 | **HIGH** | — |
| S1.4 | Dominator tree (§1.2): `crates/mir/src/dom_tree.rs`, Cooper-Harvey-Kennedy | S1.3 | Medium | Parallel-safe with S1.10 |
| S1.5 | Phi MIR instruction + codegen-side block-param support (§1.3 prep) | S1.4 | Medium | — |
| S1.6 | SSA renaming via Cytron algorithm (§1.3 main): rename all function bodies to SSA, activate SSA checker | S1.5 | **HIGH** | — |
| S1.7 | `Refine` instruction + isinstance narrowing at CFG successors (§1.4 prep) | S1.6 | Medium | — |
| S1.8 | Unified `TypeInferencePass` (§1.4 main): replace 3 legacy inference paths with one SSA-based pass | S1.7 | **HIGH** | — |
| S1.9 | Delete legacy type maps (§1.4 tail): purge `prescan_var_types`, `refined_var_types`, `narrowed_union_vars`, `apply_narrowings`, `restore_types` | S1.8 | Medium | — |
| S1.10 | Call graph (§1.5): `crates/optimizer/src/call_graph.rs`, SCCs via Tarjan | S1.3 | Medium | Parallel-safe with S1.4-S1.9 |
| S1.11 | WPA parameter inference (§1.6): fixed-point pass over call graph | S1.9, S1.10 | **HIGH** | — |
| S1.12 | WPA field inference (§1.7): cross-call field type join | S1.11 | **HIGH** | — |
| S1.13 | Pass migration: DCE + constfold (§1.8 part 1) | S1.9 | Medium | Parallel-safe with S1.14-S1.15 (different passes) |
| S1.14 | Pass migration: inlining (§1.8 part 2) | S1.13 | High | Parallel-safe with S1.15 |
| S1.15 | Pass migration: peephole, devirtualize, flatten_properties (§1.8 part 3) | S1.9 | Medium-High | Parallel-safe with S1.13, S1.14 |
| S1.16 | Codegen SSA migration (§1.9): MIR Phi → Cranelift block params, delete manual phi emulation | S1.6, S1.15 | Medium-High | — |
| S1.17 | Phase 1 final cleanup + acceptance (§1.10): grep-verify deletions, benchmark check, docs update | S1.11, S1.12, S1.16 | Low-Medium | — |

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

*Last updated: Phase 0 pre-start. Phases 1, 2, 3 not yet begun.*
