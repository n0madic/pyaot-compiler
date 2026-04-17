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

- Integer arithmetic hot loops (`sum(range(10_000_000))`).
- Float arithmetic (`sum(i * 0.5 for i in range(1_000_000))`).
- Polymorphic arithmetic (dunder-dispatched, Value-class-like).
- Dict / list allocation and iteration.
- String interning / concat.
- GC stress (allocation-heavy tight loops).
- Class instantiation + method dispatch.
- Closure creation + call.

Each benchmark:
- Has a Python source file under `bench/py/`.
- Has a `cargo bench` target.
- Records wall-clock time, binary size, and (if possible) max RSS.
- Establishes a **baseline** recorded in `bench/BASELINE.md`.

**Exit criterion**: `cargo bench` runs all benchmarks, produces stable
results (variance < 3% across 5 runs), baseline committed.

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
