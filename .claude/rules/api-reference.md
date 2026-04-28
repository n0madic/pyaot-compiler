# Key APIs

## Type System

- **Primitives**: `int` (i64), `float` (f64), `bool` (i8), `str`, `None`
- **Containers**: `list[T]`, `dict[K,V]`, `defaultdict[K,V]`, `tuple[T1,...,Tn]` (fixed-length heterogeneous → `Type::Generic{BUILTIN_TUPLE_CLASS_ID, [T1..Tn]}`), `tuple[T, ...]` (variable-length homogeneous → `Type::Generic{BUILTIN_TUPLE_VAR_CLASS_ID, [T]}`, PEP 484/585), `set[T]`, `bytes`. **All five builtin container types use `Type::Generic{base, args}`** since S3.2c. Construct via `Type::list_of(T)`, `Type::dict_of(K,V)`, `Type::set_of(T)`, `Type::tuple_of(elems)`, `Type::tuple_var_of(T)`; query via `list_elem()`, `dict_kv()`, `set_elem()`, `tuple_elems()`, `tuple_var_elem()`, `is_list_like()`, `is_dict_like()`, `is_set_like()`, `is_tuple_like()`.
- **Special**: `Union[T, U]`, `Optional[T]`, `Iterator[T]`, `Any`, `HeapAny`, `NotImplementedT`
- **Collections**: `Type::DefaultDict(K, V)`, `Type::RuntimeObject(TypeTagKind::Counter)`, `Type::RuntimeObject(TypeTagKind::Deque)`
- **Classes**: `Type::Class { class_id, name }`
- **Exceptions**: `Type::BuiltinException(BuiltinExceptionKind)`
- **Any vs HeapAny**: `Any` = ambiguous (raw i64 or pointer), `HeapAny` = guaranteed `*mut Obj` (safe for runtime dispatch in print/compare)
- **Tuple variants** (both now `Type::Generic`): `Generic{TUPLE_ID, [T1..Tn]}` (fixed-length) and `Generic{TUPLE_VAR_ID, [T]}` (variable-length) share the same runtime (`TupleObj` with uniform `[Value]` storage); the distinction is compile-time only. Fixed tuples support per-slot typed indexing and static bounds checks; variable tuples emit `rt_tuple_get` with runtime bounds checks and return the homogeneous element type. Detect with `tuple_elems()` (returns `Some` for fixed) vs `tuple_var_elem()` (returns `Some` for variable). Merge rule: `a.join(&b)` via `TypeLattice` — same-length tuples merge element-wise (fixed shape); different-length tuples → `Union[...]`; TupleVar ⊔ Tuple → TupleVar(element join).
- **TypeLattice API (S3.1)**:
  - `TypeLattice::join(&self, &Self) -> Self` — least upper bound (⊔). Numeric tower: `Bool ⊔ Int = Int`, `Int ⊔ Float = Float`. Replaces `unify_field_type`, `unify_numeric`, `unify_tuple_shapes`, `normalize_union`.
  - `TypeLattice::meet(&self, &Self) -> Self` — greatest lower bound (⊓). Replaces `narrow_to`.
  - `TypeLattice::minus(&self, &Self) -> Self` — set difference. Replaces `narrow_excluding`.
  - `TypeLattice::is_subtype_of(&self, &Self) -> bool` — lattice order.
  - `TypeLattice::top() -> Self` — `Type::Any` (absorbs in `join`).
  - `TypeLattice::bottom() -> Self` — `Type::Never` (identity in `join`).
  - `Type::promote_numeric(a, b) -> Option<Type>` — PEP 3141 tower (`bool ⊂ int ⊂ float`); `None` for non-numeric pairs. Still public; used internally by `join` and by `dunders.rs`.
- **Dunder reflections**: `pyaot_types::dunders::reflected_name(forward)` returns the reflected counterpart for binary numeric (`__add__` ↔ `__radd__` etc.), binary bitwise (`__and__` ↔ `__rand__`), **and** comparison ops (`__lt__` ↔ `__gt__`, `__le__` ↔ `__ge__`; `__eq__` / `__ne__` are self-reflected). `hir::CmpOp::reflected_dunder_name` / `is_ordering` mirror these at the HIR level for comparison lowering.
- **TypeVar API (S3.3a)**:
  - `Type::Var(InternedString)` — TypeVar placeholder, preserved end-to-end through HIR→MIR. Erased to concrete type by `MonomorphizePass` before codegen; codegen must never see a Var.
  - `Type::contains_var() -> bool` — short-circuit walk: true iff any `Var` leaf appears anywhere in the type tree.
  - `Type::collect_var_names(out: &mut HashSet<InternedString>)` — fills `out` with all distinct Var names in the type tree.
  - `Type::substitute(subst: &HashMap<InternedString, Type>) -> Type` — recursively replaces every `Var(name)` with `subst[name]`; unmapped vars are left as-is; all other shapes are cloned.
  - `TypeVarDef { constraints: Vec<Type>, bound: Option<Type> }` — in `pyaot_types`; threaded `hir::Module → mir::Module` for Mono.
  - `mir::Function::is_generic_template: bool` — set by lowering when any param/return type `contains_var()`. WPA early-returns on template functions.
  - `mir::Function::typevar_params: Vec<InternedString>` — distinct Var names from the function signature; empty when `is_generic_template` is false.
  - `monomorphize::derive_subst(template_params, call_arg_types) -> Option<HashMap<InternedString, Type>>` — builds Var→concrete substitution; returns `None` on conflict or structural mismatch.
  - `monomorphize::specialize_function(template, subst, fresh_id, name) -> Function` — deep-clones template, remaps IDs, substitutes types.

