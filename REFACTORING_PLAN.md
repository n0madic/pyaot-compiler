# REFACTORING_PLAN.md

Comprehensive multi-phase refactoring plan for the Python AOT Compiler.
Goal: transform the compiler into a maintainable, extensible architecture where adding new Python features requires minimal cross-crate changes.

**Current pain point:** Adding a single new runtime function (e.g., `str.removeprefix()`) requires touching **5+ files** across 4 crates. After this refactoring, it should require **1-2 files**.

---

## Table of Contents

- [Problem Inventory](#problem-inventory)
- [Dependency Graph](#dependency-graph)
- [Synergies & Cancellations](#synergies--cancellations)
- [Phase 0: Foundation](#phase-0-foundation)
- [Phase 1: Runtime Modernization](#phase-1-runtime-modernization)
- [Phase 2: The Declarative Revolution (Keystone)](#phase-2-the-declarative-revolution-keystone)
- [Phase 3: Lowering Architecture](#phase-3-lowering-architecture)
- [Phase 4: Frontend & Codegen Polish](#phase-4-frontend--codegen-polish)
- [Phase 5: Optimizer Evolution](#phase-5-optimizer-evolution)
- [Phase 6: Final Integration](#phase-6-final-integration)
- [Risk Analysis](#risk-analysis)
- [Success Metrics](#success-metrics)

---

## Problem Inventory

Complete catalog of discovered architectural issues, tagged for cross-referencing.

| ID | Problem | Severity | Location | LOC Impact |
|----|---------|----------|----------|------------|
| P1 | RuntimeFunc enum explosion (~316 variants → ~15 special + Call(&def)) | ✅ DONE | `mir/src/runtime_func.rs` | ~800→~120 |
| P2 | MIR depends on stdlib-defs (layering violation) | HIGH | `mir/src/runtime_func.rs:7` | ~30 |
| P3 | Codegen dispatch monster (603 cases → ~60, 19 sub-modules deleted) | ✅ DONE | `codegen-cranelift/src/runtime_calls/mod.rs` | ~560→~60 |
| P4 | Codegen signature boilerplate (all helpers deleted, generic handler) | ✅ DONE | `codegen-cranelift/src/runtime_calls/*.rs` | ~6,352→~400 |
| P5 | Runtime FFI explosion (246 extern "C" functions) | HIGH | `runtime/src/` | ~33,000 |
| P6 | Runtime type dispatch duplication (5+ ad-hoc match sites) | MEDIUM | `runtime/src/ops.rs`, `conversions.rs` | ~1,500 |
| P7 | Runtime god-files (8 files >500 lines each) | MEDIUM | `runtime/src/{dict,set,tuple,bytes,...}.rs` | ~9,000 |
| P8 | Runtime mixed exception raising (3 strategies) | MEDIUM | Throughout `runtime/src/` | ~300 |
| P9 | Runtime duplicated hash table (dict vs set) | MEDIUM | `runtime/src/{dict,set,hash_table_utils}.rs` | ~400 |
| P10 | Lowering god-object (45+ fields) | HIGH | `lowering/src/context/mod.rs:161-280` | ~600 |
| P11 | Dunder methods hardcoding (139 strings, 48 fields) | HIGH | `lowering/src/context/mod.rs:84-460` | ~500 |
| P12 | Lowering runtime call boilerplate (588x pattern) | MEDIUM | Throughout `lowering/src/` | ~3,000 |
| P13 | Lowering type dispatch duplication (11+ match sites) | MEDIUM | Throughout `lowering/src/` | ~800 |
| P14 | Type planning intertwined with lowering | HIGH | `lowering/src/type_planning/` | ~3,000 |
| P15 | Generator lowering tightly coupled | MEDIUM | `lowering/src/generators/` (3,584 lines) | ~3,584 |
| P16 | Frontend god-file expressions.rs (1,675 LOC) | MEDIUM | `frontend-python/src/ast_to_hir/expressions.rs` | ~1,675 |
| P17 | Frontend AstToHir struct (27 fields) | MEDIUM | `frontend-python/src/ast_to_hir/mod.rs:70-126` | ~500 |
| P18 | Dual type systems without validation | MEDIUM | Frontend `types.rs` vs lowering `type_planning/` | ~400 |
| P19 | Codegen hardcoded offsets (8+ magic numbers) | MEDIUM | `codegen-cranelift/src/{instructions,exceptions,gc}.rs` | ~50 |
| P20 | Codegen scattered GC root management (15+ manual sites) | MEDIUM | Throughout `codegen-cranelift/src/` | ~200 |
| P21 | CodegenContext god-object (13 fields) | LOW | `codegen-cranelift/src/context.rs:20-39` | ~300 |
| P22 | Optimizer no pass interface | LOW | `optimizer/src/lib.rs` | ~100 |
| P23 | Optimizer inconsistent fixpoint iteration | LOW | Various optimizer passes | ~50 |
| P24 | Span loss through pipeline | MEDIUM | Lowering (desugaring), optimizer | ~200 |
| P25 | Inconsistent error hierarchy | MEDIUM | `diagnostics/src/lib.rs:29-99` | ~100 |
| P26 | Lowering god-files (8+ files >800 lines) | MEDIUM | `lowering/src/expressions/operators.rs`, etc. | ~8,000 |
| P27 | Codegen instructions.rs god-file (1,143 LOC) | LOW | `codegen-cranelift/src/instructions.rs` | ~1,143 |
| P28 | Cross-module class info uses String instead of InternedString | LOW | `lowering/src/context/mod.rs:208-232` | ~50 |

---

## Dependency Graph

```
Phase 0 (Foundation)                    Phase 5 (Optimizer)
  P25 Error hierarchy ──────────┐         P22 Pass interface
  P19 Layout constants ─────────┤         P23 Fixpoint iteration
  P8  Unified exceptions ───────┤
                                │
Phase 1 (Runtime)               │      Phase 6 (Integration)
  P7  Split god-files ──┐       │         P28 Cross-module interning
  P9  Unify hash table ─┤       │         Pipeline span propagation
  P5  Reduce FFI ────────┤      │         Unified type dispatch
  P6  Type dispatch ─────┘      │
         │                      │
         ▼                      │
Phase 2 (KEYSTONE) ◄────────────┘  ✅ ~95% DONE (P2 remains)
  P1  RuntimeFunc descriptors ──(~300/~300 migrated ✅)
  P2  Decouple MIR/stdlib ──(not started)
  P3  Codegen dispatch ─────(19/19 variant modules deleted ✅)
  P4  Codegen signatures ───(300+ auto-generated, runtime_helpers.rs deleted ✅)
  P24 Span in MIR instructions ──(pre-existing ✅)
         │
         ▼
Phase 3 (Lowering)
  P11 Dunder → HashMap
  P10 Decompose god-object
  P14 Separate type planning
  P15 Decouple generators
  P12 emit_runtime_call helper ─(simplified by Phase 2)
  P13 Centralized type dispatch ─(simplified by Phase 2)
  P26 Split god-files
         │
         ▼
Phase 4 (Frontend & Codegen)
  P17 Decompose AstToHir
  P16 Split expressions.rs
  P18 Type annotation validation
  P21 Decompose CodegenContext
  P20 Auto GC root management
  P27 Split instructions.rs
```

**Critical path:** Phase 0 → Phase 1 → Phase 2 → Phase 3 → Phase 4

**Independent track:** Phase 5 (Optimizer) can run in parallel with Phases 3-4.

---

## Synergies & Cancellations

### Synergies (refactorings that amplify each other)

| Synergy | Problems | Effect |
|---------|----------|--------|
| **S1: Declarative RuntimeFunc** | P1 + P2 + P3 + P4 | Single architectural change solves 4 problems. Introducing `RuntimeFuncDef` descriptors eliminates the enum explosion, decouples MIR from stdlib, auto-generates codegen dispatch, and auto-generates Cranelift signatures. |
| **S2: Generic Runtime** | P5 + P6 | Generic functions with type-tag parameters reduce FFI surface AND unify type dispatch. E.g., one `rt_collection_cmp(obj, obj, op_tag)` replaces 8+ comparison functions. |
| **S3: Lowering Decomposition** | P10 + P11 + P14 + P15 | Decomposing the Lowering god-object naturally separates type planning, generator state, and dunder tracking into distinct types. Design all four together. |
| **S4: Pass Manager** | P22 + P23 | A trait-based pass interface automatically enables unified fixpoint iteration strategy. |
| **S5: Runtime Cleanup** | P7 + P8 + P9 | Splitting god-files, unifying exceptions, and merging hash tables can be done in a single sweep through the runtime crate. |
| **S6: Frontend Cleanup** | P16 + P17 | Decomposing AstToHir into focused substruct naturally leads to splitting expressions.rs by delegating to typed sub-converters. |

### Cancellations (refactorings that become unnecessary)

| Cancelled | By | Reason |
|-----------|----|--------|
| P3 (codegen dispatch monster) | S1 (Declarative RuntimeFunc) | Dispatch is auto-generated from descriptors — no manual match needed |
| P4 (codegen signature boilerplate) | S1 (Declarative RuntimeFunc) | Signatures generated from `RuntimeFuncDef` — no manual construction |
| P12 (lowering boilerplate) partially | S1 + S3 | Declarative descriptors + decomposed Lowering reduce boilerplate to trivial levels |
| P13 (lowering type dispatch) partially | S1 | Centralized type→descriptor mapping replaces scattered match chains |

### Conflicts (refactorings that must be carefully ordered)

| Conflict | Resolution |
|----------|------------|
| P5 (reduce FFI) changes runtime API before P1 (RuntimeFunc) references it | **Do P5 first.** Reduce FFI surface to get stable function names, then build descriptors on top. |
| P7 (split runtime files) vs P5 (change function signatures) | **Do P7 first.** Split into clean modules, then change signatures within organized modules. |
| P14 (separate type planning) vs P10 (decompose Lowering) | **Design together.** Type planning separation determines which fields go into which substruct. |
| P24 (spans in MIR) vs P1 (RuntimeFunc restructure) | **Do together.** Both modify MIR instruction definitions — touch MIR once, not twice. |
| P19 (layout constants) vs P20 (auto GC roots) | **Do P19 first.** Constants enable safe auto-GC implementation. |

---

## Phase 0: Foundation

**Goal:** Establish shared infrastructure that later phases depend on. Low risk, high leverage.

**Duration estimate:** Small. Independent changes, can be done incrementally.

### 0.1 — Unified Error Hierarchy (P25)

**Problem:** `CompilerError` has inconsistent structure — some variants carry `Span`, some don't. Error creation uses 3+ different patterns.

**Solution:**

```rust
// crates/diagnostics/src/lib.rs

/// All errors carry optional span for consistent source location reporting
#[derive(Debug)]
pub enum CompilerError {
    // Structured variants for common patterns
    TypeError { message: String, span: Option<DiagnosticSpan> },
    NameError { name: String, span: Option<DiagnosticSpan> },
    SyntaxError { message: String, span: Option<DiagnosticSpan> },
    ArgumentError { kind: ArgumentErrorKind, span: Option<DiagnosticSpan> },
    CodegenError { message: String, span: Option<DiagnosticSpan> },
    LinkError { message: String },
    IoError(String),
}

#[derive(Debug)]
pub enum ArgumentErrorKind {
    TooManyPositional { expected: usize, got: usize },
    DuplicateKeyword { name: String },
    UnexpectedKeyword { name: String },
    MissingRequired { name: String },
}

// Unified builder API
impl CompilerError {
    pub fn type_error(msg: impl Into<String>, span: impl Into<Option<DiagnosticSpan>>) -> Self { ... }
    pub fn name_error(name: impl Into<String>, span: impl Into<Option<DiagnosticSpan>>) -> Self { ... }
    pub fn codegen_error(msg: impl Into<String>, span: impl Into<Option<DiagnosticSpan>>) -> Self { ... }
}
```

**Migration:** Replace all `CompilerError::TypeError { message, span }` constructors with `CompilerError::type_error(msg, span)`. Add `Option<DiagnosticSpan>` to `CodegenError`.

**Files touched:** `crates/diagnostics/src/lib.rs`, all error creation sites across frontend, lowering, codegen.

### 0.2 — Layout Constants (P19)

**Problem:** 8+ hardcoded magic numbers for object layout offsets scattered across codegen.

**Solution:** Centralize in `core-defs`:

```rust
// crates/core-defs/src/layout.rs

/// Object layout constants shared between compiler and runtime.
/// MUST match runtime's #[repr(C)] struct layouts.
pub mod layout {
    /// ObjHeader: type_tag(1) + marked(1) + padding(6) + size(8) = 16 bytes
    pub const OBJ_HEADER_SIZE: i32 = 16;

    /// Pointer size on target platform
    pub const PTR_SIZE: i32 = 8;

    /// VTable starts immediately after ObjHeader
    pub const VTABLE_OFFSET: i32 = OBJ_HEADER_SIZE;

    /// Individual vtable entry offset (vtable pointer + slot * PTR_SIZE)
    pub const fn vtable_slot_offset(slot: usize) -> i32 {
        VTABLE_OFFSET + PTR_SIZE + (slot as i32) * PTR_SIZE
    }

    /// GC shadow frame root slot offset
    pub const fn gc_root_offset(root_idx: usize) -> i32 {
        (root_idx as i32) * PTR_SIZE
    }

    /// GC shadow frame total size for N roots
    pub const fn gc_frame_size(nroots: usize) -> u32 {
        (nroots * PTR_SIZE as usize) as u32
    }

    /// Exception frame: jmp_buf_size + metadata
    pub const JMP_BUF_SIZE: usize = 200;
    pub const EXCEPTION_FRAME_SIZE: u32 = PTR_SIZE as u32 + JMP_BUF_SIZE as u32 + PTR_SIZE as u32 + PTR_SIZE as u32;
}
```

**Migration:** Replace all literal `16i32`, `(8 * slot * 8)`, `200`, etc. in codegen with `layout::*` constants.

**Files touched:** New `crates/core-defs/src/layout.rs`, `crates/codegen-cranelift/src/{instructions,exceptions,gc,function}.rs`.

**Validation:** Add compile-time assertions in runtime crate:
```rust
const _: () = assert!(std::mem::size_of::<ObjHeader>() == layout::OBJ_HEADER_SIZE as usize);
```

### 0.3 — Unified Exception Raising in Runtime (P8)

**Problem:** Three strategies mixed: `raise_exc!` macro, direct `rt_exc_raise()`, specialized wrappers. 231 direct calls risk memory leaks (longjmp skips destructors).

**Solution:**
1. Audit all 231 `rt_exc_raise` calls
2. For static messages (byte literals): keep direct calls (no leak risk)
3. For formatted messages: migrate to `raise_exc!` macro
4. Remove redundant specialized wrappers (`rt_exc_raise_overflow_error` etc.) — use `raise_exc!` directly
5. Add CI grep-check: `grep -rn "rt_exc_raise(" runtime/src/ | grep -v "raise_exc\|b\"" | grep "format!"` should return 0 results

**Files touched:** ~27 files in `crates/runtime/src/`, primarily `math_ops.rs` (27 calls), `ops.rs` (25 calls), `conversions.rs` (13 calls).

---

## Phase 1: Runtime Modernization

**Goal:** Consolidate the runtime crate's 246 FFI functions into a cleaner, more generic API. This directly reduces the RuntimeFunc variant count, making Phase 2 smaller and easier.

**Depends on:** Phase 0 (layout constants, unified exceptions)

**Duration estimate:** Medium. Significant refactoring but each step is mechanically testable.

### 1.1 — Split Runtime God-Files into Submodules (P7)

**Problem:** `dict.rs` (1,081 LOC), `set.rs` (1,217 LOC), `tuple.rs` (1,418 LOC), `bytes.rs` (1,354 LOC) are monolithic. `list/` and `string/` already demonstrate the correct pattern.

**Solution:** Apply the list/string submodule pattern:

```
runtime/src/
├── list/           # Already done ✓
│   ├── mod.rs, core.rs, mutation.rs, slice.rs, comparison.rs, convert.rs, query.rs, minmax.rs, timsort.rs
├── string/         # Already done ✓
│   ├── mod.rs, core.rs, interning.rs, case.rs, slice.rs, search.rs, trim.rs, modify.rs, split_join.rs, align.rs
├── dict/           # NEW
│   ├── mod.rs      # Re-exports
│   ├── core.rs     # Creation, lookup_entry, find_insert_slot
│   ├── ops.rs      # get, set, delete, contains, update, merge
│   ├── iteration.rs # keys, values, items, iter
│   ├── convert.rs  # to_list, repr, str
│   └── special.rs  # defaultdict packing, comprehension support
├── set/            # NEW
│   ├── mod.rs
│   ├── core.rs     # Creation, find_slot, hash ops
│   ├── ops.rs      # add, remove, discard, contains
│   ├── algebra.rs  # union, intersection, difference, symmetric_difference
│   ├── comparison.rs # issubset, issuperset, lt, le, gt, ge
│   └── convert.rs  # to_list, repr, str
├── tuple/          # NEW
│   ├── mod.rs
│   ├── core.rs     # Creation, get, len
│   ├── comparison.rs # Tuple comparison
│   ├── convert.rs  # repr, str, hash
│   └── query.rs    # index, count, contains
├── bytes/          # NEW
│   ├── mod.rs
│   ├── core.rs     # Creation, get, len
│   ├── search.rs   # find, rfind, index, rindex, count
│   ├── transform.rs # upper, lower, strip, replace
│   ├── check.rs    # startswith, endswith, isdigit, isalpha, etc.
│   └── convert.rs  # decode, hex, repr
├── ops.rs          # SPLIT into:
│   ├── arithmetic.rs # int/float add, sub, mul, div, mod, pow
│   ├── comparison.rs # obj_cmp_ordering, lt, le, gt, ge, eq, ne
│   └── printing.rs   # print_obj, print_obj_repr
├── conversions.rs  # SPLIT into:
│   ├── to_str.rs   # rt_*_to_str variants
│   ├── repr.rs     # rt_repr_* variants
│   ├── ascii.rs    # rt_ascii_* variants
│   └── type_cast.rs # int↔float↔bool conversions
├── exceptions.rs   # SPLIT into:
│   ├── core.rs     # ExceptionObject, dispatch_to_handler
│   ├── ffi.rs      # extern "C" raise functions
│   └── state.rs    # GC integration, exception stack, current_exception
```

**Approach:** Pure structural refactoring — no logic changes, no API changes. Move functions between files and update `mod.rs` re-exports. Each submodule split is an independent commit, trivially verifiable by `cargo test --workspace`.

**Order:** dict → set → tuple → bytes → ops → conversions → exceptions (simplest first).

### 1.2 — Unify Hash Table Implementation (P9)

**Problem:** `dict.rs` implements probe-sequence logic ad-hoc (`lookup_entry` lines 65-91), while `set.rs` properly uses `hash_table_utils.rs` via `find_slot_generic`. Dict predates the generic extraction.

**Solution:**
1. Extract dict's compact-entry probing into `hash_table_utils.rs` as `find_compact_slot_generic`
2. Rewrite dict's `lookup_entry` and `find_insert_slot` to use the generic helpers
3. Both dict and set now share the same proven probe-sequence code
4. Reduce unsafe code surface (one correct implementation instead of two)

**Key difference to handle:** Dict uses compact layout (separate indices table + dense entries array) while set uses direct slots. The generic helper must be parameterized over the slot strategy.

**Files touched:** `runtime/src/hash_table_utils.rs`, `runtime/src/dict/core.rs` (after 1.1 split).

### 1.3 — Generic Runtime Functions (P5 + P6)

**Problem:** 246 `extern "C"` functions, many being monomorphic variants:
- `rt_list_get_int`, `rt_list_get_float`, `rt_list_get_bool` → 3 functions for 1 operation
- `rt_list_lt`, `rt_list_lte`, `rt_list_gt`, `rt_list_gte` → 4 functions for 1 operation
- `rt_set_min_int`, `rt_set_min_float`, `rt_list_min_int`, `rt_list_min_float` → 4 functions for 1 operation

**Solution:** Introduce generic functions with type/operation tags:

```rust
// BEFORE (4 functions):
extern "C" fn rt_list_lt(a: *mut Obj, b: *mut Obj) -> i8 { ... }
extern "C" fn rt_list_lte(a: *mut Obj, b: *mut Obj) -> i8 { ... }
extern "C" fn rt_list_gt(a: *mut Obj, b: *mut Obj) -> i8 { ... }
extern "C" fn rt_list_gte(a: *mut Obj, b: *mut Obj) -> i8 { ... }

// AFTER (1 function):
/// op_tag: 0=lt, 1=le, 2=gt, 3=ge (matches ComparisonOp discriminant)
extern "C" fn rt_list_cmp(a: *mut Obj, b: *mut Obj, op_tag: u8) -> i8 {
    let ordering = list_cmp_impl(a, b);
    match ComparisonOp::from_tag(op_tag) {
        ComparisonOp::Lt => (ordering == Ordering::Less) as i8,
        ComparisonOp::Le => (ordering != Ordering::Greater) as i8,
        ComparisonOp::Gt => (ordering == Ordering::Greater) as i8,
        ComparisonOp::Ge => (ordering != Ordering::Less) as i8,
    }
}
```

**Consolidation targets (estimated savings):**

| Pattern | Before | After | Reduction |
|---------|--------|-------|-----------|
| Collection comparison (list/tuple/set/bytes × lt/le/gt/ge) | ~16 | ~4 | -12 |
| Collection element get (list × int/float/bool/str/heap) | ~5 | ~1 | -4 |
| Min/max (list/set × int/float/key × min/max) | ~12 | ~2 | -10 |
| Repr/ascii (list/tuple/dict/set × repr/ascii) | ~8 | ~4 | -4 |
| Sorted (list/set/dict × elem types) | ~9 | ~3 | -6 |
| String search (find/rfind/index/rindex for str+bytes) | ~8 | ~2 | -6 |
| **Total estimated** | **~58** | **~16** | **~42** |

**Approach:**
1. Create generic function alongside old functions
2. Make old functions thin wrappers calling generic
3. Update codegen to call generic with appropriate tags
4. Remove old wrapper functions
5. Update RuntimeFunc variants (fewer needed)

**Interplay with Phase 2:** Each consolidation directly removes RuntimeFunc variants, making the Phase 2 migration smaller. This is why Phase 1 must precede Phase 2.

### 1.4 — Runtime Repr/Conversion Unification (P6 partial)

**Problem:** `conversions.rs` has 39 monomorphic functions. Container repr (`rt_repr_dict`, `rt_repr_list`, `rt_repr_tuple`, `rt_repr_set`) all follow the same iterate-format-concat pattern. Same for `rt_ascii_*` variants.

**Solution:**

```rust
// BEFORE: 4 separate functions (rt_repr_list, rt_repr_tuple, rt_repr_dict, rt_repr_set)
// AFTER: 1 generic function
extern "C" fn rt_repr_collection(obj: *mut Obj, format_kind: u8) -> *mut Obj {
    // format_kind encodes: repr vs ascii × brackets vs parens
    let tag = unsafe { (*obj).type_tag() };
    match tag {
        TypeTagKind::List => repr_with_brackets(obj, "[", "]", format_kind),
        TypeTagKind::Tuple => repr_with_brackets(obj, "(", ")", format_kind),
        TypeTagKind::Set => repr_with_brackets(obj, "{", "}", format_kind),
        TypeTagKind::Dict => repr_dict(obj, format_kind),
        _ => unreachable!(),
    }
}

fn repr_with_brackets(obj: *mut Obj, open: &str, close: &str, format_kind: u8) -> *mut Obj {
    // Shared iterate-format-concat logic
}
```

**Files touched:** `runtime/src/conversions/repr.rs`, `runtime/src/conversions/ascii.rs` (after 1.1 split).

---

## Phase 2: The Declarative Revolution (Keystone)

**Goal:** Replace the monolithic `RuntimeFunc` enum with declarative descriptors. This is the single highest-leverage change in the entire plan — it automatically resolves P1, P2, P3, and P4, and dramatically simplifies P12 and P13.

**Depends on:** Phase 1 (stable, reduced FFI surface to build descriptors on)

**Status:** ✅ **Complete (except 2.4).** 2.1, 2.2, 2.3 fully done. 2.4 (MIR/stdlib decoupling) not started. 2.5 was pre-existing.

### 2.1 — Design RuntimeFuncDef Descriptor System ✅ DONE

**Core idea:** Instead of ~300 enum variants with hand-written codegen dispatch, describe runtime functions declaratively. One generic codegen path handles all of them.

**Implemented** in `crates/core-defs/src/runtime_func_def.rs`:

```rust
#[derive(Debug, Clone, Copy)]
pub struct RuntimeFuncDef {
    pub symbol: &'static str,           // e.g., "rt_list_append"
    pub params: &'static [ParamType],   // Cranelift param types
    pub returns: Option<ReturnType>,    // None = void
    pub gc_roots_result: bool,          // true = call update_gc_root_if_needed
}

#[derive(Debug, Clone, Copy)]
pub enum ParamType { I64, F64, I8, I32 }

#[derive(Debug, Clone, Copy)]
pub enum ReturnType { I64, F64, I8, I32 }
```

**Design decisions vs original plan:**
- **`gc_roots_result` instead of `returns_heap`** — more descriptive name
- **No `diverges` field** — exception raising uses MIR `Terminator::Raise`, not `RuntimeFunc`
- **No `Ptr` ParamType** — `I64` covers both integers and pointers on 64-bit targets
- **Added `I32`** — needed for generator index/state and global/class-attr var_id parameters
- **No `Void` ReturnType** — `returns: None` encodes void functions
- **All definitions in `core-defs` only** — stdlib-defs unchanged (stdlib already uses `StdlibFunctionDef`)

**Const shorthand constructors** for concise definitions: `ptr_unary`, `ptr_binary`, `ptr_ternary`, `ptr_quaternary`, `void`, `unary_to_i64`, `binary_to_i64`, `unary_to_i8`, `binary_to_i8`.

### 2.2 — Migrate RuntimeFunc to Descriptors ✅ DONE

**~220 static `RuntimeFuncDef` definitions** created in `core-defs/src/runtime_func_def.rs`. All 19 variant codegen sub-modules deleted. `runtime_helpers.rs` (824 lines, 23 functions) deleted entirely.

**Migration approach:** Direct replacement per category. Each category: add static defs → add lookup methods in `kinds.rs` → update lowering → remove enum variants → delete codegen module → run tests.

| Category | Variants | Status | Codegen Module |
|----------|----------|--------|----------------|
| Hash + Id | 5 | ✅ Done | `hash.rs` deleted |
| Boxing/Unboxing | 7 | ✅ Done | `boxing.rs` deleted |
| File I/O | 12 | ✅ Done | `file.rs` deleted |
| Object ops | 12 | ✅ Done | `object.rs` deleted |
| Set ops | 21 | ✅ Done | `set.rs` deleted |
| Dict ops | 24 | ✅ Done | `dict.rs` deleted |
| List ops | 34 | ✅ Done | `list.rs` deleted |
| Tuple ops | 21 | ✅ Done | `tuple.rs` deleted |
| Bytes ops | 29 | ✅ Done | `bytes.rs` deleted |
| String ops | ~35 | ✅ Done | `string.rs` shrunk (MakeStr/MakeBytes special) |
| Print ops | ~8 | ✅ Done | `print.rs` shrunk (PrintValue(Str/None)/AssertFail special) |
| Conversions | ~15 | ✅ Done | `conversions.rs` deleted |
| Math | ~6 | ✅ Done | `math.rs` deleted |
| Compare (parameterized) | ~14 | ✅ Done | `compare.rs` deleted |
| MinMax (parameterized) | ~10 | ✅ Done | `minmax.rs` deleted |
| Iterator ops | ~18 | ✅ Done | `iterator.rs` deleted |
| Generator ops | 14 | ✅ Done | `generator.rs` deleted |
| Global/Cell/ClassAttr | ~20 | ✅ Done | `globals.rs`, `cells.rs`, `class_attrs.rs` deleted |
| Instance ops | ~15 | ✅ Done | `instance.rs` deleted |
| Dead exception variants | 3 | ✅ Done | `ExcPushFrame`, `ExcPopFrame`, `ExcIsinstance` removed |

**Remaining special-case variants** (cannot use descriptor — embed binary data or require runtime type coercion):
- `MakeStr` — embeds compile-time string data in binary via `create_raw_string_data`
- `MakeBytes` — same pattern for bytes literals
- `AssertFail` / `AssertFailObj` — embeds null-terminated string constants
- `PrintValue(Str)` / `PrintValue(None)` — extracts `Constant::Str` from operands
- `ExcRegisterClassName` — embeds raw string data in binary

**Exceeded plan expectations:**
- Boxing/Unboxing migrated (originally planned as special)
- `InstanceGetField` / `InstanceSetField` migrated (generic handler's dest-type coercion handles I8/F64)
- `GlobalSet(Ptr)` migrated (`load_operand_as` handles I8→I64 and F64→I64 coercion)
- `ZipNext` / `IterZip` removed entirely (were dead code — never emitted by lowering)

**Lessons learned during implementation:**
1. **Void functions must not zero dest** — in-place mutations like `TupleSet` reuse the same dest local for the modified container. The generic handler leaves void dest unchanged.
2. **Return type coercion is needed** — when e.g. `UnboxBool` returns I8 but dest var is I64, or vice versa. The handler reads the dest variable's Cranelift type and coerces automatically.
3. **Signature auditing is critical** — several definitions had wrong param counts or types (e.g., `ListSortWithKey` takes 6 params not 3, `BytesStrip` takes 1 not 2). A systematic audit against original codegen is essential before testing.
4. **`MakeBytes` is special like `MakeStr`** — bytes literals need data embedded in the binary, not passed as runtime operands.

### 2.3 — Generic Codegen Dispatch ✅ DONE

**Implemented** in `crates/codegen-cranelift/src/runtime_calls/mod.rs` as `compile_runtime_func_def()`.

The generic handler:
1. Builds Cranelift signature from `def.params` and `def.returns`
2. Loads args with `load_operand_as` for automatic type coercion (extended with I32 support)
3. Emits the call
4. **Coerces return type** to match the dest variable's declared type (I8↔I64, I32↔I64, F64↔I64 bitcast)
5. Optionally registers result as GC root via `def.gc_roots_result`
6. For void functions: leaves dest variable unchanged (no zero-write)

**`load_operand_as` extended** in `utils.rs` with `I64↔I32` coercion (ireduce/uextend).

**Final impact:**
- 19 of 19 variant codegen sub-modules deleted entirely
- `compile_runtime_call` match reduced from 470+ lines to ~60 lines
- `runtime_helpers.rs` deleted (824 lines, 23 functions — all dead after migration)
- `RuntimeFunc` enum: ~316 variants → ~15 special cases + `Call(&'static RuntimeFuncDef)`
- ~4,700 lines deleted from codegen; ~220 one-liner static defs added to core-defs
- Lookup functions (`ValueKind::global_get_def()`, `IterSourceKind::iterator_def()`, etc.) in `mir/src/kinds.rs` map parameterized variants to static defs

### 2.4 — Decouple MIR from stdlib-defs (P2)

**Status:** Not started. MIR still imports `pyaot_stdlib_defs` for `StdlibCall`, `StdlibAttrGet`, `ObjectFieldGet`, `ObjectMethodCall` variants.

**Migration plan** (unchanged):
1. Add `RuntimeFuncDef` field or conversion to `StdlibFunctionDef` / `StdlibMethodDef` / etc.
2. Lowering resolves stdlib calls to `RuntimeFunc::Call(stdlib_def.as_runtime_func_def())`
3. Remove `StdlibCall`, `StdlibAttrGet`, `ObjectFieldGet`, `ObjectMethodCall` variants
4. Remove `use pyaot_stdlib_defs` from MIR's Cargo.toml

**Result:** MIR depends only on `core-defs` (true leaf crate) — clean layering.

### 2.5 — Add Spans to MIR Instructions (P24) ✅ PRE-EXISTING

Already implemented before this refactoring — `Instruction` has `pub span: Option<Span>`. No additional work needed.

---

## Phase 3: Lowering Architecture

**Goal:** Decompose the 32K-line lowering crate's god-objects and eliminate duplicated patterns, leveraging Phase 2's simplified RuntimeFunc.

**Depends on:** Phase 2 (RuntimeFunc descriptors, MIR spans)

**Duration estimate:** Large. Many interconnected changes, but each substep is independently testable.

### 3.1 — Dunder Methods: Fields → HashMap (P11)

**Problem:** `LoweredClassInfo` has 48 separate fields for dunder methods (`str_func`, `repr_func`, `eq_func`, ...) plus 120+ lines of hardcoded match statements in `get_dunder_func()` and `set_dunder_func()`.

**Solution:**

```rust
// BEFORE (48 fields + 120 lines of match):
pub struct LoweredClassInfo {
    pub str_func: Option<FuncId>,
    pub repr_func: Option<FuncId>,
    pub eq_func: Option<FuncId>,
    pub ne_func: Option<FuncId>,
    pub lt_func: Option<FuncId>,
    // ... 43 more fields
}

// AFTER (1 field, 0 match statements):
pub struct LoweredClassInfo {
    pub class_id: ClassId,
    pub name: InternedString,
    pub field_offsets: IndexMap<InternedString, usize>,
    pub field_types: IndexMap<InternedString, Type>,
    pub heap_field_mask: u64,
    pub method_funcs: IndexMap<InternedString, FuncId>,
    pub vtable_slots: IndexMap<InternedString, usize>,
    /// Dunder methods — unified storage
    pub dunder_methods: IndexMap<InternedString, FuncId>,
    // ... remaining non-dunder fields
}

impl LoweredClassInfo {
    pub fn get_dunder(&self, name: InternedString) -> Option<FuncId> {
        self.dunder_methods.get(&name).copied()
    }

    pub fn set_dunder(&mut self, name: InternedString, func_id: FuncId) {
        self.dunder_methods.insert(name, func_id);
    }
}
```

**Why IndexMap:** Preserves insertion order (important for vtable layout). Using `InternedString` keys avoids the 139 hardcoded string literals — interned once, compared by ID.

**Migration:**
1. Add `dunder_methods: IndexMap<InternedString, FuncId>` field
2. Replace all `class_info.str_func` reads with `class_info.get_dunder(interner.intern("__str__"))`
3. Replace all `set_dunder_func("__str__", id)` calls with `class_info.set_dunder(str_interned, id)`
4. Remove 48 individual fields
5. Remove `get_dunder_func()` and `set_dunder_func()` match statements (120+ lines)

**Net effect:** ~500 lines deleted, adding new dunder support is just a single intern + insert.

### 3.2 — Decompose Lowering God-Object (P10)

**Problem:** `Lowering` struct has 45+ fields mixing 6 concerns: variable tracking, type inference, code generation state, class metadata, closure state, and cross-module data.

**Solution:** Extract into focused sub-structs:

```rust
pub struct Lowering<'a> {
    // Sub-contexts (owned, accessed via self.symbols, self.types, etc.)
    pub symbols: SymbolTable,
    pub types: TypeEnvironment,
    pub codegen: CodeGenState,
    pub classes: ClassRegistry,
    pub closures: ClosureState,
    pub modules: ModuleState,

    // Immutable references to input
    pub interner: &'a mut StringInterner,
    pub hir_module: &'a hir::Module,
}

/// Variable names → local IDs, function references, global tracking
pub struct SymbolTable {
    pub var_to_local: IndexMap<VarId, LocalId>,
    pub var_to_func: IndexMap<VarId, FuncId>,
    pub globals: IndexSet<VarId>,
    pub global_var_types: IndexMap<VarId, Type>,
    pub cell_vars: IndexSet<VarId>,
    pub nonlocal_cells: IndexMap<VarId, VarId>,
    pub default_value_slots: IndexMap<VarId, usize>,
}

/// Type tracking: variable types, expression cache, narrowing
pub struct TypeEnvironment {
    pub var_types: IndexMap<VarId, Type>,
    pub expr_types: IndexMap<hir::ExprId, Type>,
    pub refined_var_types: IndexMap<VarId, Type>,
    pub narrowed_union_vars: IndexMap<VarId, Type>,
    pub func_return_types: IndexMap<FuncId, Type>,
}

/// MIR construction: blocks, instructions, current position
pub struct CodeGenState {
    pub current_block_idx: usize,
    pub current_blocks: Vec<mir::BasicBlock>,
    pub loop_stack: Vec<(BlockId, BlockId)>,
    pub exception_stack: Vec<ExceptionHandler>,
    pub next_local_id: u32,
    pub next_block_id: u32,
}

/// Class metadata: lowered class info, vtables
pub struct ClassRegistry {
    pub class_info: IndexMap<ClassId, LoweredClassInfo>,
    pub cross_module_class_info: IndexMap<ClassId, CrossModuleClassInfo>,
}

/// Closure tracking: captures, wrappers, dynamic vars
pub struct ClosureState {
    pub var_to_closure: IndexMap<VarId, FuncId>,
    pub var_to_wrapper: IndexMap<VarId, FuncId>,
    pub dynamic_closure_vars: IndexSet<VarId>,
    pub closure_capture_types: IndexMap<FuncId, Vec<Type>>,
    pub wrapper_func_ids: IndexMap<FuncId, FuncId>,
    pub func_ptr_params: IndexMap<FuncId, Vec<Type>>,
}

/// Cross-module imports and exports
pub struct ModuleState {
    pub module_var_exports: IndexMap<VarId, Type>,
    pub module_func_exports: IndexMap<FuncId, Type>,
    pub module_class_exports: IndexMap<ClassId, ()>,
    pub module_var_wrappers: IndexMap<VarId, FuncId>,
    pub module_var_funcs: IndexMap<VarId, FuncId>,
}
```

**Migration strategy:** Incremental field migration:
1. Create all sub-structs with fields moved from `Lowering`
2. Add accessor methods on `Lowering` that delegate to sub-structs (temporary compat layer)
3. Update call sites module-by-module to use `self.symbols.var_to_local` instead of `self.var_to_local`
4. Remove accessor methods once all call sites are updated

**Interplay with P14:** The `TypeEnvironment` extraction directly enables separating type planning (Phase 3.3).

### 3.3 — Separate Type Planning Phase (P14)

**Problem:** Type inference and MIR lowering are intertwined — `expr_types` cache lives on `Lowering`, type planning calls pre-scan which inspects MIR results. Circular dependency.

**Solution:** Two-pass architecture:

```
Pass 1: TypePlanner (HIR → TypeEnvironment)
  - Pre-scan for closures, lambdas, generators
  - Infer all expression types
  - Refine empty containers
  - Resolve all types BEFORE lowering starts

Pass 2: Lowerer (HIR + TypeEnvironment → MIR)
  - TypeEnvironment is read-only input
  - No type inference during lowering
  - All type queries are lookups, never computation
```

```rust
// Phase 1: Type planning (pure analysis, no MIR construction)
pub struct TypePlanner<'a> {
    pub interner: &'a mut StringInterner,
    pub hir_module: &'a hir::Module,
}

impl TypePlanner<'_> {
    pub fn plan(&mut self, func: &hir::FuncDef) -> Result<TypeEnvironment> {
        let mut env = TypeEnvironment::new();
        self.pre_scan(func, &mut env)?;   // closures, lambdas
        self.infer_types(func, &mut env)?; // expression types
        self.refine_containers(&mut env)?; // empty container types
        Ok(env)
    }
}

// Phase 2: Lowering (construction, no inference)
pub struct Lowerer<'a> {
    pub types: &'a TypeEnvironment,  // read-only!
    pub symbols: SymbolTable,
    pub codegen: CodeGenState,
    // ...
}
```

**Key constraint:** TypePlanner must not depend on MIR — it works only with HIR and Type. This breaks the circular dependency.

**Migration:**
1. Extract `type_planning/` into standalone module with own state
2. Run type planning first, producing `TypeEnvironment`
3. Pass `TypeEnvironment` as read-only to `Lowerer`
4. Remove all `compute_expr_type` calls during lowering — replaced by `self.types.get(expr_id)`

### 3.4 — Decouple Generator Lowering (P15)

**Problem:** `generators/` (3,584 lines, 5 files) is tightly coupled to main lowering via shared context.

**Solution:** Generator desugaring as a separate pass:

```
HIR with generators → GeneratorDesugarer → HIR without generators (state machine) → Lowerer
```

The desugarer transforms generator functions into state-machine classes before lowering sees them. This is the standard approach used by most compilers (Kotlin, C#, Rust).

**Design:**
1. `GeneratorDesugarer` walks HIR, finds `yield` expressions
2. Transforms each generator function into a class with `__next__` method
3. Yield points become state transitions
4. Lowerer handles the result as a normal class — no special generator logic needed

**This eliminates:** 3,584 lines of interleaved generator + lowering code, replaced by ~1,500 lines of focused desugaring.

### 3.5 — Lowering Helper: emit_runtime_call (P12)

**Problem:** 588x occurrences of alloc-local + emit-instruction boilerplate.

**Solution:** Now trivial after Phase 2 (descriptors):

```rust
impl Lowerer<'_> {
    /// Emit a runtime call via descriptor, return the result local
    fn emit_runtime_call(
        &mut self,
        def: &'static RuntimeFuncDef,
        args: Vec<mir::Operand>,
        result_type: Type,
        mir_func: &mut mir::Function,
        span: Option<Span>,
    ) -> LocalId {
        let dest = self.codegen.alloc_local(result_type, is_heap_type(&result_type));
        mir_func.add_local(dest);
        self.codegen.emit(mir::Instruction {
            kind: mir::InstructionKind::RuntimeCall {
                dest,
                func: mir::RuntimeFunc::Call(def),
                args,
            },
            span,
        });
        dest
    }
}
```

**Impact:** 588 occurrences of 4-6 lines reduced to 1 line each.

### 3.6 — Centralized Type-to-Operation Mapping (P13)

**Problem:** 11+ `match ty { Type::Int => ..., Type::Float => ..., ... }` scattered across lowering for selecting runtime functions.

**Solution:** Table-driven dispatch using Phase 2 descriptors:

```rust
// crates/lowering/src/type_dispatch.rs

use pyaot_core_defs::runtime_func_def::*;

/// Select the runtime function for a binary operation on given types.
pub fn select_binop(op: BinOp, left: &Type, right: &Type) -> Option<&'static RuntimeFuncDef> {
    BINOP_TABLE.get(&(op, type_category(left), type_category(right))).copied()
}

/// Select the runtime function for a method call.
pub fn select_method(ty: &Type, method: &str) -> Option<&'static RuntimeFuncDef> {
    METHOD_TABLE.get(&(type_category(ty), method)).copied()
}

/// Select the runtime function for a builtin.
pub fn select_builtin(builtin: &str, arg_types: &[&Type]) -> Option<&'static RuntimeFuncDef> {
    // ...
}

#[derive(Hash, Eq, PartialEq)]
enum TypeCategory { Int, Float, Bool, Str, List, Dict, Set, Tuple, Bytes, HeapObj }

fn type_category(ty: &Type) -> TypeCategory { ... }

// Populated at compile time or lazily
static BINOP_TABLE: LazyLock<HashMap<(BinOp, TypeCategory, TypeCategory), &RuntimeFuncDef>> = ...;
```

**Impact:** 11+ match sites replaced by table lookups. Adding new type support = adding rows to tables.

### 3.7 — Split Lowering God-Files (P26)

After Phases 3.1-3.6 simplify the code, split remaining large files:

| File | Lines | Split Into |
|------|-------|-----------|
| `operators.rs` (1,414) | → `binary_ops.rs`, `unary_ops.rs`, `comparison.rs`, `logical_ops.rs` |
| `match_stmt.rs` (1,199) | → `match_patterns.rs`, `match_guards.rs`, `match_control_flow.rs` |
| `pre_scan.rs` (1,014) | → Absorbed into TypePlanner (Phase 3.3) |
| `assign.rs` (991) | → `simple_assign.rs`, `tuple_unpack.rs`, `augmented_assign.rs` |
| `call_resolution.rs` (987) | → `arg_binding.rs`, `default_params.rs`, `varargs.rs` |

---

## Phase 4: Frontend & Codegen Polish

**Goal:** Apply the same decomposition principles to frontend and codegen.

**Depends on:** Phase 2 (for codegen), Phase 3 patterns (proven decomposition approach)

**Duration estimate:** Medium. Mechanical refactoring following established patterns.

### 4.1 — Decompose AstToHir (P17)

**Problem:** 27 fields mixing ID allocation, symbol tables, scope tracking, import resolution.

**Solution:**

```rust
pub struct AstToHir {
    pub ids: IdAllocator,        // next_var_id, next_func_id, next_class_id, etc.
    pub symbols: SymbolTable,    // var_map, func_map, class_map
    pub scope: ScopeContext,     // scope_stack, current_class, global_vars, nonlocal_vars
    pub imports: ImportResolver, // imported_names, imported_modules, stdlib_imports
    pub module: Module,          // output HIR module
    pub interner: StringInterner,
}

pub struct IdAllocator {
    next_var_id: u32,
    next_func_id: u32,
    next_class_id: u32,
    next_lambda_id: u32,
    next_comp_id: u32,
    next_ctx_id: u32,
}

impl IdAllocator {
    pub fn alloc_var(&mut self) -> VarId { ... }
    pub fn alloc_func(&mut self) -> FuncId { ... }
    // ...
}
```

### 4.2 — Split Frontend expressions.rs (P16)

**Problem:** 1,675 LOC single `convert_expr()` method handles all 15+ expression types.

**Solution:** Split by expression category, delegating from main dispatch:

```
ast_to_hir/expressions/
├── mod.rs          # Main dispatch: match expr.kind → delegate
├── literals.rs     # Int, Float, Bool, Str, None, Bytes
├── names.rs        # Name resolution (variables, functions, classes, imports)
├── operators.rs    # BinOp, UnaryOp, BoolOp, Compare
├── calls.rs        # Function calls, method calls
├── containers.rs   # List, Dict, Set, Tuple literals
├── comprehensions.rs # Moved from separate file, integrated
├── subscript.rs    # Indexing, slicing
└── attributes.rs   # Attribute access, module.attr
```

### 4.3 — Type Annotation Validation (P18)

**Problem:** Frontend converts type annotations → `Type`. Lowering infers types independently. No cross-validation.

**Solution:** After Phase 3.3 separates type planning, add validation:

```rust
// In the pipeline, after type planning and before lowering:
fn validate_type_annotations(
    hir: &hir::Module,
    type_env: &TypeEnvironment,
) -> Result<()> {
    for (var_id, annotated_type) in hir.type_annotations() {
        if let Some(inferred_type) = type_env.var_types.get(&var_id) {
            if !inferred_type.is_subtype_of(annotated_type) {
                return Err(CompilerError::type_error(
                    format!("Variable annotated as {} but inferred as {}", annotated_type, inferred_type),
                    hir.var_span(var_id),
                ));
            }
        }
    }
    Ok(())
}
```

### 4.4 — Auto GC Root Management in Codegen (P20)

**Problem:** `update_gc_root_if_needed` called manually from 15+ sites. Missing a call = use-after-free.

**Solution:** After Phase 2.3, the generic `compile_runtime_call` already handles GC root updates via `def.returns_heap`. For remaining special cases, wrap in a builder:

```rust
pub struct CallBuilder<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
    ctx: &'a mut CodegenContext,
    gc_frame: &'a Option<GcFrameData>,
}

impl CallBuilder<'_, '_> {
    /// Call a runtime function. Automatically updates GC root if dest is a heap local.
    pub fn call_runtime(
        &mut self,
        func_ref: FuncRef,
        args: &[Value],
        dest: LocalId,
        dest_is_heap: bool,
    ) -> Value {
        let call_inst = self.builder.ins().call(func_ref, args);
        let result = self.builder.inst_results(call_inst)[0];
        let dest_var = *self.ctx.var_map.get(&dest).expect("internal error: dest not in var_map");
        self.builder.def_var(dest_var, result);
        if dest_is_heap {
            update_gc_root_if_needed(self.builder, &dest, result, self.gc_frame);
        }
        result
    }
}
```

### 4.5 — Decompose CodegenContext & Split God-Files (P21, P27)

**CodegenContext** decomposition:

```rust
pub struct CodegenContext<'a> {
    pub symbols: &'a CodegenSymbols,  // var_map, func_ids, func_name_ids, func_param_types
    pub module: &'a mut ObjectModule,
    pub gc: &'a GcContext,            // gc_frame_data, gc_pop_id, stack_pop_id
    pub debug: &'a DebugContext,      // line_map, return_type
    pub interner: &'a StringInterner,
}
```

**Split instructions.rs** (1,143 LOC):
```
codegen-cranelift/src/instructions/
├── mod.rs         # Main dispatch
├── copy.rs        # Copy, type coercion
├── arithmetic.rs  # BinOp, UnOp with Cranelift instructions
├── control.rs     # Branches, jumps (moved from terminators.rs)
├── calls.rs       # Call, CallVirtual, CallClosure
└── memory.rs      # Load, Store, GC operations
```

---

## Phase 5: Optimizer Evolution

**Goal:** Introduce proper pass management infrastructure.

**Independent of:** Phases 3-4. Can run in parallel.

**Depends on:** Phase 2 (MIR spans, for span-preserving transforms)

**Duration estimate:** Small-medium.

### 5.1 — Pass Trait Interface (P22)

```rust
// crates/optimizer/src/pass.rs

pub trait OptimizationPass {
    /// Human-readable pass name (for logging/debugging)
    fn name(&self) -> &str;

    /// Run one iteration of the pass. Returns true if any changes were made.
    fn run_once(&mut self, module: &mut mir::Module, interner: &mut StringInterner) -> bool;

    /// Maximum iterations for fixpoint (default: 10)
    fn max_iterations(&self) -> usize { 10 }

    /// Whether this pass should iterate to fixpoint
    fn is_fixpoint(&self) -> bool { true }
}
```

**Implement for each existing pass:**
```rust
pub struct Devirtualize;
impl OptimizationPass for Devirtualize {
    fn name(&self) -> &str { "devirtualize" }
    fn run_once(&mut self, module: &mut mir::Module, _: &mut StringInterner) -> bool { ... }
    fn is_fixpoint(&self) -> bool { false } // single pass
}

pub struct ConstantFolding;
impl OptimizationPass for ConstantFolding {
    fn name(&self) -> &str { "constfold" }
    fn run_once(&mut self, module: &mut mir::Module, interner: &mut StringInterner) -> bool { ... }
    fn max_iterations(&self) -> usize { 10 }
}
```

### 5.2 — Pass Manager (P23)

```rust
pub struct PassManager {
    passes: Vec<Box<dyn OptimizationPass>>,
}

impl PassManager {
    pub fn new() -> Self { Self { passes: Vec::new() } }

    pub fn add_pass(&mut self, pass: impl OptimizationPass + 'static) {
        self.passes.push(Box::new(pass));
    }

    pub fn run(&mut self, module: &mut mir::Module, interner: &mut StringInterner) {
        for pass in &mut self.passes {
            if pass.is_fixpoint() {
                for _ in 0..pass.max_iterations() {
                    if !pass.run_once(module, interner) {
                        break; // converged
                    }
                }
            } else {
                pass.run_once(module, interner);
            }
        }
    }
}

// CLI constructs pipeline:
pub fn build_pass_pipeline(config: &OptimizeConfig) -> PassManager {
    let mut pm = PassManager::new();
    if config.devirtualize { pm.add_pass(Devirtualize); }
    if config.flatten_properties { pm.add_pass(FlattenProperties); }
    if config.inline { pm.add_pass(Inliner::new(config.inline_threshold)); }
    if config.constfold { pm.add_pass(ConstantFolding); }
    // Peephole runs automatically if constfold or inline is enabled
    if config.constfold || config.inline { pm.add_pass(Peephole); }
    if config.dce { pm.add_pass(DeadCodeElimination); }
    pm
}
```

### 5.3 — Span Preservation in Optimizer

**Rule:** Every pass that transforms instructions must preserve the span from the original instruction.

```rust
// In each pass's run_once:
fn transform_instruction(old: &Instruction) -> Instruction {
    Instruction {
        kind: /* new kind */,
        span: old.span, // ALWAYS preserve
    }
}
```

---

## Phase 6: Final Integration

**Goal:** Address remaining cross-cutting concerns that benefit from all previous phases being complete.

**Depends on:** All previous phases.

### 6.1 — Cross-Module InternedString for Class Info (P28)

**Problem:** `CrossModuleClassInfo` uses `HashMap<String, ...>` instead of `HashMap<InternedString, ...>` because the interner is not shared across modules.

**Solution:** After Phase 3.2 (Lowering decomposition), the interner is accessible from a centralized location. Pass it through module boundaries:

```rust
pub struct CrossModuleClassInfo {
    pub field_offsets: IndexMap<InternedString, usize>,  // was String
    pub field_types: IndexMap<InternedString, Type>,      // was String
    pub method_return_types: IndexMap<InternedString, Type>, // was String
}
```

### 6.2 — Unified Type Dispatch Across All Crates

After all phases, verify that type dispatch follows a consistent pattern everywhere:

- **Runtime:** Vtable-based dispatch for common operations (print, repr, compare, hash)
- **Lowering:** Table-driven selection via `type_dispatch.rs` (Phase 3.6)
- **Codegen:** No type dispatch — all resolved at MIR level

No ad-hoc `match ty { ... }` should remain in codegen. In lowering, all type dispatch goes through the centralized tables. In runtime, only the vtable initialization needs type matching.

### 6.3 — Pipeline Span Propagation Audit

Walk through the entire pipeline and verify:
1. Frontend: all HIR nodes have spans ✓ (already true)
2. Lowering: all MIR instructions have spans (Phase 2.5 + 3.5)
3. Optimizer: spans preserved (Phase 5.3)
4. Codegen: spans used for debug info (Phase 2.5)
5. Diagnostics: all errors carry spans (Phase 0.1)

---

## Risk Analysis

### High-Risk Changes

| Change | Risk | Mitigation |
|--------|------|------------|
| Phase 2 (RuntimeFunc restructure) | Touches MIR/lowering/codegen simultaneously | Incremental migration per category (direct replacement, no aliases needed); signature audit critical; `cargo test --workspace` after each category |
| Phase 1.3 (Generic runtime functions) | Changes FFI interface codegen relies on | Keep old functions as thin wrappers; remove only after codegen is updated |
| Phase 3.3 (Separate type planning) | May expose hidden circular dependencies | Extensive testing; run full test suite after each function is moved |
| Phase 3.4 (Generator desugaring) | Complex control flow transformation | Compare output with current generator implementation for bit-exact equivalence |

### Low-Risk Changes

| Change | Why Low Risk |
|--------|-------------|
| Phase 0 (all) | Additive changes, no behavior change |
| Phase 1.1 (split files) | Pure structural refactoring, no logic changes |
| Phase 3.1 (dunder HashMap) | Simple data structure change, same semantics |
| Phase 5 (optimizer) | Additive trait layer over existing code |

### Regression Testing Strategy

Every phase must:
1. Pass `cargo test --workspace` (all unit + integration tests)
2. Pass `cargo clippy --workspace` (no new warnings)
3. Compile and run all `examples/test_*.py` files, comparing output with CPython
4. For Phases 1-2: also verify with `--emit-mir` that MIR structure is equivalent

---

## Success Metrics

### Quantitative Goals

| Metric | Original | Now (Phase 2 ~55%) | After Phase 2 | After All Phases |
|--------|----------|---------------------|---------------|-----------------|
| RuntimeFunc variants | ~316 | ~150 | ~30-40 | ~25 |
| Static RuntimeFuncDef defs | 0 | 165 | ~250 | ~250 |
| Codegen dispatch match lines | ~560 | ~210 | ~50 | ~30 |
| Codegen sub-modules | 22 | 13 | ~5 | ~3 |
| Runtime extern "C" functions | ~246 | ~246 | ~200 | ~160 |
| Lowering struct fields | 45+ | 45+ | 45+ | 6 (sub-structs) |
| Files to touch for new runtime func | 5+ | 2 (for migrated categories) | 2 | 1-2 |
| Files >1000 LOC (non-test) | ~15 | ~12 | ~10 | ~3 |
| Dunder method code (get/set) | ~500 lines | ~500 lines | ~500 lines | ~20 lines |
| Codegen signature boilerplate | 152× | ~50× | 0 | 0 |

### Qualitative Goals

- **New Python method support:** Define `RuntimeFuncDef` + implement `extern "C"` in runtime. Done.
- **New Python type support:** Add `TypeTagKind` variant + add rows to dispatch tables. Done.
- **New dunder method:** Intern the name + register in class lowering. No structural changes.
- **New optimizer pass:** Implement `OptimizationPass` trait + register in pipeline. Done.
- **Clear layering:** `core-defs` ← `types` ← `hir` ← `lowering` → `mir` → `codegen`. No backward deps.

---

## Implementation Notes

### Branching Strategy

Each phase should be developed on a dedicated branch:
- `refactor/phase-0-foundation`
- `refactor/phase-1-runtime`
- `refactor/phase-2-declarative-runtime-func`
- `refactor/phase-3-lowering`
- `refactor/phase-4-frontend-codegen`
- `refactor/phase-5-optimizer`
- `refactor/phase-6-integration`

Merge each phase to master only when all tests pass and the phase is complete. Do not interleave phases on the same branch.

### Incremental Migration Pattern

Phase 2 uses **direct replacement** per category (const aliases were planned but proved unnecessary):
1. Add `RuntimeFunc::Call(&'static RuntimeFuncDef)` variant alongside old variants
2. Add static `RuntimeFuncDef` definitions in `core-defs` for the target category
3. Update lowering to emit `RuntimeFunc::Call(&RT_XXX)` instead of `RuntimeFunc::Variant`
4. Remove old enum variants from `runtime_func.rs`
5. Remove codegen routing arm and delete the codegen sub-module file
6. Run `cargo test --workspace` (330 tests)

**Critical audit step** between 2 and 3: verify every descriptor's param count, param types, return type, and `gc_roots_result` flag against the original codegen. Signature mismatches cause Cranelift verifier errors or silent runtime corruption.

**Discovered special cases** that cannot be descriptor-ized:
- Functions that embed compile-time constants in the binary (`MakeStr`, `MakeBytes`, `AssertFail`, `ExcRegisterClassName`)
- Functions where the return type depends on the dest variable's type (`InstanceGetField`, `InstanceSetField`)
- "Functions" that aren't actually calls (`IterZip` = identity copy)

### Testing Invariants

After each commit within a phase:
- `cargo test --workspace` must pass
- `cargo clippy --workspace` must have no new warnings
- Compilation of all `examples/test_*.py` must produce identical output

This ensures the refactoring never breaks correctness, even mid-phase.
