# Phase 2 S2.7 — Atomic Codegen + Storage + ABI Migration

**Status:** Campaign plan (2026-04-24). Supersedes the single-session
S2.7 row in `ARCHITECTURE_REFACTOR.md`. Expected duration: 2–3 weeks
of focused work, 6–10 sub-sessions.

**Non-negotiable relaxation for this campaign:** `ARCHITECTURE_REFACTOR.md`
§Non-Negotiable Principle 1 ("no partial migrations") is explicitly
relaxed between stages. The codebase is allowed to be red during
intermediate steps of this campaign — only the campaign's *final*
commit must restore workspace green. Each stage commits a checkpoint
that makes progress toward the final state; that checkpoint need not
be self-contained. The user has explicitly authorized this.

**Discipline that still applies:**
- The final commit restores `cargo test --workspace --release` to
  green, fmt clean, clippy clean, GC stress suite green.
- No `TODO: fix later` dangling comments in the final commit.
- Benchmarks run at the end; perf gates in §2.7 honored.
- All deletions from the §2.3/§2.4/§2.5 amendment chain land.

## Why a campaign, not a session

The explore pass (2026-04-24) showed S2.7 scope cannot be split cleanly
along "runtime only" vs "codegen only" axes. `rt_box_int` has 30+
**runtime-internal** callers (builtins.rs, counter.rs, defaultdict.rs,
tuple/{query,comparison}.rs, random.rs) in addition to every
codegen-emitted MIR call. The three representations — raw i64,
tagged `Value`, heap `IntObj*` — all interact through containers, GC,
and user-compiled code. A genuine atomic flip requires simultaneous
movement across every subsystem. Within one session that reliably
leaves every intermediate state broken; the only honest path is a
multi-stage campaign with a documented final invariant.

This document lists the final invariant, the staging sequence to get
there, and the red-zone expectations at each checkpoint.

---

## Target invariant (end of S2.7)

At the campaign's final commit, the following must all be true:

### 1. Representation

- `pyaot_core_defs::Value` is the sole representation of a Python
  runtime value across the entire system.
- Primitives (Int, Bool, None) are immediate tagged Values. No heap
  allocation.
- Float remains heap-boxed (`*mut FloatObj`) and wrapped in
  `Value::from_ptr`; `rt_box_float` / `rt_unbox_float` stay as extern
  functions, but accept/return `Value` at the ABI boundary.
- Heap objects (Str, Bytes, List, Tuple, Dict, Set, Instance,
  Generator, Iterator, etc.) are `*mut Obj` wrapped in `Value::from_ptr`.

### 2. Container storage

- `ListObj.data: *mut Value` (landed in S2.3).
- `TupleObj.data: [Value; 0]`.
- `DictEntry { hash, key: Value, value: Value }`.
- `SetEntry { hash, elem: Value }`. `TOMBSTONE` is a Value constant
  (e.g. `Value(pyaot_core_defs::tag::RESERVED_TAG)` or a dedicated
  `Value::TOMBSTONE` const — pick during Stage C).
- `InstanceObj.fields: [Value; N]`.
- `GeneratorObj.locals: [Value; N]`.
- `DequeObj.data: *mut Value`.
- **Deleted:** `TupleObj.heap_field_mask`, `ClassInfo.heap_field_mask`,
  `GeneratorObj.type_tags`, `DequeObj.elem_tag`, `ListObj.elem_tag`.
- **`core-defs::elem_tags`** (`ELEM_HEAP_OBJ` / `ELEM_RAW_INT` /
  `ELEM_RAW_BOOL`): deleted. Container APIs no longer take an
  `elem_tag` parameter.

### 3. Extern ABI (`crates/core-defs/src/runtime_func_def.rs`)

- **Deleted:** `RT_BOX_INT`, `RT_BOX_BOOL`, `RT_UNBOX_INT`,
  `RT_UNBOX_BOOL`.
- **Deleted:** `RT_TUPLE_GET_INT`, `RT_TUPLE_GET_FLOAT`,
  `RT_TUPLE_GET_BOOL`, `RT_TUPLE_SET_HEAP_MASK`.