## Shared Definitions (`core-defs`)

Single source of truth for exceptions and type tags, shared by compiler and runtime.

```rust
// BuiltinExceptionKind: Exception, AssertionError, IndexError, ValueError,
// StopIteration, TypeError, RuntimeError, GeneratorExit, KeyError, AttributeError,
// IOError, ZeroDivisionError, OverflowError, MemoryError, NameError,
// NotImplementedError, FileNotFoundError, PermissionError, RecursionError, EOFError,
// SystemExit, KeyboardInterrupt, FileExistsError, ImportError, OSError,
// ConnectionError, TimeoutError, SyntaxError
kind.tag() -> u8
kind.name() -> &'static str
BuiltinExceptionKind::from_name("ValueError") -> Option<Self>

// TypeTagKind: Int, Float, Bool, Str, None, List, Tuple, Dict, Instance,
// Iterator, Set, Bytes, Cell, Generator, Match, File, StringBuilder, StructTime,
// CompletedProcess, ParseResult, HttpResponse, Hash, StringIO, BytesIO,
// DefaultDict, Counter, Deque
//
// Each type tag has three string representations:
tag.tag() -> u8                    // Numeric tag
tag.name() -> &'static str         // Debug name: "StructTime"
tag.type_class() -> &'static str   // Python type(): "<class 'time.struct_time'>"
tag.type_name() -> &'static str    // Short name: "time.struct_time"

// Lookup
TypeTagKind::from_tag(17) -> Some(TypeTagKind::StructTime)
TypeTagKind::from_name("StructTime") -> Some(TypeTagKind::StructTime)
```

**Adding new type tag:**
1. Edit `define_type_tags!` macro in `core-defs/src/tag_kinds.rs`
2. Add entry: `NewType = N => "NewType" => "<class 'module.NewType'>" => "module.NewType"`
3. Runtime and compiler automatically pick up the new definition

**Adding new exception:** Edit `define_exceptions!` macro in `core-defs/src/exceptions.rs`.

## MIR Parameterized Enums

MIR uses parameterized enums to reduce `RuntimeFunc` variants. See `crates/mir/src/lib.rs`:

| Enum | Purpose |
|------|---------|
| `PrintKind` | Print operations by type |
| `ReprTargetKind` + `StringFormat` | repr()/ascii() |
| `CompareKind` + `ComparisonOp` | Comparisons |
| `ContainerKind` + `MinMaxOp` + `ElementKind` | min()/max() |
| `IterSourceKind` + `IterDirection` | iter()/reversed() |
| `SortableKind` | sorted() |
| `ConversionTypeKind` | Type conversions |

After Phase 2 §F.6 the storage layer (cells, globals, class attrs) uses
uniform `Value`-typed externs (`RT_GLOBAL_GET`/`SET`, `RT_CLASS_ATTR_GET`/
`SET`, `RT_MAKE_CELL`, `RT_CELL_GET`/`SET`); lowering inserts
`UnwrapValueInt` / `UnwrapValueBool` after the load when the destination
local is typed `Int` / `Bool`.

## GC (Shadow-stack Mark-sweep)

```rust
#[repr(C)]
pub struct ShadowFrame {
    pub prev: *mut ShadowFrame,
    pub nroots: usize,
    pub roots: *mut *mut Obj,
}
```

- Heap types: `str`, `list`, `dict`, `tuple`, class instances, iterators
- Lowering marks heap locals as `is_gc_root: true`
- Codegen: `gc_push` on entry, `gc_pop` on exit (skipped when `nroots == 0`)
- Lock-free: GC state accessed via `AtomicPtr<GcState>`, no mutex
- Slab allocator: objects ≤ 64 bytes bump-allocated from 4KB pages (`slab.rs`), not tracked in `GcState.objects` Vec
- **Uniform tagged-`Value` storage** (Phase 2 §F.7c BigBang): every compound-object slot is a `pyaot_core_defs::Value`. `Value::is_ptr()` is the GC's primary filter; the residual alignment / low-page / `TypeTagKind::from_tag` guards in `mark_object` reject pointer-shaped non-objects from any code path that hasn't been fully cleaned up. The legacy per-slot side-arrays (`TupleObj.heap_field_mask`, `ClassInfo.heap_field_mask`, `GeneratorObj.type_tags`, `ListObj.elem_tag`, `DequeObj.elem_tag`) and the corresponding `rt_*` setters were removed; lowering wraps raw `Int`/`Bool` operands via `ValueFromInt` / `ValueFromBool` MIR before any container store, so the GC walker no longer needs an out-of-band tag.

