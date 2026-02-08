# CLAUDE.md

Guidance for Claude Code when working with this repository.

## Project Overview

AOT compiler for a Python subset, implemented in Rust with Cranelift backend.

**Goals:** Native executables, static typing, CPython-compatible behavior, good error diagnostics
**Non-goals:** Full CPython compatibility, dynamic features (`eval/exec`, metaclasses), arbitrary precision integers (uses i64)

**References:** [COMPILER_STATUS.md](COMPILER_STATUS.md) (features), [CONVENTIONS.md](CONVENTIONS.md) (coding standards), [INSIGHTS.md](INSIGHTS.md) (non-obvious gotchas)

## Key Principles

1. **CPython compatibility** — Compiled code must run identically in CPython
2. **Safety first** — `#![forbid(unsafe_code)]` in compiler crates (only `runtime` uses unsafe)
3. **DRY** — Reuse existing code, avoid duplication
4. **Single Source of Truth** — Shared data in leaf crates (`core-defs`, `stdlib-defs`); never duplicate definitions
5. **Generic over specific** — Unified functions for multiple types
6. **Comprehensive testing** — Test edge cases, run full suite after changes
7. **Strong typing** — Use ID types, enums, `Result` for type safety
8. **Error handling** — Avoid `.unwrap()`; use `.expect("descriptive message")` or `?`
9. **No backward compatibility** — Freely refactor without compatibility shims
10. **Leverage Rust ecosystem** — When implementing stdlib, prefer Rust's standard library and well-established crates over custom implementations

## Build & Run

```bash
cargo build --workspace --release      # Build all (release)
cargo build -p pyaot-runtime --release # Runtime library (required for linking)
cargo test --workspace                 # Run all tests
cargo fmt && cargo clippy --workspace  # Format and lint
./test_examples.sh                     # Run example tests

# Compile Python
./target/release/pyaot input.py -o output       # Compile
./target/release/pyaot input.py -o output --run # Compile and run
./target/release/pyaot input.py --emit-hir      # Debug: show HIR
./target/release/pyaot input.py --emit-mir      # Debug: show MIR
./target/release/pyaot input.py -o output --debug  # Preserve symbols (no optimizations)
```

## Architecture

### Pipeline
```
Python → AST → HIR → MIR → Cranelift → Object → Executable
```

### Project Structure

| Crate | Purpose | Key Files |
|-------|---------|-----------|
| `cli` | Entry point, orchestrates pipeline | `main.rs` |
| `core-defs` | Shared definitions (leaf crate) | `exceptions.rs`, `type_tags.rs` |
| `stdlib-defs` | Stdlib module definitions | `types.rs`, `registry.rs`, `modules/*.rs` |
| `frontend-python` | Python parsing → HIR | `ast_to_hir/` |
| `hir` | High-level IR | `lib.rs` |
| `types` | Type system | `lib.rs` |
| `lowering` | HIR → MIR transformation | `context/`, `expressions/`, `statements/` |
| `mir` | Mid-level IR (CFG) | `lib.rs` |
| `optimizer` | MIR optimization passes | `inline/` |
| `codegen-cranelift` | Native code generation | `instructions.rs`, `runtime_calls/` |
| `linker` | Object → Executable | `lib.rs` |
| `runtime` | Runtime library (staticlib) | `gc.rs`, `object.rs`, collections, stdlib |
| `utils` | IDs, string interning | `ids.rs`, `interner.rs` |
| `semantics` | Name resolution, control flow | `lib.rs` |
| `typecheck` | Type checking | `lib.rs` |
| `diagnostics` | Error reporting | `lib.rs` |

### Runtime Module Structure
```
crates/runtime/src/
├── gc.rs, object.rs, exceptions.rs, vtable.rs    # Core
├── boxing.rs, conversions.rs, hash.rs, instance.rs, math_ops.rs  # Type ops
├── dict.rs, set.rs, bytes.rs, tuple.rs, list/, string/  # Collections
├── iterator/, sorted.rs, generator.rs  # Iteration
├── globals.rs, cell.rs, class_attrs.rs  # Variable storage
├── print.rs, format.rs, file.rs, stringio.rs  # I/O
├── json.rs, os.rs, re.rs, sys.rs, time.rs  # Standard library
├── random.rs, hashlib.rs, subprocess.rs  # Standard library (cont.)
├── urllib_parse.rs, urllib_request.rs, base64_mod.rs  # Standard library (cont.)
├── copy.rs, functools.rs, abc.rs, builtins.rs  # Standard library (cont.)
└── hash_table_utils.rs, minmax_utils.rs, slice_utils.rs, utils.rs  # Utilities
```

## Key APIs

### Type System
- **Primitives**: `int` (i64), `float` (f64), `bool` (i8), `str`, `None`
- **Containers**: `list[T]`, `dict[K,V]`, `tuple[T1,...,Tn]`, `set[T]`, `bytes`
- **Special**: `Union[T, U]`, `Optional[T]`, `Iterator[T]`
- **Classes**: `Type::Class { class_id, name }`
- **Exceptions**: `Type::BuiltinException(BuiltinExceptionKind)`

### Shared Definitions (`core-defs`)

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
// CompletedProcess, ParseResult, HttpResponse, Hash, StringIO, BytesIO
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

