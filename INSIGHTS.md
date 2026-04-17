# INSIGHTS.md

Non-obvious insights, gotchas, and hard-won knowledge about the Python AOT Compiler codebase. These are things that would trip up a developer working on this project for the first time.

---

## setjmp Must Be Called Directly From Cranelift Code

`setjmp`/`longjmp` exception handling requires that `setjmp` is called directly from the function whose context should be saved. **Never wrap `setjmp` in a Rust function** — after the wrapper returns, `longjmp` would try to restore a dead stack frame, causing SIGILL. In release mode this accidentally works because LTO inlines the wrapper, but in debug mode the wrapper's frame is separate and gets destroyed. The codegen (`codegen-cranelift/src/exceptions.rs`) imports `setjmp` directly and computes the jmp_buf address as `frame_ptr + 8` (offset of `jmp_buf` in `ExceptionFrame`).

## rt_get_type_tag Must Validate Pointer Alignment

`rt_get_type_tag(obj)` can receive non-Obj values (e.g., function pointers from closures/decorators with 4-byte code alignment). It must check `obj` alignment before dereferencing to avoid UB. Same applies to `rt_isinstance_class`. See `runtime/src/instance.rs`.

---

## List Element Storage: `elem_tag` Controls Everything

Lists have a dual storage mode controlled by `elem_tag` on `ListObj` (`runtime/src/list/core.rs`):

| Tag | Constant | Storage |
|-----|----------|---------|
| 0 | `ELEM_HEAP_OBJ` | Elements are `*mut Obj` pointers to boxed heap objects |
| 1 | `ELEM_RAW_INT` | Elements are raw `i64` values packed directly into data array |
| 2 | `ELEM_RAW_BOOL` | Elements are raw `i8` values |

**Using the wrong tag causes silent corruption in release builds.** The `validate_elem_tag!` macro only checks in debug builds. For example, calling `rt_list_get_int()` on an `ELEM_HEAP_OBJ` list interprets a pointer as an i64.

The compiler must pass the correct `elem_tag` when creating lists. This is especially important for `list(iterator)` — `rt_list_from_iter` takes `elem_tag` as a second parameter from the compiler since iterators can yield either raw ints or heap objects depending on the source.

**Debug builds** (`cargo build --workspace` without `--release`) enable type tag assertions that catch these mismatches at runtime.

---

## Dict Values Are Always Boxed

Dict stores ALL values as `*mut Obj` pointers in `DictEntry.value` — even primitives like `int`, `float`, `bool`.

- **Going in**: primitives are boxed via `rt_box_int`/`rt_box_float`/`rt_box_bool` during `DictSet`
- **Coming out**: `rt_dict_get` returns a raw `*mut Obj` pointer and does NOT unbox

The lowering must emit explicit `UnboxInt`/`UnboxFloat`/`UnboxBool` calls after `DictGet` when the value type is a primitive. If the dict type is `Dict(Any, Any)`, no unboxing occurs, and pointer values get interpreted as integers — producing garbage results.

**Key pattern** (`lowering/src/expressions/access/indexing.rs`):
```
DictGet → temp (*mut Obj)
UnboxInt(temp) → result (i64)    // only if value type is int
```

---

## Dict Comprehension Type Initialization

Dict comprehensions are desugared in `frontend-python/src/ast_to_hir/comprehensions.rs`:

```python
# Source
{x: x**2 for x in range(5)}

# Desugared to
__comp_N = {}                    # Empty dict → Dict(Any, Any)
for x in range(5):
    __comp_N[x] = x ** 2        # Bind { Index { ... } }
result = __comp_N
```

The initial empty `{}` has no type hint, so the variable starts with `Dict(Any, Any)`. Without type refinement, all subsequent `DictGet` operations would skip unboxing.

**Fix**: `lowering/src/statements/assign/bind.rs` performs dynamic type refinement during `Bind { Index { ... } }`. When it detects a `Dict(Any, Any)` target and the actual key/value types are known, it updates the variable's tracked type. This only works for direct variable references (`ExprKind::Var`).

---

## Map/Filter Type Inference and Captures

Map and filter iterators support up to **8 closure captures** (capture_count 0–8). The `call_map_with_captures`/`call_filter_with_captures` functions in `iterator/composite.rs` dispatch to the appropriate function pointer type based on capture count. Exceeding 8 captures triggers `abort()`.

`map(func, iterable)` returns `Iterator(elem_type)` where `elem_type` must be inferred from the function's return type, not defaulted to `Any`.

If the type system thinks `map()` returns `Iterator(Any)`, then `list(map(...))` creates a list with `elem_tag=0` (ELEM_HEAP_OBJ) even if the actual values are raw integers. This causes the GC to try tracing raw ints as heap pointers.

The type inference in `lowering/src/type_planning/infer.rs` inspects the function argument to `map()`:
- For `FuncRef`: checks `get_func_return_type()` or `func_def.return_type`
- For `Closure` (lambda with captures): same logic via extracted `func_id`
- For lambdas: uses `infer_lambda_return_type()`
- Fallback: `Type::Any`

**Gotcha**: Both `FuncRef` and `Closure` must be handled. Lambdas with captures are `Closure`, not `FuncRef`. Missing the `Closure` case causes `map(lambda_with_capture, ...)` to return `Iterator(Any)`.

---

## Release vs Debug Runtime Mismatch

The compiler links against a prebuilt `libpyaot_runtime.a` library found via `pyaot_linker::Linker`. It does NOT rebuild the runtime automatically.

**The gotcha**: If you modify runtime code and rebuild the compiler (`cargo build -p pyaot`) without rebuilding the runtime (`cargo build -p pyaot-runtime --release`), the compiler will link against a stale runtime library. This causes:
- Segfaults from ABI mismatches
- Wrong behavior from old function implementations
- Missing symbols if new functions were added

**Always use `cargo build --workspace --release`** to rebuild everything together.

---

## Floor Division and Modulo Semantics

Python's `//` and `%` operators have different semantics from Rust's `/` and `%` for negative numbers:

| Expression | Rust result | Python result |
|-----------|-------------|---------------|
| `-7 / 2` | `-3` (truncate toward 0) | `-4` (floor toward -inf) |
| `-7 % 2` | `-1` (sign of dividend) | `1` (sign of divisor) |

The runtime (`runtime/src/ops.rs`) adjusts after Rust's native operation:
```rust
// Floor division adjustment
let d = a / b;
let r = a % b;
if r != 0 && (r ^ b) < 0 {  // Signs differ (branchless check)
    d - 1
}
```

The `(r ^ b) < 0` trick is a branchless way to check if the remainder and divisor have different signs. Float floor division uses `f64::floor(a / b)` directly.

---

## Iterator Boxing: Raw Values Must Be Boxed for Generic Use

Some iterators yield raw `i64` values internally but callers may expect `*mut Obj`:

| Iterator source | Yields raw i64? |
|----------------|-----------------|
| `range()` | Always |
| `bytes` iterator | Always |
| `list[int]` with `ELEM_RAW_INT` | Yes |
| `list[str]`, `dict.keys()`, etc. | No (heap objects) |

The function `box_if_raw_int_iterator()` in `runtime/src/iterator/mod.rs` checks the iterator kind and element storage mode, boxing when needed. This is called when an iterator value needs to be passed to a generic context (e.g., `print()`, tuple construction).

---

## Type Refinement During Lowering

Variable types can change mid-lowering, not just at declaration. The lowering context tracks types in `var_type` map and updates them:

1. **Dict comprehension refinement**: `Dict(Any, Any)` → `Dict(int, int)` on first IndexAssign
2. **Union narrowing**: After `isinstance()` checks in if-branches, the variable type is narrowed
3. **Reassignment**: Assigning a different-typed value can update the tracked type

This means the same variable can have different types at different points in the lowering. Downstream operations (boxing decisions, unboxing, runtime function selection) depend on the type at that specific point.

---

## GC Shadow Stack Protocol

Every function that may allocate heap objects must:
1. Create a `ShadowFrame` on the stack (prologue)
2. Register it via `gc_push(frame_addr)`
3. Initialize all root slots to null
4. Update root slots before any allocation
5. Call `gc_pop()` on **every** return path (epilogue)

Missing `gc_pop()` on any return path corrupts the shadow stack — subsequent GC collections traverse dangling stack frames, leading to crashes or silent memory corruption.

The codegen (`codegen-cranelift/src/function.rs`) handles this automatically, but manual runtime functions must be careful.

---

## String Interning

Strings under 256 bytes are interned (deduplicated) at runtime:
- Compile-time string literals use `rt_make_str_interned`
- Dict keys are automatically interned (performance optimization for lookups)
- `sys.intern()` exposes this to Python code

Interned strings enable pointer equality for dict key comparisons — `eq_hashable_obj` checks `a == b` before any byte comparison. This is why dict lookup is fast but only works correctly when keys are interned consistently.

**Small string optimization**: `rt_make_str_impl` rounds allocation sizes to slab size classes (24/32/48/64 bytes). Strings ≤8 bytes → 32-byte slot, ≤24 → 48-byte slot, ≤40 → 64-byte slot. This ensures most strings use the slab allocator's O(1) bump allocation instead of system malloc.

---

## Closure Cell Variables

`nonlocal` variables use cell-based indirection (`runtime/src/cell.rs`):
- A cell is a heap-allocated box holding a single value
- The enclosing function allocates the cell and passes it to the closure
- Both the enclosing function and closure read/write through the cell

**Transitive capture**: Named nested functions and lambdas capture variables transitively at arbitrary depth. The free variable scanner recurses into nested function bodies and lambda expressions, bubbling up free variables to intermediate scopes. Intermediate functions automatically capture and forward variables (including cell pointers for `nonlocal` chains) to inner closures.

