# pyaot-compiler

A static **AOT compiler for a typed subset of Python 3** → native executables,
built on Cranelift.

Goal: compile real, idiomatic Python scripts (e.g. `microgpt.py`) unchanged or
with minimal changes *within standard Python syntax*. Arbitrary-precision `int`.
Dynamic features incompatible with AOT (`eval`/`exec`, metaclasses, `__dict__`,
dynamic attribute names, `import *`) are out of scope by design.

- **[ARCHITECTURE.md](ARCHITECTURE.md)** — the design and the six invariants.
- **[PITFALLS.md](PITFALLS.md)** — known traps in AOT-compiling Python, and how
  this architecture avoids them. Read before building any front-half crate.
- **[PLAN.md](PLAN.md)** — the phased roadmap to a working compiler.

## Layout

```
crates/
  # substrate + runtime contract — stable, changed deliberately when compiler development requires (e.g. bignum); see ARCHITECTURE.md
  core-defs/  format-shared/  utils/  diagnostics/  linker/  stdlib-defs/  runtime/
  # compiler front-half (built fresh from the design)
  types/             # SemTy (semantic) + Repr (physical) — the two-layer split  [implemented]
  hir/  semantics/  typeck/  mir/  lowering/  optimizer/  codegen-cranelift/  frontend-python/  cli/   [scaffolds]
corpus/              # .py files: the CPython differential-test gate
```

## Build

```bash
cargo check --workspace --exclude pyaot-runtime   # fast: type-check the front-half
cargo build -p pyaot-runtime                      # build the runtime staticlib
cargo build --workspace                           # everything
```

## Usage

```bash
pyaot script.py                 # → ./script (output defaults to the input stem)
pyaot script.py -o build/app    # explicit output path
pyaot script.py --run           # compile, then run it (propagates the exit code)
```

Common flags (`pyaot --help` for the full list):

| Flag | Effect |
|---|---|
| `-o, --output <PATH>` | Output executable. Defaults to the input path with its extension stripped (`foo.py` → `foo`). |
| `--run` | Run the compiled executable after a successful link, propagating its exit code. |
| `-O, --optimize` | Enable optimizations (alias for `--opt-level speed`). |
| `--opt-level <none\|speed\|speed-and-size>` | Optimization level. `speed` is the default; `none` is fully conservative. |
| `--debug` | Keep debug symbols / DWARF; also defaults `--opt-level` to `none` unless one is given explicitly. |
| `--module-path <DIR>` | Extra import search directory (repeatable); tried after the entry script's own directory. |
| `--emit-hir` / `--emit-types` / `--emit-mir` | Dump the resolved HIR / typed HIR / verified MIR to stdout and exit (no codegen). |
| `-v, --verbose` | Print each pipeline stage to stderr. |
| `--runtime-lib <PATH>` | Path to `libpyaot_runtime.a` (overrides auto-detection). |

## Slim runtime (binary size)

The runtime's stdlib surface is feature-gated. The default build enables
`stdlib-full` (= `stdlib-json`, `stdlib-regex`, `stdlib-crypto`,
`stdlib-base64`, `stdlib-network`); scripts that use none of those can link a
slim runtime:

```bash
# build the slim staticlib into its own target dir (don't clobber target/)
cargo build --release -p pyaot-runtime --no-default-features \
    --target-dir /tmp/pyaot_slim
# link against it
pyaot script.py -o script --runtime-lib /tmp/pyaot_slim/release/libpyaot_runtime.a
```

Re-enable individual features with `--features stdlib-json` etc.
`benchmarks/size.sh` measures the difference on a hello-world (the linker
already dead-strips: full ≈ 405 KB, slim ≈ 355 KB executable on macOS arm64).

Known failure mode: compiling a script that *does* use `json` / `re` /
`hashlib` / `base64` / `urllib` against a slim runtime fails at link time
with undefined `rt_*` symbols — rebuild the runtime with the matching
`--features` instead.

## Benchmarks

`benchmarks/run.sh` compiles each bench, validates its stdout against
CPython byte-for-byte, times both with hyperfine, and appends a table to
`benchmarks/results.md`. See `benchmarks/README.md` for the method and
targets.

## Status

All compiler phases through Phase 9 (optimization & polish) are implemented:
the full differential corpus — including `corpus/microgpt.py` — matches
CPython byte-for-byte, with a MIR optimizer pipeline (inline, constant
folding, peephole, DCE, cold-block layout) and Cranelift `opt_level=speed`
as the default.