### Stdlib Definitions (`stdlib-defs`)

Declarative stdlib definitions with automatic lowering/codegen.

```rust
pub struct StdlibFunctionDef {
    pub name: &'static str,           // Python name
    pub runtime_name: &'static str,   // Runtime function
    pub params: &'static [ParamDef],
    pub return_type: TypeSpec,
    pub min_args: usize,              // Minimum required arguments
    pub max_args: usize,              // Maximum allowed arguments
    pub hints: LoweringHints,         // variadic_to_list, auto_box
}

// Registry API
get_module("sys") -> Option<&StdlibModuleDef>
get_function("sys", "exit") -> Option<&StdlibFunctionDef>
get_constant("math", "pi") -> Option<&StdlibConstDef>
```

**Adding stdlib module:**
1. Create `crates/stdlib-defs/src/modules/newmod.rs` with `StdlibModuleDef`
2. Register in `modules/mod.rs`: add to `ALL_MODULES`
3. Implement runtime functions in `crates/runtime/src/newmod.rs`

No changes needed in lowering or codegen — hints handle everything.

**Implementation guidelines:**
- Prefer Rust's standard library (`std::*`) over custom implementations
- Use well-established, lightweight crates (e.g., `regex`, `serde_json`) when needed
- Avoid reinventing functionality that already exists in the Rust ecosystem
- Keep dependencies minimal and only add crates with active maintenance

### Object Types (`stdlib-defs/object_types.rs`)

Declarative registry for runtime object types (Match, StructTime, CompletedProcess, File).
Defines fields and methods with metadata for automatic lowering/codegen.

```rust
pub struct ObjectTypeDef {
    pub type_tag: TypeTagKind,
    pub name: &'static str,
    pub fields: &'static [ObjectFieldDef],    // Field accessors
    pub methods: &'static [&'static StdlibMethodDef],  // Object methods
    pub display_format: DisplayFormat,
}

// Lookup API
lookup_object_type(TypeTagKind::Match) -> Option<&ObjectTypeDef>
lookup_object_field(TypeTagKind::Match, "start") -> Option<&ObjectFieldDef>
lookup_object_method(TypeTagKind::Match, "group") -> Option<&StdlibMethodDef>
```

**Adding object methods:**
1. Define `StdlibMethodDef` in the module (e.g., `re.rs`)
2. Add to `ObjectTypeDef.methods` array in `object_types.rs`
3. Implement runtime function in `crates/runtime/src/*.rs`

No lowering or codegen changes needed — uses generic `ObjectMethodCall` variant.

**Note:** File methods use separate dispatch due to I/O complexity and state management.

### MIR Parameterized Enums

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

### GC (Shadow-stack Mark-sweep)

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
- Codegen: `gc_push` on entry, `gc_pop` on exit

### Runtime Object Header

```rust
#[repr(C)]
pub struct ObjHeader {
    pub type_tag: TypeTagKind,  // From core-defs (single source of truth)
    pub marked: bool,
    pub size: usize,
}
```

The runtime uses `TypeTagKind` directly from `core-defs` — no duplicate enum.

### Type Tag Architecture

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

### String Interning

Runtime string pool for deduplication (strings < 256 bytes):
- Compile-time literals → `rt_make_str_interned`
- Dict keys auto-interned
- Python API: `sys.intern(string)`

## Error Handling

```rust
// Avoid .unwrap(), use .expect() with context:
let var = var_map.get(&id).expect("internal error: local not in var_map");

// Propagate errors with ?:
let func_id = declare_runtime_function(&mut module, "name", &sig)?;
```

**Common patterns:**
- Mutex: `.expect("GLOBALS mutex poisoned")`
- Layout: `.expect("Allocation size overflow")`
- Invariants: `.expect("list must have at least one element")`

## Testing

```bash
./test_examples.sh        # Run all example tests
cargo test --workspace    # Unit + integration tests
cargo clippy --workspace  # Lint all crates
cargo fmt --all           # Format all crates
```

Tests organized by domain in `examples/` (~1000+ assertions). Add to **existing** files by section.
**Use descriptive variable names** to avoid conflicts (prefix with test context if needed)
**Run `./test_examples.sh`** to verify all tests pass

Only create a new test file if:
- The feature requires special compilation flags (like `--module-path`)
- The test has side effects that could affect other tests
- The feature is fundamentally different from all existing categories

Use direct file editing instead of cat with heredocs.

**Debug builds** include type tag assertions:
```bash
cargo build --workspace  # Assertions enabled
# Type mismatch → panic: "rt_list_get: expected List, got Dict"
```

**Note:** `--debug` flag is different — it preserves symbols in generated executable for assembly-level debugging.

## Documentation

When implementing features:
1. Update `COMPILER_STATUS.md` — feature status
2. Update `README.md` — if significant user-facing changes
3. Add tests to appropriate existing file in `examples/`
4. Record non-obvious insights in `INSIGHTS.md`

## Dependencies

- Parsing: `rustpython-parser`
- Backend: `cranelift-codegen`, `cranelift-frontend`, `cranelift-module`, `cranelift-object`
- Data: `indexmap`, `hashbrown`, `smallvec`