- **Retained:** `RT_BOX_FLOAT`, `RT_UNBOX_FLOAT` (floats stay boxed).
- **Retyped:** every `rt_*` extern function takes/returns `Value`
  where it previously took/returned `*mut Obj`, `i64` scalar, or `i8`
  scalar representing a Python value. Typed scalars that aren't
  Python values (lengths, indices, hashes, sizes, error codes) stay
  as `i64`/`i8`. `Value` is `#[repr(transparent)] u64`, so ABI is
  identical to the pre-migration `i64`/`*mut Obj` — only semantics
  change.
- Grep `extern "C" fn rt_.*\b(i64|i8|\*mut Obj)\b` returns zero
  matches for value-carrying parameters.

### 4. Runtime internals

- Every `(*int_obj).value` / `(*bool_obj).value` read in
  `crates/runtime/src/**/*.rs` is replaced with `Value::unwrap_int` /
  `Value::unwrap_bool`.
- Every `boxing::rt_box_int(x)` / `boxing::rt_box_bool(b)` call in
  internal Rust code is replaced with `Value::from_int(x)` /
  `Value::from_bool(b)`.
- `IntObj` and `BoolObj` struct definitions: **deleted** (unused —
  the small-int / bool singleton pools go away; primitives are
  immediates). `FloatObj` stays.
- `boxing.rs` itself reduces to just the float path:
  `rt_box_float` / `rt_unbox_float`, plus a `rt_to_value` / similar
  helper if needed for FFI.

### 5. Codegen

- `crates/codegen-cranelift/src/**/*.rs` emits inline tag arithmetic
  for primitive box/unbox:
  - Box int: `(x << 3) | 1` (INT_TAG).
  - Box bool: `Value::TRUE.0` or `Value::FALSE.0` (lookup).
  - Unbox int: `(v as i64) >> 3` (arithmetic shift).
  - Unbox bool: `(v >> 3) != 0`.
- `ValueKind` enum in `crates/mir/src/kinds.rs`: **deleted** per
  §2.5 non-negotiable (MIR ops emit uniform `I64` Value; the
  runtime_selector dispatch on ValueKind disappears).
- Closure tuples: raw function pointers at slot 0 are wrapped as
  `Value::from_int(func_ptr as i64)` before store. `Value::is_ptr()`
  on that slot returns false, GC skips correctly.
- §2.5 fast-path inlining for arithmetic (SSA-known Int+Int) emits
  inline tag ops with overflow fallback to slow-path polymorphic
  runtime.

### 6. Lowering

- `box_primitive_if_needed` / `unbox_func_for_type` / `emit_heap_field_mask`
  in `crates/lowering/src/lib.rs`: **deleted**.
- `is_useless_container_ty`: kept (it's about type-inference shape,
  not runtime representation).
- Every call site that previously emitted `RT_BOX_INT` etc. is
  rewritten to emit `ValueFromInt` MIR instruction (or the equivalent
  inline op in Cranelift).
- Container ops (RT_TUPLE_SET, RT_LIST_SET, RT_DICT_SET, RT_SET_ADD):
  take `Value` args in MIR; lowering no longer passes a separate
  `elem_tag` constant.

### 7. Optimizer

- Peephole: BOX/UNBOX round-trip patterns deleted (they never match
  after lowering stops emitting RT_BOX).
- `abi_repair.rs`: BOX/UNBOX normalization rules deleted; replaced
  with `ValueFromInt`/`UnwrapValueInt` round-trip elimination if
  needed.
- Type inference: `RT_BOX_INT returns HeapAny` edges removed; the new
  `ValueFromInt` instruction returns `Value`-typed MIR local
  (typically `Type::Int` at the SSA level).

### 8. GC

- `mark_object(v: Value)` body collapses to:
  ```rust
  fn mark_object(v: Value) {
      if !v.is_ptr() { return; }
      let obj = v.unwrap_ptr::<Obj>();
      if obj.is_null() || (*obj).is_marked() { return; }
      (*obj).set_marked(true);
      // ... existing per-type match ...
  }
  ```
  The address-heuristic filter (`< 0x1000`, non-8-aligned) is
  **deleted** — every slot feeding `mark_object` is a properly-tagged
  Value.