## Runtime Object Header

```rust
#[repr(C)]
pub struct ObjHeader {
    pub type_tag: TypeTagKind,  // From core-defs (single source of truth)
    pub marked: bool,
    pub size: usize,
}
```

The runtime uses `TypeTagKind` directly from `core-defs` — no duplicate enum.

## Type Tag Architecture

Type metadata flows from a single source in `core-defs`:

```
core-defs (SSOT)
├── TypeTagKind enum
│   ├── tag()        → u8 (numeric value for gc_alloc)
│   ├── name()       → "StructTime" (debug/internal)
│   ├── type_class() → "<class 'time.struct_time'>" (Python type())
│   └── type_name()  → "time.struct_time" (error messages, repr)
│
├──► runtime (uses TypeTagKind directly)
│    ├── ObjHeader.type_tag: TypeTagKind
│    ├── rt_type_name() calls type_class()
│    └── error messages use type_name()
│
└──► compiler crates (via stdlib-defs)
     └── Type system, lowering, codegen
```

**Key principle:** No hardcoded type strings in runtime — all strings come from `TypeTagKind` methods.

## Exception Raising API

Exception raising is split into external (called from codegen) and internal (called from Rust runtime code) paths:

**External (extern "C", called from compiled code):**
```rust
rt_exc_raise(exc_type_tag: u8, message: *const u8, len: usize) -> !  // copies message
rt_exc_raise_from(exc_type_tag, msg, len, cause_type, cause_msg, cause_len) -> !
rt_exc_raise_from_none(exc_type_tag, message, len) -> !
rt_exc_raise_custom(class_id, message, len) -> !
rt_exc_raise_custom_with_instance(class_id, message, len, instance) -> !
rt_exc_reraise() -> !
```

**Internal (Rust-only, zero-copy for leak-free raising):**
```rust
rt_exc_raise_owned(exc_type_tag: u8, msg_ptr: *mut u8, msg_len: usize, msg_capacity: usize) -> !
raise_exc!(ExceptionType::ValueError, "format: {}", arg)  // macro: format + forget + raise_owned
raise_value_error_owned(msg: String) -> !   // utils.rs
raise_io_error_owned(msg: String) -> !      // utils.rs
raise_runtime_error_owned(msg: String) -> ! // utils.rs
```

**Shared internals (in exceptions.rs):**
- `dispatch_to_handler(Box<ExceptionObject>) -> !` — stores exc, unwinds GC/traceback, longjmps
- `raise_with_owned_message(exc_type, ptr, len, cap) -> !` — builds ExceptionObject, calls dispatch
- `dispatch_existing_exception() -> !` — for reraise (exc already in current_exception)

## Optimizer Pass Interface

Passes implement the `OptimizationPass` trait and are orchestrated by `PassManager` (`optimizer/src/pass.rs`):

```rust
pub trait OptimizationPass {
    fn name(&self) -> &str;
    fn run_once(&mut self, module: &mut Module, interner: &mut StringInterner) -> bool;
    fn max_iterations(&self) -> usize { 10 }
    fn is_fixpoint(&self) -> bool { true }
}
```

**Pass types:**

| Pass | Struct | Fixpoint | Max Iter | Notes |
|------|--------|----------|----------|-------|
| **Monomorphize** | `MonomorphizePass` / `monomorphize::run()` | No | 1 | Runs before `abi_repair`, after first WPA |
| Devirtualize | `DevirtualizePass` | No | 1 | |
| Flatten Properties | `FlattenPropertiesPass` | No | 1 | |
| Inline | `InlinePass::new(threshold)` | No (internal) | 1 | |
| Constant Folding | `ConstantFoldPass` | Yes | 10 | |
| Peephole | `PeepholePass` | Yes | 10 | |
| DCE | `DcePass` | Yes | 20 | |

**Pipeline construction:**
```rust
let mut pm = build_pass_pipeline(&config);  // Configures based on OptimizeConfig flags
pm.run(&mut module, &mut interner);         // Runs all enabled passes sequentially
```

**Adding a new pass:**
1. Create struct implementing `OptimizationPass` in `optimizer/src/mypass/mod.rs`
2. Register in `build_pass_pipeline()` in `optimizer/src/pass.rs`

## String Interning

Runtime string pool for deduplication (strings < 256 bytes):
- Compile-time literals → `rt_make_str_interned`
- Dict keys auto-interned
- Python API: `sys.intern(string)`
- Lock-free: single `UnsafeCell<HashMap>` (no sharded mutexes)
- Small string optimization: sizes rounded to slab classes (24/32/48/64) in `rt_make_str_impl`
- `eq_hashable_obj` checks pointer equality first (`a == b`), then `slice`-based memcmp
