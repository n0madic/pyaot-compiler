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
3. Avoid cross-module user-class return-type reliance. The compiler currently doesn't propagate user-function return types across module boundaries — callers of your package functions may need explicit annotations. Prefer returning **stdlib RuntimeObject types** (like `HTTPResponse`) so attributes resolve automatically.
4. Avoid default arguments on public functions if cross-module callers matter — callers must currently pass all positional args explicitly. Track this limitation in a follow-up.
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
