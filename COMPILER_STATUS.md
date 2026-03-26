# Python AOT Compiler - Implementation Status

## Overview

A functional Python Ahead-of-Time (AOT) compiler written in Rust that compiles a statically-typed Python subset to native executables using Cranelift as the backend.

---

## Architecture

```
Python Source (.py)
    ↓
[Parser] (rustpython-parser)
    ↓
Abstract Syntax Tree (AST)
    ↓
[AST → HIR Converter]
    ↓
High-level IR (HIR) - Desugared Python with types
    ↓
[Semantic Analysis] (name resolution, control flow)
    ↓
[Type Inference] (type_planning module in lowering)
    ↓
[HIR → MIR Lowering]
    ↓
Mid-level IR (MIR) - CFG with basic blocks
    ↓
[MIR Optimizer] (optional, --inline / --dce flags)
    ↓
[Cranelift Code Generator]
    ↓
Object File (.o)
    ↓
[Linker] (gcc/clang + runtime library)
    ↓
Native Executable
```

## Feature Status

### Types

| Type | Status | Limitations |
|------|--------|-------------|
| int (i64) | ✅ | |
| float (f64) | ✅ | |
| bool | ✅ | |
| str | ✅ | |
| list[T] | ✅ | |
| tuple[T1, ...] | ✅ | |
| dict[K, V] | ✅ | Keys: all hashable types (str/int/bool/float/tuple/None); insertion order preserved (Python 3.7+) |
| set[T] | ✅ | Elements: all hashable types (str/int/bool/float/tuple/None) |
| bytes | ✅ | |
| None | ✅ | |
| Union[A, B] | ✅ | Full operator support: ==, !=, <, <=, >, >=, is, is not, in, not in |
| Optional[T] | ✅ | Desugared to Union[T, None] |
| Iterator[T] | ✅ | |
| File | ✅ | |
| Generic TypeVar | ❌ | Not planned |
| Protocol | ❌ | Not planned |

### Operators

| Category | Operators | Status | Limitations |
|----------|-----------|--------|-------------|
| Arithmetic | + - * / // % ** | ✅ | Mixed int/float with auto-promotion |
| Comparison | == != < <= > >= | ✅ | Mixed int/float supported |
| Chained Comparison | a < b < c, a == b == c | ✅ | Short-circuit evaluation; middle operands evaluated once |
| Identity | is, is not | ✅ | Pointer comparison for heap types; value comparison for primitives; Union support |
| Logical | and or not | ✅ | Short-circuit evaluation; returns last evaluated value (Python semantics) |
| Bitwise | & \| ^ << >> ~ | ✅ | int only |
| Membership | in, not in | ✅ | Union container support via runtime dispatch |
| Dict merge | \| \|= | ✅ | `dict \| dict`, `dict \|= dict` (Python 3.9+) |
| Augmented | += -= *= /= //= %= **= &= \|= ^= <<= >>= | ✅ | All 12 variants |

### Control Flow

| Feature | Status | Limitations |
|---------|--------|-------------|
| if/elif/else | ✅ | Implicit truthiness for all types (int, str, list, dict, set, etc.) |
| while | ✅ | |
| for | ✅ | range, iterables (list/tuple/dict/str/set/bytes/file), unpacking with starred expressions |
| for...else / while...else | ✅ | else block runs when loop completes without break |
| break/continue | ✅ | |
| try/except/else/finally | ✅ | Full exception objects with `.args`, custom fields, `str(e)` |
| Multiple except types | ✅ | `except (ValueError, TypeError) as e:` |
| del statement | ✅ | `del dict[key]`, `del list[index]` |
| Walrus operator `:=` | ✅ | `if (n := len(items)) > 10:` |
| with (context managers) | ✅ | Exception suppression; `__exit__` receives `(exc_instance, exc_instance, None)` on exception, `(None, None, None)` otherwise |
| assert | ✅ | Supports f-string messages |
| match (pattern matching) | ✅ | Literal, singleton, capture, or, sequence, starred, mapping, class patterns; guards; `**rest` in mapping |
| Multiple assignment | ✅ | `a = b = c = 5` |

### Functions & Classes

| Feature | Status | Limitations |
|---------|--------|-------------|
| Functions | ✅ | Type annotations required for params |
| Default parameters | ✅ | |
| Keyword arguments | ✅ | |
| *args, **kwargs (definition) | ✅ | Function parameters: `def f(*args, **kwargs)` |
| *args unpacking (call-site) | ✅ | Compile-time literals: `f(*[1,2,3])`, `f(*(1,2,3))` |
| **kwargs unpacking (call-site) | ✅ | Compile-time literals: `f(**{"a":1,"b":2})` |
| Runtime unpacking | ✅ | Tuple/list variables: `f(*args_tuple)`, dict variables: `f(**kwargs_dict)` |
| Keyword-only parameters | ✅ | With defaults and bare * syntax |
| Lambda | ✅ | Read-only captures; default parameters supported |
| Nested functions | ✅ | |
| nonlocal | ✅ | Cell-based storage |
| global | ✅ | All types supported |
| Generators | ✅ | `yield from` supported; yield with ternary conditions; `not` with truthiness in filter conditions; throw() not supported |
| Classes | ✅ | Single inheritance; class attrs accessible through instances |
| `__init__` | ✅ | Fields from `self.field = value` auto-discovered |
| `__str__`, `__repr__` | ✅ | Fallback to default repr for classes without dunder methods |
| `__eq__`, `__ne__` | ✅ | `__ne__` auto-negates `__eq__` if not defined |
| `__lt__`, `__le__`, `__gt__`, `__ge__` | ✅ | Enables sorted(), min(), max() on custom objects |
| `__add__`, `__sub__`, `__mul__` | ✅ | Arithmetic operators on custom objects; explicit calls (`a.__add__(b)`) supported |
| `__radd__`, `__rsub__`, `__rmul__`, etc. | ✅ | Reverse arithmetic dunders; enables `2 + obj` when left operand has no forward dunder |
| `__neg__`, `__pos__` | ✅ | Unary minus/plus on custom objects |
| `__abs__` | ✅ | `abs(obj)` on custom classes |
| `__invert__` | ✅ | `~obj` on custom classes |
| `__int__`, `__float__`, `__bool__` | ✅ | `int(obj)`, `float(obj)`, `bool(obj)` conversion dunders |
| `__getitem__`, `__setitem__`, `__delitem__`, `__contains__` | ✅ | Container protocol for custom classes |
| `__iter__`, `__next__` | ✅ | Iterator protocol for custom classes (for loops, iter(), next()) |
| `__call__` | ✅ | Callable objects: `obj(args)` dispatches to `__call__` |
| `__enter__`, `__exit__` | ✅ | Context manager protocol for user-defined classes |
| `__hash__`, `__len__` | ✅ | Raises TypeError for classes without these methods |
| @staticmethod | ✅ | |
| @classmethod | ✅ | cls receives class_id as int |
| @property | ✅ | Getter and setter |
| User decorators | ✅ | Identity, wrapper, chained decorators, and `*args` forwarding (up to 8 args) |
| @abstractmethod | ✅ | Compile-time enforcement |
| `__slots__` | ✅ | Parsed and ignored (AOT compiler handles memory layout statically) |
| Inheritance | ✅ | Single only |
| Virtual dispatch (vtables) | ✅ | |
| super() | ✅ | |
| Class attributes | ✅ | |

### Built-in Functions

| Function | Status | Limitations |
|----------|--------|-------------|
| print() | ✅ | sep, end, file, flush kwargs |
| len() | ✅ | |
| range() | ✅ | start/stop/step |
| str(), int(), float(), bool() | ✅ | int() supports base parameter: int("ff", 16) |
| list(), tuple(), dict(), set() | ✅ | Constructors from iterables |
| iter(), next() | ✅ | |
| enumerate(), zip() | ✅ | zip supports 2 and 3+ iterables |
| map(), filter() | ✅ | Single iterable only; supports builtin functions as args; proper element type inference for `list(map(...))` |
| functools.reduce() | ✅ | Supports initial value and closures with captures |
| format() | ✅ | Format specs: d, b, o, x, X, f, e, g, width, fill, alignment, grouping (`,`, `_`) |
| reversed(), sorted() | ✅ | sorted() supports key= (incl. builtins like abs) and reverse= |
| min(), max(), sum() | ✅ | Supports lists, tuples, sets, ranges, and iterators/generators |
| abs(), pow(), round() | ✅ | |
| hash(), id() | ✅ | |
| isinstance() | ✅ | |
| issubclass() | ✅ | |
| chr(), ord() | ✅ | |
| all(), any() | ✅ | |
| bin(), hex(), oct() | ✅ | |
| repr(), type() | ✅ | |
| divmod() | ✅ | |
| open() | ✅ | r/w/a/rb/wb modes |
| input() | ✅ | |
| getattr(obj, name[, default]) | ✅ | Static attribute names only |
| setattr(obj, name, value) | ✅ | Static attribute names only |
| hasattr(obj, name) | ✅ | Static attribute names only |
| callable(obj) | ✅ | |

### Collections

| Feature | Status | Limitations |
|---------|--------|-------------|
| List literals | ✅ | |
| List methods | ✅ | append, pop, insert, remove, sort(key= incl. builtins, reverse=), reverse, extend, etc. |
| List slicing | ✅ | With step; slice assignment `list[1:3] = [10, 20]` |
| Tuple literals | ✅ | |
| Tuple unpacking | ✅ | Nested patterns and starred expressions |
| Tuple methods | ✅ | index, count |
| Dict literals | ✅ | Including `**` unpacking in dict displays: `{**d1, "key": val, **d2}` |
| Dict methods | ✅ | get, keys, values, items, update, pop, setdefault, popitem, fromkeys, etc. |
| Dict operators | ✅ | `dict \| other`, `dict \|= other` (merge/update, Python 3.9+) |
| Set literals | ✅ | |
| Set methods | ✅ | add, remove, discard, pop, union, intersection, difference, symmetric_difference, update, intersection_update, difference_update, symmetric_difference_update, issubset, issuperset, isdisjoint, etc. |
| Set operators | ✅ | `\|`, `&`, `-`, `^` |
| Bytes methods | ✅ | decode, startswith, endswith, find, rfind, count, replace, split, rsplit, strip, lstrip, rstrip, upper, lower, join, hex, index; concatenation, repetition |
| Comprehensions | ✅ | list, dict, set, generator |

### Strings

| Feature | Status | Limitations |
|---------|--------|-------------|
| Literals | ✅ | |
| Concatenation, multiplication | ✅ | |
| Slicing, indexing | ✅ | |
| Methods | ✅ | upper, lower, strip, split, join, find, rfind, rindex, replace, removeprefix, removesuffix, splitlines, partition, rpartition, expandtabs, rsplit, encode, etc. |
| Predicate methods | ✅ | isdigit, isalpha, isalnum, isspace, isupper, islower, isascii |
| f-strings | ✅ | {expr:.Nf} for floats, {expr!r}, {expr!s}, {expr!a} conversion flags, {expr=} debug format, width/alignment/fill (`{:>10}`, `{:*^10}`), grouping separators (`{:,}`, `{:_}`) |
| .format() | ✅ | {}, {0}, {name}, {:>10}, {:<5}, {:^20}, {:*>10}, {:.2f} |
| String interning | ✅ | Compile-time constants and dict keys under 256 bytes |

### Module System

| Feature | Status | Limitations |
|---------|--------|-------------|
| from X import name | ✅ | |
| import X | ✅ | Module namespace access |
| from typing import ... | ✅ | Compile-time only |
| Packages (__init__.py) | ✅ | Dotted imports supported |
| Relative imports | ✅ | See supported patterns below |
| import * | ❌ | Not planned |

### Standard Library

| Module | Functions/Constants |
|--------|---------------------|
| abc | abstractmethod |
| sys | argv, exit, intern |
| os | environ, remove, getcwd, chdir, listdir, mkdir, makedirs, rmdir |
| os.path | join, exists |
| re | search, match, sub; Match.group(), .start(), .end(), .groups(), .span() |
| json | dumps, loads, dump, load |
| math | pi, e, tau, inf, nan (constants); sqrt, sin, cos, tan, asin, acos, atan, atan2, sinh, cosh, tanh, ceil, floor, trunc, log, log2, log10, exp, fabs, fmod, copysign, hypot, pow, degrees, radians, factorial, gcd, lcm, comb, perm, isnan, isinf, isfinite |
| time | time, sleep, monotonic, perf_counter, ctime, struct_time, localtime, gmtime, strftime, strptime |
| subprocess | run, CompletedProcess |
| urllib.parse | urlparse, urlencode, quote, unquote, urljoin, parse_qs; ParseResult fields and geturl() |
| urllib.request | urlopen; HTTPResponse fields (status, url, headers), methods (read, geturl, getcode) |
| string | ascii_letters, ascii_lowercase, ascii_uppercase, digits, hexdigits, octdigits, punctuation, whitespace, printable |
| random | random, randint, choice, choices, shuffle, seed, uniform, randrange, sample, gauss |
| hashlib | md5, sha256, sha1; Hash.hexdigest(), Hash.digest() |
| base64 | b64encode, b64decode, urlsafe_b64encode, urlsafe_b64decode |
| copy | copy, deepcopy |
| functools | reduce |
| itertools | chain, islice (with for-loop and next(); both import styles) |
| io | StringIO (write, read, readline, getvalue, seek, tell, close, truncate), BytesIO (write, read, readline, getvalue, seek, tell, close, truncate) |

Stdlib function calls support **keyword arguments** mapped to parameter positions (e.g., `random.choices([1,2,3], weights=[...], k=5)`).

#### Stdlib Architecture

The stdlib uses a **Single Source of Truth** pattern with declarative definitions:

```
stdlib-defs (definitions) → frontend (import validation)
                          → lowering (hints-based processing)
                          → codegen (generic signature building)
                          → runtime (actual implementation)
```

**Key features:**
- **Declarative hints**: Functions declare `LoweringHints` for special handling
- **Auto-boxing**: Primitives automatically boxed when passed to `Any` parameters
- **Variadic-to-list**: Variadic functions collect args into list automatically
- **Default values**: Optional parameters use `ParamDef.default` values
- **Compile-time constants**: `math.pi` etc. inlined as literals (no runtime call)

**Adding new stdlib function:** Only 2 files need changes:
1. `stdlib-defs/modules/X.rs` - define `StdlibFunctionDef` with hints
2. `runtime/src/X.rs` - implement `rt_X_func`

No changes needed in lowering or codegen - hints handle everything declaratively.

**Adding object methods** (e.g., Match.group(), CompletedProcess.returncode):
1. `stdlib-defs/object_types.rs` - add `StdlibMethodDef` to `ObjectTypeDef.methods`
2. `runtime/src/X.rs` - implement `rt_X_method`

Uses generic `ObjectMethodCall` and `ObjectFieldGet` variants for automatic dispatch.

### Exceptions

| Type | Status |
|------|--------|
| Exception | ✅ |
| ValueError, TypeError, KeyError | ✅ |
| IndexError, AttributeError | ✅ |
| RuntimeError, IOError, OSError | ✅ |
| StopIteration, GeneratorExit | ✅ |
| AssertionError | ✅ |
| ZeroDivisionError, OverflowError, MemoryError | ✅ |
| NameError, NotImplementedError | ✅ |
| FileNotFoundError, FileExistsError, PermissionError | ✅ |
| RecursionError, EOFError | ✅ |
| SystemExit, KeyboardInterrupt | ✅ |
| ImportError, ConnectionError, TimeoutError | ✅ |
| SyntaxError | ✅ |
| Custom exception classes | ✅ |

### Optimizations

| Optimization | Status | CLI Flag | Description |
|--------------|--------|----------|-------------|
| Function Inlining | ✅ | `--inline` | Inlines small functions at call sites to reduce call overhead |
| Dead Code Elimination | ✅ | `--dce` | Removes unreachable blocks, dead instructions, and unused locals |

**Function Inlining Details:**
- Inlines leaf functions with ≤10 instructions automatically
- Considers functions with ≤50 instructions (configurable via `--inline-threshold`)
- Never inlines: recursive functions, generators, exception handlers
- Preserves GC roots during transformation
- Multiple iterations for transitive inlining

**Dead Code Elimination Details:**
- Unreachable block elimination: BFS from entry block, removes blocks not reachable via CFG
- Dead instruction elimination: removes pure instructions whose results are never used
- Dead local elimination: cleans up unused local variable entries
- Iterates to fixpoint for cascading dead code removal
- Preserves all side-effectful instructions (calls, GC, exception handling, arithmetic that may raise)

### Binary Size Optimization

