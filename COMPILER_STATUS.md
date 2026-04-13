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
[MIR Optimizer] (optional, --devirtualize / --flatten-properties / --inline / --constfold / --dce flags)
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
| TypeAlias | ✅ | PEP 613 (`x: TypeAlias = T`) and PEP 695 (`type X = T`) |
| Literal[value] | ✅ | Erased to base type (`Literal[42]` → `int`) |
| TypeVar | ✅ | Type erasure: unconstrained → untyped (inference), constrained → Union, bounded → bound type |
| Protocol | ✅ | Structural subtyping with name-based vtable dispatch; works across different vtable layouts |

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
| try/except/else/finally | ✅ | Full exception objects: `.args`, `__class__.__name__`, `str(e)`, `raise e`, BaseException hierarchy |
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
| Generators | ✅ | HIR-level desugaring; `yield from`; ternary yields; filter conditions; `send()`/`close()`; throw() not supported |
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
| `__and__`, `__or__`, `__xor__`, `__lshift__`, `__rshift__` | ✅ | Bitwise operators on custom objects, with reverse variants |
| `__matmul__` (`@`) | ✅ | Matrix multiply operator on custom objects, with `__rmatmul__` |
| `__index__` | ✅ | Custom objects as list/str/tuple indices |
| `__format__` | ✅ | `format(obj, spec)` on custom classes |
| `__new__` | ✅ | Custom constructor; `object.__new__(cls)` for allocation |
| `__del__` | ✅ | Finalizer called during GC sweep; must not allocate |
| `__copy__`, `__deepcopy__` | ✅ | Custom `copy.copy()`/`copy.deepcopy()` behavior (simplified `__deepcopy__` without memo) |
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
| open() | ✅ | r/w/a/rb/wb/ab, r+/w+/a+, r+b/w+b/a+b modes; encoding= kwarg (utf-8, ascii, latin-1) |
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
| Cross-module user class values | ✅ | Field access (`r.x`), method calls (`r.foo()`), function return types, kwargs + simple defaults on imported functions |
| Cross-module user class annotations | ✅ | `r: Foo`, `r: mymod.Foo`, `def f(x: Foo) -> mymod.Foo`. Placeholders allocated at parse time, resolved in `mir_merger` before lowering |

### Standard Library

| Module | Functions/Constants |
|--------|---------------------|
| abc | abstractmethod |
| sys | argv, exit, intern, path (lazy-init from exe dir + CWD + PYTHONPATH; mutations persist on the cached singleton) |
| os | environ, remove, getcwd, chdir, listdir, mkdir, makedirs, rmdir |
| os.path | join, exists |
| re | search, match, sub; Match.group(), .start(), .end(), .groups(), .span() |
| json | dumps, loads, dump, load |
| math | pi, e, tau, inf, nan (constants); sqrt, sin, cos, tan, asin, acos, atan, atan2, sinh, cosh, tanh, ceil, floor, trunc, log, log2, log10, exp, fabs, fmod, copysign, hypot, pow, degrees, radians, factorial, gcd, lcm, comb, perm, isnan, isinf, isfinite |
| time | time, sleep, monotonic, perf_counter, ctime, struct_time, localtime, gmtime, strftime, strptime |
| subprocess | run, CompletedProcess |
| urllib.parse | urlparse, urlencode, quote, unquote, urljoin, parse_qs; ParseResult fields and geturl() |
| urllib.request | urlopen(url_or_Request, data=None, timeout=30.0), urlretrieve(url, filename, reporthook=None, data=None), Request(url, data=None, headers=None, method=None); Request fields (full_url, data, headers, method) |
| http.client | HTTPResponse class: CPython-standard fields (`.status`, `.url`, `.headers`) + methods (`.read()`, `.geturl()`, `.getcode()`), plus pip-requests-compatible conveniences (`.status_code`, `.ok`, `.content`, `.text`, `.json()`) so bundled `requests` is drop-in with the pip package |
| urllib.error | HTTPError, URLError (stdlib exception classes — must be imported explicitly, then catchable via try/except. Parent: OSError, so `except OSError:` also catches them) |
| string | ascii_letters, ascii_lowercase, ascii_uppercase, digits, hexdigits, octdigits, punctuation, whitespace, printable |
| random | random, randint, choice, choices, shuffle, seed, uniform, randrange, sample, gauss |
| hashlib | md5, sha256, sha1; Hash.hexdigest(), Hash.digest() |
| base64 | b64encode, b64decode, urlsafe_b64encode, urlsafe_b64decode |
| copy | copy, deepcopy |
| collections | defaultdict(int/float/str/bool/list/dict/set), Counter(iterable).most_common/.total/.update/.subtract, deque(maxlen=N).append/.appendleft/.pop/.popleft/.extend/.extendleft/.rotate/.reverse/.clear/.copy/.count, OrderedDict.move_to_end/.popitem(last=) |
| functools | reduce |
| itertools | chain, islice (with for-loop and next(); both import styles) |
| io | StringIO (write, read, readline, getvalue, seek, tell, close, truncate), BytesIO (write, read, readline, getvalue, seek, tell, close, truncate) |