- Tuple arm: iterate `[Value]`, dispatch via `is_ptr()`. No
  `heap_field_mask`.
- Instance arm: iterate `[Value]`, dispatch via `is_ptr()`. No
  `ClassInfo.heap_field_mask`.
- Generator arm: iterate `[Value]` locals, dispatch via `is_ptr()`.
  No `type_tags` array.
- Deque arm: iterate `[Value]`, dispatch via `is_ptr()`. No
  `elem_tag == 0` branch.
- `gc.rs` should shrink to ≤150 lines per §2.4 "30% shrink" target.

### 9. Benchmarks (§2.7 exit gate)

- Int/Bool arithmetic: within ±3% of Phase 1 baseline.
- Float arithmetic: within ±10% of Phase 1 baseline (float stays
  boxed; minor regression expected).
- Polymorphic arithmetic: **improved** by ≥20% (no boxing dance).
- GC scan time: **improved** by ≥15% (no heap_field_mask or
  type_tags lookups).

---

## Stage sequence

Each stage is a logical checkpoint. Stages A, G, and the final
cleanup commit must be green; stages B through F may be red. Each
stage lists: goal, touched files, expected red symptoms, verification
at the stage's end.

### Stage A — Additive infrastructure (green)

**Goal:** introduce new MIR instructions and Cranelift lowerings for
inline Value tag arithmetic, without hooking them into lowering.

**Touched:**
- `crates/mir/src/lib.rs` — add MIR instruction kinds
  `ValueFromInt { dest, src: Operand }`, `UnwrapValueInt { dest, src }`,
  `ValueFromBool { dest, src }`, `UnwrapValueBool { dest, src }`.
- `crates/codegen-cranelift/src/instructions/` — add Cranelift lowering
  for each new kind. `ValueFromInt` emits `(src << 3) | 1`;
  `UnwrapValueInt` emits `sshr_imm src, 3`; similar for bool.
- `crates/mir/src/` passes (verify, propagate, substitute) — handle
  the new kinds alongside `Const`, `Copy`, etc.

**Tests:**
- Add unit tests for each new MIR instruction: emit, serialize,
  optimize (peephole should treat `ValueFromInt(UnwrapValueInt(x)) → x`
  as a trivial round-trip elimination — add that pattern in Stage G).

**Verification:** workspace build + test green. New instructions exist
but are unused.

### Stage B — Break-point: retype `rt_box_int` / `rt_box_bool` semantics (red)

**Goal:** make `rt_box_int(i) -> *mut Obj` physically return the bit
pattern of `Value::from_int(i)` instead of a heap IntObj pointer.
Same for bool. **This is the commit where things break;** document
the break in the commit message.

**Touched:**
- `crates/runtime/src/boxing.rs` — `rt_box_int` body rewritten to
  `Value::from_int(i).0 as *mut Obj`. `rt_unbox_int` body reads the
  tagged bits: `(p as u64 as i64) >> 3`. Similarly for bool.
  `IntObj` / `BoolObj` struct definitions: **deleted**.
- `crates/runtime/src/object.rs` — `IntObj`, `BoolObj` removed. Type
  tags `TypeTagKind::Int` / `TypeTagKind::Bool` stay (GC / error
  messages still use them for dispatch via `primitive_type`), but no
  heap objects carry these tags.

**Expected red symptoms:**
- Every `match tag { TypeTagKind::Int => (*(ptr as *mut IntObj)).value }`
  dereferences Value-tagged bits as if they were an IntObj header →
  SIGSEGV on many paths: `hash_table_utils::eq_hashable_obj`,
  `conversions::to_str::rt_obj_to_str`, `copy.rs` deep copy,
  `ops/comparison.rs`, etc.
- Any container that stored a "boxed int" (dict values, set elements,
  instance fields, generator locals, tuple slots) now holds a Value
  bit pattern but reading code expects `*mut IntObj`.