| Optimization | Status | Description |
|--------------|--------|-------------|
| Post-link strip | ✅ | Automatic `strip` after linking removes symbol tables |
| panic = "abort" | ✅ | Eliminates unwinding infrastructure |
| Linux --gc-sections | ✅ | Removes unused code sections on ELF targets |
| Runtime feature-gating | ✅ | Optional deps (json, regex, crypto, network, base64) behind cargo features |

**Hello world binary size (macOS arm64):**
- Full runtime (default): ~396 KB
- Minimal runtime (`--no-default-features`): ~347 KB

**Minimal runtime build:** `cargo build -p pyaot-runtime --release --no-default-features`

Available runtime features: `stdlib-json` (serde_json), `stdlib-regex` (regex-lite), `stdlib-crypto` (sha2, md-5), `stdlib-base64` (base64), `stdlib-network` (ureq). Default: all enabled via `stdlib-full`.

### Debugging

| Feature | Status | CLI Flag | Description |
|---------|--------|----------|-------------|
| Debug Information | ✅ | `--debug` | DWARF debug info with source line mappings |

**Debug Flag Details:**

What `--debug` provides:
- ✅ **DWARF debug info** — `.debug_info`, `.debug_line`, `.debug_abbrev`, `.debug_str` sections in object files
- ✅ **Source line+column mappings** — line number table maps machine code addresses to Python source lines with column precision
- ✅ **Function debug entries** — `DW_TAG_subprogram` DIEs for user-defined Python functions (compiler internals filtered out)
- ✅ **Parameter info** — `DW_TAG_formal_parameter` with names and type references for each function parameter
- ✅ **Base type definitions** — `DW_TAG_base_type` for `int`, `float`, `bool`, `str` with correct sizes and encodings
- ✅ **Disables Cranelift optimizations** (sets `opt_level` to `none` instead of `speed`)
- ✅ **Preserves all symbols** (disables symbol stripping during linking)
- ✅ **Enables frame pointers** for better stack traces and profiling
- ✅ **Enables Cranelift IR verifier** for compiler correctness checks
- ✅ **macOS**: Automatically runs `dsymutil` after linking; preserves `.o` file for debug map

What `--debug` does NOT provide (yet):
- ❌ **No variable locations** — parameter/variable names and types visible in DWARF, but runtime locations (register/stack) not tracked — debugger can't print values
- ❌ **No multi-file DWARF** — only the main source file gets debug info (imported modules excluded)

**Debugging with lldb/gdb:**
```bash
pyaot program.py -o program --debug

# Inspect DWARF info in object file
dwarfdump program.o

# Set breakpoints on function names
lldb -o "b add" -o "r" program

# View backtrace with Python function names
lldb -o "b add" -o "r" -o "bt" program
```

---

## Known Limitations

1. **Type Annotations**: Function parameters require type annotations; local variables infer types from initializers

---

## Not Planned (AOT Design Decisions)

These features are intentionally not supported because they conflict with AOT compilation goals:

| Feature | Reason |
|---------|--------|
| `eval()`, `exec()` | Requires runtime interpreter |
| `compile()` | Requires runtime compiler |
| Dynamic `getattr(obj, name_var)` | Runtime-computed attribute names; static `getattr(obj, "literal")` is supported |
| `__dict__` access | Would require dictionary storage for all objects |
| Metaclasses | Excessive runtime complexity for static compilation |
| Multiple inheritance | Complex MRO, vtable conflicts |
| `import *` | Unclear namespace pollution, complicates static analysis |
| `globals()`, `locals()` | Requires runtime introspection of symbol tables |
| Stack traces | Would require debug info overhead in optimized code |
| `inspect` module | Runtime introspection incompatible with AOT |
| Dynamic class creation | `type(name, bases, dict)` requires runtime class generation |

---

## Roadmap

See **[ROADMAP.md](ROADMAP.md)** for the full development roadmap with detailed implementation plans.

---

## Implementation Reference

This section documents internal implementation details for developers working on the compiler.

### Type Inference for Literals

The frontend automatically infers types for literal expressions (Int, Float, Bool, Str, None) and stores them in HIR. This is critical for lowering — without it, module-level variables would have type `Any` causing Cranelift type mismatches.

### Type Inference for Container Constructors

The type planning module (`lowering/src/type_planning/`) implements bidirectional type inference for container constructors and related builtins, inferring element types from arguments:

- **Helper method**: `extract_iterable_element_type()` extracts element types from iterable types (DRY pattern)
- **Container constructors**:
  - `list(iterable)` → infers element type from iterable argument
  - `tuple(iterable)` → infers element type from iterable argument
  - `set(iterable)` → infers element type from iterable argument
  - `dict(a=1, b=2)` → infers `dict[str, value_type]` from keyword arguments
- **Iterator builtins**:
  - `iter(iterable)` → `Iterator[elem_type]`
  - `reversed(iterable)` → `Iterator[elem_type]`
  - `sorted(iterable)` → `list[elem_type]`
  - `enumerate(iterable)` → `Iterator[tuple[int, elem_type]]`
  - `zip(iter1, iter2, ...)` → `Iterator[tuple[elem1, elem2, ...]]`
  - `filter(func, iterable)` → `Iterator[elem_type]` (from second argument)
  - `range(...)` → `Iterator[int]`
- **Element type extraction** (from `extract_iterable_element_type`):
  - `list[T]` → `T`
  - `tuple[T, ...]` → first element type (or `Any` for empty)
  - `set[T]` → `T`
  - `dict[K, V]` → `K` (key type, since dict iteration yields keys)
  - `str` → `str` (characters)
  - `bytes` → `int` (byte values)
  - `Iterator[T]` → `T`

### Augmented Assignment

Augmented assignments (`x += 5`, `obj.field -= 1`, `list[i] *= 2`) are desugared at the AST→HIR stage:

- **Desugaring approach**: `target op= value` → `target = target op value`
- **Implementation**: `statements.rs` handles `py::Stmt::AugAssign`
- **Target types supported**:
  - Simple variables: `x += 5` → `StmtKind::Assign { target, value: BinOp }`
  - Attributes: `obj.field += 5` → `StmtKind::FieldAssign { obj, field, value: BinOp }`
  - Subscripts: `list[i] += 5` → `StmtKind::IndexAssign { obj, index, value: BinOp }`
- **Operators**: All 12 Python augmented assignment operators map to existing `BinOp` variants
- **Reuses existing infrastructure**: No changes needed in HIR, MIR, lowering, or codegen

### Assert Statement

- **HIR**: `StmtKind::Assert { cond: ExprId, msg: Option<ExprId> }`
- **MIR**: Lowered to conditional branch + `RuntimeCall AssertFail` or `AssertFailObj`
- **Codegen**: String literal messages passed directly as `mir::Constant::Str`, pointer passed to `rt_assert_fail`
- **Runtime**:
  - `rt_assert_fail(msg: *const i8)` - for string literals (C string pointer)
  - `rt_assert_fail_obj(msg: *const Obj)` - for string objects (f-strings, variables)
  - Both print `AssertionError: <message>` and call `std::process::exit(1)`
- **Message handling**:
  - String literals: passed directly as C strings via `AssertFail`
  - F-strings and variables: lowered as string objects, passed via `AssertFailObj`

### Print Function

- **HIR**: `ExprKind::BuiltinCall { builtin: Builtin::Print, args, kwargs }`
- Supports `sep` and `end` keyword arguments
- **MIR**: Sequence of `RuntimeCall` instructions (`PrintInt`, `PrintFloat`, etc.)
- Custom `sep`/`end` or defaults (`PrintSep` for space, `PrintNewline` at end)

### Keyword Arguments & Default Parameters

- **HIR**: `KeywordArg { name: InternedString, value: ExprId, span }`
- **HIR**: `Param { ..., default: Option<ExprId> }` for defaults
- **Lowering**: `resolve_call_args()` resolves positional → kwargs → defaults
- Arguments reordered to match function parameter order at compile time

### Keyword-Only Parameters

Keyword-only parameters (parameters after `*args` or bare `*`) must be passed by name and cannot receive positional arguments:

```python
def func(a: int, *args: int, b: int = 10, c: int = 20) -> int:
    # a: regular param (positional or keyword)
    # *args: variadic positional
    # b, c: keyword-only (must use b=..., c=...)
    return a + b + c

func(1, b=5)           # OK: a=1, b=5, c=20 (default)
func(1, 2, 3, b=5)     # OK: a=1, args=(2,3), b=5, c=20
func(1, 2, b=5, c=10)  # OK: all specified
# func(1, 2, 5)        # ERROR: can't pass b positionally

def bare_star(x: int, *, y: int = 5) -> int:
    # * without name: no extra positional args accepted
    # y: keyword-only, must use y=...
    return x + y

bare_star(1, y=10)     # OK
# bare_star(1, 10)     # ERROR: too many positional args
```

**Implementation**:
- **HIR** (`hir/lib.rs`):
  - `ParamKind::KeywordOnly` - distinguishes keyword-only from regular parameters
  - `Param { ..., default: Option<ExprId>, kind: ParamKind }` - default values extracted from AST
- **Frontend** (3 files):
  - `frontend-python/src/ast_to_hir/functions.rs` - top-level functions
  - `frontend-python/src/ast_to_hir/statements/nested_functions.rs` - nested functions
  - `frontend-python/src/ast_to_hir/classes.rs` - class methods
  - Extract defaults from `kwonly_arg.default` field (rustpython-parser `ArgWithDefault`)
  - Mark parameters with `ParamKind::KeywordOnly`
- **Lowering** (`lowering/src/lib.rs`):
  - `resolve_call_args()` separates regular, varargs, keyword-only, kwargs parameters
  - Positional args only match regular params (not keyword-only)
  - Keyword args can match both regular and keyword-only params
  - Default values filled for missing keyword-only params
  - Parameter order in call: regular → *args → keyword-only → **kwargs
- **Type Planning** (`lowering/src/type_planning/check.rs`):
  - `check_call_args()` validates parameter kinds separately
  - Checks required regular params satisfied by positional OR keyword args
  - Checks required keyword-only params provided via keyword args
  - Rejects excess positional args when no `*args` present
  - Error messages distinguish "missing required argument" vs "missing keyword-only argument"

**Supported features**:
- Keyword-only params with defaults: `def f(*, x: int = 10)`
- Keyword-only params without defaults (required): `def f(*, x: int)`
- Mixed regular and keyword-only: `def f(a: int, *, b: int = 5)`
- After `*args`: `def f(a: int, *args: int, b: int = 10)`
- Bare `*` syntax: `def f(a: int, *, b: int = 5)` (no extra positional args)
- Expression defaults: `def f(*, count: int = 5 + 5)`
- Multiple keyword-only params: `def f(*, a: int, b: int = 10, c: str = "x")`

### Call-Site Unpacking (*args, **kwargs)

Call-site unpacking allows expanding collections into function arguments at the point of the call.

**Implemented (compile-time literals)**:
```python
def func(a: int, b: int, c: int) -> int:
    return a + b + c

# *args unpacking with literal list/tuple
func(*[1, 2, 3])         # Expanded to: func(1, 2, 3)
func(*(10, 20, 30))      # Expanded to: func(10, 20, 30)

# **kwargs unpacking with literal dict
func(**{"a": 5, "b": 10, "c": 15})  # Expanded to: func(a=5, b=10, c=15)

# Mixed regular and unpacked arguments
func(1, *[2, 3])         # Expanded to: func(1, 2, 3)
func(a=1, **{"b": 2, "c": 3})  # Expanded to: func(a=1, b=2, c=3)

# Multiple unpacking operations
func(*[1, 2], *[3])      # Expanded to: func(1, 2, 3)
func(*[], *[1, 2, 3])    # Expanded to: func(1, 2, 3)
```

**Implementation**:
- **HIR** (`hir/lib.rs`):
  - `CallArg` enum - distinguishes `Regular(ExprId)` vs `Starred(ExprId)`
  - `Call { args: Vec<CallArg>, kwargs_unpack: Option<ExprId> }`
- **Frontend** (`frontend-python/src/ast_to_hir/expressions.rs`):
  - `convert_call_args()` - handles `py::Expr::Starred` → `CallArg::Starred`
  - `convert_keywords()` - extracts `**kwargs` (keyword with `arg: None`)
  - Returns `(Vec<KeywordArg>, Option<ExprId>)` for both regular kwargs and **kwargs
- **Type Planning** (`lowering/src/type_planning/check.rs`):
  - Expands compile-time literals before argument validation
  - `ExprKind::List/Tuple` elements extracted from starred args
  - `ExprKind::Dict` pairs extracted from **kwargs
  - After expansion, validates against function signature
- **Lowering** (`lowering/src/expressions/calls.rs`):
  - `lower_call()` receives `&[CallArg]` and `&Option<ExprId>`
  - Compile-time expansion identical to type checker
  - Expanded args passed to `resolve_call_args()` for matching
- **Semantics** (`semantics/src/lib.rs`):
  - Updated to analyze both `CallArg::Regular` and `CallArg::Starred` expressions
  - Handles `kwargs_unpack` expression analysis

**Test Coverage**:
- Tests in `examples/test_functions.py` covering:
  - Literal list/tuple unpacking with `*args`
  - Literal dict unpacking with `**kwargs`
  - Runtime tuple and list variable unpacking
  - Runtime dict variable unpacking (`f(**my_dict)`)
  - Mixed explicit and runtime kwargs
  - Mixed regular and unpacked arguments
  - Multiple unpacking operations in single call
  - Empty literal unpacking
  - Unpacking into varargs functions
  - Unpacking with default parameter fallback
  - Float/bool/string typed list unpacking
  - Runtime kwargs with keyword-only parameters

### Nested Unpacking and Starred Expressions

**Nested Unpacking**:
```python
# Basic nested unpacking
a, (b, c) = (1, (2, 3))

# Deeper nesting (arbitrary depth supported)
x, (y, (z, w)) = (10, (20, (30, 40)))

# Mixed tuple/list nesting
g, [h, i] = (1, [2, 3])

# Multiple nested groups
(m1, m2), (m3, m4) = ((1, 2), (3, 4))
```

**Starred Expressions in For Loops**:
```python
# Starred at beginning, middle, or end
for first, *rest in [(1, 2, 3), (4, 5, 6)]:
    print(f"first={first}, rest={rest}")  # rest is a list

for *start, last in [(1, 2, 3), (4, 5, 6)]:
    print(f"start={start}, last={last}")

for a, *mid, z in [(1, 2, 3, 4), (5, 6, 7, 8)]:
    print(f"a={a}, mid={mid}, z={z}")
```

**Implementation**:
- **HIR**:
  - `StmtKind::NestedUnpackAssign { targets: Vec<UnpackTarget>, value: ExprId }`
  - `UnpackTarget` enum: `Var(VarId)` or `Nested(Vec<UnpackTarget>)` for recursive patterns
  - `StmtKind::ForUnpackStarred { before_star, starred, after_star, iter, body }`
- **Lowering**:
  - Nested unpacking: Recursive extraction using `TupleGet`/`ListGet` with proper type-based function selection
  - Mixed-type tuples: Automatically boxes primitives when `elem_tag=ELEM_HEAP_OBJ`, unboxes with typed Get functions
  - Starred in for loops: Uses `TupleSliceToList`/`ListSlice` for starred portion with `i64::MAX` sentinel for unbounded end
- **Type Checking**: Recursive type checking for nested patterns ensuring structure matches
- **Key Fix**: Tuples with mixed types (e.g., `(int, tuple)`) now properly box primitive values and use typed Get functions (`TupleGetInt`, `TupleGetFloat`, `TupleGetBool`) for extraction

**Test Coverage**:
- `examples/test_collections_list_tuple.py`:
  - Basic nested unpacking (2 levels)
  - Deep nested unpacking (3+ levels)
  - Mixed tuple/list nesting
  - Multiple nested groups
- `examples/test_iteration.py`:
  - Starred expressions in various positions
  - Multiple starred patterns in same loop
  - Edge cases (empty rest, single element)

### Runtime Variable Unpacking in Function Calls

**Feature**: Unpack tuple and list variables in function call arguments:
```python
def sum_three(a: int, b: int, c: int) -> int:
    return a + b + c

# Runtime unpacking with tuple variable
args_tuple: tuple[int, int, int] = (10, 20, 30)
result: int = sum_three(*args_tuple)  # 60

# Runtime unpacking with list variable
args_list: list[int] = [10, 20, 30]
result: int = sum_three(*args_list)  # 60

# Mixed positional and unpacked arguments
first: int = 100
rest_tuple: tuple[int, int] = (200, 300)
result2: int = sum_three(first, *rest_tuple)  # 600

# Unpacking function result
def make_tuple() -> tuple[int, int, int]:
    return (1, 2, 3)
result3: int = sum_three(*make_tuple())  # 6

# With varargs - collects extra elements into *rest
def sum_varargs(a: int, *rest: int) -> int:
    total: int = a
    for x in rest:
        total = total + x
    return total

my_list: list[int] = [1, 2, 3, 4, 5]
result4: int = sum_varargs(*my_list)  # 15 (a=1, rest=(2,3,4,5))

# With default parameters - uses defaults for missing elements
def with_defaults(a: int, b: int, c: int = 100) -> int:
    return a + b + c

short_list: list[int] = [1, 2]
result5: int = with_defaults(*short_list)  # 103 (a=1, b=2, c=100 default)
```