Stdlib function calls support **keyword arguments** mapped to parameter positions (e.g., `random.choices([1,2,3], weights=[...], k=5)`).

### Third-Party Packages (experimental)

**Python-based packages (site-packages/)**

Bundled Python packages live under `site-packages/<name>/` in the repo and are auto-added to the compiler's module search path (alongside existing `--module-path` handling). No Rust code involved — they compile with the same Python → native pipeline as user code.

| Package | Surface | Notes |
|---------|---------|-------|
| requests | `get/post/put/delete(url, params=None, headers=None, auth=None, timeout=30.0)`; post/put also accept `data=bytes\|str\|dict` and `json=dict`. Returns `http.client.HTTPResponse` extended with pip-requests conveniences (`.status_code`, `.ok`, `.content`, `.text`, `.json()`). | **Drop-in for pip `requests`** on the covered API slice — `test_requests.py` runs identically against `pip install requests`, pyaot's bundled `site-packages/requests`, or a pyaot-compiled binary. Pure Python over the standard `urllib.request.Request + urlopen` pattern; `HTTPError` is caught internally so 4xx/5xx statuses return the response object (matching pip). TODOs: `verify` / `allow_redirects` need `ssl.SSLContext` + `build_opener()`. |

`urllib.request` exposes the standard `Request(url, data=None, headers=None, method=None)` class; `urlopen()` accepts either a URL string or a Request object. `urlretrieve(url, filename, reporthook=None, data=None)` downloads to a local file and returns a `(filename, headers)` tuple; deviation from CPython — `filename` is **required** (no tempfile fallback), `reporthook` is accepted but never invoked, and `headers` is a `dict[str, str]` rather than an `http.client.HTTPMessage`. `http.client.HTTPResponse` carries both CPython-standard attributes and pip-requests conveniences. `urllib.error` exposes `HTTPError` and `URLError` as catchable stdlib exception classes (reserved `class_id`s, `parent = OSError`), so `except OSError:` also catches them uniformly.

**Cross-module user-function kwargs + defaults** are supported: a function declared in one module can be called from another with keyword arguments, and unset optional params auto-fill from their declared defaults (simple constants — `None`/`int`/`float`/`bool`/`str`). This is what lets our `requests.get(url, params={...}, timeout=10.0)` call compile without listing every positional slot.

**Rust-based packages (crates/pkg/<name>/)**

Infrastructure remains for packages that genuinely need native Rust code (e.g. numeric libraries with BLAS). Registry is currently empty. See `.claude/rules/pkg-dev.md` for authoring either category.

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
| BaseException | ✅ |
| Custom exception classes | ✅ |
| `raise e` (re-raise variable) | ✅ |
| `e.__class__.__name__` | ✅ |
| BaseException/Exception hierarchy | ✅ | `except Exception` excludes SystemExit/KeyboardInterrupt/GeneratorExit |

### Optimizations

All passes implement the `OptimizationPass` trait and are orchestrated by `PassManager` (`optimizer/src/pass.rs`). Fixpoint passes iterate automatically until stable; the pipeline is configured via `build_pass_pipeline()`.

| Optimization | Status | CLI Flag | Description |
|--------------|--------|----------|-------------|
| Devirtualization | ✅ | `--devirtualize` | Replaces virtual method calls with direct calls when receiver type is statically known |
| Property Flattening | ✅ | `--flatten-properties` | Inlines trivial `@property` getters as direct field access |
| Function Inlining | ✅ | `--inline` | Inlines small functions at call sites to reduce call overhead |
| Constant Folding & Propagation | ✅ | `--constfold` | Evaluates constant expressions at compile time and propagates known values |
| Peephole Optimizations | ✅ | (with `-O`) | Identity elimination, strength reduction, box/unbox elimination |
| Dead Code Elimination | ✅ | `--dce` | Removes unreachable blocks, dead instructions, and unused locals |
| Cold Block Annotation | ✅ | (always on) | Marks exception handlers and error paths as cold for better register allocation |

