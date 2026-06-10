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
  # frozen substrate + contract — frozen by default, yields when the plan needs it (e.g. bignum); see ARCHITECTURE.md
  core-defs/  format-shared/  utils/  diagnostics/  linker/  stdlib-defs/  runtime/
  # compiler front-half (built fresh from the design)
  types/             # SemTy (semantic) + Repr (physical) — the two-layer split  [implemented]
  hir/  semantics/  typeck/  mir/  lowering/  optimizer/  codegen-cranelift/  frontend-python/  cli/   [scaffolds]
corpus/              # .py files: the CPython differential-test gate
```

## Build

```bash
cargo check --workspace --exclude pyaot-runtime   # fast: type-check the front-half
cargo build -p pyaot-runtime                      # build the frozen runtime staticlib
cargo build --workspace                           # everything
```

## Status

Skeleton: substrate builds, `types` is implemented, the rest are typed scaffolds
documenting their responsibility.