**Implementation**:
- **HIR**: Uses existing `CallArg::Starred(ExprId)` for both compile-time and runtime unpacking
- **Lowering** (`expressions/calls.rs`, `lib.rs`):
  - `ExpandedArg` enum: `Regular(ExprId)`, `RuntimeUnpackTuple(ExprId)`, `RuntimeUnpackList(ExprId)`
  - Starred arguments with literal tuples/lists → compile-time expansion
  - Starred arguments with tuple variables → runtime extraction with typed Get functions
  - Starred arguments with list variables → runtime extraction in `resolve_call_args()`
  - **Varargs support**: Uses `ListTailToTuple`/`ListTailToTupleFloat`/`ListTailToTupleBool` to extract remaining elements
  - **Default parameter support**: Generates conditional MIR blocks that either extract from list or use default value
- **MIR Instructions**:
  - Tuple extraction: `TupleGet`/`TupleGetInt`/`TupleGetFloat`/`TupleGetBool`
  - List extraction: `ListGet`/`ListGetInt`/`ListGetFloat`/`ListGetBool`
  - Varargs tail: `ListTailToTuple`, `ListTailToTupleFloat`, `ListTailToTupleBool`
  - Tuple concatenation: `TupleConcat` for mixed `f(1, 2, *list)` calls
- **Runtime Functions** (`runtime/src/list/convert.rs`):
  - `rt_list_tail_to_tuple(list, start)` - extracts `list[start:]` as tuple
  - `rt_list_tail_to_tuple_float(list, start)` - unboxes float elements
  - `rt_list_tail_to_tuple_bool(list, start)` - unboxes bool elements
- **Type Planning** (`lowering/src/type_planning/check.rs`):
  - `check_call_args()` detects tuple/list types and computes `effective_arg_count`
  - Tracks `tuple_unpack_positions` to validate element types against parameter types
  - Single-element tuples handled correctly

**Key Design Decision**: Unpacking happens at MIR level during lowering, not at HIR level, because `hir_module` is immutable and we cannot create new HIR expressions. The `ExpandedArg` enum tracks which arguments need runtime unpacking.

**Test Coverage**:
- `examples/test_functions.py`:
  - Basic runtime tuple and list unpacking
  - Mixed regular and unpacked arguments
  - Unpacking function results
  - Unpacking with default parameters
  - Unpacking into varargs (`*args`)
  - Multiple unpacking (`f(1, 2, *list)`)
  - Float/bool/string list unpacking
  - Empty varargs tail (`f(*[1])` → `rest=()`)
  - Single-element tuple unpacking

### Runtime Dict Unpacking (**kwargs)

**Feature**: Unpack dict variables into keyword arguments at function call sites:
```python
def greet(name: str, age: int) -> str:
    return f"{name} is {age}"

# Runtime dict unpacking
kwargs: dict[str, int] = {"name": "Alice", "age": 30}
result: str = greet(**kwargs)  # "Alice is 30"

# Mixed explicit and runtime kwargs (explicit takes priority)
d: dict[str, int] = {"a": 100, "b": 200}
result: int = f(a=1, **d)  # a=1 (explicit), b=200 (from dict)

# With default parameters
def f(a: int, b: int = 10) -> int:
    return a + b
d: dict[str, int] = {"a": 5}
result: int = f(**d)  # 15 (b uses default)

# With keyword-only parameters
def f(*, x: int, y: int = 0) -> int:
    return x + y
d: dict[str, int] = {"x": 10}
result: int = f(**d)  # 10 (y uses default)
```

**Implementation**:
- **Lowering** (`lowering/src/expressions/calls.rs`):
  - Detects non-literal dict expressions in `kwargs_unpack`
  - Stores dict operand in `pending_kwargs_from_unpack` for processing
- **Lowering** (`lowering/src/lib.rs`):
  - `resolve_call_args()` generates CFG branches for each parameter:
    - Check if key exists in dict with `DictContains`
    - If present: extract value with `DictGet` (raw value, no unboxing needed)
    - If absent: use default value or emit runtime error for required params
  - Tracks consumed keys for potential `**kwargs` remainder handling
- **Type Planning** (`lowering/src/type_planning/check.rs`):
  - Added `has_runtime_kwargs` flag to skip required argument validation
  - Runtime kwargs can't be validated at compile time (keys unknown)
- **Key Insight**: Dict values are stored as raw values (int/float/bool as i64), not boxed objects, so no unboxing is needed when extracting

**Test Coverage**:
- `examples/test_functions.py`:
  - Basic runtime `**kwargs` unpacking
  - Mixed explicit and runtime kwargs
  - Runtime kwargs with default parameters
  - Explicit kwargs override runtime dict
  - Runtime kwargs with keyword-only parameters
  - Partial dict with multiple defaults
  - Function result dict unpacking

### Type Inference for Local Variables

- **HIR**: `StmtKind::Assign { target, value, type_hint: Option<Type> }`
- Priority: explicit type hint > inferred from RHS
- `get_expr_type()` handles: literals, containers, BinOp/UnOp, function calls, method calls
- Mixed-type list inference: `[1, 2, "str"]` → `list[Union[int, str]]` (checks all elements)
- Mixed-type dict inference: checks all key-value pairs for type consistency
- Empty containers (`[]`, `{}`) are refined during type planning when elements are added later; fall back to `List(Any)` / `Dict(Any, Any)` if usage doesn't clarify the type

### Float Operations

- **Codegen**: `is_float_operand()` helper checks types
- Float arithmetic: `fadd`, `fsub`, `fmul`, `fdiv`, `floor` (for `//`)
- Float comparisons: `fcmp` with `FloatCC::*` conditions
- **Mixed-type promotion**: `is_int_operand()` + `promote_to_float()` via `fcvt_from_sint`

### Range Implementation

`range()` desugared to while loop (not a first-class iterator):

- **Step direction**: `get_step_direction()` analyzes at compile time
  - Positive step: uses `Lt` (i < stop)
  - Negative step: uses `Gt` (i > stop)
  - Unknown: runtime comparison `(step > 0 && i < stop) || (step <= 0 && i > stop)`
- **Loop structure** (4 blocks): Initialize → Header → Body → Increment
- Break/continue via `loop_stack`

### Iterable For-Loop

Lists, tuples, dicts, strings desugared to indexed iteration:

```python
# for x in items: body
__iter = items
__len = len(__iter)
__idx = 0
while __idx < __len:
    x = __iter[__idx]
    body
    __idx = __idx + 1
```

- **Type inference**: `List[T]` → T, `Tuple[T1,...]` → T1, `Dict[K,V]` → K, `Str` → Str
- **Dict**: Uses `DictKeys` runtime to get keys list, then iterates
- **Implementation**: `get_iterable_info()`, `lower_for_iterable()` in `statements.rs`

### GC Integration

- **Lowering** (`lib.rs`):
  - `is_heap_type()`: str, list, dict, tuple, class instances, iterators
  - `get_or_create_local()` sets `is_gc_root: is_heap_type(&var_type)`
- **Codegen** (`lib.rs`):
  - `GcFrameData`: roots_slot, gc_roots mapping
  - Prologue: ShadowFrame init (24 bytes) + `gc_push`
  - Epilogue: `gc_pop` before every `Return`
  - `update_gc_root_if_needed()` updates roots array on assignment
- **ShadowFrame**: `prev (8) + nroots (8) + roots ptr (8) = 24 bytes`
- **Shadow Stack Depth Limit**: Max 10,000 frames tracked; exceeding causes panic (detects frame leaks)
- **Object Finalization**: `sweep()` calls type-specific finalization before dealloc:
  - `list_finalize()`: Frees data array allocated separately from ListObj
  - `dict_finalize()`: Frees entries array allocated separately from DictObj
  - `set_finalize()`: Frees entries array allocated separately from SetObj
  - `file_finalize()`: Closes unclosed file handles
  - `generator_finalize()`: Frees type_tags array for precise GC tracking
- **Object Marking**: `mark_object()` traces all heap pointers for each type:
  - List: marks elements based on elem_tag (ELEM_HEAP_OBJ only; raw ints/bools skipped)
  - Dict: marks all keys and values in entries array
  - Set: marks all elements in entries array
  - Tuple: marks elements based on elem_tag (ELEM_HEAP_OBJ only)
  - Instance: marks only heap fields using `heap_field_mask` (registered via `rt_register_class_fields`; raw value fields like int/float/bool are skipped)
  - Iterator: marks source container
  - Match: marks groups tuple and original string
  - Generator: **precise tracking** using type_tags array (LOCAL_TYPE_PTR only; raw values skipped)
  - Cell: marks contained heap object
  - File: marks filename string
- **Element Tag Validation**: Debug assertions via `validate_elem_tag!` macro detect type mismatches
  - Validates elem_tag consistency in lists/tuples during set/push operations
  - Prevents GC crashes from treating raw integers as heap pointers
- **String Pool Pruning**: `prune_string_pool()` removes dead interned strings during sweep phase

### String Interning

Runtime string interning reduces memory usage by deduplicating strings:

- **Location**: `runtime/src/string/interning.rs`
- **Pool structure**: Single `HashMap<u64, Vec<PoolEntry>>` keyed by FNV-1a hash
- **Lock-free access**: Uses `UnsafeCell` for zero-overhead single-threaded access
- **Size threshold**: Only strings under 256 bytes are interned (balanced approach)
- **When strings are interned**:
  - **Compile-time constants**: Codegen calls `rt_make_str_interned()` for string literals
  - **Dict keys**: `rt_dict_set()` interns string keys under 256 bytes
- **GC integration**:
  - `init_string_pool()` called from `rt_init()`
  - `shutdown_string_pool()` called from `rt_shutdown()`
  - `prune_string_pool()` called during sweep phase before clearing marks
  - Pool entries are removed when their strings are no longer marked as reachable
- **Runtime functions**:
  - `rt_make_str_interned(data, len)` - create or return cached interned string
  - `init_string_pool()` / `shutdown_string_pool()` - lifecycle management
  - `prune_string_pool()` - remove dead strings during GC
- **Memory savings**: For JSON workloads with repetitive keys (e.g., 10k records × 100 unique keys), reduces 1M string allocations to ~100 interned strings

### Hash Table Optimization

Dict and set use open addressing with optimized probing and hashing:

- **Location**: `runtime/src/hash_table_utils.rs`, `runtime/src/dict.rs`, `runtime/src/set.rs`
- **Triangular probing**: Uses probe sequence `index = (base + i*(i+1)/2) & mask`
  - Eliminates primary clustering (unlike linear probing)
  - Guarantees visiting all slots exactly once for power-of-2 table sizes
  - Probe offsets: 0, 1, 3, 6, 10, 15, 21, ... (triangular numbers)
- **SplitMix64 integer hashing**: Better distribution for sequential integers
  - Replaces simple multiplicative hash that caused clustering for 0, 1, 2, 3...
  - Uses avalanche mixing: `x ^= x >> 30; x *= K1; x ^= x >> 27; x *= K2; x ^= x >> 31`
- **Power-of-2 capacity**: Enables fast `& mask` instead of slow `% capacity`
- **Load factor**: Resize at 75% occupancy, grow by 2x

### String Search Optimization

String operations use Boyer-Moore-Horspool algorithm for efficient substring search:

- **Location**: `runtime/src/string/search.rs`, `runtime/src/string/modify.rs`, `runtime/src/string/split_join.rs`
- **Boyer-Moore-Horspool (BMH)**: O(n/m) average case vs O(n*m) naive
  - Uses 256-byte bad character table to skip positions on mismatch
  - Compares pattern from right to left for maximum skipping
- **Applies to**: `find()`, `contains()`, `count()`, `replace()`, `split()`
- **Threshold**: Falls back to naive search for patterns < 4 chars (less overhead)
- **Performance**: Up to 100x faster for long patterns in large strings

### Lock-Free Runtime

The runtime uses lock-free data structures for zero-overhead access in single-threaded AOT-compiled programs:

- **GC state**: `AtomicPtr<GcState>` — gc_push/gc_pop/gc_alloc have zero synchronization overhead
- **Boxing pools**: `UnsafeCell`-based storage — small int pool (-5..256) and bool singletons accessed without locking
- **String pool**: Single `UnsafeCell<HashMap>` — no sharded mutexes
- **Globals/class attrs**: `UnsafeCell<HashMap>` — lock-free get/set
- **VTable/class registry**: `UnsafeCell<[T; 256]>` — direct array access
- **Key comparison**: Pointer equality fast path in `eq_hashable_obj` catches interned strings, pooled ints, and bool singletons
- **String comparison**: Uses `slice` comparison (SIMD-optimized `memcmp`) instead of byte-by-byte loops

### Lazy String Pool Initialization

String pool uses lazy initialization instead of pre-populating 256 single-char strings:

- **Location**: `runtime/src/string/interning.rs`
- **Before**: `init_string_pool()` pre-allocated 256 single-char strings at startup (~10-12ms)
- **After**: Pool starts empty, strings interned on demand when `rt_make_str_interned` is called
- **Benefit**: Faster startup for programs that don't use all single-char strings

### Small Integer Pool

Integer boxing reuses pre-allocated objects for common small integers:

- **Location**: `runtime/src/boxing.rs`
- **Range**: -5 to 256 (CPython-compatible range, 262 integers total)
- **Implementation**: Static pool initialized at `rt_init()`, checked in `rt_box_int()`
- **Benefit**: Reduces GC pressure for common integer operations (loop counters, dict keys)
- **Fallback**: Integers outside range allocated normally via `gc_alloc()`

### StringBuilder Pattern

String concatenation chains use StringBuilder for O(n) instead of O(n²) copying:

- **Location**:
  - Runtime: `runtime/src/string/builder.rs`
  - Lowering: `lowering/src/expressions/operators.rs`
  - Codegen: `codegen-cranelift/src/runtime_calls/string.rs`
- **Chain detection**: Detects left-associative `a + b + c + d...` chains with 3+ string operands
- **StringBuilderObj**: Heap type with growing buffer (TypeTag = 16)
  - `rt_make_string_builder(capacity)` - create builder with estimated capacity
  - `rt_string_builder_append(builder, str)` - append string to buffer
  - `rt_string_builder_to_str(builder)` - finalize to StrObj
- **Growth strategy**: 2x capacity, minimum 64 bytes
- **GC integration**: Builder buffer freed in `string_builder_finalize()` during sweep
- **Performance**: ~50% faster for string concatenation chains (100k iterations benchmark)

### Exception Handling

Uses setjmp/longjmp (no overhead on happy path):

- **Central Registry** (`types/src/exceptions.rs`):
  - Single source of truth for built-in exception types via `define_exceptions!` macro
  - Generates `BuiltinExceptionKind` enum with compile-time lookup methods
  - Current exceptions (tags 0-27): Exception, AssertionError, IndexError, ValueError, StopIteration, TypeError, RuntimeError, GeneratorExit, KeyError, AttributeError, IOError, ZeroDivisionError, OverflowError, MemoryError, NameError, NotImplementedError, FileNotFoundError, PermissionError, RecursionError, EOFError, SystemExit, KeyboardInterrupt, FileExistsError, ImportError, OSError, ConnectionError, TimeoutError, SyntaxError
  - **BuiltinExceptionKind methods:**
    - `tag() -> u8` — get numeric tag
    - `name() -> &'static str` — get Python name
    - `from_tag(u8) -> Option<Self>` — create from tag
    - `from_name(&str) -> Option<Self>` — create from name
    - `ALL: &[Self]` — all variants
  - **Backward-compatible functions:** `exception_name_to_tag()`, `exception_tag_to_name()`, `is_builtin_exception_name()`
  - **Adding new exceptions:** See "Adding New Built-in Exceptions" below