**Devirtualization Details:**
- Converts `CallVirtual` (indirect vtable dispatch) to `CallDirect` (static dispatch) when the receiver's `Type::Class` is statically known
- Eliminates 2 memory loads (vtable pointer + method pointer) and indirect call overhead
- Enables downstream inlining: functions previously blocked by `has_uninlinable_calls` become inlinable
- Pass order: runs before inlining to maximize inlining opportunities

**Property Flattening Details:**
- Detects trivial `@property` getters whose body is a single `InstanceGetField` return
- Replaces `CallDirect` to these getters with inline `InstanceGetField`, eliminating function call overhead
- Example: `@property def x(self): return self._x` → direct field read at known offset
- Pass order: runs after devirtualization (to catch devirtualized property calls) and before inlining

**Function Inlining Details:**
- Inlines leaf functions with ≤10 instructions automatically
- Considers functions with ≤50 instructions (configurable via `--inline-threshold`)
- Never inlines: recursive functions, generators, exception handlers
- Preserves GC roots during transformation
- Multiple iterations for transitive inlining

**Constant Folding & Propagation Details:**
- Folds integer arithmetic (`+`, `-`, `*`, `//`, `%`, `**`, bitwise ops) with overflow checking
- Folds float arithmetic (`+`, `-`, `*`, `/`, `//`, `%`, `**`)
- Folds string literal concatenation (`"hello" + " world"`)
- Folds boolean logic (`and`, `or`, `not`) and all comparisons
- Folds type conversions (`BoolToInt`, `IntToFloat`, `FloatToInt`, `FloatAbs`) on constants
- Propagates single-definition constant locals into all uses
- Simplifies constant conditional branches to unconditional jumps
- Python-compatible floor division and modulo semantics for negative operands
- Safe: skips fold on overflow, division by zero, negative int exponents, NaN/infinity conversions
- Iterates propagation + folding to fixpoint for transitive constant chains

**Peephole Optimization Details:**
- Identity elimination: `x + 0` → `x`, `x * 1` → `x`, `x | 0` → `x`, `x & -1` → `x`, `x ** 1` → `x`, etc.
- Zero/absorbing: `x * 0` → `0`, `x & 0` → `0`, `x | -1` → `-1`, `x ** 0` → `1`
- Strength reduction: `x * 2` → `x + x`, `x * 2^n` → `x << n`, `x // 2^n` → `x >> n`, `x ** 2` → `x * x`
- Same-operand: `x - x` → `0`, `x ^ x` → `0`
- Box/unbox elimination: `UnboxInt(BoxInt(x))` → `x`, same for Float/Bool
- Bitcast roundtrip: `IntBitsToFloat(FloatBits(x))` → `x` and vice versa
- Double negation: `Neg(Neg(x))` → `x`, `Not(Not(x))` → `x`, `Invert(Invert(x))` → `x`
- Float patterns: `x + 0.0` → `x`, `x * 1.0` → `x`, `x / 1.0` → `x`
- Runs automatically when any optimization pass is enabled; iterates to fixpoint

**Dead Code Elimination Details:**
- Unreachable block elimination: BFS from entry block, removes blocks not reachable via CFG
- Dead instruction elimination: removes pure instructions whose results are never used
- Dead local elimination: cleans up unused local variable entries
- Iterates to fixpoint for cascading dead code removal
- Preserves all side-effectful instructions (calls, GC, exception handling, arithmetic that may raise)

**Cold Block Annotation Details:**
- Uses Cranelift's `set_cold_block()` to hint the register allocator and code layout engine
- Exception handler entries (TrySetjmp handler blocks) are marked cold
- Error paths (blocks ending in Raise/RaiseCustom/Reraise/Unreachable) are marked cold
- Transitive propagation: blocks reachable only from cold blocks are also marked cold
- Effect: register allocator spills more aggressively in cold blocks, keeping registers free for hot paths; cold code is placed farther away, improving instruction cache locality

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
| `__class_getitem__` | Types erased at runtime; `MyClass[int]` handled by type system at compile time |
| Descriptors (`__get__`, `__set__`, `__delete__`) | Requires dynamic attribute protocol at every field access; `@property` covers static use cases |
| `collections.namedtuple` | `namedtuple("Point", ["x", "y"])` creates classes dynamically from string arguments at runtime — fundamentally incompatible with AOT compilation where all types must be known at compile time. The `typing.NamedTuple` class-based syntax could be supported in the future by auto-generating dunder methods during class compilation, but the dynamic form cannot |

---

## Roadmap

See **[ROADMAP.md](ROADMAP.md)** for the full development roadmap with detailed implementation plans.

---


## Implementation Reference

See **[IMPLEMENTATION_REFERENCE.md](IMPLEMENTATION_REFERENCE.md)** for internal implementation details.
