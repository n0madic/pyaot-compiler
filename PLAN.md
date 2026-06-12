# Implementation Plan — broadening the language subset

The compiler reached the plan's original definition of "working" (see below):
Phases 1–9 and the post-Phase-9 hardening backlog (items 1–7) are **all
complete** — the gated `corpus/` + `microgpt.py` diff clean vs CPython, bignum,
table-based zero-cost unwinding + real tracebacks, raw-int specialization
(Phase 3c intervals + interprocedural), MRO-aware lattice joins, cached
`StrObj.char_len`, and the release-safe pre-codegen MIR verifier all landed. That
completed phase log now lives in git history (commits + each crate's `lib.rs`
doc) and the auto-memory; this plan no longer carries it.

What remains is **breadth**: the differential gate is green on its allowlist, but
~17 aspirational `corpus/test_*.py` files exercise valid Python 3 the compiler
does not yet accept. The full, deduplicated inventory of those gaps is the
**[Remaining backlog](#remaining-backlog--broaden-the-language-subset)** below.
The principles and anti-patterns that governed the build still govern every item
in it.

## Definition of "working" (reached)

A `pyaot` binary that:

- compiles the static-Python `corpus/` and runs **`microgpt.py`** (the north-star
  real script) unchanged or with only standard-syntax tweaks;
- produces output **identical to CPython** on every corpus file (differential gate);
- has **arbitrary-precision `int`**;
- links against `runtime` and produces a native executable;
- reaches competitive performance *after* the optimization phase — not before
  (correctness never waits on the optimizer; see Principle 2).

Out of scope (too dynamic for AOT): `eval`/`exec`/`compile`, metaclasses,
`__dict__` mutation, **dynamic** `getattr(obj, name_var)` (a literal-name
`getattr(obj, "x")` is in scope — see backlog §5), `globals()`/`locals()`,
`inspect`, `import *`, runtime class creation.

---

## Guiding principles

These are *why* features are built the way they are, and they bind every backlog
item below.

1. **Vertical slice before breadth.** Get the thinnest program through the
   *entire* pipeline first, then widen feature-by-feature. Building each layer
   deeply in isolation strands you in inference work before a stable end-to-end
   contract exists.

2. **★ Correctness lives on the Tagged baseline; inference precision is a
   performance lever, not a correctness requirement. ★** The central enabler.
   `repr_of` gives a *safe* representation for every `SemTy`, and `Repr::Tagged`
   is always correct, so a weak `typeck` yields *slower* code, never *wrong* code.
   A working (slow) compiler can ship on minimal inference and improve afterwards
   with no correctness risk. (See PITFALLS A2 for the failure mode this inverts.)

3. **Safe representation is free; only two narrowings carry a proof obligation.**
   `repr_of` already maps `Float→Raw(F64)`, `Bool→Raw(I8)`, `Str→Heap`,
   containers/classes→`Heap`. The *only* representation optimizations needing a
   proof: (a) `int`: `Tagged` (fixnum-or-bignum) → `Raw(I64)` under a
   range/no-overflow proof; (b) narrowing `Dyn`/`Union` → a concrete type.

4. **One coercion pass, one verifier.** All box/unbox/tag/untag/numeric-widen
   coercions are inserted by `lowering::legalize` and nowhere else. The MIR
   verifier runs in debug at *every* pass boundary and rejects representation
   mismatch.

5. **Inference is one algorithm: collect → solve → materialize.** A single
   constraint pass over a lattice with union-find. Never a fixpoint of mutually
   recursive monotone passes (PITFALLS A3).

6. **ABI = f(Repr), deterministic.** No per-function ABI flags, no marker bits,
   no ABI-repair stage. HOF callbacks are uniform `fn(Value)→Value` at baseline;
   unboxed call paths are a proven optimization, not an escape hatch (PITFALLS A4).

7. **Build the front-half fresh from the design; the substrate is a stable
   base.** Front-half crates are implemented from ARCHITECTURE.md + each crate's
   `lib.rs`, using established algorithms (constraint solving, C3 MRO, standard
   optimizer passes) on their own merits. The substrate (`runtime`, `core-defs`,
   …) is a stable dependency, edited deliberately rather than rewritten.

8. **The runtime contract evolves deliberately with the compiler.** Its `Value`
   ABI + `rt_*` signatures are the seam the whole compiler targets, so changes
   are deliberate, never casual. When compiler development requires a runtime
   change — fixing a runtime bug, a new `rt_*`/ABI, a layout extension — make
   it, document it as a contract change, and back it with corpus coverage
   (precedents: bignum, `StrObj.char_len`). What stays forbidden is editing the
   runtime to paper over a front-half bug or to dodge front-half work.

9. **Differential testing is the spine.** The corpus-vs-CPython harness gates
   every feature. A backlog item is not done until its construct diffs clean vs
   CPython on the corpus (lift the aspirational `test_*.py` files onto the gate
   allowlist as their gaps close).

### Anti-patterns to avoid (do not reintroduce)

A fixpoint-of-passes inference • per-function ABI flags / argument-marker bits /
a separate ABI-repair stage • a dual logical/physical type field with an optional,
dual-meaning sentinel • GC per-slot side-table tag masks • a 61-bit tagged int •
any representation-ambiguous "could be raw or pointer" type. Full catalogue with
rationale in **[PITFALLS.md](PITFALLS.md)** — Part A.

---

## Remaining backlog — broaden the language subset

Every gap below is **valid Python 3** that the current compiler rejects or
crashes on, harvested by probing each failing `corpus/test_*.py` construct-by-
construct (the parser stops at the first error, so whole-file compiles hide
most of them) and confirmed two ways: `python3` accepts the snippet, `pyaot`
does not. None of these widen the "Out of scope" list — they are subset breadth,
not new dynamism.

**Where the work lands.** Syntax/semantic gaps (§1–4, §7–8, §12–14) are
front-half work — `frontend-python` (parse/desugar), `typeck` (constraints), and
`lowering` — gated by Principle 9 and the verifier. Builtin **functions** (§5–6)
and **methods** on builtin/stdlib types (§9–11) follow the declarative
two-file pattern of Principle 8 (`stdlib-defs` descriptor + `runtime` `rt_*`),
unless they need true HOF/representation handling (`map`/`filter`, `type()`).
Keep every gradual/raw seam on the checked-coerce path (PITFALLS A2/A3).

### Highest-leverage first
These few gaps block the most files — close them before the long tail.

| Gap | Files blocked | First error |
|---|---|---|
| **kwargs on indirect/builtin calls** (`sorted(reverse=)`, `dict(a=1)`, `enumerate(start=)`) | ≥6 | `keyword arguments are not supported on indirect calls` |
| **kwargs on method calls** (`list.sort(key=)`, `str.format(name=)`, `str.split(sep=)`) | ≥4 | `keyword arguments are not supported for method calls` |
| **`is`/`is not` against non-`None`** (`x is True`, `a is b`) | 6 | `is / is not is only supported against None` |
| **`del`** statement (`del d[k]`, `del name`, `del obj.attr`) | 4 | `unsupported statement for this milestone` |
| **`*seq` spread into a non-`*args` callee** | 3 | `f() takes no *args, cannot spread * into it` |
| **Nested destructuring** `a, (b, c) = …` (assign / `for` / comprehension) | 2 | `tuple/list unpacking assignment is not yet supported` |
| **`type()` builtin** (incl. `type(x).__name__`) | 3 | `builtin Type not supported in Phase 2` |
| **int→float numeric tower through a `float` slot** | 2 | `int cannot be returned/assigned to a float slot` |

### 1. Calls & arguments
- **kwargs on indirect/builtin calls** — `sorted(xs, key=, reverse=)`, `dict(a=1,b=2)`, `enumerate(xs, start=1)`. (`min`/`max(key=)` already work — the exception.)
- **kwargs on method calls** — `list.sort(key=)`, `str.format(name=)`, `str.split(sep=, maxsplit=)`, `str.encode(encoding=)`, `str.replace(count=)`.
- **`*seq` spread** — four sub-cases: into a fixed-arity callee `f(*[1,2,3])`; mixed with positionals `f(1, *seq, 4)`; covering the leading fixed params of a varargs callee `def f(a, *rest)`; into a **decorated** function (distinct error).
- **`**d` spread into a call** — `f(**{"a":1})` (was Phase 6C out-of-scope).
- **Mutable default parameter** — `def f(x, lst=[])`, `d={}`.
- **Non-literal default** — `def f(count=5+5)`.

### 2. Operators & expressions
- **`is`/`is not` against non-`None`** — `x is True`, `a is b`, `type(x) is T`.
- **Walrus `:=`** — in `if`/`while`/nested expressions, everywhere.
- **Matrix-multiply `@` / `__matmul__`**.

### 3. Statements
- **`del`** — entirely unimplemented: `del d[k]`, `del li[i]`, `del name`, `del obj.attr`.
- **PEP 695 `type X = T`** — `type IntPair = tuple[int, int]`.
- **`X: TypeAlias = T`** (PEP 613) — RHS type rejected as a value.
- **`...` (Ellipsis) as a statement / stub body** — `def f() -> int: ...` (Protocol stubs).

### 4. Unpacking & loop targets
- **Nested destructuring** — `a, (b, c) = …`, `(m1,m2),(m3,m4) = …`, `g, [h, i] = …` in assignment, `for`, and comprehension/genexpr targets. Flat and starred forms already work.
- **Attribute / subscript as a `for` target** — `for obj.attr in …`, `for lst[i] in …`.
- **`range()` for-loop with a non-literal step** — `range(10,0,-(-1))`, a variable step, `range(0,10,len(xs))` (the `list(range(...))` value form already works; the restriction is only in the for-loop desugar).

### 5. Builtins — `undefined name`
`map`, `filter`, `round`, `pow`, `all`, `any`, `id`, `divmod`, `bin`, `hex`,
`oct`, `format`, `ascii`, `getattr` (literal-name), `setattr`, `hasattr`,
`issubclass`, `object` (`object.__new__`), `NotImplemented`.

### 6. Builtins — `Phase 2 codegen not supported`
- **`type()`** — incl. `type(x).__name__`, `str(type(x))`.
- **`hash()`**.

### 7. `isinstance`
- **Tuple of types** — `isinstance(x, (int, str))` (single-type form works).
- **Container targets** — `isinstance(x, list/dict/tuple/set)`.
- **Gradual/`Any` receiver** — "runtime type query on a gradual value is out of scope" (decide: support via a runtime tag query, or keep out-of-scope and document).

### 8. Numeric tower (int↔float)
- An `int`/widened local returned through `-> float`; a literal `return 0` in a `-> float` function; an unannotated mixed `return 1.5 / return 0` inferred as `Any` and rejected at a `float` slot. (bool↔int promotion already works — the gap is specifically int↔float.)

### 9. Methods on builtin types
- **`int`**: `bit_length`, `bit_count`, `conjugate`, `__index__`.
- **`str`**: `format`, `split`, `rsplit`, `replace`, `lstrip`/`rstrip`, `removeprefix`/`removesuffix`, `expandtabs`, `splitlines`, `partition`/`rpartition`, `rindex`, `encode`, predicates `isdigit`/`isalpha`/`isalnum`/`isspace`/`isupper`/`islower`/`isascii`. (`upper`/`lower`/`strip`/`find`/`title`/`center`/… already work.)
- **`bytes`**: `startswith`, `endswith`, `find`, `rfind`, `count`, `replace`, `split`/`rsplit`, `strip`/`lstrip`/`rstrip`, `upper`/`lower`, `join` — only `.decode()` is supported today.
- **`tuple`**: `index`, `count`.
- **`dict`**: `popitem`, `fromkeys`.
- **`set`**: `issubset`, `issuperset`, `isdisjoint`, `intersection_update`, `difference_update`, `symmetric_difference_update`. (`union`/`intersection`/`|&-^` already work.)

### 10. `collections` module
- **`Counter`** — `undefined symbol rt_make_counter` at link (the `rt_*` does not exist); `.total()`, `.most_common()` also unsupported.
- **`defaultdict`** — a type passed as the factory (`defaultdict(int)`); subscript-store `dd[k]=v`.
- **`deque`** — all mutating/query methods (`append`/`appendleft`/`pop`/`popleft`/`rotate`/…) and item assignment `dq[i]=v`. (Construction, read, iteration, `list/sum/sorted(dq)` already work.)
- **`OrderedDict`** — `move_to_end`, `popitem`.

### 11. Classes / OOP
- **`@abstractmethod`** and general method decorators.
- **Class decorators** — any `@deco` on a class (incl. `@runtime_checkable`).
- **`object` / `object.__new__(cls)`** and **`NotImplemented`** (see §5).
- **`abs()` on a user class** — the `__abs__` result is not statically typed, so a later `.attr` on it fails.

### 12. Typing / generics
- **Subscripted instance annotation** — `b: Box[int] = …` (the `Generic[T]` base parses; the `Name[T]` *annotation use* fails).
- **`Protocol` base class** — `unknown base class Protocol` (structural subtyping unsupported); also **`Protocol[T]`** subscripted base.
- **`zip()` with 3+ iterables** — exactly two are supported.

### 13. f-strings
- **Dynamic/nested format specs** — `f"{x:.{n}f}"`, `f"{x:{w}d}"`.
- **`!a` conversion** — `f"{x!a}"`, `f"{x=!a}"`. (`!r`/`!s`, debug `=`, static specs already work.)

### 14. Literals
- **Non-UTF-8 bytes literals** — any byte ≥ `\x80`, e.g. `b"\xff"` (this is what fails `test_file_io.py`; ASCII and `\x00`–`\x7f` already work).

---

## Cross-cutting

- **Differential harness (the spine):** every corpus file is the spec. A feature
  isn't done until its corpus entry diffs clean vs CPython; close a backlog item
  by lifting the relevant `test_*.py` onto the gate allowlist.
- **Verifier discipline:** the MIR verifier is a debug-build invariant at every
  pass boundary, plus one mandatory release-safe pass at final pre-codegen.
- **Specialization is always optional:** if a representation optimization is
  unsure, it must fall back to `Tagged` / safe `repr_of` — never guess.
- **Before adding a flag/side-table/special-case:** stop and re-read PITFALLS
  Part A — that is the smell. Fix the representation or the constraint instead.