- Small-int / bool singleton pools in `boxing.rs` become meaningless
  and should be deleted in this stage.

**Verification:** workspace does NOT build. Commit message
explicitly marks it as intentional red. Do not push past this
checkpoint until Stage C rebuilds the callers.

### Stage C — Migrate every runtime-internal `IntObj`/`BoolObj` consumer (reduce red)

**Goal:** rewrite every `(*int_obj).value` / `(*bool_obj).value` read
in `crates/runtime/src/**` to `Value::unwrap_int` / `Value::unwrap_bool`
on the surrounding Value context.

**Touched (~7 files, ~89 branches from the S2.7a explore):**
- `crates/runtime/src/copy.rs` — `deep_copy_recursive` tuple/list
  branches: no int/bool heap-copy needed (they're immediates, return
  as-is).
- `crates/runtime/src/format.rs` — repr/str formatting for int/bool.
- `crates/runtime/src/builtins.rs` — arithmetic/hash dispatch (abs,
  sum, etc.).
- `crates/runtime/src/conversions/to_str.rs` — int/bool to str.
- `crates/runtime/src/tuple/comparison.rs` — tuple element comparison
  for int/bool slots (direct Value compare).
- `crates/runtime/src/hash_table_utils.rs` — dict/set key equality
  with Int/Bool cross-type (1 == 1.0 == True). Rewrite the
  `eq_hashable_obj` match so Int/Bool are Value immediates, not
  deref'd IntObj/BoolObj.
- `crates/runtime/src/ops/comparison.rs`, `ops/printing.rs` — same
  pattern.
- All internal `boxing::rt_box_int(x)` / `rt_box_bool(b)` call sites
  (builtins, counter, defaultdict, tuple/query, random) replaced with
  `Value::from_int(x)` / `Value::from_bool(b)`. Return type annotations
  updated from `*mut Obj` to `Value` where appropriate.

**Expected red symptoms:**
- Compilation errors fall as you fix each file. Test suite will be
  partially running but with crashes still possible on untouched
  container paths (Stage D handles those).
- Some paths will compile but have logic bugs until Stage D lines up.

**Verification:** workspace build green, but tests likely still fail
on container-heavy paths. Track test count; it should trend upward
across sub-commits in this stage.

### Stage D — Container storage flip + rt_* ABI retype (most of the delete)

**Goal:** flip every container's internal storage to `[Value]` and
retype every `rt_*` function's signature to take/return `Value` for
value-carrying parameters.

**Touched:**
- `crates/runtime/src/object.rs` — struct definitions:
  - `TupleObj.data: [Value; 0]`, delete `heap_field_mask`.
  - `DictEntry { hash, key: Value, value: Value }`.
  - `SetEntry { hash, elem: Value }`. Define `pub const
    TOMBSTONE: Value = Value(...)`.
  - `InstanceObj.fields: [Value; N]`.
  - `GeneratorObj.locals: [Value; N]`, delete `type_tags`.
  - `DequeObj.data: *mut Value`, delete `elem_tag`.
  - `ListObj`: delete `elem_tag` (S2.3 kept it for ABI; now gone).
- `crates/runtime/src/**/*.rs` — every internal read/write of the
  above fields migrates.
- `crates/runtime/src/boxing.rs` — `rt_box_float` / `rt_unbox_float`
  now take/return `Value`. Delete the int/bool singleton pools.
- `crates/core-defs/src/runtime_func_def.rs` — every `RuntimeFuncDef`
  that takes `PI64` for a value parameter is retyped (still i64 at
  Cranelift level since `Value` is transparent, but documentation and
  future signature typing).
- `crates/core-defs/src/elem_tags.rs` — **delete** the
  `ELEM_HEAP_OBJ` / `ELEM_RAW_INT` / `ELEM_RAW_BOOL` module.
- `crates/runtime/src/vtable.rs` — delete
  `rt_register_class_fields(..., heap_field_mask)` parameter; it
  becomes `rt_register_class(...)` with no mask.