- **Runtime** (`exceptions.rs`):
  - `ExceptionFrame`: prev + jmp_buf (200 bytes) + gc_stack_top = 216 bytes
  - `ExceptionObject`: exc_type, custom_class_id, message, cause (`__cause__`), context (`__context__`), suppress_context, instance (heap-allocated exception object)
  - Core functions: `rt_exc_push_frame`, `rt_exc_pop_frame`, `rt_exc_setjmp`, `rt_exc_raise`, `rt_exc_raise_from`, `rt_exc_raise_from_none`, `rt_exc_reraise`, `rt_exc_clear`
  - Handling markers: `rt_exc_start_handling`, `rt_exc_end_handling` (for `__context__` capture)
  - `rt_exc_get_current()` - returns current exception as heap-allocated instance (lazy for built-ins, eager for custom classes)
  - `rt_exc_get_current_message()` - returns exception message as string (backward compat)
  - `rt_exc_instance_str()` - converts exception instance to string (for `str(e)`)
  - `rt_exc_raise_custom_with_instance()` - raises custom exception with pre-created instance
  - `rt_exc_isinstance(type_tag)` - checks if current exception matches type
  - `gc::unwind_to()` - unwinds shadow stack to target frame (for exception unwinding)
- **Exception Chaining (PEP 3134)**:
  - **Explicit cause** (`raise X from Y`): Sets `__cause__`, displayed as "The above exception was the direct cause..."
  - **Context suppression** (`raise X from None`): Sets `suppress_context=true`, no context displayed
  - **Implicit context** (`__context__`): Automatically captured when raising during exception handling
    - If handler raises new exception, the original exception becomes the new one's `__context__`
    - Displayed as "During handling of the above exception, another exception occurred:"
    - Only displayed if `suppress_context=false` and no explicit `__cause__`
  - **Display priority**: `__cause__` takes precedence over `__context__`
- **MIR Instructions**: `ExcPushFrame`, `ExcPopFrame`, `ExcClear`, `ExcGetType`, `ExcHasException`, `ExcGetCurrent`, `ExcCheckType`, `ExcStartHandling`, `ExcEndHandling`
- **MIR Terminators**: `TrySetjmp`, `Raise` (with optional `RaiseCause` and `suppress_context` flag), `Reraise`
- **Lowering**: `lower_try()` creates CFG (try_body, else_block, handler_dispatch, finally, exit)
  - `build_exception_dispatch()` generates type-based handler selection
  - Typed handlers (`except ValueError:`) check exception type before entering
  - Exception binding (`except E as e:`) emits `ExcGetCurrent` to bind message
  - **Frame Management**: Unified pop strategy for correct cleanup
    - Frame pushed at try entry
    - **Normal path**: Frame popped at end of try body (before else/finally)
    - **Exception path**: `rt_exc_raise` pops frame before longjmp
    - **After handler**: Frame already popped (even if exception suppressed)
    - **Finally block**: Never pops frame (already popped by one of above paths)
  - **Else block**: Exception frame already popped before else, wrapped in recursive try/finally
    - Exceptions in else block are NOT caught by same try's handlers (correct Python semantics)
    - Finally block still runs via recursive wrapper before exception propagates
- **Context Managers** (`with` statements):
  - Desugared to try/except/finally with `__enter__`/`__exit__` calls
  - Exception suppression: `__exit__` returning True prevents exception propagation
  - `__exit__` receives `(exc_instance, exc_instance, None)` on exception, `(None, None, None)` otherwise
  - `exc_type` arg is the exception instance (truthy) rather than a type object (not yet supported)
  - `exc_tb` is always None (traceback objects not yet supported)
- **Variable Preservation Across Exception Unwinding**:
  - Variables assigned in try blocks are automatically wrapped in heap-allocated cells
  - Cells preserve values across setjmp/longjmp exception unwinding (immune to register restoration)
  - After try/except/finally, values are extracted from cells back to normal locals
  - Transparent to user code - no special syntax required

#### Adding New Built-in Exceptions

To add a new built-in exception (e.g., `FileNotFoundError`), modify only **1 file**:

**Single source of truth** (`crates/core-defs/src/exceptions.rs`):
```rust
define_exceptions! {
    Exception = 0 => "Exception",
    // ... existing exceptions ...
    MemoryError = 13 => "MemoryError",
    FileNotFoundError = 14 => "FileNotFoundError",  // ADD: new exception
}
```

The `pyaot-core-defs` crate is a leaf crate (no pyaot-* dependencies) that both `types` and `runtime` crates depend on. This eliminates the need to keep two files in sync.

**What the macro generates automatically:**
- `BuiltinExceptionKind::FileNotFoundError` — shared enum variant
- `Type::BuiltinException(BuiltinExceptionKind::FileNotFoundError)` — type representation (via types crate re-export)
- `ExceptionType::FileNotFoundError` — runtime exception type (derived from BuiltinExceptionKind)
- All lookup functions (`from_name`, `from_tag`, `name()`, `tag()`, etc.)
- Handler dispatch in lowering and codegen

**No changes needed in:**
- `crates/types/src/exceptions.rs` — re-exports from core-defs
- `crates/runtime/src/exceptions.rs` — uses BuiltinExceptionKind from core-defs
- `crates/types/src/lib.rs` — uses `Type::BuiltinException(kind)`
- `crates/hir/src/lib.rs` — uses `Builtin::BuiltinException(kind)`
- `crates/lowering/src/exceptions.rs` — dispatches via `kind.tag()`
- `crates/frontend-python/src/ast_to_hir/` — uses `BuiltinExceptionKind::from_name()`
- `crates/codegen-cranelift/src/` — uses `kind.tag()`

#### Adding New Runtime Type Tags

To add a new runtime type (e.g., `Regex`), modify only **1 file**:

**Single source of truth** (`crates/core-defs/src/type_tags.rs`):
```rust
define_type_tags! {
    // ... existing types ...
    CompletedProcess = 18 => "CompletedProcess" => "<class 'subprocess.CompletedProcess'>" => "subprocess.CompletedProcess",
    Regex = 19 => "Regex" => "<class 're.Pattern'>" => "re.Pattern",  // ADD: new type
}
```

Each entry defines:
- **Variant name** (`Regex`) — the enum variant
- **Tag number** (`21`) — unique u8 value for GC allocation
- **Debug name** (`"Regex"`) — internal/debug identifier
- **Type class** (`"<class 're.Pattern'>"`) — Python `type()` output
- **Type name** (`"re.Pattern"`) — short name for error messages and repr

**What the macro generates automatically:**
- `TypeTagKind::Regex` — shared enum variant used by both compiler and runtime
- `tag.tag()` → `19u8` — numeric value for `gc_alloc()`
- `tag.name()` → `"Regex"` — debug/internal name
- `tag.type_class()` → `"<class 're.Pattern'>"` — Python `type()` result
- `tag.type_name()` → `"re.Pattern"` — for error messages and repr output
- All lookup functions (`from_tag`, `from_name`)

**Runtime uses `TypeTagKind` directly** (no duplicate enum):
- `ObjHeader.type_tag: TypeTagKind` — object header uses core-defs type
- `rt_type_name()` calls `type_class()` — single source for Python type() output
- Error messages use `type_name()` — single source for type names
- `gc_alloc(size, TypeTagKind::Regex.tag())` — allocation with type tag

**No changes needed in:**
- Runtime files — automatically pick up new `TypeTagKind` variant
- `rt_type_name()` — uses `type_class()` method (no hardcoded strings)
- Print/repr functions — use `type_name()` method where applicable

### Abstract Methods (`@abstractmethod`)

Abstract methods prevent instantiation of classes that don't implement all required methods:

```python
from abc import abstractmethod

class Shape:
    @abstractmethod
    def area(self) -> int:
        pass

class Circle(Shape):
    def area(self) -> int:
        return 314  # Implements abstract method

# shape = Shape()  # Compile error: Cannot instantiate abstract class
circle = Circle()  # OK: all abstract methods implemented
```

**Implementation**:
- **HIR** (`hir/lib.rs`):
  - `Function.is_abstract: bool` - tracks if method is abstract
  - `ClassDef.abstract_methods: IndexSet<InternedString>` - unimplemented abstract methods
- **Frontend** (`frontend-python/src/ast_to_hir/classes.rs`):
  - Detects `@abstractmethod` decorator on methods
  - Propagates abstract methods from parent classes
  - Removes from set when method is overridden by subclass
- **Semantic Analysis** (`semantics/lib.rs`):
  - Checks `ClassRef` calls for non-empty `abstract_methods`
  - Emits error: `"Cannot instantiate abstract class 'X' with unimplemented methods: [m1, m2]"`

**Validation rules**:
- Abstract methods can have `pass` bodies (default return values generated)
- Subclasses inherit parent's abstract methods
- Overriding a method removes it from the abstract set
- Partial implementations remain abstract (cannot be instantiated)

### User Decorators

User-defined decorators support two patterns:

**Identity decorators** return the original function unchanged:
```python
def my_decorator(func):
    return func  # Returns the same function

@my_decorator
def greet(name: str) -> str:
    return "Hello " + name
```

**Wrapper decorators** return a closure that wraps the original function:
```python
def double_result(func):
    def wrapper(x: int) -> int:
        return func(x) * 2
    return wrapper

@double_result
def get_value(n: int) -> int:
    return n + 5

result = get_value(10)  # Returns (10 + 5) * 2 = 30
```

**Implementation**:
- **Lowering context** (`lowering/src/context.rs`):
  - `var_to_wrapper: IndexMap<VarId, (FuncId, FuncId)>` - maps decorated function variable to (wrapper_func_id, original_func_id)
  - `wrapper_func_ids: IndexSet<FuncId>` - tracks which functions are wrappers (closures returned by decorators)
  - `func_ptr_params: IndexSet<VarId>` - tracks function pointer parameters in wrapper functions
  - `find_returned_closure()` - detects if a decorator returns a closure
- **Assignment lowering** (`lowering/src/statements/assign.rs`):
  - Detects wrapper decorators by checking if decorator returns a closure
  - Registers mapping in `var_to_wrapper` for call resolution
- **Call lowering** (`lowering/src/expressions/calls.rs`):
  - `lower_wrapper_call()` - passes original function address as first argument to wrapper
  - `lower_indirect_call()` - handles calls through function pointers inside wrapper functions
- **MIR** (`mir/lib.rs`):
  - `InstructionKind::FuncAddr { dest, func }` - gets function address for passing to wrapper
  - `InstructionKind::Call { dest, func, args }` - indirect call through function pointer operand
- **Codegen** (`codegen-cranelift/src/instructions.rs`):
  - `FuncAddr` emits `func_addr` instruction to get function pointer
  - `Call` with operand uses `call_indirect` for indirect function calls
- **Type inference** (`lowering/src/type_planning/infer.rs`):
  - Checks `var_to_wrapper` to return wrapper's return type for decorated function calls

**Chained wrapper decorators** are supported:
```python
def add_one(func):
    def wrapper() -> int:
        return func() + 1
    return wrapper

def triple(func):
    def wrapper() -> int:
        return func() * 3
    return wrapper

@triple
@add_one
def base() -> int:
    return 5

result = base()  # Returns (5 + 1) * 3 = 18
```

**Chained decorator implementation**:
- At compile time, detects chains via nested `Call` expressions in decorator pattern
- Uses `chain_contains_wrapper_decorator()` to distinguish from chained identity decorators
- Closure tuples use nested format `(func_ptr, (cap0, cap1, ...))` — outer tuple always has 2 elements
- `lower_indirect_call()` uses runtime type checking to handle both raw function pointers (single decorators) and closure tuples (chained decorators)
- `emit_closure_call()` helper handles extraction and dispatch for 0-8 captures
- Single wrapper decorators use efficient static shortcut; chained wrappers evaluate full chain at runtime

**Decorator factories** (`@decorator(arg)`) are supported:
```python
def multiply(factor: int):
    def decorator(func):
        def wrapper(x: int) -> int:
            return func(x) * factor
        return wrapper
    return decorator

@multiply(3)
def get_value(x: int) -> int:
    return x + 5

result = get_value(10)  # Returns (10 + 5) * 3 = 45
```

**Decorator factory implementation**:
- Detected via `Call { func: Call(...), args: [FuncRef] }` pattern in HIR (outer call applies decorator, inner call is factory)
- Marked as globals for runtime evaluation in `scan_module_decorated_functions()`
- Closures use nested tuple format `(func_ptr, (cap0, cap1, ...))` to capture factory arguments
- `emit_closure_call()` extracts func_ptr and captures tuple, dispatches based on capture count (0-8 supported)

**`*args` wrapper support**:
- Wrapper functions can use `*args` to forward arguments to the decorated function
- Caller packs args into a varargs tuple via `resolve_call_args` + `build_varargs_tuple`
- Inside the wrapper, `func(*args)` uses `rt_call_with_tuple_args` runtime trampoline
- Supports up to 8 arguments; handles both raw function pointers and closure tuples
- Type inference uses the original function's return type (not the wrapper's `Any`)

**Limitations**:
- `**kwargs` in wrapper signatures not supported

### Class Definition

- **HIR**: `ClassDef { id, name, fields, methods, init_method, span }`
  - `ExprKind::Attribute { obj, attr }`, `ExprKind::ClassRef(ClassId)`
  - `StmtKind::FieldAssign { obj, field, value }`
- **Frontend**: Method name mangling `ClassName$methodname` (e.g., `Point$__init__`)
- **Runtime**:
  - `TypeTag::Instance = 8`
  - `InstanceObj`: header + vtable_ptr + class_id + field_count + fields array (i64 per field)
  - `rt_make_instance`, `rt_instance_get_field`, `rt_instance_set_field`
- **Lowering**: `LoweredClassInfo` tracks field_offsets, field_types, method_funcs, vtable_slots
  - Instantiation: `MakeInstance` + `CallDirect` to `__init__`
  - Method lookup via `interner.lookup()` to extract original name from mangled

### Virtual Method Dispatch (Vtables)

Method calls on class instances use virtual dispatch for polymorphism:

```python
class Animal:
    def speak(self) -> str:
        return "..."

class Dog(Animal):
    def speak(self) -> str:
        return "Woof!"

dog = Dog()
dog.speak()  # Dispatches to Dog.speak via vtable
```

**Implementation**:
- **Vtable structure** (`runtime/src/vtable.rs`):
  - `VTABLE_REGISTRY`: Global array of vtable pointers indexed by class_id
  - Vtable layout: `[num_slots: u64, method_ptrs: [*const (); num_slots]]`
  - `rt_register_vtable(class_id, vtable_ptr)` - register vtable at startup
  - `rt_get_vtable(class_id)` - lookup vtable for instance creation
- **Instance layout** (`runtime/src/object.rs`):
  - `InstanceObj` includes `vtable: *const u8` pointer after header
  - `rt_make_instance()` sets vtable from `VTABLE_REGISTRY`
- **MIR** (`mir/lib.rs`):
  - `InstructionKind::CallVirtual { dest, obj, slot, args }` - virtual call via vtable
  - `VtableInfo { class_id, entries }` - vtable metadata for codegen
  - `VtableEntry { slot, method_func_id }` - method slot assignment
- **Lowering** (`lowering/src/expressions/access.rs`):
  - `lower_class_method_call()` emits `CallVirtual` using `vtable_slots`
  - `vtable_slots: IndexMap<InternedString, usize>` computed per class
  - `super()` calls still use `CallDirect` (static dispatch to parent)
- **Codegen** (`codegen-cranelift/src/lib.rs`):
  - `create_vtable_data_sections()` - emit data sections with function pointers
  - `generate_vtable_registration()` - emit `__pyaot_init_vtables__` function
  - Main entry point calls `__pyaot_init_vtables__` before `__pyaot_module_init__`
- **Codegen** (`codegen-cranelift/src/instructions.rs`):
  - `CallVirtual` loads vtable from instance (offset 16 after ObjHeader)
  - Loads method pointer from vtable at `8 + slot * 8`
  - Indirect call with self + args

**Vtable slot computation** (`lowering/src/context.rs`):
- Methods inherit parent's vtable slots
- New methods get next available slot
- Overridden methods keep parent's slot number

**Memory overhead**: +8 bytes per instance (one vtable pointer)

### Dunder Methods

Dunder (double-underscore) methods enable custom behavior for built-in operations:

**Tracking (`lowering/src/context.rs`)**:
- `LoweredClassInfo` stores dunder method FuncIds for:
  - String: `__str__`, `__repr__`
  - Comparison: `__eq__`, `__ne__`, `__lt__`, `__le__`, `__gt__`, `__ge__`
  - Hash/len: `__hash__`, `__len__`
  - Arithmetic: `__add__`, `__sub__`, `__mul__`, `__truediv__`, `__floordiv__`, `__mod__`, `__pow__`
  - Reverse arithmetic: `__radd__`, `__rsub__`, `__rmul__`, `__rtruediv__`, `__rfloordiv__`, `__rmod__`, `__rpow__`
  - Unary: `__neg__`, `__pos__`, `__abs__`, `__invert__`, `__bool__`
  - Conversion: `__int__`, `__float__`
  - Container: `__getitem__`, `__setitem__`, `__delitem__`, `__contains__`
  - Iterator: `__iter__`, `__next__`
  - Callable: `__call__`