---

## Exception Handling: setjmp/longjmp

Exception handling uses C-style `setjmp`/`longjmp`:
- `ExceptionFrame` layout: `prev` (8 bytes) + `jmp_buf` (200 bytes) + `gc_stack_top` (8 bytes) + `traceback_depth` (8 bytes) = 224 bytes
- `try` pushes a frame via `rt_exc_push_frame`, then Cranelift calls `setjmp(frame_ptr + 8)` **directly** (not through a Rust wrapper — see "setjmp Must Be Called Directly")
- `raise` calls `longjmp` to jump to the most recent frame
- The GC stack top and traceback depth are saved/restored to prevent shadow stack and traceback corruption on longjmp

Exception objects store: type tag, custom class ID, message string, `__cause__` (for `raise X from Y`), `__context__` (implicit chaining), and `suppress_context` flag.

### Leak-Free Exception Raising

`longjmp` skips Rust destructors, so `format!()` Strings allocated before a raise are leaked. The runtime provides two leak-free patterns:

1. **`raise_exc!` macro** (for Rust callers with dynamic messages): formats the string, `forget()`s it, then calls `rt_exc_raise_owned` which transfers the buffer directly to `ExceptionObject`. The buffer is freed when the exception is dropped.
   ```rust
   raise_exc!(ExceptionType::ValueError, "invalid value: '{}'", user_input);
   ```

2. **`raise_*_error_owned(String)`** helpers in `utils.rs`: same zero-copy ownership transfer for code using the `raise_value_error`/`raise_io_error`/`raise_runtime_error` pattern.
   ```rust
   crate::utils::raise_io_error_owned(format!("read error: {}", e));
   ```

For static messages (no runtime data), use `b"..."` byte string literals — they have no `Drop` and never leak:
```rust
let msg = b"len() argument is None";
rt_exc_raise(TypeError, msg.as_ptr(), msg.len());
```

The internal dispatch is factored into `dispatch_to_handler(Box<ExceptionObject>)` and `raise_with_owned_message(exc_type, ptr, len, cap)`, shared by all raise variants (`rt_exc_raise`, `rt_exc_raise_owned`, `rt_exc_raise_from`, `rt_exc_raise_custom`, etc.).

---

## Comprehension Variable Scoping

List/dict/set comprehensions are desugared into:
```python
__comp_N = []  # or {} for dict/set
for x in iterable:
    __comp_N.append(expr)  # or __comp_N[k] = v for dict
```

The loop variable `x` is created in the current scope (not isolated like in CPython 3). This matches CPython behavior for module-level code but differs for function-level code where CPython uses a nested function for comprehension scoping.

The `__comp_N` naming uses a counter to avoid collisions with nested comprehensions.

---

## MIR Parameterized Enums

MIR uses parameterized enums (in `mir/src/kinds.rs`) to avoid an explosion of `RuntimeFunc` variants. Instead of separate variants for `PrintInt`, `PrintFloat`, `PrintStr`, etc., there is one `Print` with a `PrintKind` parameter.

When adding a new operation that works across types, prefer adding a new kind enum over new RuntimeFunc variants. The mapping from `Type` to the appropriate kind is done in `lowering/src/runtime_selector.rs` via `type_to_value_kind()` and similar helpers.

---

## Stdlib Declarative Hints

Stdlib functions use `LoweringHints` to control how they're lowered without any custom code:

- `variadic_to_list`: Collects variadic args into a list before calling runtime function
- `auto_box`: Automatically boxes primitive arguments to `*mut Obj`
- `min_args` / `max_args`: Argument count validation at compile time

This means adding a new stdlib function requires only 2 files: the definition in `stdlib-defs` and the implementation in `runtime`. No lowering or codegen changes are needed unless the function has unusual semantics.

## GC Heap Field Mask for Instances and Tuples

Fields in instances and tuples can be mixed: some are heap pointers, some are raw values (int, float, func_ptr). The GC must not dereference raw values as pointers.

**Instances** use a per-class mask registered at class definition:
- `ClassInfo` in `vtable.rs` has `heap_field_mask: u64` — bit i set means field i is a heap pointer
- Compiler emits `RegisterClassFields(class_id, mask)` at module init

**Tuples** use a per-instance mask stored in the object:
- `TupleObj` has `heap_field_mask: u64` alongside `elem_tag`
- For homogeneous tuples (all `ELEM_RAW_INT`): `mask = 0`; for all `ELEM_HEAP_OBJ`: `mask = u64::MAX`
- For mixed-type tuples (closure captures with int + heap values): per-field bits via `rt_tuple_set_heap_mask` / `TupleSetHeapMask` MIR instruction
- GC's `mark_object` for tuples iterates fields using the mask, not `elem_tag`

**Key**: closure capture tuples `(func_ptr, captures_tuple)` always need a mask because func_ptr is raw but captures_tuple is heap. The lowering computes the mask from `operand_type()` of each capture and emits `TupleSetHeapMask` for any `ELEM_HEAP_OBJ` tuple with mixed types.

Three closure creation paths must all set the mask:
1. `statements/assign.rs` — nested function closures (`def inner(): ...`)
2. `expressions/mod.rs` — lambda closures passed as values
3. `expressions/builtins/iteration.rs` — map/filter captures

## Tuple Type System — Fixed vs Variable-Length

Area D of `MICROGPT_PLAN.md` split the tuple type into two variants to track PEP 484's distinction at compile time:

```rust
pub enum Type {
    ...
    Tuple(Vec<Type>),          // fixed-length heterogeneous — tuple[int, str, float]
    TupleVar(Box<Type>),       // variable-length homogeneous — tuple[int, ...]
    ...
}
```

**Same runtime layout.** Both map onto `TupleObj` with `elem_tag` — the distinction lives purely in the compiler. `t[k]` on a fixed `Tuple` uses compile-time bounds-checked indexing with the static per-slot type; `t[k]` on `TupleVar` emits `rt_tuple_get` with a runtime bounds-check and returns the homogeneous element type. Iteration (`for x in t`), `len(t)`, `zip(t, ...)` dispatch identically via the existing runtime.

### Shape unification — `Type::unify_tuple_shapes`

`crates/types/src/lib.rs::unify_tuple_shapes` is the canonical decision tree for merging two tuple-typed values (used by class-field inference when the same `self.<field>` is assigned tuples of different shapes in different methods):

| LHS | RHS | Result |
|---|---|---|
| `Tuple([])` (empty) | anything | absorbed into RHS (empty is compatible with every tuple shape) |
| `Tuple([a1..an])` | `Tuple([b1..bn])` (same length) | element-wise union, **keep fixed shape** — `Tuple([union(a_i, b_i)])` |
| `Tuple([a1..an])` | `Tuple([b1..bm])` (diff lengths) | collapse to `TupleVar(normalize_union(all a_i ∪ all b_j))` |
| `TupleVar(e1)` | `TupleVar(e2)` | `TupleVar(normalize_union([e1, e2]))` |
| `TupleVar(e)` | `Tuple([t_j])` (either side) | `TupleVar(normalize_union([e] ∪ t_j))` — fixed absorbed into variable |
| non-tuple | non-tuple | falls back to `normalize_union` |

### Class-field scan walks all methods

`frontend-python/src/ast_to_hir/classes.rs::scan_method_for_self_fields` now walks **every** method in the class body (not just `__init__`). Each site's inferred type merges via `unify_tuple_shapes` when the observed types are tuples. `infer_field_type_from_rhs` recognises tuple literals (`()`, `(a,)`, `(a, b)`) and tuple-typed constants, so literal shape info feeds the merge directly. Parameter defaults without annotations are also honoured — `class Node: def __init__(self, children=())` contributes `Tuple([])` for the `_children` field even though no explicit annotation exists.

### Cross-tag list equality — `rt_list_eq`

`crates/runtime/src/list/compare.rs::list_elem_eq` is a new helper that dispatches on the `(tag_a, tag_b)` pair per element. `rt_list_eq` calls it for every index instead of assuming both lists share one `elem_tag`. This closes the Area A §A.6 #2 segfault where `rest == [2, 3]` crashed because `rest` ended up `ELEM_HEAP_OBJ` (from a heterogeneous-tuple starred slice) while `[2, 3]` stayed `ELEM_RAW_INT` (int-literal list). The pattern mirrors the tuple mixed-storage comparison already in `hash_table_utils.rs::eq_hashable_obj`.

### PEP 563 string forward references

`ast_to_hir/types.rs::convert_type_annotation` recognises string-constant annotations and re-parses them eagerly:

```rust
py::Expr::Constant(c) if let py::Constant::Str(source) = &c.value => {
    let reparsed = rustpython_parser::parse(source, Mode::Expression, "<annotation>")?;
    // recurse into the parsed expression
}
```

This handles `other: "V"`, `children: "tuple[Node, ...]"`, forward refs to later-declared classes, and recursive self-references. Two complementary mechanisms make it work:

1. **Top-level class pre-scan.** Before converting any class body, the frontend walks module-level `StmtClassDef` nodes and populates `symbols.class_map` with every class name → `ClassId`. Forward refs to later-declared classes resolve immediately. Duplicate top-level class names (a common test-file pattern) each get a fresh `ClassId`; earlier `ClassRef`s keep their original id; `class_map` rebinds to the latest.
2. **`from __future__ import annotations`** parses through as a documentation marker — our AOT is eager-evaluate by design, so every annotation (stringified or not) is resolved at compile time. The import has no effect on type resolution but is not rejected as a syntax error.