- `crates/lowering/src/emit_heap_field_mask` — **delete**.
- `crates/lowering/src/statements/assign/mod.rs`,
  `crates/lowering/src/expressions/mod.rs`,
  `crates/lowering/src/expressions/builtins/iteration/composite.rs` —
  the 5 `emit_heap_field_mask` call sites disappear with the function
  deletion.

**Expected red symptoms:**
- Massive. Every container touch surface changes. Runtime tests fail
  until each module migrates.
- Plan sub-commits within this stage per container: D.1 Tuple,
  D.2 Dict, D.3 Set, D.4 Instance, D.5 Generator, D.6 Deque. Sub-commit
  messages document which specific crashes are still expected and
  which should have cleared.

**Verification at stage end:** workspace builds. Runtime tests pass
for each container individually. Still some integration tests may fail
waiting on Stage E codegen.

### Stage E — Codegen migration (resolve codegen-induced crashes)

**Goal:** rewire lowering to emit the Stage A MIR instructions and
Value-typed container calls. Kill `box_primitive_if_needed` /
`unbox_func_for_type` chokepoints and inline their replacements at
every call site.

**Touched:**
- `crates/lowering/src/lib.rs` — delete `box_primitive_if_needed`,
  `unbox_func_for_type`. Inline the tag-op emission at every caller.
- All 36 box/unbox emitter sites (see explore report section 1 & 2).
- `crates/lowering/src/type_dispatch.rs` — remove
  `RT_TUPLE_GET_INT/FLOAT/BOOL` dispatch; everything becomes
  `RT_TUPLE_GET` returning a Value.
- `crates/lowering/src/runtime_selector.rs` — remove
  `ValueKind` dispatch. Replace with uniform Value/I64 codegen.
- `crates/mir/src/kinds.rs` — delete `ValueKind` enum.
- `crates/codegen-cranelift/src/runtime_calls/runtime_selector.rs` —
  delete `type_to_value_kind`.
- `crates/codegen-cranelift/src/instructions/arithmetic.rs` — §2.5
  fast-path inline tag tests for hot int/bool ops; fall through to
  slow-path polymorphic runtime on overflow or Any-typed operands.
- Closure tuple emission: slot 0 (raw function pointer) now emits
  `ValueFromInt(func_ptr_as_i64)`. No more raw store.

**Expected red symptoms:**
- Codegen path is the last major cluster. Any missed rewrite shows
  up as a type dispatch SEGV or wrong arithmetic. Use `gc_stress_test`
  mode early and often to catch Value mis-tagging.

**Verification:** workspace tests largely green. A handful of
esoteric paths (Area F format specifiers, weird stdlib corners) may
still be red — Stage F mops them up.

### Stage F — Optimizer + final cleanup (restore green)

**Goal:** rewire the optimizer's ABI-repair / peephole patterns to
the new instruction set, delete every dead helper, and push tests
to full green.

**Touched:**
- `crates/optimizer/src/peephole/patterns.rs` — delete the three
  BOX/UNBOX round-trip patterns. Add the analogous
  `ValueFromInt(UnwrapValueInt(x)) → x` and `UnwrapValueInt(ValueFromInt(x)) → x`
  patterns (simpler since both are SSA single-def).
- `crates/optimizer/src/abi_repair.rs` — delete every BOX/UNBOX-
  specific rule. Keep whatever remains about operand-type coercion.
- `crates/optimizer/src/type_inference.rs` — remove RT_BOX_FLOAT /
  RT_UNBOX_FLOAT type-inference edges; the remaining float box/unbox
  still exists, but its signature now takes/returns `Value`.
- Clean up any lingering `#[allow(unused)]` / stub shims introduced
  during Stages B–E for intermediate greening.
- `crates/runtime/src/gc.rs` — `mark_object` collapses to the
  3-line form from the target invariant. Delete the address-
  heuristic filter. Delete heap_field_mask / type_tags / elem_tag
  reads. `gc.rs` should now be ≤150 lines.

**Verification:** workspace fmt clean, clippy clean, `cargo test
--workspace --release` green, `RUSTFLAGS="--cfg gc_stress_test" cargo
test -p pyaot --test runtime --release` green.

### Stage G — Benchmark + final commit