- Dunder methods are detected during class metadata building (`lowering/src/class_metadata.rs`)
- Inherited from parent classes if not overridden

**Implementation**:
1. **`__str__` and `__repr__`** (`lowering/src/expressions/builtins/conversions.rs` and `introspection.rs`):
   - `str(obj)`: Calls `__str__` if defined, falls back to `__repr__`, then default repr
   - `repr(obj)`: Calls `__repr__` if defined, falls back to default repr
   - Default repr: `rt_obj_default_repr()` returns `"<instance at 0x...>"`
   - **MIR**: `RuntimeFunc::ObjDefaultRepr`

2. **`__eq__`** (`lowering/src/expressions/operators.rs`):
   - `obj1 == obj2`: Calls `left.__eq__(right)` if left has `__eq__`
   - `obj1 != obj2`: Negates result of `__eq__` call
   - Fallback to pointer comparison (identity) if no `__eq__`
   - **Parameter Type Inference** (`frontend-python/src/ast_to_hir/classes.rs`):
     - Dunder methods (`__eq__`, `__ne__`, `__lt__`, `__le__`, `__gt__`, `__ge__`, arithmetic ops) automatically infer second parameter type to match class type
     - Allows field access on untyped `other` parameter: `def __eq__(self, other) -> bool: return self.x == other.x`
     - Explicit type annotations take precedence over automatic inference
   - **MIR**: Direct `CallDirect` to `__eq__` method

3. **`__hash__`** (`lowering/src/expressions/builtins/introspection.rs`):
   - `hash(obj)`: Calls `__hash__` method if defined
   - Raises `TypeError` if class has no `__hash__` (instances unhashable by default)
   - **MIR**: Direct `CallDirect` to `__hash__` method
   - **Runtime**: No special runtime function needed

4. **`__len__`** (`lowering/src/expressions/builtins/collections.rs`):
   - `len(obj)`: Calls `__len__` method if defined
   - Raises `TypeError` if class has no `__len__`
   - **MIR**: Direct `CallDirect` to `__len__` method
   - **Runtime**: No special runtime function needed

**Example**:
```python
class Point:
    x: int
    y: int

    def __str__(self) -> str:
        return f"Point({self.x}, {self.y})"

    def __hash__(self) -> int:
        return self.x * 31 + self.y

    def __len__(self) -> int:
        return 2  # Always 2 coordinates

p = Point(3, 4)
str(p)   # "Point(3, 4)" via __str__
hash(p)  # 97 via __hash__
len(p)   # 2 via __len__
```

5. **Container dunders** (`lowering/src/expressions/access/indexing.rs`, `lowering/src/statements/assign.rs`, `lowering/src/expressions/operators.rs`):
   - `obj[key]`: Calls `__getitem__(self, key)` if defined
   - `obj[key] = value`: Calls `__setitem__(self, key, value)` if defined
   - `del obj[key]`: Calls `__delitem__(self, key)` if defined
   - `item in obj` / `item not in obj`: Calls `__contains__(self, item)` if defined
   - Return type of `__getitem__` is inferred from the method's return type annotation
   - **MIR**: Direct `CallDirect` to respective dunder methods

**Example**:
```python
class IntList:
    items: list[int]
    size: int

    def __init__(self, items: list[int]) -> None:
        self.items = items
        self.size = len(items)

    def __getitem__(self, index: int) -> int:
        return self.items[index]

    def __setitem__(self, index: int, value: int) -> None:
        self.items[index] = value

    def __contains__(self, value: int) -> bool:
        i: int = 0
        while i < self.size:
            if self.items[i] == value:
                return True
            i = i + 1
        return False

c = IntList([10, 20, 30])
c[0]         # 10 via __getitem__
c[1] = 99    # via __setitem__
20 in c      # True via __contains__
```

**Inheritance**: Dunder methods are inherited like regular methods - child classes can override parent's dunder methods.

6. **Iterator protocol** (`lowering/src/statements/loops/class_iterator.rs`, `lowering/src/expressions/builtins/iteration.rs`):
   - `for x in obj`: Calls `__iter__(self)` then loops calling `__next__(self)` with try/except StopIteration
   - `iter(obj)`: Calls `__iter__(self)` via `CallDirect`
   - `next(obj)`: Calls `__next__(self)` via `CallDirect`
   - Element type inferred from `__next__` return type annotation
   - **MIR**: Uses `ExcPushFrame`/`TrySetjmp`/`ExcCheckClass(StopIteration)` for loop termination
   - Supports `for...else`, `break`, `continue`, and inheritance

**Example**:
```python
class CountUp:
    current: int
    stop: int
    def __init__(self, stop: int) -> None:
        self.current = 0
        self.stop = stop
    def __iter__(self) -> CountUp:
        return self
    def __next__(self) -> int:
        if self.current >= self.stop:
            raise StopIteration()
        val: int = self.current
        self.current = self.current + 1
        return val

for x in CountUp(5):  # 0, 1, 2, 3, 4
    print(x)
```

### isinstance()

- **Primitives** (int, float, bool, None): resolved at compile-time
- **Heap types** (str, list, tuple, dict): runtime `GetTypeTag` comparison
- **Classes**: runtime `IsinstanceClass` with class_id
- **Cross-type safety**: primitives vs classes → `false` at compile-time
- **Implementation**: `lower_isinstance()` in `builtins.rs`

### hash()