**Cross-references:**
- `crates/types/src/lib.rs::unify_tuple_shapes` — shape merge
- `crates/frontend-python/src/ast_to_hir/classes.rs::scan_method_for_self_fields` — all-methods field scan
- `crates/frontend-python/src/ast_to_hir/types.rs::convert_type_annotation` — string forward-ref re-parse
- `crates/runtime/src/list/compare.rs::list_elem_eq` — cross-tag list equality
- `crates/runtime/src/list/compare.rs::rt_list_eq` — per-element tag dispatch

### Known limitation — call-site cross-field inference (deferred to Area E)

The intra-class `scan_method_for_self_fields` walks assignments *inside* the class body. Widening a field's type based on **call-site argument types** remains out of scope:

```python
class Node:
    def __init__(self, data, children=()):
        self._children = children

a = Node(1)                     # children=() → Tuple([])
b = Node(2, (a,))               # caller hands in Tuple([Class(Node)])
c = Node(3, (a, b))             # caller hands in Tuple([Class(Node), Class(Node)])
# Desired: Node._children : TupleVar(Class(Node))
# Actual:  Node._children stays at whatever the default-value inference produces (Tuple([]))
```

This requires cross-site inference that threads argument types at every constructor call back into the class's field type. Area E (cross-site type inference for class attributes with numeric promotion) is the natural home. Until then, widen explicitly with an annotation: `children: tuple[Node, ...] = ()`.

## Expected Type Propagation for Empty Collections

Empty collection literals (`[]`, `{}`) have no elements to infer the type from, defaulting to `List(Any)` → `ELEM_HEAP_OBJ`. This causes GC issues when ints are later appended.

The lowering context has `expected_type: Option<Type>` — set before lowering the RHS of an assignment from the target variable's known type. `lower_list` checks this for empty lists to determine the correct `elem_tag`.

Propagation sites:
- `lower_assign` — from variable type hint or existing var type
- `emit_mutable_default_initializations` — from parameter type annotation
- `desugar_list_comprehension` — from `infer_comprehension_elem_type()` (checks if all generators are `range()` or int-list, and element expression is int)

## GlobalSet(Ptr) Type Coercion

When storing values into global pointer slots via `GlobalSet(Ptr)`, the runtime expects i64. Values from different types need coercion:
- `None` (i8) → `uextend` to i64
- `float` (f64) → `bitcast` to i64
- `int/str/list/...` (i64) → pass through

The coercion must check the Cranelift value type directly (not the MIR operand kind), because `Constant(None)` doesn't have an associated MIR type annotation the way `Local` operands do.

## Dict/Set to List Boxing Mismatch

Dict and set store ALL elements as `*mut Obj` (boxed heap objects), even for `dict[str, int]` or `set[int]`. When converting to a list (via `.keys()`, `.values()`, `sorted()`, etc.), the result list must match the compiler's expected `elem_tag`:

- `list[int]` → `ELEM_RAW_INT` (raw i64 in data array)
- `list[str]` → `ELEM_HEAP_OBJ` (pointers in data array)

If the runtime always creates `ELEM_HEAP_OBJ` lists from dict/set, the compiler's `rt_list_get_int` reads a pointer as an integer → garbage values.

**Fix**: `rt_dict_keys`, `rt_dict_values`, `rt_sorted_set`, `rt_sorted_dict` all accept an `elem_tag` parameter. When `elem_tag == ELEM_RAW_INT`, they unbox `IntObj` values to raw i64 before storing in the result list. The lowering passes the correct `elem_tag` via `Self::elem_tag_for_type()`.

**Codegen note**: The `elem_tag` parameter uses `u8` in runtime but `i64` in MIR constants. The codegen must `ireduce` from i64 to i8 before the call.

## GC Architecture

The GC uses lock-free global state via `static GC_STATE_PTR: AtomicPtr<GcState>` (`runtime/src/gc.rs`). All access is through `unsafe fn gc_state() -> &'static mut GcState` — no mutex, no locking.

**Single-threaded design**: The project has no threading support (no async/await, no threading module). All runtime statics (`GcState`, boxing pools, string pool, globals, class attrs, vtable/class registries) use `UnsafeCell` for zero-overhead access. Any future work on parallelism must add synchronization.

**Slab allocator**: Objects ≤ 64 bytes are allocated from a slab allocator (`runtime/src/slab.rs`) with 4 size classes (24/32/48/64 bytes). Bump-pointer allocation from 4KB pages is ~10x faster than system malloc. Slab-allocated objects are NOT tracked in `GcState.objects` Vec — they are swept by iterating slab pages directly. Only objects > 64 bytes use system malloc and the Vec.

**Shadow stack leaf optimization**: Functions with `nroots == 0` (no heap-type locals) skip `gc_push`/`gc_pop` entirely. The codegen checks this at compile time (`function.rs:135`). Pure-computation functions (only int/float/bool locals) have zero GC overhead.

**Map iterator boxing flag**: The map callback ABI is `fn(*mut Obj) -> *mut Obj` for all callbacks. User-defined functions compiled by Cranelift receive raw native types (i64 for int), so raw int elements from `ELEM_RAW_INT` lists pass through correctly. But builtin functions (like `str`, `int`) genuinely expect `*mut Obj` pointers. To distinguish, the compiler sets bit 7 of `capture_count` in `MapIterObj` (0x80 flag) for builtins, causing `iter_next_map` to box raw elements via `box_if_raw_int_iterator` before calling the callback. See `lowering/src/expressions/builtins/iteration.rs` and `runtime/src/iterator/next.rs`.

**Test isolation**: `gc::init()` is idempotent (checks for null AtomicPtr). `gc::shutdown()` frees all objects but doesn't reset the AtomicPtr. Tests use `RUNTIME_TEST_LOCK` mutex in `lib.rs` for serialization.

---

## Unified Type System: `type_planning/`

All type inference is in one module `crates/lowering/src/type_planning/`:

```
type_planning/
  mod.rs       — public API: get_type_of_expr_id, get_expr_type, run_type_planning
  infer.rs     — bottom-up: compute_expr_type(&mut self) → Type (memoized in expr_types)
  pre_scan.rs  — pre-scan: closure/lambda/decorator discovery before codegen
  check.rs     — top-down: check_expr_type validates types, reports CompilerWarning::TypeError
```

**No RefCell** — `compute_expr_type(&mut self)` stores results directly in `expr_types: HashMap<ExprId, Type>`. Memoized types persist across functions (ExprIds are unique per-module).

**Bidirectional propagation** via `lower_expr_expecting(expr, expected_type, ...)`:
- Assignment: `x: list[int] = []` → expected = `list[int]`
- Return: `def f() -> list[int]: return []` → expected = return type
- Call args: `f([])` where `f(x: list[int])` → expected = param type
- Defaults: `def f(x: list[int] = [])` → expected = param type
- Empty containers read `expected_type` to determine elem_tag

**Type checking** at 3 points — reports `CompilerWarning::TypeError`:
- `x: int = "str"` → assignment mismatch
- `return "str"` where `-> int` → return mismatch
- `f("str")` where `def f(x: int)` → arg mismatch + missing arg detection
- Python `int → float` promotion allowed; `*args`/`**kwargs` skip checking

---

## Init-Only Field Discovery

Instance fields can be declared two ways:
1. Class-level annotation: `x: int` (without value) in the class body
2. Assignment in `__init__`: `self.x = value`

The frontend (`frontend-python/src/ast_to_hir/classes.rs`) scans `__init__` bodies for `self.field = value` patterns to discover fields not declared at the class level. Type inference:
- If the RHS is a simple parameter reference (`self.x = x`) and the parameter has a type annotation, the annotation type is used
- Otherwise, `Type::Any` is used

Fields declared at the class level take precedence (no duplicates). The scan recurses into `if`/`for`/`while`/`try` blocks.

## Class Attribute Access Through Instances

Python allows accessing class attributes through instances: `instance.class_attr`. The lowering (`lowering/src/expressions/access/attributes.rs`) checks in order:
1. `@property` getters
2. Instance fields (`field_offsets`)
3. Class attributes (`class_attr_offsets`) — fallback

This matches Python's MRO: instance dict first, then class dict. Assignment through instances (`instance.class_attr = value`) modifies the shared class attribute (not an instance-specific shadow, unlike CPython).

---

## Type Inference: Two Parallel Functions, Cannot Merge

`compute_expr_type` (codegen, `&mut self`) and `infer_expr_type_inner` (pre-scan, `&self`) in `type_planning/infer.rs` have nearly identical match arms but **cannot be unified into one function** due to a memoization constraint.

During lowering, `var_types` evolves as statements are processed (e.g., after `x = "hello"`, `x` changes from `Any` to `Str`). `compute_expr_type` recurses through `get_type_of_expr_id` which caches sub-expression types in `expr_types`. This cache freezes types at first access, ensuring the same expression consistently returns the same type throughout codegen. Without this cache, the same variable expression would return `Any` before assignment and `Str` after — producing inconsistent MIR that causes runtime segfaults.

`infer_expr_type_inner` runs during pre-scan (before lowering starts) and takes `&self`, so it cannot call `get_type_of_expr_id` (`&mut self`). It also must not write to `expr_types` — doing so would freeze pre-scan types that the codegen path would later read as authoritative.

The two functions share complex logic via `resolve_*` helper methods (`resolve_method_on_type`, `resolve_call_target_type`, `resolve_builtin_with_overrides`, `resolve_attribute_on_type`, `resolve_index_with_getitem`). The remaining duplication is only in the thin match arms that resolve sub-expressions differently and apply fallbacks.

