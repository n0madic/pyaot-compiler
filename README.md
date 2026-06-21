# pyaot-compiler

A static **ahead-of-time compiler for a typed subset of Python 3** that emits
standalone native executables, built on [Cranelift](https://cranelift.dev/).

No interpreter, no bytecode VM, no Python runtime to ship — `pyaot script.py`
produces a single native binary you can run directly. The goal is to compile
**real, idiomatic Python** (classes, closures, generators, exceptions,
comprehensions, a working slice of the standard library, arbitrary-precision
`int`) unchanged, or with minimal changes that stay *within standard Python
syntax*.

```bash
cargo build -p pyaot-runtime          # build the runtime staticlib once
cargo run -p pyaot-cli -- hello.py --run
```

```python
# hello.py — ordinary, type-annotated Python
from dataclasses import dataclass

@dataclass
class Point:
    x: int
    y: int

    def norm2(self) -> int:
        return self.x * self.x + self.y * self.y

def closest(points: list[Point]) -> Point:
    return min(points, key=lambda p: p.norm2())

pts = [Point(3, 4), Point(1, 1), Point(8, 0)]
print(closest(pts))          # Point(x=1, y=1)
print(2 ** 200)              # arbitrary-precision int, byte-exact with CPython
```

This compiles to a native executable whose stdout matches CPython **byte for
byte** — which is exactly how every feature is verified (see
[The correctness discipline](#the-correctness-discipline)).

---

## The idea

Compiling a dynamic language ahead-of-time usually forces a hard choice: stay
dynamic and box everything (slow), or demand full static types and reject most
real programs (impractical). pyaot's design dissolves that tension with **one
load-bearing decision**: it keeps *what a value is* and *how a value is stored*
as two separate types.

- **`SemTy`** — the semantic, Python-level type (`int`, `list[str]`, a class,
  or the gradual `Dyn`).
- **`Repr`** — the physical representation (`Tagged` boxed value, unboxed
  `Raw(i64/f64)`, or a typed `Heap` pointer).

`Repr::Tagged` is **always correct** for every value. `Raw` and `Heap` are
*optimizations* that the type checker must *prove* safe — never a default that
could corrupt memory. The consequence is the project's central enabler:

> A weak type checker produces **slower** code, never **wrong** code.
> Inference precision is a *performance lever*, not a correctness requirement.

So a working (if slow) compiler can exist on minimal inference, and precision
can grow afterward with no correctness risk — there is no "representation
cliff" where imprecise inference becomes a miscompile. The whole front-half is
built around protecting that invariant. The full rationale, and the six
invariants every change answers to, are in **[ARCHITECTURE.md](ARCHITECTURE.md)**;
the AOT-specific traps this design avoids are catalogued in
**[PITFALLS.md](PITFALLS.md)**.

## What works

The compiler handles a broad, practical slice of Python 3 — enough to compile
Karpathy-style numeric code (`corpus/microgpt.py`, a from-scratch autograd MLP)
byte-exact against CPython. A non-exhaustive sketch:

- **Language** — classes (single + multiple inheritance, C3 MRO, `super`,
  `@property`, `@staticmethod`/`@classmethod`, dunders, `@dataclass`), closures
  & nested functions, generators (`yield`/`yield from`/`send`/`close`), `lambda`,
  decorators, comprehensions, `with`, `try/except/else/finally` with real
  tracebacks, structural `match`, walrus `:=`, f-strings, `*args`/`**kwargs` and
  spread, unpacking.
- **Types** — `int` (**arbitrary precision** / bignum), `float`, `bool`, `str`
  (Unicode, codepoint-correct), `bytes`, `list`, `tuple`, `dict`, `set`,
  `range`, `None`; constraint-based inference, generics/monomorphization,
  `typing.Protocol` (structural), gradual `Dyn`.
- **Builtins & stdlib** — most scalar/collection builtins; `math`, `random`,
  `sys`, `os`/`os.path`, `time`, `re`, `json`, `itertools`, `functools`,
  `collections` (`Counter`/`defaultdict`/`deque`/`OrderedDict`/`namedtuple`),
  `urllib`, `hashlib`, `base64`, `copy`, `typing`, and file I/O.

The complete, always-current feature matrix — keyed to the differential corpus,
with ✅/🟡/❌ per item — lives in **[COMPILER_STATUS.md](COMPILER_STATUS.md)**.

**Out of scope by design** (too dynamic for AOT, or incompatible with the
runtime's tracing GC): `eval`/`exec`/`compile`, metaclasses, `__dict__`
mutation, dynamic `getattr(obj, name_var)`, `globals()`/`locals()`, `inspect`,
`import *`, runtime class creation, `async`/`await`, and `__del__` finalizers.

## Performance

pyaot is competitive with — and on numeric/container code, faster than —
CPython 3.x. Ratios below are `cpython_time / pyaot_time`, so **higher is
better** and `>1.0` means pyaot wins (latest run, Apple-silicon, `--opt-level
speed`):

| benchmark | ratio vs CPython | notes |
|---|---:|---|
| `bench_float_kernel` | **2.33×** | unboxed `Raw(f64)` arithmetic |
| `bench_containers` | **1.79×** | typed list/dict ops |
| `bench_calls` | **1.57×** | direct calls + devirtualization |
| `bench_str` | **1.04×** | codepoint string model |
| `bench_exc_hotpath` | 0.80× | table-based unwinding overhead |
| `bench_int_loop` | 0.50× | irreducible-loop raw-int limitation (PITFALLS A7) |

Backing this: a MIR optimizer pipeline (inlining, devirtualization, constant
folding, peephole, DCE, cold-block layout) and Cranelift `opt_level=speed` by
default, on top of the tagged baseline.

## How it works

A linear, single-pass-per-stage pipeline — name resolution and **one**
constraint-based type inference finish *before* lowering; all boxing/coercion
is inserted by a single `legalize` pass; a verifier runs at every MIR pass
boundary in debug builds.

```
source ─▶ frontend-python ─▶ HIR ─▶ semantics ─▶ typeck ─▶ lowering (+legalize)
       ─▶ MIR (verify) ─▶ optimizer (verify) ─▶ codegen-cranelift ─▶ linker ─▶ exe
```

The codebase is split into a **stable substrate + runtime contract** and a
**front-half built fresh from the design in this repo**:

```
crates/
  # substrate + runtime contract — a stable seam (the Value-level ABI + rt_* calls),
  # changed deliberately when compiler development requires (e.g. bignum)
  core-defs/  format-shared/  utils/  diagnostics/  linker/  stdlib-defs/  runtime/

  # compiler front-half
  types/              # SemTy (semantic) + Repr (physical) — the two-layer split
  hir/                # High-level IR (CFG)
  semantics/          # name resolution + class collection
  typeck/             # one constraint-based type inference
  mir/                # representation-typed Mid-level IR + verifier
  lowering/           # HIR → MIR, with the single legalize/coercion pass
  optimizer/          # passes over typed MIR (inline, devirt, constfold, …)
  codegen-cranelift/  # typed MIR → native code
  frontend-python/    # parse + desugar → HIR
  cli/                # the `pyaot` binary; orchestrates the pipeline + linker

corpus/               # .py files: the CPython differential-test gate
benchmarks/           # perf harness vs CPython
```

Every compiler crate is `#![forbid(unsafe_code)]`; only `runtime` uses `unsafe`.

## The correctness discipline

There are no hand-written expected-output fixtures. The
[`corpus/`](corpus) is **38 consolidated `test_*.py` categories plus the
`microgpt.py` capstone**, each one ordinary Python full of `assert`s. The
differential harness (`crates/cli/tests/differential.rs`) compiles every entry
with `pyaot`, runs the binary, runs the *same file* under `python3`, and
compares the two stdouts **byte for byte** — CPython is the live oracle.

A Python feature is only considered done once it is in this gate and matches
CPython exactly. That makes the corpus the executable specification, and
`cargo test -p pyaot-cli --test differential` the single source of truth for
"does it still work".

## Build

```bash
cargo check --workspace --exclude pyaot-runtime   # fast: type-check the front-half
cargo build -p pyaot-runtime                      # build the runtime staticlib
cargo build --workspace                           # everything
```

The runtime staticlib (`libpyaot_runtime.a`) must exist before you compile a
script, so the linker can find the `rt_*` symbols.

## Usage

```bash
pyaot script.py                 # → ./script  (output defaults to the input stem)
pyaot script.py -o build/app    # explicit output path
pyaot script.py --run           # compile, then run it (propagates the exit code)
pyaot script.py --run -v        # -v prints each pipeline stage + timing to stderr
```

Common flags (`pyaot --help` for the full list):

| Flag | Effect |
|---|---|
| `-o, --output <PATH>` | Output executable. Defaults to the input path with its extension stripped (`foo.py` → `foo`). |
| `--run` | Run the compiled executable after a successful link, propagating its exit code. |
| `-O, --optimize` | Enable optimizations (alias for `--opt-level speed`). |
| `--opt-level <none\|speed\|speed-and-size>` | Optimization level. `speed` is the default; `none` is fully conservative. `speed-and-size` adds a post-link `strip` for minimal binary size (~8–10 ms slower per compile). |
| `--debug` | Keep debug symbols / DWARF; also defaults `--opt-level` to `none` unless one is given explicitly. |
| `--module-path <DIR>` | Extra import search directory (repeatable); tried after the entry script's own directory. |
| `--emit-hir` / `--emit-types` / `--emit-mir` | Dump the resolved HIR / typed HIR / verified MIR to stdout and exit (no codegen). |
| `-v, --verbose` | Print each pipeline stage, its duration, and the total to stderr. |
| `--runtime-lib <PATH>` | Path to `libpyaot_runtime.a` (overrides auto-detection). |

### External packages (`site-packages/`)

Pure-Python packages written against the supported stdlib subset can be dropped
into a `site-packages/` directory and imported like any third-party library —
they are discovered and compiled exactly like a user `.py` import (no separate
native-package path). The repo bundles a `requests` facade
(`site-packages/requests`, a thin wrapper over `urllib.request`) as a worked
example:

```python
import requests
resp = requests.get("https://api.example.com/items", params={"q": "x"})
print(resp.status_code, resp.text)
```

Search roots are tried in this order, first match wins (so a user module always
shadows a same-named package):

1. the entry script's directory, then any `--module-path <DIR>`;
2. `$PYAOT_SITE_PACKAGES` — a `PATH`-style (`:`-separated) list of extra roots;
3. `<exe_dir>/site-packages` next to the `pyaot` binary;
4. `<repo_root>/site-packages` (the bundled packages, baked in for dev builds).

## Slim runtime (binary size)

The runtime's stdlib surface is feature-gated. The default build enables
`stdlib-full` (= `stdlib-json`, `stdlib-regex`, `stdlib-crypto`,
`stdlib-base64`, `stdlib-network`); a script that uses none of those can link a
slim runtime:

```bash
# build the slim staticlib into its own target dir (don't clobber target/)
cargo build --release -p pyaot-runtime --no-default-features \
    --target-dir /tmp/pyaot_slim
# link against it
pyaot script.py -o script --runtime-lib /tmp/pyaot_slim/release/libpyaot_runtime.a
```

Re-enable individual features with `--features stdlib-json` etc. The linker
already dead-strips: on macOS arm64, a hello-world is ≈ 405 KB (full) vs
≈ 355 KB (slim). Compiling a script that *does* use `json`/`re`/`hashlib`/
`base64`/`urllib` against a slim runtime fails at link time with undefined
`rt_*` symbols — rebuild the runtime with the matching `--features`.

## Benchmarks

`benchmarks/run.sh` compiles each bench, validates its stdout against CPython
byte-for-byte, times both with [hyperfine](https://github.com/sharkdp/hyperfine),
and appends a table to `benchmarks/results.md`. See `benchmarks/README.md` for
the method and targets.

## Documentation

- **[ARCHITECTURE.md](ARCHITECTURE.md)** — the design, the seam, and the six invariants.
- **[PITFALLS.md](PITFALLS.md)** — AOT-Python traps and how this architecture avoids them. Read before touching any front-half crate.
- **[COMPILER_STATUS.md](COMPILER_STATUS.md)** — the per-feature ✅/🟡/❌ coverage matrix.
- Each crate's `lib.rs` doc comment states that crate's single responsibility.

## Status

All compiler phases through Phase 9 (optimization & polish) are implemented and
load-bearing — not scaffolds. The full differential corpus, including
`corpus/microgpt.py`, matches CPython byte-for-byte, on a MIR optimizer pipeline
with Cranelift `opt_level=speed` as the default.
