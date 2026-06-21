# COMPILER_STATUS.md

Implementation status of `pyaot-compiler` relative to standard Python 3.

**Legend**

| Mark | Meaning |
|---|---|
| ✅ | Fully supported — covered by the differential corpus, byte-exact vs CPython |
| 🟡 | Partial — works with caveats noted in the *Notes* column |
| ❌ | Not supported — out of scope by design, or not implemented |

> Deliberately **out of scope** (too dynamic for AOT): `eval` / `exec` /
> `compile`, metaclasses, `__dict__` mutation, dynamic `getattr(obj, name_var)`
> with a non-literal name, `globals()` / `locals()`, `inspect`, `import *`,
> runtime class creation, `async` / `await`. See `ARCHITECTURE.md`.
>
> Also out of scope for a **GC-model** reason (tracing collector ≠ CPython's
> deterministic refcount finalization): `__del__` finalizers and manual GC
> control (`gc.collect()`, the `gc` module). Use `with` / context managers for
> deterministic cleanup.

---

## 1. Statements & syntax

| Feature | Status | Notes |
|---|---|---|
| Assignment `x = …` | ✅ | |
| Annotated assignment `x: T = …` | ✅ | Annotation drives representation/inference |
| Augmented assignment `+= -= *= …` | ✅ | All operators, incl. `@=`, in-place container ops |
| Tuple/list unpacking `a, b = …` | ✅ | |
| Nested destructuring `(a, (b, c)) = …` | ✅ | Recursive, also in `for`/comprehension targets |
| Starred assignment target `a, *rest = …` | ✅ | Prefix/middle/suffix star (`*init, last`, `a, *mid, z`); also in `for` (`for h, *t in …`) and `with … as (h, *r)` targets |
| `if` / `elif` / `else` | ✅ | |
| `while` / `while … else` | ✅ | |
| `for` / `for … else` | ✅ | `range` fast-path + general iterator protocol |
| `break` / `continue` | ✅ | |
| `pass` | ✅ | |
| `del` (name / item / slice) | ✅ | `del` of a captured nonlocal/closure var is unsupported |
| `with` (single & multiple items) | ✅ | Items nest left-to-right; `__enter__`/`__exit__` |
| `async with` | ❌ | Out of scope |
| `try` / `except` / `else` / `finally` | ✅ | Multi-except, exception binding, re-raise |
| `except*` (exception groups) | ❌ | Not implemented |
| `raise` / `raise … from …` | ✅ | Real tracebacks with line numbers. `… from <cause>` is supported for builtin-exception constructors; a custom/stdlib-exception or variable cause is out of scope |
| `assert` | ✅ | |
| `global` / `nonlocal` | ✅ | |
| `import` / `from … import` | ✅ | Multi-module, packages, re-exports; conditional top-level imports in `if`/`try` (known modules only) |
| `import *` | ❌ | Out of scope |
| `match` / `case` | ✅ | Literal/class/capture/guard/OR/sequence/mapping patterns |
| `type X = T` (PEP 695 alias) | ✅ | Compile-time annotation binding |
| `def` (functions) | ✅ | |
| `async def` | ❌ | Out of scope |
| `class` | ✅ | Incl. nested (capture-free) classes |
| Decorators `@…` | ✅ | Functions, classes, decorator factories |
| `lambda` | ✅ | First-class `Callable` values; full parameter forms (literal defaults, keyword-only, `*args`, `**kwargs`) everywhere — module-level, nested, and in expression position. A *mutable/computed* default is only supported via the module-level `name = lambda …=…` → `def` desugar (once-eval slot); elsewhere it is a clean error (no per-closure default slot), as for a nested `def` |
| `yield` / `yield from` | ✅ | Generators (frontend state-machine desugar) |
| `await` | ❌ | Out of scope |
| Walrus `:=` | ✅ | All scopes (if/while/comprehension/global) |
| f-strings | ✅ | Conversions, nested fields, dynamic format specs |
| f-string `=` self-doc (`f"{x=}"`) | ❌ | PEP 501 spec form not implemented |

---

## 2. Built-in data types