**When adding a new `ExprKind`:** add the match arm to BOTH functions. Do NOT add explicit literal arms to `compute_expr_type` — it relies on the `_ => expr.ty` fallback for literals to preserve consistency with the caching model.

---

## Decorator `*args` Forwarding — Runtime Trampoline

Decorator wrappers with `*args` use a runtime trampoline (`rt_call_with_tuple_args`) to forward variable-length argument tuples through indirect calls. The caller packs user args into a varargs tuple via `resolve_call_args`, and inside the wrapper, `func(*args)` calls the trampoline which dispatches based on tuple length (up to 8 args). For chained decorators (closure case), captures and args are concatenated via `rt_tuple_concat` before trampolining. The type inference for decorated function calls uses the **original** function's return type (not the wrapper's `Any`) via `module_var_wrappers` lookup.

---

## DWARF Debug Info: macOS vs Linux

DWARF debug sections (`.debug_info`, `.debug_line`, `.debug_abbrev`, `.debug_str`) are generated in the `.o` object file by `codegen-cranelift/src/debug_info.rs` using `gimli::write`. The behavior differs by platform:

**Linux (ELF)**: The linker copies DWARF sections from `.o` into the executable. Source-level breakpoints (`b file.py:10`) work directly in `gdb`/`lldb` because the debugger reads DWARF from the executable.

**macOS (Mach-O)**: The macOS linker (`ld`) does NOT copy `__DWARF` sections from `.o` files to the executable. Instead, it creates "debug map" entries (N_OSO stabs) pointing to the `.o` files, and `dsymutil` later reads both to build a `.dSYM` bundle. Since Cranelift doesn't generate N_OSO stab entries, `dsymutil` doesn't know to extract our DWARF sections from the `.o` file.

**Consequences on macOS**:
- Function-name breakpoints (`b add`, `b my_function`) work — these use the symbol table, not DWARF
- Source-level breakpoints (`b file.py:10`) do NOT work — `lldb` can't find the line table
- `dwarfdump program.o` shows correct DWARF data (line tables, subprograms)
- The `.o` file is kept in debug mode (not deleted after linking) so `dwarfdump` can inspect it

**Workaround**: Use `dwarfdump --debug-line program.o` to find the address for a source line, then set an address breakpoint in lldb: `breakpoint set -a 0x<address>`.

**Future fix options**: Generate N_OSO stab entries, embed DWARF in the executable post-link, or use `ld -r` (relocatable link) to merge DWARF before final linking.

---

## DWARF Span Propagation: Ambient Span Pattern

Source locations flow through the compiler via an "ambient span" pattern to minimize code churn:

1. **HIR**: Every `Stmt` and `Expr` carries `span: Span` (byte offset range in source file)
2. **Lowering**: `Lowering.current_span: Option<Span>` is set at statement/expression entry:
   - `lower_stmt()` sets `self.current_span = Some(stmt.span)` before dispatch
   - `lower_expr()` saves/restores: sets span from `expr.span`, restores previous span after
3. **MIR**: `Instruction.span: Option<Span>` is populated from `current_span` in `emit_instruction()`
4. **Codegen**: For each instruction, `builder.set_srcloc(SourceLoc::new(span.start))` stores the byte offset in Cranelift IR (not line number — byte offset preserves column information)
5. **Cranelift**: Preserves `SourceLoc` through compilation. `compiled_code().buffer.get_srclocs_sorted()` returns `Vec<MachSrcLoc>` mapping code offsets to byte offsets
6. **DWARF**: `DebugInfoBuilder` collects srclocs per function, converts byte offsets to `(line, column)` via `LineMap.line_col()`, then `gimli::write` generates the `.debug_line` section with column precision

Additional DWARF entries:
- `DW_TAG_base_type` for `int`, `float`, `bool`, `str` — created once at compilation unit root
- `DW_TAG_formal_parameter` with `DW_AT_name` + `DW_AT_type` — parameter names come from `MIR Local.name` (set from `hir_param.name` during lowering)
- Compiler-internal functions (`__pyaot_*`, `__module_*`) are filtered out of DWARF output

Generator-created instructions (state machine, dispatch) use synthetic spans from the generator function definition. After HIR-level desugaring, the desugared functions are lowered as regular functions, and their instructions get the source span of the original generator function definition.

---

## ExceptionFrame Size Must Match Between Runtime and Codegen

`ExceptionFrame` is `#[repr(C)]` and its size is hardcoded in codegen as `EXCEPTION_FRAME_SIZE` (`codegen-cranelift/src/exceptions.rs`). The runtime struct definition (`runtime/src/exceptions.rs`) and the codegen constant **must be updated in lockstep**. If they diverge, `jmp_buf` offsets will be wrong, causing SIGILL on longjmp. Current layout (224 bytes): `prev` (8) + `jmp_buf` (200) + `gc_stack_top` (8) + `traceback_depth` (8). All raise variants now share `dispatch_to_handler()` for the longjmp logic, eliminating duplication.

## Traceback Stack Depth Unwinding on longjmp

When an exception is raised and a handler exists, `longjmp` skips all intermediate function returns — their `rt_stack_pop` calls never execute. To keep the traceback stack consistent, each `ExceptionFrame` saves the traceback depth at try-block entry (`rt_exc_push_frame`). All raise functions restore this depth before `longjmp`, exactly like the GC shadow stack unwinding pattern.

## Exception Instances Survive longjmp via GC Root Scanning

Exception instances are heap-allocated `InstanceObj` objects. When `longjmp` unwinds the GC shadow stack, roots pushed after `setjmp` are lost. But exception instances stored in `ExceptionObject.instance` (thread-local `ExceptionState`) survive because `gc::mark_roots()` explicitly scans `ExceptionState` via `mark_exception_pointers()`. This walks both `current_exception` and `handling_exception` chains (including cause/context).

## Lazy vs Eager Exception Instance Creation

Built-in exceptions (ValueError, etc.) create instances lazily — only when `except E as e:` binds the variable. The instance is created by `create_builtin_exception_instance()` with a single `.args` field containing a tuple of the message. Custom exception classes with `__init__` create instances eagerly at raise time via `lower_class_instantiation()`, so custom fields (e.g., `self.status`) are preserved. The instance pointer is passed to `rt_exc_raise_custom_with_instance()`.

## Exception Class ID Space

Built-in exception type tags 0-27 are reserved in the class registry. `FIRST_USER_CLASS_ID = BUILTIN_EXCEPTION_COUNT` (28). All 28 built-in exceptions are registered in `rt_init_builtin_exception_classes()`. User-defined exception classes get IDs starting at 28+. Exception isinstance checks and raise operations use raw class_id (not offset-adjusted), which is consistent within single-module compilation. Multi-module exception class IDs may need adjustment.

## str(e) Reads Message from ExceptionState, Not Instance Fields

`rt_exc_instance_str()` first checks thread-local `ExceptionState` for the message (matching by instance pointer), then falls back to reading field 0 (.args) from the instance. This is necessary because custom exception instances may not have `.args` as field 0 — their fields come from `__init__` (e.g., `self.status`). The ExceptionState always has the original message string from the raise site.

---

## MIR BinOp/UnOp Are NOT Pure for DCE

In the optimizer's dead code elimination pass, `BinOp` and `UnOp` must **not** be treated as side-effect-free. This compiler uses i64 arithmetic, so:
- `Add`, `Sub`, `Mul`, `Pow` can raise `OverflowError` (i64 overflow)
- `Div`, `FloorDiv`, `Mod` can raise `ZeroDivisionError`
- `Neg` (UnOp) can overflow on `i64::MIN`

If DCE removes a "dead" `x = 10 // 0` because `x` is unused, the `ZeroDivisionError` that should fire inside a `try` block is silently lost. Only `Const`, `Copy`, `FuncAddr`, `BuiltinAddr`, and safe type conversions (`BoolToInt`, `IntToFloat`, `FloatBits`, `IntBitsToFloat`, `FloatAbs`) are truly pure. See `optimizer/src/dce/mod.rs:instruction_is_pure()`.

---

## Generator Truthiness and Bool Yield

Since generators are desugared at HIR level (`lowering/src/generators/desugaring.rs`), yield value expressions are handled by the **full expression lowering engine** — no separate code paths needed. The desugared resume function's body contains the original HIR expressions, which are lowered with complete type dispatch including truthiness checks via `convert_to_bool`.

