---
paths:
  - "crates/pkg/**"
  - "site-packages/**"
---

# Package Development

Two categories of bundled packages exist. **Prefer Python unless you genuinely need native code.**

## Python-based packages (`site-packages/<name>/`)

Pure Python modules shipped with the compiler. They compile through the same Python→native pipeline as user code, can import anything from the stdlib, and need no Rust changes when they only recombine existing features.

Layout:
```
site-packages/<name>/__init__.py
```

The CLI auto-adds `site-packages/` to the module search path. Two candidate roots are probed (both are optional, used only if present):
1. `<exe_dir>/site-packages` — for installed/copied binaries
2. `<repo_root>/site-packages` — dev fallback, baked in at compile time via `env!("CARGO_MANIFEST_DIR")`

Users can still override or extend via `--module-path DIR`. See `site-packages/requests/` as the reference example (HTTP client built on top of `urllib.request`).

**Authoring checklist:**
1. Write ordinary Python with type annotations — classes, `@property`, stdlib imports all work.
2. **Stay CPython-compatible.** If a runtime capability is missing, extend the **stdlib in the standard CPython shape**, never add pyaot-specific functions. Example: `requests` needs per-method HTTP dispatch — the CPython way is `Request(url, data, headers, method=...) → urlopen(req)`, so we added `urllib.request.Request` as a runtime type and extended `urlopen` to accept it. A pyaot-only `http_request()` function would break on real CPython and is not allowed.
3. **Cross-module user classes are full citizens — including in type annotations.** A function like `def get(...) -> Response` from another module is valid — callers can read fields (`resp.status_code`), call methods (`resp.ok()`), AND annotate with the imported class (`r: Response = ...`, `r: mymod.Response = ...`, `def f(x: mymod.Foo) -> Foo`). Mechanism: `Type::Class` round-trips through `mir_merger::type_to_raw/raw_to_type` for exports; cross-module type-annotation placeholders get a unique id at parse time (`AstToHir::alloc_external_class_ref`) and `mir_merger` rewrites them to the real remapped ids before lowering (see `resolve_external_class_refs`). Stdlib `RuntimeObject` types (`HTTPResponse`, `StructTime`) still take a simpler path via the stdlib registry.
4. Default arguments on public functions are supported for callers in other modules — see `pyaot_lowering::SimpleDefault` in `crates/lowering/src/context/mod.rs`. Only `None`, `int`, `float`, `bool`, and `str` constants are eligible; complex defaults still require explicit args at the callsite.
5. **Optional[Heap] None-handling.** Python boxes `None` as the `NoneObj` singleton. Runtime functions accepting `Optional[Heap]` must accept BOTH null pointer (from default-filled stdlib calls) and `NoneObj` (from explicit user-level `None`). Use `crate::utils::is_none_or_null(obj)` instead of `obj.is_null()`.

## Rust-based packages (`crates/pkg/<name>/`)

Reserved for packages that genuinely need native Rust (BLAS, FFI to C libraries, hot loops, or hardware access). The selective-linking infrastructure lives in:
- `pyaot-pkg-defs` — registry with type aliases over `StdlibModuleDef`
- `crates/linker/src/lib.rs` — `Linker::link(..., extra_archives)`
- `hir::Module::used_packages` — tracked at import resolution

Currently the registry is empty — `requests` moved to `site-packages/` as Python.

**Authoring checklist (Rust pkg):**
1. Create `crates/pkg/<name>/` with `crate-type = ["staticlib", "rlib"]`.
2. Depend on `pyaot-core-defs` and `pyaot-stdlib-defs`.
3. Export `<NAME>_MODULE: StdlibModuleDef` with metadata + `extern "C"` runtime functions. Reuse existing `TypeSpec`/`TypeTagKind` values where possible.
4. Register under a `#[cfg(feature = "pkg-<name>")]` entry in `pyaot-pkg-defs::ALL_PACKAGES`.
5. Add to workspace `members` in the root `Cargo.toml`.

No changes in `frontend-python`, `lowering`, `codegen-cranelift`, `linker`, or `cli` — the pipeline resolves packages through `pyaot_pkg_defs::get_item` and handles archive linking via `hir::Module::used_packages`.

## Cross-cutting guidance

- **`stdlib-network` depends on `stdlib-json`** so HTTPResponse's `.json()` always works when network is enabled.
- When adding fields/methods to a stdlib RuntimeObject (e.g. HTTPResponse), extend `crates/stdlib-defs/src/object_types.rs` + implement the getter in the relevant runtime module. No lowering/codegen changes needed.