| Type | Status | Notes |
|---|---|---|
| `int` | ✅ | **Arbitrary precision** (bignum) |
| `float` | ✅ | IEEE-754 double; numeric-tower coercions |
| `bool` | ✅ | Subtype of `int` semantics |
| `str` | ✅ | Unicode, codepoint model (len/slice/iter/case/align) |
| `bytes` | ✅ | Constructors (incl. `bytes.fromhex`), methods (find/rfind/index/rindex/count/replace/split/strip/case/join/decode/startswith/endswith), slicing |
| `list` | ✅ | |
| `tuple` | ✅ | Fixed-arity & homogeneous; lexicographic compare |
| `dict` | ✅ | Insertion-ordered; merge `|`/`|=`, `fromkeys` |
| `set` | ✅ | `\| & - ^`, `pop`, and update/relational methods |
| `range` | ✅ | Lazy iterator, negative/variable step |
| `NoneType` | ✅ | `None` sentinel, `is`/`is not` |
| `frozenset` | ✅ | Pragmatic core: ctors, protocol, hashable (dict key/set elem), `\| & - ^`, union/intersection/difference/symmetric_difference/copy/issubset/issuperset/isdisjoint. Gap: ordering (`< <=`), multi-element repr order follows set model |
| `bytearray` | ✅ | Pragmatic core: ctors, len/in/iter/index/slice/`==`, mutators append/extend/`ba[i]=v`/`+=`, hex/decode/find/rfind/count/startswith/endswith. Gap: insert/pop/remove/reverse/clear, slice-assign/`del`, `*`/`*=`, split/strip/replace/case |
| `complex` | ❌ | Not implemented |
| `memoryview` | ❌ | Not implemented |

---

## 3. Operators

| Group | Status | Notes |
|---|---|---|
| Arithmetic `+ - * / // % **` | ✅ | Bignum + float tower; raw-int specialization |
| Matrix multiply `@` | ✅ | `__matmul__` / `__rmatmul__` |
| Bitwise `& \| ^ ~ << >>` | ✅ | |
| Comparison `< <= > >= == !=` | ✅ | Rich-comparison dunders, chained compares |
| Identity `is` / `is not` | ✅ | Bit-identity + `None` normalization |
| Membership `in` / `not in` | ✅ | Containers + user `__contains__` |
| Boolean `and` / `or` / `not` | ✅ | Short-circuit, truthiness via `__bool__`/`__len__` |
| Ternary `a if c else b` | ✅ | |
| Reflected & in-place dunders | ✅ | `__radd__`, `__iadd__`, … |

---

## 4. Functions & scoping

| Feature | Status | Notes |
|---|---|---|
| Positional / default params | ✅ | Mutable defaults eval-once (CPython aliasing) |
| `*args` | ✅ | |
| `**kwargs` | ✅ | |
| Keyword-only / positional-only params | ✅ | `kwonlyargs` / `posonlyargs` |
| Keyword arguments at call site | ✅ | Eval-order preserving |
| `*seq` / `**dict` argument spread | ✅ | |
| Closures / free variables / cells | ✅ | `Repr::Closure` env-tuple |
| `nonlocal` / `global` | ✅ | |
| Decorators / decorator factories | ✅ | |
| Recursion | ✅ | |
| First-class functions / callables as values | ✅ | Uniform value-call ABI |
| Generators (`yield`, `send`, `close`, `yield from`) | ✅ | |
| Generator expressions | ✅ | |

---

## 5. Classes & OOP