**Key pitfalls** (all addressed):
- Generator resume functions always return i64, so `Bool` yield values must be widened via `BoolToInt` before return, and `infer_generator_yield_type_for_desugar` normalizes `Bool` → `Int`.
- The desugaring pass uses `expr.ty` from the frontend (not `get_type_of_expr_id`, which requires type planning that hasn't run yet) for yield type inference.

---

## collections.namedtuple Is Fundamentally Incompatible With AOT

`namedtuple("Point", ["x", "y"])` creates a new class dynamically at runtime from string arguments. This is impossible in an AOT compiler where all types must be known at compile time. The class name and field names are arbitrary strings — the compiler cannot generate code for a type whose structure is determined by runtime values.

The `typing.NamedTuple` class-based syntax (`class Point(NamedTuple): x: int; y: int`) could theoretically be supported because the class definition is available at compile time, but it would require auto-generating `__init__`, `__repr__`, `__eq__`, `__hash__`, `__iter__`, `__len__`, `_asdict()`, `_replace()`, and `_fields` in the class compilation pipeline — a significant change.

## collections.defaultdict Factory Is Resolved at Compile Time

Unlike CPython where `defaultdict` stores an arbitrary callable as the factory, this AOT compiler resolves the factory to a numeric tag at compile time. This means only builtin type constructors are supported as factories: `int`, `float`, `str`, `bool`, `list`, `dict`, `set`. User-defined functions or lambdas cannot be used as defaultdict factories.

The factory resolution happens in the frontend (`ast_to_hir/expressions.rs:resolve_defaultdict_factory`), which maps the Python name to an integer tag (0-6). The runtime (`defaultdict.rs:create_default_value`) uses this tag to construct the default value.

## DefaultDict/Counter Use DictObj Directly — No Separate Struct

DefaultDict and Counter use the **same `DictObj` struct** as regular dicts (identical memory layout and size). They differ only by `TypeTagKind` in the object header. This ensures all dict operations (`rt_dict_set`, `rt_dict_get`, `rt_dict_keys`, `rt_dict_contains`, etc.) work correctly via simple pointer casting.

For defaultdict, the factory tag is packed into the **high byte (bits 56–63) of `DictObj::entries_capacity`**. This works because real capacities are power-of-two values well below 2^56 on 64-bit platforms. Helpers in `dict.rs` (`real_entries_capacity`, `set_real_entries_capacity`) mask the tag byte when reading/writing capacity. This avoids a separate registry keyed by pointer address (which would break when the slab allocator reuses addresses for new objects).

## Type::HeapAny — Distinguishing Heap Pointers From Raw Values

`Type::Any` is ambiguous — a value typed as `Any` could be a raw i64 (e.g., from dict unboxing) or a `*mut Obj` heap pointer (e.g., from `list[i]` where element type is unknown). This distinction matters for print and compare dispatchers that need to know whether to dereference the value.

`Type::HeapAny` was introduced as a guaranteed `*mut Obj` variant. Print uses `PrintKind::Obj` (runtime type-tag dispatch via `rt_print_obj`) and comparison uses `CompareKind::Obj` (runtime dispatch via `rt_obj_eq`) for `HeapAny`, while `Any` keeps the legacy `PrintKind::Int` / `BinOp` behavior.

**HeapAny producers** (places that set result type to `HeapAny`):
- `AnyGetItem` result in `indexing.rs` (runtime-dispatched subscript)
- `List(Any)` and `Tuple([Any])` element access in `resolve_index_type` and subscript lowering
- `ObjectMethodCall` returns with `TypeSpec::Any` in `stdlib.rs`

## Cross-Module Types Must Round-Trip Through `RawType`

Each module has its own `StringInterner` — `InternedString(7)` in module A and module B refer to different strings. A `Type::Class { class_id, name }` stored in module A carries A's local class id (e.g. 61) and A's interned name. Cloning that `Type` into module B's lowering context misroutes both: B's `cross_module_class_info` is keyed by B-side *remapped* class ids, and B's interner would resolve `InternedString(7)` to whatever happens to live at that slot.

`mir_merger::type_to_raw` serializes Types into `RawType` (interner-free mirror with class ids already offset-adjusted via `class_id + class_id_offset`). `raw_to_type` reconstructs per caller, re-interning names through the caller's interner. This applies to:

- `module_var_exports` values
- `module_func_exports` return types
- `cross_module_class_info` field types and method return types

When storing method return types in `raw_cross_module_class_info`, HIR func_def names are mangled as `ClassName$method` — strip the prefix with `rsplit_once('$')` before inserting so callers can look up `info.method_return_types[method_name]` with a bare method name.

## Cross-Module User-Class Annotations Use Placeholder Class Ids

Annotations like `r: mymod.Response` are parsed BEFORE `mir_merger` runs, so the real remapped class id isn't known yet. The frontend (`AstToHir::alloc_external_class_ref`) hands out unique placeholder ids from `u32::MAX` downward and records `(module, class_name)` in `hir::Module.external_class_refs`. Placeholder ids can't collide with real user class ids because those grow upward from `FIRST_USER_CLASS_ID` (61).

`mir_merger`'s second pass builds a `placeholder → (real_remapped_id, class_name)` map per module (via `module_class_exports` lookup), then `resolve_external_class_refs` walks the HIR and rewrites every `Type::Class` with a placeholder id. The walker covers:

- `func_def.params[_].ty` and `func_def.return_type`
- `class_def.fields[_].ty` and `class_def.class_attrs[_].ty`
- `expr.ty` on every expression
- `StmtKind::Assign { type_hint }`

The rewrite descends into `Type::List`, `Dict`, `Tuple`, `Union`, `Function`, etc., so `list[mymod.Foo]` and `Optional[Foo]` also work.

## Type Narrowing on `Any` / `HeapAny` Must Route to the Target

`isinstance(x, T)` inside an `if` branch needs to narrow the variable's compile-time type so that downstream dispatchers (`lower_len`, `lower_print`, `select_compare_func`) pick the right runtime call. For `Union` types this works naturally — the narrowed type is the Union element that matches `T`. For `Type::Any` and `Type::HeapAny` it didn't: `narrow_to` fell through to the generic arm, asked `types_match_for_isinstance(Any, T)` (which has no `Any` case and returns `false`), and produced `Type::Never`.

The symptom was subtle: `len(x)` where `x: Any` under `isinstance(x, str)` returned `0` silently (the `_ => Int(0)` fallback in `lower_len`). For `requests.post(data="string")`, the `data.encode()` path reached `encode()` with the narrowed type set to `Never`, so the compiler emitted a no-op body, and the server received an empty request.

Fix: `Type::Any` and `Type::HeapAny` narrow to the isinstance target type. See `Type::narrow_to` in `crates/types/src/lib.rs`. The else-branch still returns `Any` via `narrow_excluding`'s non-Union arm (correct — after `if isinstance(x, str): ... else:`, `x` can still be anything except `str`).

## Don't Cache `Var` Expression Types

`get_type_of_expr_id` memoizes expression types in `TypeEnvironment.expr_types`. This is correct for derived expressions (Call, BinOp, Attribute) whose type is a pure function of their sub-expressions. It's WRONG for `Var(id)` expressions: `Var`'s type comes from `get_var_type(id)`, which changes between `apply_narrowings` and `restore_types` calls. If the cache was populated before narrowing, a subsequent lookup inside the narrowed branch returns the stale pre-narrow type.

`get_type_of_expr_id` now bypasses the cache for `Var` expressions and re-runs `compute_expr_type` each time. Recomputation is a single HashMap lookup — no measurable cost.

## `mir_merger` Must Remap `CallDirect` FuncIds

When merging per-module MIR into one flat `HashMap<FuncId, Function>`, `mir_merger` assigns fresh `FuncId`s to avoid collisions — but instruction-embedded references are NOT in the main name-remap pass. For years this didn't matter because existing multi-module tests only used `CallNamed` (symbol-name-based, immune to id renumbering), never `CallDirect`.

The moment a user defines their own function in `main.py` alongside any import — e.g. `def foo(x): ...` called directly as `foo(5)` — `mir_merger` must also walk every `CallDirect.func` and `VtableEntry.method_func_id` through the per-module `old → new FuncId` table. Without this, the local `CallDirect` dispatches to whichever function happens to land at that id in the source-module slice, typically producing `mismatched argument count` in the Cranelift verifier.

See `mir_merger::remap_instruction_func_ids` — only `CallDirect` carries a raw `FuncId`; `CallNamed` routes by symbol, `Call` by operand, `CallVirtual{,Named}` by vtable slot. None of those need remapping.

## `Type::File(binary)` Flows From AST, Not Type-Planning

`open(path, "rb").read()` must type-infer as `bytes`, but `open(path, "r").read()` as `str`. The binary/text flag lives on `Type::File(bool)` (see `crates/types/src/lib.rs`), and its authoritative source is the AST lowering — `ast_to_hir/builtins.rs` inspects the mode literal (positional arg 1 or `mode=` kwarg) and stamps `Expr.ty = Some(Type::File(is_binary))`.

`resolve_builtin_call_type(Builtin::Open, ...)` in `lowering/src/type_planning/helpers.rs` intentionally returns `None` so the caller falls back to `expr.ty`. If that helper returns `Some(Type::File(false))` instead, it **silently overwrites** the frontend-computed flag and breaks binary reads — the lowering paths that dispatch on `Type::File(binary)` (e.g. `lower_file_method` routing `.read()` to str vs bytes return) see the wrong flag.

Any new caller that constructs a `Type::File(_)` should mirror this rule: the mode literal is authoritative, and non-literal/runtime-computed modes default to text.

## `with ctx as f:` + Rebinding `f` Needs `__enter__` Return Types

`with open(path) as f: ...` desugars to `f = <mgr>.__enter__()` at HIR-time. The type of `f` is set from `resolve_method_return_type(<mgr type>, "__enter__")` in `lowering/src/type_planning/helpers.rs`. If that match arm doesn't include `__enter__`, the return type resolves to `None`, callers fall back to `expr.ty.unwrap_or(Any)`, and a later `f = open(...)` rebind with a non-trivial type can't re-infer method return types — `f.read()` silently falls through to `NoneType`.

Every context-manager-aware type (currently `Type::File(_)`) must handle both `__enter__` (returns self — i.e. `File(binary)`) and `__exit__` (returns `Bool`) in `resolve_method_return_type`. If you add a new stdlib type that supports `with`, don't forget these dunder arms.

## `Tuple[Any]` Destructuring Needs HeapAny Promotion

Tuple indexing (`t[0]`) promotes `Type::Any` elements to `Type::HeapAny` when the tuple uses `ELEM_HEAP_OBJ` storage (see `expressions/access/indexing.rs::175-209`). Destructuring (`a, b = t`) used to skip this and assign `Type::Any` directly, which causes `print(a)` to emit the raw pointer as an integer instead of dispatching on the object's type tag (`type_dispatch.rs::select_print_func`).

`statements/assign/bind.rs` (`lower_tuple_pattern`) mirrors the indexing logic: it computes `uses_heap_obj = !elem_types.iter().all(|t| *t == Int)` and promotes `Any → HeapAny` before storing in temp locals / nested recursive extraction. If you add a new unpacking code path (e.g. pattern matching), apply the same promotion — otherwise print/compare/len on the unpacked variables silently misbehaves.

---

## Cross-Module Placeholder ClassIds Must Not Be Offset in First Pass

The frontend allocates placeholder `ClassId`s from the top of `u32` space (`u32::MAX - N`) for imported user classes whose real ids are only known after `mir_merger`'s second pass. The first pass in `MirMerger::compile_modules` scans `module_init_stmts` for variable types and calls `type_to_raw` — if a module-level variable carries a `Type::Class` with a placeholder id (e.g., `anno_local: Point = ...`), adding `class_id_offset` to `u32::MAX` wraps and panics with "attempt to add with overflow".

Fix: `type_to_raw` uses `checked_add` and falls back to `RawType::Any` on overflow. The placeholder will be resolved to the real class id during the second pass when `resolve_external_class_refs` rewrites all `Type::Class` references in the HIR.

---

## Unified Binding Targets — One Enum for Every LHS

Every Python binding site (`x = ...`, `for x in ...`, `with f() as x:`, comprehension `for x in ...`) historically had its own helpers and its own HIR statement variant. The fragmentation meant `[a+b for a, b in pairs]` rejected at parse time while `for a, b in pairs:` worked — same grammar, different frontend path. The `BindingTarget` refactor collapses all of it to a single type.

### HIR shape

`crates/hir/src/lib.rs` — one enum covers every valid Python LHS:

```rust
pub enum BindingTarget {
    Var(VarId),
    Attr { obj: ExprId, field: InternedString, span: Span },
    Index { obj: ExprId, index: ExprId, span: Span },
    ClassAttr { class_id: ClassId, attr: InternedString, span: Span },
    Tuple { elts: Vec<BindingTarget>, span: Span },
    Starred { inner: Box<BindingTarget>, span: Span },
}

pub enum StmtKind {
    Bind { target: BindingTarget, value: ExprId, type_hint: Option<Type> },
    ForBind { target: BindingTarget, iter: ExprId, body: Vec<StmtId>, else_block: Vec<StmtId> },
    // ... other variants
}
```

Nine legacy variants (`Assign`, `UnpackAssign`, `NestedUnpackAssign`, `FieldAssign`, `IndexAssign`, `ClassAttrAssign`, `For`, `ForUnpack`, `ForUnpackStarred`) and `UnpackTarget` are gone. Adding a new Python feature that binds names — e.g. pattern guards extending match scope — plugs into the existing `BindingTarget` instead of spawning its own `StmtKind`.

### Frontend contract

`crates/frontend-python/src/ast_to_hir/variables.rs::bind_target(&py::Expr) -> Result<BindingTarget>` is the single entry point. It:

- Accepts any Python LHS grammar (Name / Attribute / Subscript / Tuple / List / Starred).
- Detects `ClassName.attr` via `symbols.class_map` and routes to `ClassAttr` (class-level and instance-level writes dispatch to different runtime paths at lowering).
- Validates shape inline — at most one `Starred` per `Tuple` level, no `Starred(Starred(_))` — with clear errors and spans.
- Folds former `mark_var_initialized` bookkeeping into the `Var` arm, so every `BindingTarget::Var` leaf is automatically recorded in `scope.initialized_vars`.

Bespoke paths remain intentionally for grammatically-restricted sites: walrus (`expressions/mod.rs`, PEP 572 restricts to a bare Name) and `except ... as NAME` (CPython grammar). Match patterns (`statements/match_stmt/`) use a separate `Pattern` AST per PEP 634 — different semantics (refutable) — and are not merged.

### Lowering contract

`crates/lowering/src/statements/assign/bind.rs::lower_binding_target` is the single recursive MIR-emission entry point:

- Dispatches on the target variant; leaves call small operand-taking helpers (`bind_var_op`, `bind_attr_op`, `bind_index_op`, `bind_class_attr_op`).
- `Tuple { elts }` recurses via `lower_tuple_pattern` which handles both the no-star and one-star branches with identical element-type inference (`uses_heap_obj` promotion for `Any → HeapAny` on heap-obj-backed tuples).
- For-loops go through `loops/bind.rs::lower_for_bind`, a true single entry point — range fast-path, class-iterator (`__iter__`), general iterable, enumerate optimisation, and flat starred unpack all dispatch directly from it. No legacy wrapper indirection.

### Shared walker

`BindingTarget::for_each_var<F: FnMut(VarId)>` recurses through `Tuple` and `Starred`, invoking the closure on every `Var` leaf (skipping Attr/Index/ClassAttr — they don't bind a new name). Used everywhere downstream needs the set of variables a statement binds:

- `lowering/src/exceptions.rs::collect_assigned_vars`
- `lowering/src/type_planning/{closure_scan, container_refine}.rs`
- `lowering/src/generators/{vars, utils}.rs`

New code should use the walker rather than pattern-matching each binding shape separately.

### Adding a new binding site

If Python gains (or you discover) another LHS grammar — say, walrus expanded to accept attributes — extend `bind_target_inner` in `variables.rs`. No new HIR variant, no new lowering helper, no new downstream match arms. The whole pipeline flows through the existing `Bind` / `ForBind`.

### Known gap: generator expressions with tuple targets

**Closed in Area C §C.6 via desugar-time `VarTypeMap`.** `detect_for_loop_generator` accepts any `BindingTarget`; `build_for_loop_direct` / `build_for_loop_filtered` emit a recursive `Bind` per iteration; and the element-type probe in `generators/desugaring.rs` walks a module-wide `VarTypeMap` indexing function params, module-level/function-body Bind RHS, and ForBind iter expressions. The `shape_infer_type` helper recursively resolves:

- Literals (`Int`/`Float`/`Bool`/`Str`/`None`, tuple/list/set/dict literals).
- `Var(vid)` — param annotation → for-loop iter element type → Bind RHS shape.
- `BuiltinCall(Zip|Enumerate|Range)` — synthesise `Iterator<Tuple<...>>` / `Iterator<Int>`.
- `MethodCall(obj.items()/keys()/values())` when obj infers to Dict — returns `List<Tuple<K,V>>` / `List<K>` / `List<V>`.
- `Attribute { obj, attr }` — walks `class_defs` to find the field type when obj is class-typed.

`infer_yield_type_raw` uses the same probe with an augmented `VarTypeMap` that records tuple-target leaves → corresponding iter element types. This is what makes `max((v, i) for i, v in enumerate([3,1,4,1,5]))` return `(5, 4)` (the yield type shape-infers as `Tuple([Int, Int])`, so `max` dispatches over tuples rather than treating the yield as `Int`).

Working canonical idioms:

- `sum(x * y for x, y in zip([1,2,3], [4,5,6]))` — inline literals.
- `sum(x * y for x, y in [(1,4), (2,5)])` — list-of-tuples literal.
- `sum(a * b + c for a, (b, c) in [...])` — nested unpack.
- `max((v, i) for i, v in enumerate([...]))` — tuple yield, module-level iter.
- `sum(x * y for x, y in zip(var_a, var_b))` — module-level `list[int]` captures.

**Still limited**: function-scoped closures of heap-typed variables into gen-exprs (e.g. `def f(a): return sum(x for x in a)`, `linear(x, w)` with closure over params). These fail at runtime with SIGSEGV even for the simple single-var `for x in a` form — a pre-existing bug in how the gen-expr resume function marshals heap captures, independent of `§C.6`'s type-inference scope. Workaround: materialise the iterable with a leading `list(...)` or move the gen-expr to module scope. `min()` on gen-expr yielding tuples also has a separate pre-existing bug (returns the first element); `max()` works because its dispatch is through a different runtime path.

---

## Numeric Tower & Dunder Parameter Types

Area B of `MICROGPT_PLAN.md` added four interconnected mechanisms for correct operator overloading. They are described together because they interact: changing one without the others produces subtle Cranelift verifier failures or silent wrong-type dispatch.

### 1. Polymorphic `other` parameter

CPython does **not** constrain the `other` parameter of binary operator dunders — the dunder is expected to inspect `other` at runtime via `isinstance` and either produce a result or return `NotImplemented`. This means the compiler must widen `other` to at least the numeric tower when no annotation is supplied.

`pyaot_types::dunders::polymorphic_other_type(kind, self_ty)` (`crates/types/src/dunders.rs`) encodes the canonical widening:

| `DunderKind` | Default `other` type |
|---|---|
| `BinaryNumeric` | `Union[Self, int, float, bool]` |
| `BinaryBitwise` | `Union[Self, int, bool]` (floats excluded — bitwise on floats is a Python TypeError) |
| `Comparison` | `Any` (CPython guarantees `a == b` never raises) |
| `Unary` / `Conversion` / `Container` / `Lifecycle` | `None` — no `other` |

Without this widening, `2.5 * V(3.0)` (which calls `V.__rmul__(2.5)`) fails the Cranelift IR verifier with `"arg 1 has type i64, expected f64"` — the compiler had narrowed `other: V` from the single forward call site `v * v`.

An explicit annotation overrides the default: `def __mul__(self, other: float)` gives direct `f64 * f64` arithmetic instead of boxed dispatch. Use explicit annotations when you need maximum performance and know the caller types.

### 2. Why `other: Union[...]` produces boxed dunder bodies

With `other: Union[Self, int, float, bool]`, expressions like `self.x * other` (where `self.x: float`) cannot emit a direct `f64` multiply — `other` may be a boxed heap object. The lowering routes through `rt_obj_mul` (runtime type-tag dispatch), which is correct (matches CPython semantics) but slower.

`V(self.x * other)` hits a further wrinkle: `__init__(self, x: float)` requires `f64`, but `self.x * other` returns `Union` (boxed pointer). The call-site resolver (`lowering/src/lib.rs::resolve_call_args`, step 7.6) handles this: when a `float` parameter receives a `Union`/`Any`/`HeapAny` argument, it emits `rt_unbox_float` before the call. No special casing needed in `__init__` itself.

### 3. Subclass-first reflected rule (CPython §3.3.8)

When the right operand is a *strict subclass* of the left and defines the reflected dunder, CPython tries the reflected dunder **first**:

```python
Base() * Derived()   # calls Derived.__rmul__, not Base.__mul__
```

Implemented at the top of `lowering/src/expressions/binary_ops.rs::lower_binop` via `is_proper_subclass(right_ty, left_ty, class_defs)`. The check is done entirely at compile time using the static class hierarchy in `class_defs`.

### 4. `NotImplemented` fallback protocol

When a forward dunder *may* return the `NotImplemented` sentinel, the binary-op lowering emits a runtime branch:

1. Call the forward dunder, capturing the result.
2. Compare the result pointer to `rt_not_implemented_singleton`.
3. If equal, dispatch the reflected dunder on the right operand.

The "may return" predicate is `dunder_may_return_not_implemented` (walks the HIR body for `return NotImplemented` statements). Only functions that actually have such a branch incur the extra comparison — functions that always return a concrete value take the fast path with no branch.

**Why `NotImplementedT` must stay in the return type union.** When a dunder only ever returns `NotImplemented` (no other branch yet), the type planner would infer return type `None` (the i8 sentinel from `NoneType`). The Cranelift signature would emit i8. Later, the binary-op lowering casts the result to i64 for the sentinel comparison — silently truncating the pointer. `Type::NotImplementedT` keeps the return type as a heap pointer (i64) even in the degenerate case.

### 5. Comparison dunders return `bool`, not heap

`__eq__`, `__lt__`, etc. lower to i8 (`Type::Bool`) in Cranelift. Returning `NotImplemented` (an i64 heap pointer) from a comparison dunder would require flipping the signature to `Union[Bool, NotImplementedT]` and boxing every concrete `True`/`False` return through `rt_box_bool`. This is out of scope — comparison dunders must return a concrete `bool`. The standard CPython idiom (`return False` in the unhandled branch of `__eq__`) works correctly as-is.

### 6. `class_defs.get(class_id).name` is unreliable during method body conversion

`class_defs` (the `IndexMap<ClassId, ClassDef>` in the frontend) is populated **after** the class body is fully converted. During conversion of a method body, looking up the current class by `class_id` returns `None`, so the fallback is the first parameter name (e.g. `"self"`). This produces `Type::Class { class_id, name: "self" }` — a drifted name that breaks `Union` deduplication: two entries with the same `class_id` but different `name` strings won't deduplicate, and the union grows unboundedly.

Fix: track the canonical class name alongside `scope.current_class` in a `scope.current_class_name: Option<InternedString>` field, set when the frontend enters the class body. Method bodies read from `current_class_name` instead of doing a `class_defs` lookup.

---

## Built-in Reductions & User Dunders (Area C §C.3)

`sum()` on a list/iterator/set of class instances folds through the operator-dunder protocol, not a raw `BinOp::Add` at MIR level. Three pieces work together:

### 1. `dispatch_class_binop` — the shared dispatch core

Extracted from `lower_binop` into `crates/lowering/src/expressions/operators/binary_ops.rs::dispatch_class_binop`. The full §3.3.8 state machine (subclass-first → forward → NotImplemented fallback → reflected) is now a single `pub(crate)` method consumed by both binary expressions and the reduction helper. No duplication — any future Area B improvement automatically applies to reductions.

### 2. Accumulator seeding — skip the `0 + V(x)` dance

CPython's `sum([V(1), V(2)])` does `0 + V(1) → NotImplemented → V(1).__radd__(0) → V(1)`, then folds from there. `crates/lowering/src/expressions/builtins/reductions/mod.rs::lower_reduction_class_fold` short-circuits: pull the first element from the iterator, use it as the initial accumulator, fold from the second onwards via `dispatch_class_binop(BinOp::Add, acc, elem)`. Keeps the accumulator type stable at `Type::Class { .. }` throughout — no Union accumulator, no extra dispatch overhead.

The shortcut applies only when `start` is default (absent) **or** explicitly a same-class instance. Primitive `start` with class elements falls through to the numeric fast path (matches legacy — CPython would raise at runtime in the `0 + V(x)` step if `__radd__` doesn't handle int; compilation-time shortcut is less relevant).

### 3. Inter-procedural `NotImplemented` analysis (§C.7)

`dunder_may_return_not_implemented` (used to gate the fallback compare+branch) used to be a private body scanner in `binary_ops.rs` — only direct `return NotImplemented` counted. Moved to `crates/lowering/src/type_planning/ni_analysis.rs::func_may_return_not_implemented` with fixed-point propagation: a dunder that tail-calls a helper inherits the helper's NI-producing status. Cache is lazy (populated on first query), cycles broken by a `Computing` marker treated as `No` on re-entry.

Conservative default for unresolved callees (cross-module, Union/Any receiver): treat as `may return NI`. One extra compare+branch at the call site is cheap; a false-negative would silently produce wrong results.

### 4. Type inference for `Call sum(...)` on class elements

`crates/lowering/src/type_planning/helpers.rs::builtin_return_type` for `Builtin::Sum` / `Builtin::Min` / `Builtin::Max` returns the element type when it's a `Type::Class`. Without this, `sum([V(...)]).x` / `min([V(...)]).x` would be flagged as "unknown attribute".

### 5. `min()` / `max()` on user classes — rich-comparison dunders

`try_lower_minmax_class_elem` in `reductions/mod.rs` is the parallel to `try_lower_sum_class_elem` for rich-comparison reductions. Seeds `best` with the first element, then loops calling:

- `min` → `elem.__lt__(best)` when `__lt__` exists; else `best.__gt__(elem)` (swapped args).
- `max` → `elem.__gt__(best)` when `__gt__` exists; else `best.__lt__(elem)` (swapped args).

If the dunder returns truthy, `best := elem`. Comparison dunders return `Bool` (i8) so no Union-boxing or NI fallback is involved (see §E.7 for the `NotImplemented` path on comparisons, still deferred).

### 6. `sum(list, primitive_start)` with class elements

When `start_ty` is `Int`/`Float`/`Bool` and `elem_ty` is a class, the fold bootstraps via `dispatch_class_binop(primitive + first_elem)` — the dispatch skips the subclass-first / forward-on-left branches (left is not a class) and falls through to the reflected dunder on the right operand (`first_elem.__radd__(primitive)`), producing a class-typed result. The accumulator slot stays class-typed throughout; subsequent iterations are the normal class + class fold. Matches CPython's `0 + V(x) → NotImplemented → V(x).__radd__(0)` dance but skips the explicit NI round-trip.

### Known limitations

- Heterogeneous iterables (`list[V | int]`): accumulator would need to be a Union, dispatching via `rt_obj_add` each iteration. Not implemented; users pre-homogenise.
- Empty iterable + primitive start (`sum([], 0)` with a hypothetical class-only path): we write a null placeholder in the class-typed accumulator slot. In practice the path isn't exercised because the empty-iter type inference returns the class type, not `Int`. Document as best-effort parity.

---

## Numeric Tower & Cross-Site Type Unification (Area E)

Class fields and function-local variables both need a single static
type even when they're written from many sites with different primitive
types. Area E formalises this as a three-layer system.

### 1. Unification primitive: `Type::unify_field_type`

Central entry-point in `crates/types/src/lib.rs`. Decision tree:

```
unify_field_type(a, b)
├── either side is Tuple/TupleVar → Type::unify_tuple_shapes   # Area D
└── otherwise                      → Type::unify_numeric
                                     ├── promote_numeric(a, b) if both numeric
                                     │   Bool ⊂ Int ⊂ Float (PEP 3141)
                                     └── normalize_union([a, b])          # fallthrough
```

`promote_numeric` covers the full 9-cell numeric matrix and returns
`None` for non-numeric pairs. `unify_numeric` falls through to
`normalize_union` so `{Int, Str}` becomes `Union[Int, Str]`.

### 2. Class fields: all-methods scan + write-site coercion (§E.3)

`ast_to_hir/classes.rs::scan_stmts_for_self_fields` walks every method
(not just `__init__`) and sees three site shapes on `self.<field>`:

- `Assign` — RHS type via `infer_field_type_from_rhs` (covers parameter
  references, tuple literals, primitive literals, and — new in §E.3 —
  narrow numeric-BinOp inference so `self.x = self.x + 0.5` yields
  `Float` instead of collapsing to `Any`).
- `AnnAssign` — explicit annotation overwrites prior inference.
- `AugAssign` — added in §E.3; `self.total += x` was previously
  invisible to the scanner.

Each observation is merged via `Type::unify_field_type`. Annotation
still wins over inference when both exist.

**Write-site coercion** — `lower_binding_target`'s `Attr` branch passes
`value_type` to `bind_attr_op`, which calls `coerce_to_field_type`
before `rt_instance_set_field`. `(Int | Bool, Float)` emits
`IntToFloat`; primitive-into-Union boxes via `box_primitive_if_needed`.
Without this, `self.total = 1` into a Float-widened field would store
the bit-pattern of `1_i64` — read back as `5e-324`.

### 3. Locals: pre-scan + consumer priority (§E.6)

`type_planning/local_prescan.rs` runs once per function after
return-type inference. Walks `Bind` / `ForBind` / control-flow bodies,
skipping nested function defs. Per-VarId it records the merged type in
`per_function_prescan_var_types[func_id]`. A few special rules:

1. **Scope filter.** `FuncRef` / `Closure` RHS values are skipped —
   `infer_expr_type_inner` synthesises the callee's return type for
   them, which is nonsense for a binding target (which holds a function
   pointer, not the call result). Cell / nonlocal variables are dropped
   from the output map so they stay in cell storage, not in plain MIR
   locals.

2. **Narrow merge.** On a rebind, we only widen the scratch type when
   the pair is numeric-tower (`Bool | Int | Float` on both sides) or
   tuple-shaped. Other combinations preserve the first-write type —
   Union-aware narrowing, cell handling, and existing tests depend on
   first-write-wins for non-numeric rebinds.

3. **Post-loop replace.** If a variable was observed only inside a
   `for` / `while` body and the next write is outside any loop, the
   outer write **replaces** the loop-inferred type. Fixes the §A.6 #3
   idiom where `for _, c in pairs: pass; c = SomeClass()` previously
   failed with "unknown attribute `c.x`" because the loop had typed `c`
   as `Int`.

**Consumer priority**. `get_or_create_local` and `lower_assign` both
consult: `refined_var_types` (Area D empty-container refinement) >
prescan (unless "uselessly wide" like `Dict(Any, Any)`) > prior
`var_types` > RHS-inferred. The "uselessly wide" filter matters for
module-level empty dict / list / set literals that later refine to a
concrete element type — without the filter the prescan snapshot
would shadow `refined_var_types`.

**Return-type re-inference**. `reinfer_return_types_with_prescan`
runs after prescan for un-annotated functions: seeds the `param_types`
map with prescan entries and re-scans return statements. Makes
`def f(): x = 0; x += 0.5; return x` type-infer return as `Float` — the
initial return-type pass only saw `x: Int` from the first write.

### 4. Comparison dunders with `NotImplemented` (§E.7)

Semantically identical to the §3.3.8 arithmetic-dunder state machine
but with unboxing at the end. `comparison.rs::lower_compare` now sizes
the dunder result via `alloc_dunder_result` — `Bool` when
`func_may_return_not_implemented` says no, else `Union[Bool,
NotImplementedT]`. The NI-aware branch calls
`emit_comparison_ni_fallback`:

```
forward_result = left.__op__(right)
if forward_result is NotImplementedSingleton:
    if right.__refl_op__ exists:
        refl = right.__refl_op__(left)
        if refl is NI: default_fallback(op)
        else: unbox_bool(refl)
    else: default_fallback(op)
else:
    unbox_bool(forward_result)
```

`default_fallback`: `==`/`!=` → pointer-identity compare; `<`/`<=`/`>`/
`>=` → `raise TypeError` (via `mir::Terminator::Raise`).

**Return-site boxing**. A comparison dunder with both `return True` and
`return NotImplemented` has signature `Union[Bool, NotImplementedT]`
(I64 pointer at Cranelift level). `lower_return` auto-boxes primitive
returns (`Bool`/`Int`/`Float`/`None`) when the enclosing signature is a
Union containing `NotImplementedT` — without this, `return True` emits
an I8 where a pointer is expected.

**Reflected comparison names** live alongside the arithmetic and
bitwise pairs in `pyaot_types::dunders::reflected_name`:
`__lt__` ↔ `__gt__`, `__le__` ↔ `__ge__`, and self-reflected `__eq__`
and `__ne__` per the data model.

## Generator Heap Captures (Area G §G.3)

Generator expressions desugar at the frontend into a regular
`__genexp_N` function whose body is `for <target> in <iter>: yield <elt>`.
Before Area G the synthesized function was built with **empty params** —
every free variable in the body kept its outer `VarId`. That worked for
module globals (lowering resolves them through `rt_global_get_*` at use
sites) but segfaulted for function-local or function-param heap captures:
nothing initialises the outer `VarId` inside the `__genexp_N` scope, so
the body reads an uninitialised local.

### Shape of the fix

Mirror the lambda capture pattern in
`frontend-python/src/ast_to_hir/comprehensions.rs::desugar_generator_expression`:

1. **Walk** `genexp.elt`, each `generator.iter`, and each `generator.ifs`
   with `collect_free_variables` (shared with `convert_lambda`),
   excluding the loop-target names collected via `add_target_to_scope`.
2. **Partition** free vars: module globals (via
   `self.scope.module_level_assignments.contains(var_id)` OR
   `self.module.globals.contains(var_id)`) keep their outer `VarId` —
   `module.globals` is only populated in `finalize_module`, so the
   `module_level_assignments` check catches progressively-assigned
   globals. Everything else becomes a real capture.
3. For each capture, allocate a fresh `VarId`, name it `__capture_NAME`,
   insert it into `var_map[NAME]` so the body resolves references to it,
   and push a `Param` with `ty: None` (filled in by lambda param-type
   inference at lowering time).
4. Restore the outer `var_map` after body generation.
5. Emit the call as
   `Call { func: Closure { func: __genexp_N, captures: [Var(outer_id)...] }, args: [] }`.
   `lower_closure_call` prepends capture values to the creator call,
   matching CPython's snapshot-on-create semantics.

### Downstream plumbing

- **Slot 0 reservation.** The desugaring in
  `lowering/src/generators/desugaring.rs::build_creator_body` hardcodes
  generator slot 0 for the for-loop iterator (stored via
  `mk_set_local(creator_gen_var, 0, iter_call, …)`). Capture params
  would collide with slot 0, so `collect_generator_vars` (vars.rs) now
  starts `next_idx = 1` when the function is a for-loop generator.
- **Per-slot GC marking.** `GeneratorObj.type_tags: *mut u8` already
  carries a one-byte-per-slot tag; `gc.rs` traces a slot only when
  `type_tags[i] == LOCAL_TYPE_PTR` (value `3`). Before Area G, slot 0
  (iter) was the only marked slot. Capture params now get the same
  treatment — `mk_set_local_type_ptr` emits
  `rt_generator_set_local_type(slot, 3)` immediately after `SetLocal`
  whenever `gv.ty.is_heap()`. The two-call pair must stay adjacent
  (no `gc_alloc` between them) or GC between calls would leave the
  slot untagged — confirmed by reading the codegen'd MIR.
- **Capture param types.**
  `context/function_lowering.rs` routes gen-expr creators
  (`name.starts_with("__genexp_")`) through
  `infer_lambda_param_types`, which in turn reads
  `closure_capture_types` recorded by `precompute_closure_capture_types`
  during type planning. Without this, capture params default to `Any`,
  which mis-tags raw-int lists as `ELEM_HEAP_OBJ` on iteration and
  cascades into pointer-valued tuple elements.
- **Call-target type inference.**
  `type_planning/infer.rs::resolve_call_target_type` grew a `Closure`
  arm so immediate calls of the form `Call { func: Closure { … } }`
  propagate the wrapped function's return type — needed for
  downstream `sum` / `min` / `max` to see `Iterator(…)` and dispatch
  correctly.

### Known gaps

- **Nested gen-exprs over captures** (e.g.
  `[sum(wi * xi for wi, xi in zip(wo, x)) for wo in w]`) — the outer
  `w` is captured correctly, but the inner zip's tuple element types
  collapse to `Any`, so `wi * xi` multiplies pointers and overflows.
  The gap is in how capture types propagate into a nested gen-expr's
  for-loop iter type inference. Single-level captures are correct.
- **`dict.items()` inside a gen-expr** (both module-level and
  function-local) silently fails: the `(_k, v)` tuple-unpack doesn't
  carry the dict value type. Pre-existing — separate from §G.3.

## Lexicographic min/max on Tuple-Yielding Iterators (Area G §G.4)

`min((v, i) for i, v in enumerate([3, 1, 4, 1, 5]))` used to return
`(3, 0)` — the first yielded element. The primitive Iterator path in
`lower_minmax_builtin` (`expressions/builtins/math/minmax.rs`) assumes
`elem_ty` is `Int` / `Float` and compares raw i64 values with
`BinOp::Lt` / `Gt`. For tuple elements the i64 is a `*mut Obj` pointer,
so `cand < best` was always false against later allocations (the
accumulator never updates). `max` accidentally returned the correct lex
answer because higher addresses happened to also be lex-greater.

Area G §G.4 inserts a tuple-dispatch arm in the
`Type::Iterator(elem_ty)` branch. When `elem_ty` is `Type::Tuple(_)` or
`Type::TupleVar(_)`, the new `lower_minmax_tuple_iter_fold` helper
seeds `best` with the first element, loops via `rt_iter_next_no_exc` +
`rt_generator_is_exhausted`, and compares each candidate against the
running best with
`rt_tuple_cmp(cand, best, op_tag)` — `op_tag` is `0` (Lt) for min and
`2` (Gt) for max. Strict comparison means the first-seen best stays on
tie, matching CPython. Empty-iterable semantics mirror
`lower_minmax_class_fold`: null accumulator + exit (CPython-strict
`ValueError` is out of scope).

`rt_tuple_cmp` is registered as `RT_CMP_TUPLE_ORD` in
`core-defs/src/runtime_func_def.rs:612` (symbol is the Python-visible
`rt_tuple_cmp`; the static name follows the `RT_CMP_*` convention for
comparison runtime functions).
