//! Registry of third-party packages available to the compiler.
//!
//! Mirrors the role of `pyaot-stdlib-defs` but for packages that live in
//! separate crates under `crates/pkg/<name>/`. Each package is a workspace
//! crate that both exposes a `StdlibModuleDef` (compile-time metadata for the
//! compiler) and produces a `staticlib` (the `.a` file linked into the
//! user's binary only when the Python source imports the package).
//!
//! Currently empty — see `site-packages/` for Python-based packages that use
//! the ordinary module-discovery path instead of this registry. This crate
//! stays because the selective-linking infrastructure it drives (Linker
//! `extra_archives`, `hir::Module::used_packages`) remains the right design
//! for future packages that genuinely need native Rust code (e.g. numeric
//! libraries with BLAS).
//!
//! The schema piggybacks on `StdlibModuleDef` through type aliases so
//! packages describe themselves with the same declarative format as the
//! standard library. The aliases provide a stable surface for the compiler:
//! when the schemas need to diverge in the future, we can introduce a
//! dedicated `PackageModuleDef` type here without touching call sites.

#![forbid(unsafe_code)]

// Re-exported aliases so the rest of the compiler refers to "packages" as a
// distinct concept even though the underlying layout is shared with stdlib.
pub type PackageModuleDef = pyaot_stdlib_defs::StdlibModuleDef;
pub type PackageFunctionDef = pyaot_stdlib_defs::StdlibFunctionDef;
pub type PackageAttrDef = pyaot_stdlib_defs::StdlibAttrDef;
pub type PackageConstDef = pyaot_stdlib_defs::StdlibConstDef;
pub type PackageClassDef = pyaot_stdlib_defs::StdlibClassDef;

/// All registered packages. Native Rust packages add themselves here via a
/// `#[cfg(feature = "...")]` entry (see `Cargo.toml` of this crate for the
/// feature list). Currently empty — `requests` lives in `site-packages/`
/// as pure Python.
pub static ALL_PACKAGES: &[&PackageModuleDef] = &[];

/// Look up a package module definition by exact name.
pub fn get_package(name: &str) -> Option<&'static PackageModuleDef> {
    ALL_PACKAGES.iter().copied().find(|m| m.name == name)
}

/// Check whether `name` (or its root segment, for dotted imports) names a
/// registered package.
pub fn is_package(name: &str) -> bool {
    if get_package(name).is_some() {
        return true;
    }
    let root = get_root_package(name);
    ALL_PACKAGES
        .iter()
        .any(|m| m.name.split('.').next().unwrap_or(m.name) == root)
}

/// Extract the root segment of a dotted module path (e.g. `"requests.auth"`
/// -> `"requests"`). Kept as a free function to mirror the stdlib registry.
pub fn get_root_package(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

/// Iterate over the unique top-level package names. Used by the CLI to map
/// recorded imports onto archive file names on disk.
pub fn all_package_root_names() -> impl Iterator<Item = &'static str> {
    ALL_PACKAGES
        .iter()
        .map(|m| m.name.split('.').next().unwrap_or(m.name))
}

/// Look up a named item (function / attr / constant / class) in a registered
/// package. Mirrors `pyaot_stdlib_defs::get_item` so call sites that handle
/// stdlib items can fall through to packages with the same match arms.
pub fn get_item(module_name: &str, item_name: &str) -> Option<pyaot_stdlib_defs::StdlibItem> {
    let module = get_package(module_name)?;
    if let Some(func) = module.get_function(item_name) {
        return Some(pyaot_stdlib_defs::StdlibItem::Function(func));
    }
    if let Some(attr) = module.get_attr(item_name) {
        return Some(pyaot_stdlib_defs::StdlibItem::Attr(attr));
    }
    if let Some(cnst) = module.get_constant(item_name) {
        return Some(pyaot_stdlib_defs::StdlibItem::Constant(cnst));
    }
    module
        .get_class(item_name)
        .map(pyaot_stdlib_defs::StdlibItem::Class)
}

/// List all names defined in a package module. Mirrors
/// `pyaot_stdlib_defs::list_all_names` for error messages.
pub fn list_all_names(module_name: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    if let Some(m) = get_package(module_name) {
        out.extend(m.functions.iter().map(|f| f.name));
        out.extend(m.attrs.iter().map(|a| a.name));
        out.extend(m.constants.iter().map(|c| c.name));
        out.extend(m.classes.iter().map(|c| c.name));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_currently_empty() {
        // Sanity: the Rust-pkg registry holds no entries right now
        // (`requests` moved to site-packages/, future native pkgs will
        // register themselves here).
        assert!(ALL_PACKAGES.is_empty());
    }

    #[test]
    fn unknown_package_is_none() {
        assert!(get_package("definitely_not_a_real_package").is_none());
        assert!(!is_package("definitely_not_a_real_package"));
    }
}