| Feature | Status | Notes |
|---|---|---|
| Class definition, fields, methods | ✅ | |
| `__init__` / instance construction | ✅ | |
| Single inheritance | ✅ | |
| Multiple inheritance + C3 MRO | ✅ | MRO-aware type lattice |
| `super()` | ✅ | |
| `@property` | ✅ | |
| `@staticmethod` / `@classmethod` | ✅ | `cls` is a compile-time alias of the enclosing class (`cls()` / `cls.attr` / `cls.method`); statically enclosing class, not a runtime subclass |
| Dunder methods (arith/compare/container/iter) | ✅ | Return Tagged, read via `rt_is_truthy` |
| Iterator protocol (`__iter__`/`__next__`) | ✅ | Per-class compiled iter-next thunk |
| `__slots__` | 🟡 | Accepted but ignored (no layout effect) |
| Abstract base classes (`abc.ABC`, `@abstractmethod`) | ✅ | Recognized at parse time |
| `typing.Protocol` / structural `isinstance` | ✅ | Instance-only via `rt_obj_has_method` |
| Nested classes | ✅ | Capture-free only; capturing enclosing locals rejected |
| Gradual / `Dyn`-receiver method dispatch | ✅ | Tag-dispatch `rt_obj_method` — list/dict/set/deque/tuple/int + **str** (case/strip/split/replace/find/encode/join/predicates/…) + user instances |
| Class attribute access by literal name | ✅ | |
| Dynamic attribute by variable name `getattr(o, var)` | ❌ | Out of scope |
| `__dict__` mutation / dynamic attributes | ❌ | Out of scope |
| Metaclasses | ❌ | Out of scope |
| `__del__` finalizers | ❌ | Out of scope — tracing GC can't match CPython's deterministic finalization timing; use `with` |
| `__copy__` / `__deepcopy__` hooks | ✅ | `copy.copy` / `copy.deepcopy` dispatch to the user dunder via a per-class thunk; `__deepcopy__` gets a fresh memo dict (not the runtime's cycle tracker) |
| `@dataclass` | ✅ | Frontend-only desugar: synthesizes missing `__init__`/`__repr__`/`__eq__` from field annotations + literal defaults; `ClassVar` excluded. Out of scope: `@dataclass(frozen=/order=/…)` & kwargs, `field()`/`default_factory`, mutable defaults, inheritance, `InitVar`, `__hash__` |
| `enum.Enum` | ❌ | Not implemented |
| `collections.namedtuple` | ✅ | Frontend-only desugar to a class with positional fields: synthesizes `__init__`/`__repr__`/`__eq__`/`__len__`/`__getitem__`/`__iter__`/`__contains__`. Supports field access, indexing, `len`, unpacking, iteration, membership, `*`-spread, list/tuple/space-or-comma-string field specs. Out of scope: `rename=`/`defaults=`/`module=`, the `_make`/`_asdict`/`_replace`/`_fields` API, equality vs a real `tuple`, negative indices/slices |
| `typing.NamedTuple` / `TypedDict` | ❌ | Not implemented (the `class X(NamedTuple)` / functional `typing` forms; use `collections.namedtuple`) |

---

## 6. Iteration & comprehensions

| Feature | Status | Notes |
|---|---|---|
| List comprehension | ✅ | |
| Dict comprehension | ✅ | |
| Set comprehension | ✅ | |
| Generator expression | ✅ | |
| Nested / multi-clause comprehensions | ✅ | Outermost-iterable scope handled |
| Conditional clauses (`if`) | ✅ | |
| `for` over user iterators | ✅ | Lazy `__iter__`/`__next__` |
| Slicing (list/str/tuple, with step) | ✅ | Slice assignment & deletion for lists |

---

## 7. Exceptions

| Feature | Status | Notes |
|---|---|---|
| `raise` / `raise … from …` | ✅ | `… from <cause>` for builtin-exception constructors; custom/stdlib-exception or variable cause out of scope |
| `try`/`except`/`else`/`finally` | ✅ | Table-based unwinding (no setjmp) |
| Multiple `except` clauses | ✅ | |
| Exception binding `except E as e` | ✅ | |
| Custom exception subclasses | ✅ | |
| Built-in exception hierarchy | ✅ | `ValueError`, `TypeError`, `IndexError`, `IOError`, … |
| Real tracebacks (file/line) | ✅ | Lazy PC−1 resolution, `pyaot_tb_table` |
| `except*` / `ExceptionGroup` | ❌ | Not implemented |

---

## 8. Pattern matching (`match`)

| Pattern | Status | Notes |
|---|---|---|
| Literal patterns | ✅ | |
| Capture / wildcard `_` | ✅ | |
| Class patterns | ✅ | With type narrowing |
| Sequence patterns | ✅ | |
| Mapping patterns | ✅ | |
| OR patterns `\|` | ✅ | Incl. capturing alternatives (`case A(x) \| B(x)`); every alternative must bind the same name set (CPython rule) |
| Guards (`if`) | ✅ | |

---

## 9. Built-in functions

| Builtin(s) | Status | Notes |
|---|---|---|
| `print` | ✅ | `sep`/`end`/`file=sys.stdout\|sys.stderr`/`flush=True\|False`, byte-exact formatting |
| `len`, `range`, `enumerate`, `zip`, `reversed` | ✅ | `zip(N≥3)` supported |
| `map`, `filter` | ✅ | Eager desugar (single iterable) |
| `sorted`, `reversed`, `min`, `max`, `sum` | ✅ | `key=` / `reverse=` |
| `abs`, `round`, `pow`, `divmod` | ✅ | |
| `all`, `any`, `id`, `hash` | ✅ | |
| `bin`, `hex`, `oct`, `ord`, `chr` | ✅ | |
| `repr`, `str`, `format`, `ascii` | ✅ | Full format mini-language; user `__repr__`/`__format__` |
| `int`, `float`, `bool` | ✅ | Incl. `int(str, base)` |
| `list`, `dict`, `set`, `tuple`, `bytes` | ✅ | |
| `type(x)` (1-arg) | ✅ | |
| `isinstance` / `issubclass` | ✅ | Incl. tuple-of-types form |
| `getattr` / `setattr` / `hasattr` | ✅ | Literal attribute name only |
| `iter` / `next` | ✅ | |
| `open` | ✅ | File I/O (text), iteration over lines |
| `super`, `staticmethod`, `classmethod`, `property` | ✅ | |
| `frozenset` / `bytearray` | ✅ | Pragmatic core (see Built-in types) — `RuntimeObject`-modeled, byte-exact in the corpus gate |
| `complex`, `memoryview` | ❌ | Backing types not implemented |
| `input` | ✅ | Reads a line from stdin (optional prompt to stdout); `EOFError` at end of input |
| `callable` | ✅ | Static fold from the value's type — a `Callable`, or a class instance whose class defines `__call__`; class / top-level-function names fold to `True`; a `Dyn`/`Union` value is rejected |
| `vars`, `dir`, `delattr` | ❌ | Not implemented / out of scope |
| `eval`, `exec`, `compile` | ❌ | Out of scope by design |
| `globals`, `locals` | ❌ | Out of scope by design |
| `type(name, bases, dict)` (3-arg / runtime class) | ❌ | Out of scope by design |

---

## 10. Modules & imports

| Feature | Status | Notes |
|---|---|---|
| `import module` | ✅ | |
| `from module import name` | ✅ | |
| `import module as alias` / `from … as …` | ✅ | |
| Multi-file projects / packages | ✅ | `--module-path`, `__init__.py` |
| External `site-packages/` packages | ✅ | Pure-Python packages on the stdlib subset, compiled like user imports. Roots: `$PYAOT_SITE_PACKAGES`, `<exe_dir>/site-packages`, `<repo_root>/site-packages`. Bundled example: `requests` (urllib facade, CPython-faithful `get`/`post`/`put`/`delete` typed `-> HTTPResponse`). |
| Re-exports | ✅ | |
| Conditional top-level import (`if`/`elif`/`else`, `try`/`except`/`else`/`finally`) | ✅ | Optional-dependency pattern. Module must be resolvable at compile time (stdlib or on-disk user module — known modules only); binding is module-wide (compile-time), the runtime `<init>`/snapshot runs only when the branch is taken. One import site per module recommended (init-once ordering). Imports in a function body / `while`/`for`/`with` stay out of scope. |
| `__name__ == "__main__"` entry guard | ✅ | |
| `from __future__ import annotations` | ✅ | |
| `import *` | ❌ | Out of scope |

---

## 11. Standard library modules

Supported surfaces are those exercised by the corpus; each module covers the
common subset, not the entire API. The runtime stdlib surface is feature-gated
(`stdlib-json`, `stdlib-regex`, `stdlib-crypto`, `stdlib-base64`,
`stdlib-network`); see README "Slim runtime".

| Module | Status | Notes |
|---|---|---|
| `math` | ✅ | Core functions + constants |
| `random` | ✅ | |
| `sys` | ✅ | `argv`, `exit`, `intern`, `path`; constants `platform`/`maxsize`/`maxunicode`/`byteorder` |
| `time` | ✅ | Live timestamps (self-checking test mode) |
| `re` | ✅ | `Match`, common pattern ops (gated `stdlib-regex`) |
| `json` | ✅ | Object types, `ensure_ascii` (gated `stdlib-json`) |
| `os` / `os.path` | ✅ | Submodule chains, `environ`, `posixpath`; constants `sep`/`linesep`/`pathsep`/`curdir`/`pardir`/`extsep`/`devnull` |
| `subprocess` | ✅ | |
| `itertools` | ✅ | |
| `functools` | ✅ | `reduce` (desugared accumulate-loop) |
| `collections` | ✅ | `Counter`, `defaultdict`, `deque`, `OrderedDict`, `namedtuple` (the last via the frontend class desugar — see OOP table) |
| `urllib.parse` / `urllib.request` / `urllib.error` | ✅ | `urlopen`/`urlretrieve` gated `stdlib-network` (live-net tests opt-in) |
| `hashlib` | ✅ | Gated `stdlib-crypto` |
| `base64` | ✅ | Gated `stdlib-base64` |
| `string` | ✅ | Constants/helpers |
| `io` | ✅ | File-object surface |
| `abc` | ✅ | `ABC` / `@abstractmethod` |
| `copy` | ✅ | |
| `typing` | ✅ | `Protocol`, `Generic`, `TypeVar`, type aliases, `Optional`/`Union` |
| `datetime`, `decimal`, `fractions`, `enum` | ❌ | Not implemented |
| `dataclasses` | 🟡 | `@dataclass` decorator only (frontend desugar — see OOP table); `field()`/`default_factory`/`fields()`/`asdict()` out of scope |
| `asyncio` | ❌ | Out of scope (no `async`/`await`) |
| `gc` (manual GC control) | ❌ | Out of scope — tracing collector; `gc.collect()` count can't match CPython |

---

## 12. Types & generics

| Feature | Status | Notes |
|---|---|---|
| Type annotations on params/returns/vars | ✅ | Drive representation & inference |
| Type inference (constraint-based) | ✅ | One solver, finished before lowering |
| Gradual typing (`Dyn` / unannotated) | ✅ | Always-correct Tagged baseline |
| Generics / monomorphization | ✅ | `Generic`, `TypeVar` |
| `typing.Protocol` (structural) | ✅ | Instance-level |
| `Optional[T]` / `Union[…]` | ✅ | Union → Tagged; `Union → Heap` admitted behind a runtime tag guard for guard-backed shapes (containers / class / stdlib `RuntimeObj` via `rt_check_heap_kind`/`rt_check_instance`/`rt_check_runtime_obj`), still rejected for guard-less `BigInt`/`Iterator` |
| `Callable[…]` | ✅ | Closures/lambdas as values |
| Forward references / string annotations | ✅ | `from __future__ import annotations` |

---

## 13. Optimization & tooling

| Feature | Status | Notes |
|---|---|---|
| Cranelift native codegen | ✅ | `opt_level=speed` default |
| MIR optimizer (inline/constfold/peephole/DCE/cold-layout) | ✅ | |
| Raw-int interval specialization | ✅ | Interprocedural fixpoint |
| Parallel codegen | ✅ | Byte-identical output |
| GC (shadow-frame roots, table unwinding) | ✅ | GC-rootness derived from `Repr` |
| Real tracebacks | ✅ | |
| `--run` / `--emit-hir/types/mir` / `-v` | ✅ | |
| Slim (feature-gated) runtime | ✅ | |

---

*Generated from the `corpus/` differential gate
(`crates/cli/tests/differential.rs`) and the front-half crate sources.
"Supported" means present in `PHASE_CORPUS` and byte-exact against CPython.*