- `hash(int)` → `rt_hash_int()` (golden ratio scramble)
- `hash(str)` → `rt_hash_str()` (FNV-1a)
- `hash(bool)` → `rt_hash_bool()` (0 for False, 1 for True)
- `hash(None)` → constant 0
- `hash(tuple)` → `rt_hash_tuple()` (Python's tuple hash algorithm: `hash * 1000003 ^ element_hash`)
- **MIR**: `HashInt`, `HashStr`, `HashBool`, `HashTuple`
- **Unhashable types**: `hash(list)`, `hash(dict)`, `hash(set)` raise `TypeError`

### id()

- `id(int)` → value itself
- `id(bool)` → 0/1 via `BoolToInt`
- `id(float)` → bit pattern via `FloatBits` instruction
- `id(None)` → constant 0
- `id(heap types)` → `rt_id_obj()` returns pointer as i64
- **MIR**: `FloatBits`, `IdObj`

### iter(), next(), reversed()

- **Type**: `Type::Iterator(Box<Type>)` with element type
- **Runtime** (`object.rs`):
  - `TypeTag::Iterator = 9`
  - `IteratorKind`: List, Tuple, Dict, String, Range, Set, Bytes, Enumerate, Map, Filter, Zip, Zip3, ZipN
  - `IteratorObj`: header, kind, exhausted, reversed, source, index, range_stop, range_step
  - `MapIterObj`: header, kind, exhausted, capture_count, func_ptr, inner_iter, captures
  - `FilterIterObj`: header, kind, exhausted, elem_tag, capture_count, func_ptr, inner_iter, captures
  - `ZipIterObj`: header, kind, exhausted, iter1, iter2
  - `Zip3IterObj`: header, kind, exhausted, iter1, iter2, iter3
  - `ZipNIterObj`: header, kind, exhausted, iters (list), count
- **Runtime functions**:
  - Forward: `rt_iter_list`, `rt_iter_tuple`, `rt_iter_dict`, `rt_iter_str`, `rt_iter_range`
  - Reversed: `rt_iter_reversed_*` variants
  - `rt_iter_next()` returns next element, raises `StopIteration`
- **MIR**: `MakeIterator { source: IterSourceKind, direction: IterDirection }` (unified), `IterNext`, `IterEnumerate`, `MapNew`, `FilterNew`
- **GC**: Iterators are roots; `mark_object()` traces source pointer
- **Lowering**: `lower_iter()`, `lower_iter_range()`, `lower_next()`, `lower_reversed()`, `lower_reversed_range()`

### enumerate()

- **Type**: `enumerate(iterable, start=0)` → `Iterator[Tuple[Int, elem_type]]`
- **HIR**: `Builtin::Enumerate` variant; `StmtKind::ForUnpack` for tuple unpacking in for-loops
- **Optimized path**: `for i, v in enumerate(items)` uses indexed iteration with counter (zero-allocation)
- **Standalone path**: `enumerate(iterable)` creates `IteratorKind::Enumerate` wrapping an inner iterator
- **Runtime**: `rt_iter_enumerate(inner_iter, start)` creates enumerate iterator; `rt_iter_next` handles `Enumerate` kind by calling inner iterator's next, boxing counter, and returning `(counter, elem)` tuple
- **For-loop tuple unpacking**: `for a, b in items` supported via `ForUnpack` HIR node, lowered to indexed loop with `TupleGet` unpacking

### sorted()

- **Return Type**: `Type::List(Box<element_type>)` - always returns a list
- **Parameters**: `sorted(iterable, key=None, reverse=False)`
  - `key` kwarg: function to extract comparison key from each element (user functions, lambdas, or builtins like `abs`)
  - `reverse` kwarg: `True` for descending, `False` (default) for ascending
- **Runtime functions**:
  - Without key: `rt_sorted_list`, `rt_sorted_tuple`, `rt_sorted_dict`, `rt_sorted_str`, `rt_sorted_range`, `rt_sorted_set`
  - With key: `rt_sorted_list_with_key(list, reverse, key_fn, elem_tag)`, `rt_sorted_tuple_with_key(tuple, reverse, key_fn, elem_tag)`
- **MIR**: `SortedList`, `SortedTuple`, `SortedDict`, `SortedStr`, `SortedRange`, `SortedSet`, `SortedListWithKey`, etc.
- **Sorting algorithm**: Insertion sort (stable, O(n²) - suitable for small collections)
- **Element comparison**: `compare_list_elements()` handles:
  - Raw i64 values (for list[int] - elements stored as bit-cast pointers)
  - Boxed objects (strings, floats) - compares by type tag then value
  - Uses heuristic to detect heap objects vs raw integers
- **Lowering**: `lower_sorted()`, `lower_sorted_range()` in `builtins/iteration.rs`
- **Key function**: Uses `FuncAddr` or `BuiltinAddr` MIR instruction to get function pointer, passed to runtime
- **Element boxing for key functions**:
  - `elem_tag` parameter tells runtime how to handle raw elements in list[int]/list[bool]
  - Builtin key functions (e.g., `key=abs`) need boxed `*mut Obj` arguments → `elem_tag=1` triggers boxing
  - User-defined key functions work with raw values directly → `elem_tag=0`

### map() and filter()

- **Type**: `map(func, iterable)` → `Iterator[result_type]`, `filter(func, iterable)` → `Iterator[elem_type]`
- **HIR**: `Builtin::Map`, `Builtin::Filter` variants
- **MIR**: `RuntimeFunc::MapNew`, `RuntimeFunc::FilterNew`
- **Runtime** (`iterator.rs`, `object.rs`):
  - `IteratorKind::Map = 9`, `IteratorKind::Filter = 10`
  - `MapIterObj { header, kind, exhausted, capture_count, func_ptr, inner_iter, captures }` - stores function pointer, inner iterator, and captures tuple
  - `FilterIterObj { header, kind, exhausted, elem_tag, capture_count, func_ptr, inner_iter, captures }` - stores predicate pointer, inner iterator, and captures tuple
  - `rt_map_new(func_ptr, iter, captures, capture_count)` - create map iterator
  - `rt_filter_new(func_ptr, iter, elem_tag, captures, capture_count)` - create filter iterator
- **Function pointer passing**:
  - Uses `extract_func_id()` to get FuncId from function reference
  - Uses `extract_func_with_captures()` to get FuncId and capture expressions for closures
  - `FuncAddr` MIR instruction gets function address at runtime
  - Function types: `MapFn = extern "C" fn(*mut Obj) -> *mut Obj`, `FilterFn = extern "C" fn(*mut Obj) -> i8`
  - With captures: `MapFn1..4`, `FilterFn1..4` with 1-4 capture parameters prepended
- **Closure support (captures)**:
  - Lambdas with captured variables work: `map(lambda x: x + offset, items)`
  - Captures stored in tuple during lowering via `lower_captures_to_tuple()`
  - Runtime extracts captures with `rt_tuple_get()` and passes to function
  - Supports up to 4 captures (covers most practical cases)
  - Non-capturing functions use fast path (capture_count=0, captures=null)
  - GC properly marks captures tuple in `mark_object()` for Map/Filter iterator kinds
- **Iteration**:
  - Map: calls inner iterator's next(), applies function with captures, returns result
  - Filter: loops calling inner iterator's next() until predicate (with captures) returns true or exhausted
- **Edge case handling**:
  - `-1` as raw i64 has same bit pattern as `EXHAUSTED_SENTINEL` (usize::MAX)
  - Fixed by checking iterator's `exhausted` flag instead of comparing return value to sentinel
  - `rt_iter_next_no_exc()` also updated to use flag-based exhaustion detection
- **Lowering** (`builtins/iteration.rs`): `lower_map()`, `lower_filter()`, `lower_captures_to_tuple()`
- **Codegen** (`runtime_calls/iterator.rs`): `MapNew` (quaternary), `FilterNew` (quinary) variants
- **filter(None, iterable)**: Supported for truthiness filtering (filters out falsy values like 0, False, empty strings, empty lists, None)
  - Uses `elem_tag` to handle raw int values vs boxed values correctly
  - `rt_is_truthy()` runtime function for heap object truthiness checking
- **Limitations**:
  - Only single-iterable map (no `map(func, iter1, iter2, ...)`)
  - Maximum 4 captures per closure (panics at runtime if exceeded)

### First-Class Builtin Functions

Builtin functions (`len`, `str`, `int`, `abs`, etc.) can be passed as first-class values to higher-order functions:

```python
# Pass abs to sorted
result = sorted([-3, 1, -2], key=abs)    # [1, -2, -3]

# Pass len to map
lengths = list(map(len, ["a", "bb", "ccc"]))  # [1, 2, 3]

# Pass str to map
strings = list(map(str, [1, 2, 3]))  # ["1", "2", "3"]

# Pass abs to min/max
result = min([-5, 2, -3], key=abs)  # 2 (smallest absolute value)
```

**Supported builtins** (defined in `core-defs/src/builtins.rs`):
- `len`, `str`, `int`, `float`, `bool`
- `abs`, `hash`, `ord`, `chr`
- `repr`, `type`

**Implementation**:
- **Definitions** (`core-defs/src/builtins.rs`):
  - `BuiltinFunctionKind` enum with compile-time IDs
  - `BUILTIN_FUNCTION_COUNT` for validation
- **Runtime** (`runtime/src/builtins.rs`):
  - Wrapper functions: `rt_builtin_len()`, `rt_builtin_abs()`, etc.
  - All wrappers take `*mut Obj` and return `*mut Obj` for uniform handling
  - `rt_get_builtin_func_ptr(id)` returns function pointer by ID
- **MIR** (`mir/lib.rs`):
  - `InstructionKind::BuiltinAddr { dest, builtin_id }` - get builtin function address
- **Lowering** (`expressions/builtins/mod.rs`):
  - Detects builtin name references in function position
  - Emits `BuiltinAddr` instruction for function pointer
- **Codegen** (`codegen-cranelift/src/instructions.rs`):
  - `BuiltinAddr` calls `rt_get_builtin_func_ptr` at runtime

**Element boxing for key functions on list[int]/list[bool]**:
- Lists store `int`/`bool` as raw values (`ELEM_RAW_INT`, `ELEM_RAW_BOOL`)
- Builtin wrappers expect boxed `*mut Obj` arguments
- Runtime functions accept `elem_tag` parameter to box raw elements before calling key function:
  - `sorted(list, key=abs, elem_tag=1)` - boxes raw ints
  - `list.sort(key=abs, elem_tag=1)` - boxes raw ints
  - `min(list, key=abs, elem_tag=1)`, `max(list, key=abs, elem_tag=1)`
- User-defined key functions work directly with raw values (no boxing needed)
- For sets (always boxed elements): `needs_unbox` parameter for user functions that expect raw values

**Files**:
- `crates/core-defs/src/builtins.rs` - `BuiltinFunctionKind` enum
- `crates/runtime/src/builtins.rs` - Wrapper functions and pointer table
- `crates/mir/src/lib.rs` - `BuiltinAddr` instruction
- `crates/lowering/src/expressions/builtins/mod.rs` - Builtin detection
- `crates/lowering/src/context/helpers.rs` - `elem_tag_for_key_func()` helper
- `crates/codegen-cranelift/src/instructions.rs` - Codegen for `BuiltinAddr`

### Collection Constructors: list(), tuple(), dict()

- **Type**: `list(iterable)` → `list[T]`, `tuple(iterable)` → `tuple[T, ...]`, `dict()` / `dict(**kwargs)` → `dict[K, V]`
- **Type Inference** (`lowering/src/type_planning/`): Element types are inferred from arguments (see "Type Inference for Container Constructors")
- **HIR**: `Builtin::List`, `Builtin::Tuple`, `Builtin::Dict` variants
- **MIR**: RuntimeFunc variants for conversions:
  - List: `ListFromTuple`, `ListFromStr`, `ListFromRange`, `ListFromIter`, `ListFromSet`, `ListFromDict`
  - Tuple: `TupleFromList`, `TupleFromStr`, `TupleFromRange`, `TupleFromIter`, `TupleFromSet`, `TupleFromDict`
  - Dict: `DictFromPairs` (for `dict(iterable_of_pairs)`)
- **Runtime functions**:
  - `rt_list_from_tuple(tuple)` - convert tuple to list
  - `rt_list_from_str(str)` - convert string to list of single-char strings
  - `rt_list_from_range(start, stop, step)` - convert range to list
  - `rt_tuple_from_list(list)` - convert list to tuple
  - `rt_tuple_from_str(str)` - convert string to tuple of single-char strings
  - `rt_tuple_from_range(start, stop, step)` - convert range to tuple
  - `rt_dict_from_pairs(list)` - create dict from list of 2-tuples
- **Supported conversions**:
  - `list()` - empty list
  - `list([1, 2, 3])` - copy list
  - `list((1, 2, 3))` - tuple to list
  - `list("abc")` - string to list of chars `['a', 'b', 'c']`
  - `list(range(5))` - range to list `[0, 1, 2, 3, 4]`
  - `tuple()` - empty tuple
  - `tuple([1, 2, 3])` - list to tuple
  - `tuple("abc")` - string to tuple of chars
  - `tuple(range(3))` - range to tuple
  - `dict()` - empty dict
  - `dict(a=1, b=2)` - dict from keyword arguments
- **Lowering** (`builtins/collections.rs`): `lower_list_builtin()`, `lower_tuple_builtin()`, `lower_dict_builtin()`
- **Codegen** (`runtime_calls/list.rs`, `runtime_calls/tuple.rs`, `runtime_calls/dict.rs`)

### min() and max() with Iterables

- **Type**: `min(iterable)` → element type, `max(iterable)` → element type
- **Supported iterables**:
  - Lists: `min([3, 1, 4])` → `1`, `max([3, 1, 4])` → `4`
  - Tuples: `min((3, 1, 4))` → `1`, `max((3, 1, 4))` → `4`
  - Sets: `min({3, 1, 4})` → `1`, `max({3, 1, 4})` → `4`
  - Ranges: `min(range(5))` → `0`, `max(range(5))` → `4`
- **key= parameter support**:
  - `min(iterable, key=func)`: Apply key function to each element for comparison, return original element
  - `max(iterable, key=func)`: Apply key function to each element for comparison, return original element
  - **Supported types**: list, tuple, and set ✅
  - **Supported key functions**: user-defined functions, lambdas, and first-class builtins (`abs`, `len`, `str`, etc.)
  - Examples:
    - `min(["apple", "pie", "banana"], key=len)` → `"pie"` (shortest string)
    - `max([-5, 2, -3, 1], key=abs)` → `-5` (largest absolute value)
    - `max({-5, 2, -3, 1}, key=abs)` → `-5` (set with largest absolute value)
    - `sorted([-3, 1, -2], key=abs)` → `[1, -2, -3]` (sorted by absolute value)
  - Implementation: Runtime receives `elem_tag` to box raw elements (list[int]/list[bool]) for builtin key functions
  - Follows same pattern as `sorted(key=func)` - applies key for comparison only, returns original element
- **Range optimization**: min/max of range computed directly without iteration
  - `min(range(start, stop, step))`: returns `start` if step > 0, else last element
  - `max(range(start, stop, step))`: returns last element if step > 0, else `start`
  - Handles negative step correctly: `min(range(5, 0, -1))` → `1`, `max(range(5, 0, -1))` → `5`
- **MIR**: RuntimeFunc variants:
  - Basic: `TupleMinInt`, `TupleMaxInt`, `TupleMinFloat`, `TupleMaxFloat`
  - Basic: `SetMinInt`, `SetMaxInt`, `SetMinFloat`, `SetMaxFloat`
  - Basic: `ListMinInt`, `ListMaxInt`, `ListMinFloat`, `ListMaxFloat`
  - With key: `ListMinWithKey`, `ListMaxWithKey`, `TupleMinWithKey`, `TupleMaxWithKey`
- **Runtime**:
  - Basic: `rt_tuple_min_int(tuple)`, `rt_tuple_max_int(tuple)`, `rt_set_min_int(set)`, `rt_set_max_int(set)`
  - With key: `rt_list_min_with_key(list, key_fn, elem_tag)`, `rt_list_max_with_key(list, key_fn, elem_tag)`, `rt_tuple_min_with_key(tuple, key_fn, elem_tag)`, `rt_tuple_max_with_key(tuple, key_fn, elem_tag)`
  - Set with key: `rt_set_min_with_key(set, key_fn, needs_unbox)`, `rt_set_max_with_key(set, key_fn, needs_unbox)` - opposite semantics (unbox for user functions)
  - Key functions receive `*mut Obj` and return `*mut Obj` (uses `compare_list_elements` from `sorted.rs`)
- **Lowering** (`builtins/math.rs`):
  - `lower_minmax_builtin()`: Extracts `key=` kwarg, emits `FuncAddr` instruction for function pointer
  - For sets with `key=`: Runtime returns boxed object, lowering emits `UnboxInt`/`UnboxFloat` for primitives
  - `lower_minmax_range()`: Direct range min/max computation

### Lambda Expressions

Lambdas are desugared to named functions with closures:

- **Desugaring approach**: Lambda becomes named function `__lambda_N`
  - No captures: `FuncRef(func_id)` - simple function reference
  - With captures: `Closure { func, captures }` - function + captured values
- **Frontend** (`ast_to_hir.rs`):
  - `convert_lambda()` generates function with unique name
  - `find_free_variables()` detects variables from outer scope
  - Captured variables become implicit leading parameters
- **Type inference** (`context.rs`):
  - `infer_lambda_param_types()` infers from body + capture types
  - `infer_lambda_return_type()` infers from body expression
  - `precompute_closure_capture_types()` scans module before function lowering
- **Call handling** (`calls.rs`):
  - `var_to_func` maps variable → FuncId for simple lambdas
  - `var_to_closure` maps variable → (FuncId, captures) for closures
  - Closure calls prepend captured values to argument list
- **Closure tuple format**: Nested structure `(func_ptr, (cap0, cap1, ...))` — outer tuple always 2 elements
  - Inner tuple contains all captures (0 to 8 supported)
  - `emit_closure_call()` helper dispatches based on capture count
  - Uniform format simplifies indirect call handling
  - Note: In decorator factories, wrapper captures `func` + factory args, so max 7 factory args with 8 total captures

**Example transformation**:
```python
# Input
multiplier: int = 10
scale = lambda x: x * multiplier

# Desugared HIR
def __lambda_0(__capture_multiplier: int, x: int) -> int:
    return x * __capture_multiplier
scale = Closure(__lambda_0, [multiplier])

# At call site: scale(5) becomes __lambda_0(multiplier, 5)
```

**Limitations**:
- Read-only captures (values copied at lambda creation time)
- Default parameters supported (evaluated at definition time, Python semantics)
- No nested lambdas
- Types must be inferable from body expression

### Nested Functions

Nested functions (functions defined inside other functions) are implemented using the same closure infrastructure as lambdas:

- **Desugaring approach**: Nested function becomes named function `__nested_<name>_N`
  - No captures: `FuncRef(func_id)` - simple function reference
  - With captures: `Closure { func, captures }` - function + captured values
- **Frontend** (`statements.rs`):
  - Handles `py::Stmt::FunctionDef` inside `convert_stmt()`
  - `find_free_variables_in_body()` detects variables from outer scope in statement bodies
  - Captured variables become implicit leading parameters
  - Nested function name registered in outer scope's `var_map`
- **Recursive calls**:
  - Function temporarily added to `func_map` during body conversion
  - Removed after conversion if function has captures (so external calls go through closure)
  - Non-capturing functions remain in `func_map` for direct calls
- **Type inference**: Same as lambdas - capture types precomputed before lowering
- **Call handling**: Same as lambdas - closure calls prepend captured values

**Example transformation**:
```python
# Input
def outer():
    multiplier: int = 10
    def scale(x: int) -> int:
        return x * multiplier
    return scale(5)

# Desugared HIR
def __nested_scale_0(__capture_multiplier: int, x: int) -> int:
    return x * __capture_multiplier
def outer():
    multiplier = 10
    scale = Closure(__nested_scale_0, [multiplier])
    return scale(5)  # becomes __nested_scale_0(multiplier, 5)
```

**Supported features**:
- Basic nested functions with typed parameters
- Recursive nested functions
- Multiple nested functions in the same outer function
- Closures capturing variables from outer scope
- Nested functions with complex bodies (if/else, loops, etc.)
- Deeply nested functions (functions inside nested functions)
- Capture chaining (inner function captures from grandparent scope)

**Limitations**:
- Captures are read-only by default (values copied at function definition time)
- Use `nonlocal` keyword to modify variables from enclosing scope

### Nonlocal Statement

The `nonlocal` keyword allows nested functions to modify variables from enclosing scopes:

```python
def outer() -> int:
    count: int = 0
    def inner() -> None:
        nonlocal count
        count = count + 1
    inner()
    inner()
    return count  # Returns 2
```

**Implementation**:
- **Frontend** (`ast_to_hir/statements.rs`):
  - Handles `py::Stmt::Nonlocal` to track which variables need cell wrapping
  - `nonlocal_vars: HashSet<InternedString>` tracks declarations per function scope
  - Variables marked as nonlocal use cell-based storage instead of direct values
- **Runtime** (`runtime/src/cell.rs`):
  - `TypeTag::Cell = 12`
  - `CellObj { header, value_tag, value }` - mutable reference holder
  - `CellValueTag`: Int, Float, Bool, Ptr (for heap objects)
  - Type-specific API: `rt_make_cell_int/float/bool/ptr`, `rt_cell_get_*/set_*`
  - GC integration: `cell_get_ptr_for_gc()` for marking contained heap objects
- **MIR** (`mir/lib.rs`):
  - `ValueKind` enum: `Int`, `Float`, `Bool`, `Ptr` (unified type classification)
  - `MakeCell(ValueKind)` - create cell with initial value
  - `CellGet(ValueKind)` - read value from cell
  - `CellSet(ValueKind)` - write value to cell
- **Lowering**:
  - Nonlocal variables wrapped in cells at declaration point
  - Reads/writes go through cell get/set operations
  - Cells passed to nested functions as captured values
- **Codegen** (`codegen-cranelift/runtime_calls/cells.rs`):
  - Type-specific cell operations mapped to runtime functions

**Supported features**:
- Basic nonlocal: `nonlocal x` to modify `x` from outer scope
- Multiple variables: `nonlocal a, b, c`
- Deeply nested: inner functions can access grandparent scope variables
- All types: int, float, bool, str, list, dict, tuple, set, bytes, class instances

### Module Execution

- Top-level code → synthetic `__pyaot_module_init__` function
- `def main()` is a regular function (NOT auto-called)
- Generated C `main()`:
  1. `rt_init()` — initialize runtime
  2. `__pyaot_module_init__()` — if exists
  3. `rt_shutdown()` — cleanup
  4. return 0
- User `main()` mangled to `__pyuser_main` to avoid conflicts

### Typing Module Imports

The compiler supports `from typing import ...` for type annotations (compile-time only):

```python
from typing import List, Dict, Set, Tuple, Optional, Union

nums: List[int] = [1, 2, 3]
data: Dict[str, int] = {"a": 1}
items: Set[int] = {1, 2, 3}
point: Tuple[int, int] = (10, 20)
```

- **Supported imports**: `List`, `Dict`, `Set`, `Tuple`, `Optional`, `Union`
- **Equivalence**: `List[T]` ≡ `list[T]`, `Dict[K,V]` ≡ `dict[K,V]`, etc. (PEP 585 style)
- **Optional[T]**: Desugared to `Union[T, None]` (same as `T | None`)
- **Union[A, B, ...]**: Desugared to `Type::Union(vec![A, B, ...])`

**Implementation** (`frontend-python`):

- **`mod.rs`**: `AstToHir.typing_imports: HashSet<InternedString>` tracks imported names
- **`statements.rs`**: `ImportFrom` statement handler stores imported names, returns `Pass`
- **`types.rs`**: `convert_type_annotation()` checks `typing_imports` when processing subscripts
  - Recognizes `List[T]`, `Dict[K,V]`, `Set[T]`, `Tuple[...]`, `Optional[T]`, `Union[A,B]`
  - Both lowercase (`list[int]`) and typing-style (`List[int]`) work when imported

**Limitations**:
- Only `from typing import ...` supported (not `import typing`)
- Imports are compile-time only (no runtime `typing` module)
- Other imports produce error: "Unsupported import: from X import ..."

### Union Types

Union types (`T | U` or `Union[T, U]`) have partial support. Values assigned to Union-typed variables are stored as boxed pointers (`*mut Obj`):

**Supported**:
```python
# Assigning None to Optional types
maybe_none: int | None = None
maybe_value: int | None = 42

# Assigning values to multi-type unions
multi_none: int | str | None = None
multi_str: int | str | None = "test"

# Lists with Union element types
items: list[int | str] = [1, "two", 3]

# Dicts with Union value types
data: dict[str, int | str] = {"a": 1, "b": "two"}
```

**Implementation**:
- **Boxing**: Primitive values (int, bool, float, None) are boxed when assigned to Union-typed variables
  - `rt_box_int`, `rt_box_bool`, `rt_box_float`, `rt_box_none` runtime functions
  - Heap types (str, list, etc.) are already pointers - no boxing needed
- **Cranelift type**: Union maps to I64 (pointer type) in `type_to_cranelift()`
- **GC tracking**: Union types are treated as heap types in `is_heap_type()`
- **MIR**: `BoxNone` RuntimeFunc for boxing None values
- **Lowering**: `box_value_for_union()` helper boxes primitives during assignment

**Runtime type dispatch functions**:
- `rt_print_obj(obj: *mut Obj)`: Print any boxed object using TypeTag dispatch (including container repr)
- `rt_obj_eq(a: *mut Obj, b: *mut Obj) -> i8`: Compare two boxed objects for equality
- `rt_obj_to_str(obj: *mut Obj) -> *mut Obj`: Convert boxed object to string (including container repr)

**Container element tags** (`elem_tag` on ListObj/TupleObj):
- `ELEM_HEAP_OBJ (0)`: Elements are `*mut Obj` with valid headers (strings, boxed values, nested containers)
- `ELEM_RAW_INT (1)`: Elements are raw i64 values stored as pointers
- `ELEM_RAW_BOOL (2)`: Elements are raw i8 values cast to pointers
- Used by: container printing, GC mark phase, `rt_make_list`/`rt_make_tuple` signatures

**Files**:
- `crates/runtime/src/boxing.rs`: `rt_box_none()` function
- `crates/runtime/src/ops.rs`: `rt_print_obj()`, `rt_obj_eq()`, `rt_obj_lt()`, `rt_obj_lte()`, `rt_obj_gt()`, `rt_obj_gte()` functions
- `crates/runtime/src/conversions.rs`: `rt_obj_to_str()` function
- `crates/mir/src/lib.rs`: `RuntimeFunc::BoxNone`, `PrintObj`, `ObjEq`, `ObjLt`, `ObjLte`, `ObjGt`, `ObjGte`, `ObjToStr` variants
- `crates/codegen-cranelift/src/utils.rs`: Union → I64 mapping
- `crates/codegen-cranelift/src/runtime_calls.rs`: BoxNone, PrintObj, ObjEq, ObjToStr codegen
- `crates/lowering/src/lib.rs`: `box_value_for_union()` helper
- `crates/lowering/src/statements.rs`: Union boxing during assignment
- `crates/lowering/src/expressions/builtins.rs`: Union support in `lower_print()`, `lower_str()`
- `crates/lowering/src/expressions/operators.rs`: Union support in `lower_compare()`
- `crates/lowering/src/expressions/collections.rs`: Union element boxing
- `crates/lowering/src/utils.rs`: Union in `is_heap_type()`

**Supported operations**:
- `print(union_value)`: Runtime type dispatch to correct print function
- `union_a == union_b`: Runtime equality comparison
- `union_a < union_b` (also `>`, `<=`, `>=`): Runtime ordering comparison
  - Compatible types (int/int, float/float, str/str, bool/bool): value comparison
  - Mixed int/float: int promoted to float before comparison
  - Incompatible types (int vs str): raises `TypeError` at runtime
  - None: raises `TypeError` (None is not orderable)
- `str(union_value)`: Runtime conversion to string
- Reassignment: `val = None` after `val: int | None = 42`

**Not yet supported**:
- Unboxing values retrieved from Union-typed containers

### Union Type Narrowing

When `isinstance()` is used to check a Union-typed variable, the compiler narrows the type in the appropriate branch:

```python
x: int | str = get_value()
if isinstance(x, int):
    print(x + 1)      # x narrowed to int - uses IntAdd
else:
    print(x.upper())  # x narrowed to str - uses StrUpper
```

**Implementation**:
- **Type methods** (`types/lib.rs`):
  - `Type::narrow_to(&self, target)` - narrow Union to matching type when isinstance is true
  - `Type::narrow_excluding(&self, excluded)` - narrow Union excluding type when isinstance is false
  - `Type::types_match_for_isinstance()` - helper for type matching
- **Narrowing analysis** (`lowering/src/narrowing.rs`):
  - `TypeNarrowingInfo` - stores var_id, narrowed_type, original_type
  - `NarrowingAnalysis` - contains then_narrowings and else_narrowings
  - `analyze_condition_for_narrowing()` - entry point, analyzes if condition
  - `extract_narrowing_info()` - recursive pattern matching for:
    - `isinstance(x, T)` - direct isinstance call
    - `not isinstance(x, T)` - negated isinstance
    - `isinstance(x, A) and isinstance(y, B)` - conjunction
  - `apply_narrowings()` / `restore_types()` - save/restore var_types for branches
- **Integration** (`lowering/src/statements/control_flow.rs`):
  - `lower_if()` analyzes condition before lowering
  - Then-branch: applies then_narrowings to var_types
  - Else-branch: applies else_narrowings to var_types
  - Types restored after each branch
- **Variable reading** (`lowering/src/expressions/literals.rs`):
  - When reading a narrowed Union variable, emits unbox code if narrowed to primitive
  - `narrowed_union_vars: IndexMap<VarId, Type>` tracks variables needing unboxing
  - Unbox functions: `UnboxInt`, `UnboxFloat`, `UnboxBool`
- **Type inference benefits**:
  - Narrowed types flow through `get_expr_type()` automatically
  - Arithmetic uses specific ops (IntAdd vs generic dispatch)
  - Method calls dispatch directly (StrUpper vs runtime lookup)
  - Print uses specific functions (PrintInt vs PrintObj)

**Supported patterns**:
- `isinstance(var, TypeRef)` - built-in types (int, str, float, bool, list, dict, tuple, set)
- `isinstance(var, ClassRef)` - user-defined classes
- `not isinstance(...)` - negated checks (else-branch gets the checked type)
- `isinstance(...) and isinstance(...)` - conjunction (both apply in then-branch)
- `isinstance(...) or isinstance(...)` - disjunction (both exclusions apply in else-branch)
- `not (isinstance(...) and isinstance(...))` - De Morgan: both apply in else-branch
- `not (isinstance(...) or isinstance(...))` - De Morgan: both exclusions apply in then-branch

**Limitations**:
- Only narrows local variables (not expressions like `func()`)
- Only narrows Union types (non-union types don't benefit)

### Dead Code Detection for isinstance Checks

The compiler detects unreachable code in isinstance checks and emits warnings:

```python
x: int = 42
if isinstance(x, str):  # WARNING: isinstance check is always False
    pass  # This branch is unreachable

y: str = "hello"
if isinstance(y, str):  # WARNING: isinstance check is always True
    pass
else:
    pass  # This branch is unreachable
```

**Implementation**:
- **Warning infrastructure** (`diagnostics/src/lib.rs`):
  - `CompilerWarning` enum with `DeadCode` variant
  - `CompilerWarnings` collection for gathering warnings
  - `emit_all()` method for displaying warnings with source location
- **Dead branch detection** (`lowering/src/narrowing.rs`):
  - `DeadBranch` enum - `ThenBranch` or `ElseBranch`
  - `IsinstanceInfo` struct includes `dead_branch: Option<DeadBranch>`
  - `types_compatible_for_isinstance()` - checks type compatibility
  - For non-Union types: always-true or always-false detection
  - For Union types: detects when narrowing produces `Type::Never`
- **Warning emission** (`lowering/src/statements/control_flow.rs`):
  - `lower_if()` checks for dead branches before lowering
  - `emit_dead_code_warning()` adds warning with span and message
- **Pipeline integration** (`cli/src/pipeline.rs`, `mir_merger.rs`):
  - `lower_module()` returns `(mir::Module, CompilerWarnings)`
  - Warnings emitted to stderr before codegen

**Detected patterns**:
- `isinstance(x: int, str)` - then-branch dead (incompatible types)
- `isinstance(x: str, str)` - else-branch dead (redundant check)
- `isinstance(x: list[int], list)` - else-branch dead (already a list)
- Works with all primitive and container types

**Warning format** (using miette):
```
  ! isinstance check is always False: variable 'x' cannot be type 'str'
   ,-[file.py:8:8]
 7 |     x: int = 42
 8 |     if isinstance(x, str):
   :        ^^^^^^^^^^^^^^^^^^ unreachable code
   `----
```

### List Comprehensions

Comprehensions are desugared at AST-to-HIR conversion time to equivalent loop constructs:

```python
# Input
result = [x * 2 for x in range(5) if x > 1]

# Desugared to HIR equivalent of:
__comp_0 = []
for x in range(5):
    if x > 1:
        __comp_0.append(x * 2)
result = __comp_0
```

- **Frontend** (`ast_to_hir.rs`):
  - `desugar_list_comprehension()` - main entry point
  - `generate_list_comprehension_loop()` - recursive helper for nested generators
  - `desugar_dict_comprehension()` - same pattern for dicts (uses IndexAssign)
  - `next_comp_id` counter for unique temp var names (`__comp_0`, `__comp_1`, ...)
  - `pending_stmts` buffer for statements that need injection
- **Scope isolation**: Loop variables don't leak to outer scope
  - Save `var_map` before processing, restore after
  - Only the result temp var remains visible
- **Statement injection**: Since comprehensions are expressions but desugaring produces statements:
  - Pending statements collected in `pending_stmts` buffer
  - Injected at statement boundaries by `take_pending_stmts()`
  - Compound statements (if/while/for) save and restore pending stmts
- **Dict comprehensions**: Fully supported
  - Same desugaring approach as list comprehensions (uses IndexAssign)
  - Key boxing handled automatically for int/bool keys

### Set Type

Sets are hash-based collections using open addressing (like dicts), storing unique elements:

- **Type**: `Type::Set(Box<Type>)` - element type tracked
- **HIR**: `ExprKind::Set(Vec<ExprId>)` for set literals, `Builtin::Set` for constructor
- **Runtime** (`lib.rs`, `object.rs`):
  - `TypeTag::Set = 10`
  - `SetEntry`: hash + elem + occupied flag
  - `SetObj`: header + len + capacity + entries pointer
  - Hash table with open addressing (linear probing)
- **Runtime functions**:
  - `rt_make_set(capacity)` - create empty set
  - `rt_set_add(set, elem)` - add element (handles boxing internally)
  - `rt_set_contains(set, elem)` - membership test
  - `rt_set_remove(set, elem)` - remove, error if missing
  - `rt_set_discard(set, elem)` - remove if present (no error)
  - `rt_set_pop(set)` - remove and return arbitrary element
  - `rt_set_len(set)` - element count
  - `rt_set_clear(set)` - remove all elements
  - `rt_set_copy(set)` - shallow copy
  - `rt_set_to_list(set)` - convert to list for iteration
  - `rt_set_update(set, other)` - add all elements from another set
  - `rt_set_intersection_update(set, other)` - keep only elements found in both
  - `rt_set_difference_update(set, other)` - remove elements found in other
  - `rt_set_symmetric_difference_update(set, other)` - keep elements in either but not both
- **Element boxing**: Like dict keys, primitive types (int, bool) are boxed via `rt_box_int`/`rt_box_bool`
- **Iteration**: Converted to list (`SetToList`), then standard index-based loop with unboxing
- **Unboxing**: `rt_unbox_int`, `rt_unbox_bool` extract values during iteration
- **GC**: Sets are roots; `mark_object()` traces all occupied entries
- **MIR**: `MakeSet`, `SetAdd`, `SetContains`, `SetRemove`, `SetDiscard`, `SetPop`, `SetLen`, `SetClear`, `SetCopy`, `SetToList`, `SetUpdate`, `SetIntersectionUpdate`, `SetDifferenceUpdate`, `SetSymmetricDifferenceUpdate`

**Set comprehension desugaring**:
```python
# Input
result = {x * 2 for x in range(5) if x > 1}

# Desugared to HIR equivalent of:
__comp_0 = set()
for x in range(5):
    if x > 1:
        __comp_0.add(x * 2)
result = __comp_0
```

- **Frontend** (`ast_to_hir.rs`):
  - `desugar_set_comprehension()` - main entry point
  - `generate_set_comprehension_loop()` - recursive helper
  - Same scope isolation as list comprehensions

### F-string Features

F-strings support format specifiers and conversion flags:

#### Format Specs

Float format specifiers like `:.2f`, `:.3f` control decimal precision:

```python
value: float = 3.14159
s = f"{value:.2f}"  # "3.14"
```

- **Parsing** (`fstrings.rs`):
  - `apply_format_spec()` checks for format spec in FormattedValue
  - `extract_format_spec_string()` extracts spec from JoinedStr
  - `parse_float_format_spec()` parses `:.Nf` patterns
- **Implementation**: Format specs are converted to `round(value, precision)` calls
  - `{x:.2f}` → `str(round(x, 2))`
  - Only float precision specs supported (`.Nf`)
  - Width, alignment, other specs not yet implemented

#### Conversion Flags

Conversion flags `!r`, `!s`, and `!a` control how values are converted to strings:

```python
name: str = "hello"
f"{name!r}"  # "'hello'" (with quotes, using repr())
f"{name!s}"  # "hello" (without quotes, using str())
f"{name!a}"  # "'hello'" (like repr but escapes non-ASCII using \xNN, \uNNNN, \UNNNNNNNN)
f"{name}"    # "hello" (default str() conversion)
```

- **Parsing** (`fstrings.rs`):
  - `convert_formatted_value()` checks `fv.conversion` field from rustpython-parser
  - Uses `ConversionFlag::Repr`, `ConversionFlag::Str`, `ConversionFlag::Ascii`, `ConversionFlag::None`
- **Order of operations**:
  - Format spec is applied FIRST to the original value
  - Conversion flag is applied AFTER format spec
- **Implementation**:
  - `!r`: Wraps in `Builtin::Repr` call (adds quotes for strings)
  - `!s`: Wraps in `Builtin::Str` call (explicit str conversion)
  - `!a`: Wraps in `Builtin::Ascii` call (like repr but escapes non-ASCII characters)
  - No flag: Default `str()` conversion for non-strings

### List Comparison

Lists support equality (`==`, `!=`) and ordering (`<`, `<=`, `>`, `>=`) comparisons:

```python
a: list[int] = [1, 2, 3]
b: list[int] = [1, 2, 3]
assert a == b           # True
assert [1, 2, 3] < [1, 2, 4]  # True (lexicographic)
assert [1, 2] < [1, 2, 3]     # True (shorter prefix is less)
```

- **Equality runtime functions** (`list/compare.rs`):
  - `rt_list_eq_int(a, b)` - compare list[int] elements
  - `rt_list_eq_float(a, b)` - compare list[float] elements
  - `rt_list_eq_str(a, b)` - compare list[str] elements
  - `rt_list_eq_any(a, b)` - compare lists with Any/Union element types using TypeTag dispatch
- **Ordering runtime functions** (`list/compare.rs`):
  - `rt_list_lt(a, b)`, `rt_list_lte(a, b)`, `rt_list_gt(a, b)`, `rt_list_gte(a, b)`
  - Element-wise lexicographic comparison using `elem_tag` for type dispatch
  - Shorter list is "less" when all compared elements are equal
- **Implementation**:
  - First checks length equality (for eq) or iterates min_len (for ordering)
  - Empty lists (len=0) are always equal regardless of data pointer
  - Element-wise comparison based on element type
  - TypeTag-based dispatch for heterogeneous lists (List[Any])
- **MIR**: `CompareKind::ListInt/ListFloat/ListStr` for equality, `CompareKind::List` for ordering
- **Lowering** (`operators.rs`): `lower_compare()` detects list types and emits appropriate runtime call

### Identity Comparison (is/is not)

Identity operators check if two variables refer to the same object (object identity):

```python
a: list[int] = [1, 2, 3]
b: list[int] = [1, 2, 3]
c: list[int] = a

assert a is c      # True - same object
assert a is not b  # True - different objects
assert a == b      # True - same values
```

- **HIR** (`hir/lib.rs`): `CmpOp::Is`, `CmpOp::IsNot` enum variants
- **Frontend** (`operators.rs`): Converts `py::CmpOp::Is` and `py::CmpOp::IsNot` to HIR
- **Lowering** (`expressions/operators.rs`):
  - Handled at start of `lower_compare()` before type-specific comparisons
  - **Cross-type comparison**: If operand types differ, returns constant False for `is`, True for `is not`
  - **Same type comparison**: Uses `BinOp::Eq` or `BinOp::NotEq` for direct comparison
- **Semantics**:
  - **Heap types** (str, list, dict, tuple, etc.): Compares pointer values (object identity)
  - **Primitive types** (int, float, bool, None): Compares values directly
  - Note: Unlike CPython, this compiler doesn't intern small integers or strings

**Bug fix**: Added explicit `Type::None` handling in `runtime_selector.rs` for global variables, cells, and class attributes. None is represented as `i8` (same as bool), so it uses `GlobalSet(ValueKind::Bool)`/`GlobalGet(ValueKind::Bool)` instead of incorrectly defaulting to int.

### Global Statement

The `global` keyword allows functions to access and modify module-level variables:

```python
counter: int = 0

def increment() -> None:
    global counter
    counter = counter + 1

def get_count() -> int:
    global counter
    return counter

increment()
increment()
assert get_count() == 2
```

**Implementation**:
- **Frontend** (`ast_to_hir/statements.rs`):
  - Handles `py::Stmt::Global` to track which variables are global
  - `global_vars: HashSet<InternedString>` tracks declarations in current scope
  - Maps global variable names to module-level VarIds via `module_var_map`
  - `global_vars` saved/restored when entering/exiting function scopes
- **HIR** (`hir/lib.rs`):
  - `Module.globals: IndexSet<VarId>` tracks all global VarIds
- **Runtime** (`runtime/src/globals.rs`):
  - `GLOBALS: UnsafeCell<HashMap<u32, GlobalEntry>>` - typed global storage keyed by VarId
  - `GlobalEntry { tag: GlobalTag, value: i64 }` with tags: Int, Float, Bool, Ptr
  - Type-specific API: `rt_global_set_int/get_int`, `rt_global_set_float/get_float`, `rt_global_set_bool/get_bool`, `rt_global_set_ptr/get_ptr`
  - GC integration: `mark_global_pointers()` and `get_global_pointers()` for heap object tracking
  - Initialized/shutdown with runtime (`rt_init`/`rt_shutdown`)
- **MIR** (`mir/lib.rs`):
  - `ValueKind` enum: `Int`, `Float`, `Bool`, `Ptr` (unified type classification)
  - Parameterized RuntimeFunc variants: `GlobalSet(ValueKind)`, `GlobalGet(ValueKind)`
- **Lowering** (`lowering/runtime_selector.rs`):
  - `type_to_value_kind(ty)` centralized type → ValueKind mapping
  - `get_global_set_func(ty)` / `get_global_get_func(ty)` return `GlobalSet(kind)` / `GlobalGet(kind)`
  - Variable reads: `lower_var()` emits `GlobalGet(kind)` for global VarIds
  - Variable writes: assignment emits `GlobalSet(kind)` for global VarIds
- **Codegen** (`codegen-cranelift/runtime_calls/globals.rs`):
  - Dispatches by `ValueKind` to type-specific runtime functions:
    - `GlobalSet/Get(ValueKind::Int)` → `rt_global_set/get_int`: i64
    - `GlobalSet/Get(ValueKind::Float)` → `rt_global_set/get_float`: f64
    - `GlobalSet/Get(ValueKind::Bool)` → `rt_global_set/get_bool`: i8
    - `GlobalSet/Get(ValueKind::Ptr)` → `rt_global_set/get_ptr`: heap types
  - GC root tracking for `ValueKind::Ptr` via `update_gc_root_if_needed()`

### Generator Functions

Generator functions (functions containing `yield`) have full iteration support:

```python
def simple_gen():
    yield 1
    yield 2
    yield 3

g = simple_gen()  # Creates generator object
print(g)          # <generator object at 0x...>

# Using next()
val = next(g)     # Returns 1
val = next(g)     # Returns 2

# Using for loop
for x in simple_gen():
    print(x)      # Prints 1, 2, 3
```

**Implementation**:
- **HIR** (`hir/lib.rs`):
  - `Function.is_generator: bool` - tracks whether function contains yield
  - `ExprKind::Yield(Option<ExprId>)` - yield expression
- **Frontend** (`frontend-python/src/ast_to_hir/expressions.rs`):
  - `current_func_is_generator` flag set when yield is encountered
  - All `Function` structs include `is_generator` field
- **Runtime** (`runtime/src/generator.rs`, `runtime/src/object.rs`):
  - `TypeTag::Generator = 13`
  - `GeneratorObj { header, func_id, state, exhausted, num_locals, type_tags, locals }` - stores execution state
  - `rt_make_generator(func_id, num_locals)` - create generator object with type_tags array
  - `rt_generator_get_state/set_state` - state management
  - `rt_generator_get_local/set_local` - local variable storage (as i64)
  - `rt_generator_get_local_ptr/set_local_ptr` - pointer local storage
  - `rt_generator_set_local_type(gen, index, type_tag)` - set type tag for precise GC
  - `rt_generator_set_exhausted/is_exhausted` - exhaustion tracking
  - `rt_generator_next(gen)` - calls external dispatcher `__pyaot_generator_resume`
  - `finalize_generator(gen)` - frees type_tags array before deallocation
  - **GC integration**: Precise tracking using type_tags array
    - Type tags: `LOCAL_TYPE_RAW_INT` (0), `LOCAL_TYPE_RAW_FLOAT` (1), `LOCAL_TYPE_RAW_BOOL` (2), `LOCAL_TYPE_PTR` (3)
    - Default: `LOCAL_TYPE_RAW_INT` (safe - won't trace as pointer)
    - Only `LOCAL_TYPE_PTR` locals are traced during mark phase
    - Eliminates crash risk from treating large integers as heap pointers
- **MIR** (`mir/lib.rs`):
  - `MakeGenerator`, `GeneratorGetState`, `GeneratorSetState`
  - `GeneratorGetLocal`, `GeneratorSetLocal`, `GeneratorGetLocalPtr`, `GeneratorSetLocalPtr`
  - `GeneratorSetLocalType` - set type tag for precise GC tracking
  - `GeneratorSetExhausted`, `GeneratorIsExhausted`
- **Lowering** (`lowering/src/generators/`):
  - Directory module with focused submodules:
    - `mod.rs` - Types (`GeneratorVar`, `YieldInfo`, `WhileLoopGenerator`, `ForLoopGenerator`), public API
    - `vars.rs` - `collect_generator_vars()`, `collect_vars_from_stmt()` for variable collection
    - `creator.rs` - `create_generator_creator()`, `lower_iter_expr_for_creator()` for creator function
    - `resume.rs` - `create_generator_resume()` for generic state machine generation
    - `while_loop.rs` - `detect_while_loop_generator()`, `create_while_loop_generator_resume()`
    - `for_loop.rs` - `detect_for_loop_generator()`, `create_for_loop_generator_resume()`
    - `utils.rs` - Helper functions for expression lowering in generators (truthiness, unary ops)
  - `lower_generator_function()` - transforms generator to creator + resume functions
  - Generator creator: allocates generator object with func_id, returns it
  - Generator resume: complete state machine with state dispatch
  - State blocks: one per yield, each yields value and transitions to next state
  - Exhaustion: final state marks generator exhausted and returns sentinel (0)
- **Codegen** (`codegen-cranelift/src/lib.rs`):
  - `generate_generator_dispatcher()` - creates `__pyaot_generator_resume` function
  - Dispatcher uses if-else chain to route to correct resume function by func_id
  - Resume functions named `funcname$resume` with id = original_id + 10000
- **Codegen** (`codegen-cranelift/src/runtime_calls/generator.rs`):
  - Cranelift code generation for all generator runtime calls
  - Proper type handling (i32 for state, i64 for pointers/values)
- **For-loop support** (`lowering/src/statements/loops.rs`):
  - `lower_for_iterator()` - iterator-protocol based iteration for generators
  - Uses `MakeIterator { source: Generator, direction: Forward }`, `IterNext`, `GeneratorIsExhausted`
  - Separate path from indexed iteration used by lists/tuples
- **Generator methods** (`runtime/src/generator.rs`, `lowering/src/expressions/access.rs`):
  - `close()` - marks generator as exhausted via `rt_generator_close()`
  - `send(value)` - fully functional via `rt_generator_send()`, yield expressions return sent values
  - New exception types: `TypeError`, `RuntimeError`, `GeneratorExit`
- **Sent value support** (`lowering/src/generators.rs`):
  - `YieldInfo` struct tracks yield points with assignment targets (`x = yield val`)
  - State machine stores sent values in generator locals on resume
  - Variables set by yields are loaded from generator locals when referenced
- **While-loop generators** (`lowering/src/generators.rs`):
  - `WhileLoopGenerator` struct detects `while cond: yield val; update` pattern
  - Parameters are saved to generator locals in creator function
  - State machine: state 0 initializes, state 1 loops with update
  - All variables persisted across yields

- **`yield from` support** (`frontend-python/src/ast_to_hir/statements/mod.rs`, `lowering/src/generators/for_loop.rs`):
  - `yield from expr` is desugared to `for __v in expr: yield __v` in the frontend
  - Statement-level desugaring in `convert_stmt` produces clean `StmtKind::For` without trailing `Expr(None)`
  - Expression-level desugaring in `convert_expr` uses `pending_stmts` for `result = yield from gen()` cases
  - `detect_for_loop_generator` extended to accept trailing yield statements after the for-loop
  - `ForLoopGenerator.trailing_yields` stores post-for-loop yield expressions
  - Resume function uses multi-state dispatch: state 0=init, 1=iter loop, 2..N=trailing yields
  - Inner iterator uses `IterNextNoExc` (not `IterNext`) to avoid StopIteration in resume context
  - Trailing yield blocks set state to next trailing state (not mark exhausted) to preserve yielded value
  - Supports: `yield from generator()`, `yield from [list]`, `yield from iterable; yield val`
  - Not supported: `send()`/`throw()` forwarding to sub-generator (v1 limitation)

**Not supported**: `throw()` method - too complex to implement correctly due to control flow and struct layout interactions.

### itertools (chain, islice)

`itertools.chain()` and `itertools.islice()` are implemented as builtin iterators following the same composite pattern as `zip()`, `map()`, and `filter()`.

- **New IteratorKind variants**: `Chain = 11`, `ISlice = 12` in `runtime/src/object.rs`
- **ChainIterObj**: Stores a list of iterators (`iters`), tracks `current_idx` and `num_iters`. Advances through iterators sequentially.
- **ISliceIterObj**: Wraps an `inner_iter`, tracks `next_yield` position, `stop`, `step`, and `current` index. Skips elements to reach yielded positions.
- **Runtime** (`runtime/src/iterator/composite.rs`): `rt_chain_new()`, `rt_islice_new()`
- **Dispatch** (`runtime/src/iterator/next.rs`): `iter_next_chain()`, `iter_next_islice()`
- **Frontend** (`frontend-python/src/ast_to_hir/expressions.rs`): Intercepts both `import itertools; itertools.chain(...)` and `from itertools import chain; chain(...)` patterns
- **Lowering** (`lowering/src/expressions/builtins/iteration.rs`): `lower_chain()` creates list of iterators via `ListPush`, `lower_islice()` parses 1-4 args for start/stop/step
- **GC** (`runtime/src/gc.rs`): Chain marks its iterator list, ISlice marks its inner iterator
- Supports: for-loop iteration, `next()`, chained `chain()+islice()` composition, both `import itertools` and `from itertools import chain, islice` styles

### Module Imports

The compiler supports importing functions from other Python modules:

```python
# utils.py
def add(a: int, b: int) -> int:
    return a + b

# main.py
from utils import add
result: int = add(2, 3)
```

**Compilation**:
```bash
# Compile with module in same directory
./target/release/pyaot main.py -o main --run

# Compile with module in different directory
./target/release/pyaot main.py --module-path /path/to/libs -o main
```

**Implementation**:
- **CLI** (`cli/main.rs`):
  - `--module-path` option for additional module search directories
  - `collect_imports()` - recursively finds all imported modules
  - `topological_sort()` - orders modules by dependencies
  - `compile_modules()` - merges all modules into single HIR/MIR
  - FuncId and InternedString remapping during module merge
- **Frontend** (`frontend-python/src/ast_to_hir/statements.rs`):
  - Handles `ImportFrom` statement, extracts module name and imported names
  - Records imports in `module.imports` for later resolution
  - Registers imported names in scope for use in expressions
- **HIR** (`hir/lib.rs`):
  - `Module.imports: Vec<ImportDecl>` - tracks import statements
  - `ImportDecl { module_path, names }` - module path and imported names
- **Lowering**:
  - Imported functions resolved to FuncId from merged module
  - `CallNamed` instruction for cross-module calls (late binding by name)
- **Codegen** (`codegen-cranelift/`):
  - Function name mangling: `__module_<modname>_<funcname>`
  - Module initialization order: dependencies first, then main module
  - `generate_main_entry_point_with_module_inits()` calls inits in order

**Module search order**:
1. Directory containing the source file
2. Directories specified via `--module-path` (in order)

**Limitations**:
- No relative imports (`from . import ...`)
- No `import *`
- No circular imports (detected and rejected with error)

### Package Imports with `__init__.py`

The compiler supports Python packages (directories with `__init__.py`):

```python
# mypackage/__init__.py
greet: str = "Hello"
def helper() -> int:
    return 42

# mypackage/math/__init__.py
PI: float = 3.14159

# mypackage/math/ops.py
def add(a: int, b: int) -> int:
    return a + b

# main.py
import mypackage                        # Access via mypackage.greet
from mypackage import helper            # Direct import from __init__.py
from mypackage.math import PI           # From subpackage __init__.py
from mypackage.math.ops import add      # From submodule
```

**Compilation**:
```bash
./target/release/pyaot main.py --module-path /path/to/packages -o main --run
```

**Implementation**:
- **Module resolution** (`cli/main.rs`):
  - `find_module_or_package()` - resolves dotted paths to files/packages
  - `ModuleResolution::File` - simple `.py` file
  - `ModuleResolution::Package` - directory with `__init__.py`
  - `ModuleResolution::Submodule` - nested module with parent `__init__.py` chain
  - Parent packages discovered and compiled before submodules
- **Frontend** (`frontend-python/src/ast_to_hir/`):
  - `imported_modules` tracks simple imports (`import pkg`)
  - `dotted_imports` tracks dotted imports (`import pkg.sub`)
  - Chained attribute access (`pkg.sub.func()`) resolved via `build_module_path_from_expr()`
- **Name mangling**:
  - Functions: `__module_<path>_<func>` where dots become underscores
  - Example: `mypackage.math.ops.add` → `__module_mypackage_math_ops_add`
- **Type inference**:
  - `get_expr_type()` handles `ModuleAttr` and `ImportedRef` via `module_var_exports`
  - Cross-module variable types correctly propagated

**Supported patterns**:
- `import pkg` → loads `pkg/__init__.py`, access via `pkg.attr`
- `import pkg.sub` → loads `pkg/__init__.py` then `pkg/sub.py`, access via `pkg.sub.attr`
- `from pkg import name` → imports `name` from `pkg/__init__.py`
- `from pkg.sub import func` → imports `func` from `pkg/sub.py`

**Limitations**:
- `from . import var` where `var` is a variable (not function) may cause type mismatch errors
- Package vs file ambiguity: if both `pkg.py` and `pkg/__init__.py` exist, package wins

### Relative Imports

Relative imports allow modules within a package to import from siblings and parent packages:

```python
# pkg/__init__.py
from .utils import helper        # Import from sibling module

# pkg/sub/__init__.py
from .. import greet             # Import from parent package
from ..utils import helper       # Import from parent's sibling

# pkg/sub/module.py
from ..sibling import func       # Import from parent's sibling module
```

**Implementation**:
- **CLI** (`cli/main.rs`):
  - `ExtractedImport { module_path, level }` - tracks relative import depth (dots)
  - `extract_imports_with_level()` - parses imports, counting leading dots
  - `resolve_relative_import()` - converts relative to absolute paths
  - `rewrite_relative_imports()` - transforms source code before parsing
- **Resolution algorithm**:
  1. Count leading dots (level: 1=`.`, 2=`..`, etc.)
  2. For regular modules, go up one level to containing package
  3. For `__init__.py` (packages), current path IS the package
  4. Go up (level-1) more levels
  5. Append remaining path after dots
- **Source rewriting**: Relative imports are rewritten to absolute before frontend parsing
  - `from .utils import func` → `from pkg.utils import func`
  - Preserves original line structure and whitespace

**Resolution examples**:

| Importing Module | Import Statement | Resolved |
|-----------------|------------------|----------|
| `pkg.sub.mod` | `from . import X` | `pkg.sub` |
| `pkg.sub.mod` | `from .utils import X` | `pkg.sub.utils` |
| `pkg.sub.mod` | `from .. import X` | `pkg` |
| `pkg.sub.mod` | `from ..other import X` | `pkg.other` |
| `pkg.__init__` | `from . import X` | `pkg` |
| `pkg.__init__` | `from .utils import X` | `pkg.utils` |

**Supported patterns**:
- `from . import name` - import from current package (`__init__.py`)
- `from .module import func` - import from sibling module
- `from .. import name` - import from parent package
- `from ..module import func` - import from parent's sibling module

**Limitations**:
- `from . import var` where `var` is a non-function variable may cause type inference issues
- Relative imports must be within a package (module with `--module-path`)

### Match Statement (Pattern Matching)

Python 3.10+ `match` statements are desugared to if/elif chains at MIR lowering:

```python
match x:
    case 1:
        result = "one"
    case 2 | 3:
        result = "two or three"
    case n if n > 10:
        result = "big"
    case [a, b]:
        result = a + b
    case [first, *rest]:
        result = first + len(rest)
    case _:
        result = "other"
```

**Supported patterns**:
- **Literal patterns**: `case 1`, `case "hello"` - direct equality check
- **Singleton patterns**: `case True`, `case False`, `case None`
- **Capture patterns**: `case x` - binds subject to variable x
- **Wildcard patterns**: `case _` - matches anything, no binding
- **As patterns**: `case pattern as name` - matches pattern and binds to name
- **Or patterns**: `case 1 | 2 | 3` - matches any alternative
- **Sequence patterns**: `case [a, b, c]` - matches list/tuple with exact length
- **Starred patterns**: `case [first, *rest]`, `case [*init, last]` - captures remaining elements
- **Mapping patterns**: `case {"key": value, **rest}` - dict key existence + value matching
- **Class patterns**: `case Point(x=0, y=val)` - isinstance check + keyword attribute matching; supports inheritance
- **Guard clauses**: `case n if n > 0` - additional condition after pattern match

**Implementation**:
- **HIR** (`hir/lib.rs`):
  - `StmtKind::Match { subject, cases }` - match statement
  - `MatchCase { pattern, guard, body }` - individual case
  - `Pattern` enum with all pattern variants
  - `MatchSingletonKind` for True/False/None
- **Frontend** (`frontend-python/src/ast_to_hir/statements/match_stmt.rs`):
  - `convert_match()` - converts AST Match to HIR
  - `convert_pattern()` - recursive pattern conversion
  - `collect_pattern_bindings()` - pre-registers bound variables
- **Lowering** (`lowering/src/statements/match_stmt.rs`):
  - `lower_match()` - evaluates subject once, chains cases
  - `lower_match_cases()` - recursive if/else generation
  - `generate_pattern_check()` - generates condition + bindings per pattern
  - Sequence patterns: length check + element extraction + recursive matching
  - Star patterns: slice extraction for captured rest elements
  - Or patterns: combine alternatives with OR
  - Guards: bindings applied before guard evaluation, combined with AND

**Desugaring approach** (match to if/elif):
```python
# Input
match x:
    case 1:
        body1
    case n if n > 0:
        body2
    case _:
        body3

# Equivalent lowered form (conceptual)
__subject = x
if __subject == 1:
    body1
elif True:  # capture always matches
    n = __subject
    if n > 0:  # guard
        body2
    else:
        body3  # wildcard
else:
    body3
```

**Limitations**:
- **__match_args__**: Not supported (positional class patterns require this)
- **Exhaustiveness checking**: No compile-time check for missing patterns
