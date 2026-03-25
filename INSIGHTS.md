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
    __comp_N[x] = x ** 2        # IndexAssign
result = __comp_N
```

The initial empty `{}` has no type hint, so the variable starts with `Dict(Any, Any)`. Without type refinement, all subsequent `DictGet` operations would skip unboxing.

**Fix**: `lowering/src/statements/assign.rs` performs dynamic type refinement during `IndexAssign`. When it detects a `Dict(Any, Any)` target and the actual key/value types are known, it updates the variable's tracked type. This only works for direct variable references (`ExprKind::Var`).

---

## Map/Filter Type Inference

`map(func, iterable)` returns `Iterator(elem_type)` where `elem_type` must be inferred from the function's return type, not defaulted to `Any`.

If the type system thinks `map()` returns `Iterator(Any)`, then `list(map(...))` creates a list with `elem_tag=0` (ELEM_HEAP_OBJ) even if the actual values are raw integers. This causes the GC to try tracing raw ints as heap pointers.

The type inference in `lowering/src/type_inference.rs` inspects the function argument to `map()`:
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

Interned strings enable pointer equality for dict key comparisons instead of byte-by-byte comparison. This is why dict lookup is fast but only works correctly when keys are interned consistently.

---

## Closure Cell Variables

`nonlocal` variables use cell-based indirection (`runtime/src/cell.rs`):
- A cell is a heap-allocated box holding a single value
- The enclosing function allocates the cell and passes it to the closure
- Both the enclosing function and closure read/write through the cell

**Three-level nesting limitation**: Currently, closures only capture from their immediate parent. A closure inside a closure inside a function cannot capture variables from the outermost function (the middle closure would need to forward the cell, which is not yet implemented).

---

## Exception Handling: setjmp/longjmp

Exception handling uses C-style `setjmp`/`longjmp`:
- `ExceptionFrame` layout: `prev` (8 bytes) + `jmp_buf` (200 bytes) + `gc_stack_top` (8 bytes) = 216 bytes
- `try` pushes a frame via `rt_exc_push_frame`, then Cranelift calls `setjmp(frame_ptr + 8)` **directly** (not through a Rust wrapper — see "setjmp Must Be Called Directly")
- `raise` calls `longjmp` to jump to the most recent frame
- The GC stack top is saved/restored to prevent shadow stack corruption on longjmp

Exception objects store: type tag, custom class ID, message string, `__cause__` (for `raise X from Y`), `__context__` (implicit chaining), and `suppress_context` flag.

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

## GC Global State Limitations

The GC uses a single `static GC_STATE: OnceLock<Mutex<GcState>>` (`runtime/src/gc.rs:115`). This has important implications:

**Single shadow stack**: `state.stack_top` is a single linked list of `ShadowFrame` pointers. Every compiled function pushes/pops to this one stack. If multiple threads execute compiled code simultaneously, their shadow frames will interleave in this list. GC mark phase would follow frames from mixed threads, leading to missed roots or dangling frame traversal.

**Mutex on hot path**: `gc_push`, `gc_pop`, `gc_alloc` all acquire the mutex through `with_gc_state()`. These are the hottest paths in compiled code — every function call does push+pop, every heap allocation takes the lock. Under contention this becomes a severe bottleneck.

**OnceLock prevents reinit**: `gc::init()` uses `get_or_init()` — it's a no-op after first call. `gc::shutdown()` frees all objects but cannot reset the `OnceLock`. This means test isolation requires external synchronization (see `RUNTIME_TEST_LOCK` in `lib.rs`).

**Map iterator boxing flag**: The map callback ABI is `fn(*mut Obj) -> *mut Obj` for all callbacks. User-defined functions compiled by Cranelift receive raw native types (i64 for int), so raw int elements from `ELEM_RAW_INT` lists pass through correctly. But builtin functions (like `str`, `int`) genuinely expect `*mut Obj` pointers. To distinguish, the compiler sets bit 7 of `capture_count` in `MapIterObj` (0x80 flag) for builtins, causing `iter_next_map` to box raw elements via `box_if_raw_int_iterator` before calling the callback. See `lowering/src/expressions/builtins/iteration.rs` and `runtime/src/iterator/next.rs`.

**Current safety**: The project is single-threaded by design (no async/await, no threading module). The mutex is uncontended and adds negligible overhead. But any future work on parallelism must address the GC architecture first.

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