**Goal:** verify perf gates from §2.7 and squash the campaign into
a clean final commit.

**Touched:**
- `bench/BASELINE.md` — record new numbers.
- `INSIGHTS.md` — update the "Tagged Value Encoding" section to
  reflect the fully-landed Phase 2 architecture. Delete obsolete
  notes about `heap_field_mask`, `type_tags`, list `elem_tag` boundary
  conversion, "GC API takes Value" transitional note.
- `ARCHITECTURE_REFACTOR.md` — mark S2.7 (and the folded S2.4/S2.5)
  complete. Clean up the amendment chain in §2.2/§2.3/§2.4.
- `COMPILER_STATUS.md` — update Phase 2 capability notes.
- `.claude/rules/architecture.md` / `api-reference.md` — refresh the
  "Runtime Object Header" + "Per-slot tagging" sections; many of
  those details are obsolete.

**Perf gates (§2.7 exit criteria, hard):**
- Int/Bool arithmetic: within ±3% of Phase 1 baseline.
- Float arithmetic: within ±10% of Phase 1 baseline.
- Polymorphic arithmetic: ≥20% improvement.
- GC scan time: ≥15% improvement.
- Binary size: within +10% of pre-migration runtime staticlib.

If a gate fails, do NOT close the campaign — investigate and
re-open the stage that regressed.

**Verification:** `cargo bench --workspace` runs clean, numbers
recorded. Full workspace test suite green. Commit messages
referenced; the campaign branch can merge.

---

## Branch strategy

The 2-3 week scope and intermediate-red stages warrant a dedicated
long-lived branch:

```
phase-2-s2.7-atomic
```

Workflow:
- Branch off current `master` (at commit `8a7b1b4`).
- Each stage A–G commits to this branch. Red stages are OK.
- No `git rebase --onto master` until Stage G passes.
- After Stage G: squash-merge (or preserve history — project
  preference) into `master` as a single atomic campaign.
- Other Phase 2 follow-up work (S2.8, S2.9, S2.10) starts from the
  post-merge master.

Rationale: a single squashed merge keeps `master`'s green-
every-commit invariant intact; the branch holds the real history.

## Risk mitigation

Three categorical risks:

1. **Hidden runtime consumer we missed.** Mitigation: after Stage C,
   grep for `*mut IntObj` / `*mut BoolObj` across the workspace. Any
   remaining match is a missed migration; either fix immediately or
   document as Stage D follow-up.

2. **Cranelift codegen regression on numeric benchmarks.** Mitigation:
   run `cargo bench --workspace` at the end of each of Stages E and F.
   If int arithmetic regresses >5%, investigate before Stage G.
   Common cause: extra register pressure from tag masking; Cranelift
   should constant-fold `(x << 3) | 1` for literal Ints but may not
   for propagated constants.

3. **GC correctness regression (sweep frees a live object).**
   Mitigation: run `RUSTFLAGS="--cfg gc_stress_test"` suite at each
   stage boundary. This mode forces a full GC on every allocation and
   catches most missing-root bugs within a few seconds of test
   execution.

## Return criteria

The campaign closes when, on the final commit:

1. Every exit criterion in `ARCHITECTURE_REFACTOR.md` §2.7 passes.
2. The §2.3 amendment chain's deferred deletions have all landed.
3. `grep -rn 'ELEM_RAW_INT\|ELEM_HEAP_OBJ\|heap_field_mask\|heap_mask\|type_tags\|ValueKind\|type_to_value_kind\|rt_box_int\|rt_box_bool\|rt_unbox_int\|rt_unbox_bool\|rt_tuple_get_int\|rt_tuple_get_float\|rt_tuple_get_bool\|rt_tuple_set_heap_mask' crates/ | wc -l` returns `0`.
4. `INSIGHTS.md` / `ARCHITECTURE_REFACTOR.md` / `COMPILER_STATUS.md` /
   `.claude/rules/*.md` reflect the new architecture.
5. Benchmarks recorded in `bench/BASELINE.md`.

---

*Last updated: 2026-04-24. Campaign status: planned, not started.*
