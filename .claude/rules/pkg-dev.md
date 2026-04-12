---
paths:
  - "crates/pkg/**"
---

# Third-Party Package Development

Packages are shipped as independent workspace crates under `crates/pkg/<name>/`. Unlike stdlib modules (which live inside `pyaot-runtime`), each package is a separate `staticlib+rlib` crate whose `.a` archive is linked into the user's binary **only when the compiled Python source imports the package**.

## Adding a Package

1. **Create the crate** at `crates/pkg/<name>/` with `crate-type = ["staticlib", "rlib"]`. Depend on `pyaot-core-defs` and `pyaot-stdlib-defs` (for the metadata types).
2. **Export `<NAME>_MODULE: StdlibModuleDef`** with the package's public functions, attrs, constants, classes. Reuse existing `TypeSpec` / `TypeTagKind` values whenever possible so no new lowering/codegen support is needed.
3. **Implement runtime functions** as `extern "C"` symbols in the same crate. For features that need runtime helpers (`gc_alloc`, `rt_make_str`, `rt_urlopen`, ...) declare them via `extern "C"` — they'll be resolved at final link time against the main runtime.
4. **Register in `crates/pkg/defs/`**: add the package as an optional dependency (feature-gated) and insert a `#[cfg(feature = "pkg-<name>")]` entry into `ALL_PACKAGES`.
5. **Add to workspace**: append `"crates/pkg/<name>"` to the `members` list in the root `Cargo.toml`.

No changes are required in `frontend-python`, `lowering`, `codegen-cranelift`, `linker`, or `cli` — the existing pipeline already resolves packages through `pyaot_pkg_defs::get_item` and tracks usage via `hir::Module::used_packages`, and the CLI maps each tracked name onto `libpyaot_pkg_<name>.a` adjacent to the runtime archive.

## Type Aliases (`pyaot-pkg-defs`)

External code refers to packages via the aliases `PackageModuleDef`, `PackageFunctionDef`, etc. The aliases currently equal their stdlib counterparts, but package authors should write their metadata using these names wherever practical so the eventual schema split is a one-crate change.

## Selective Linking

Whenever the frontend encounters `import <pkg>` or `from <pkg> import ...` where `<pkg>` is registered, it inserts the package's root name into `hir::Module::used_packages`. The CLI collects these across every parsed module and passes the corresponding archives to `Linker::link(..., extra_archives)`.

Scripts that never import a package do not cause its `.a` to be linked — verify by compiling a no-import script and grepping `nm` for `rt_<pkg>_` symbols.

## Runtime ABI (TODO)

Package crates currently redeclare runtime layouts/functions they need via `#[repr(C)]` struct dups and `extern "C"` declarations. When more than one non-trivial package exists, extract a `pyaot-runtime-abi` crate with the stable `extern "C"` surface and layout definitions so packages stop depending on layout invariants by convention.
