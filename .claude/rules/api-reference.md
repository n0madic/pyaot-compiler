# Key APIs

## Type System

- **Primitives**: `int` (i64), `float` (f64), `bool` (i8), `str`, `None`
- **Containers**: `list[T]`, `dict[K,V]`, `defaultdict[K,V]`, `tuple[T1,...,Tn]`, `set[T]`, `bytes`
- **Special**: `Union[T, U]`, `Optional[T]`, `Iterator[T]`, `Any`, `HeapAny`
- **Collections**: `Type::DefaultDict(K, V)`, `Type::RuntimeObject(TypeTagKind::Counter)`, `Type::RuntimeObject(TypeTagKind::Deque)`
- **Classes**: `Type::Class { class_id, name }`
- **Exceptions**: `Type::BuiltinException(BuiltinExceptionKind)`
- **Any vs HeapAny**: `Any` = ambiguous (raw i64 or pointer), `HeapAny` = guaranteed `*mut Obj` (safe for runtime dispatch in print/compare)

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
1. Edit `define_type_tags!` macro in `core-defs/src/type_tags.rs`
2. Add entry: `NewType = N => "NewType" => "<class 'module.NewType'>" => "module.NewType"`
3. Runtime and compiler automatically pick up the new definition

**Adding new exception:** Edit `define_exceptions!` macro in `core-defs/src/exceptions.rs`.

## MIR Parameterized Enums

MIR uses parameterized enums to reduce `RuntimeFunc` variants. See `crates/mir/src/lib.rs`:

| Enum | Purpose |
|------|---------|
| `ValueKind` | Storage ops (cells, globals, class attrs) |
| `PrintKind` | Print operations by type |
| `ReprTargetKind` + `StringFormat` | repr()/ascii() |
| `CompareKind` + `ComparisonOp` | Comparisons |
| `ContainerKind` + `MinMaxOp` + `ElementKind` | min()/max() |
| `IterSourceKind` + `IterDirection` | iter()/reversed() |
| `SortableKind` | sorted() |
| `ConversionTypeKind` | Type conversions |

Use `type_to_value_kind()` in `runtime_selector.rs` for mapping types.

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

## String Interning

Runtime string pool for deduplication (strings < 256 bytes):
- Compile-time literals → `rt_make_str_interned`
- Dict keys auto-interned
- Python API: `sys.intern(string)`
- Lock-free: single `UnsafeCell<HashMap>` (no sharded mutexes)
- Small string optimization: sizes rounded to slab classes (24/32/48/64) in `rt_make_str_impl`
- `eq_hashable_obj` checks pointer equality first (`a == b`), then `slice`-based memcmp
