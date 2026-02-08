//! Tests for import extraction and resolution

use super::*;

#[test]
fn test_simple_import() {
    let source = "import mymodule";
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].module_path, "mymodule");
    assert_eq!(imports[0].level, 0);
}

#[test]
fn test_dotted_import() {
    let source = "import pkg.submodule";
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].module_path, "pkg.submodule");
    assert_eq!(imports[0].level, 0);
}

#[test]
fn test_from_import() {
    let source = "from pkg import name";
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].module_path, "pkg");
    assert_eq!(imports[1].module_path, "pkg.name");
}

#[test]
fn test_relative_import_single_dot() {
    let source = "from .utils import func";
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].module_path, "utils");
    assert_eq!(imports[0].level, 1);
    assert_eq!(imports[1].module_path, "utils.func");
    assert_eq!(imports[1].level, 1);
}

#[test]
fn test_relative_import_double_dot() {
    let source = "from .. import name";
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].module_path, "");
    assert_eq!(imports[0].level, 2);
    assert_eq!(imports[1].module_path, "name");
    assert_eq!(imports[1].level, 2);
}

#[test]
fn test_multi_line_parenthesized_import() {
    let source = r#"from pkg import (
    a,
    b,
    c
)"#;
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 4); // pkg, pkg.a, pkg.b, pkg.c
    assert_eq!(imports[0].module_path, "pkg");
    assert_eq!(imports[1].module_path, "pkg.a");
    assert_eq!(imports[2].module_path, "pkg.b");
    assert_eq!(imports[3].module_path, "pkg.c");
}

#[test]
fn test_multi_line_backslash_import() {
    let source = r#"from pkg import \
    a, \
    b"#;
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 3); // pkg, pkg.a, pkg.b
    assert_eq!(imports[0].module_path, "pkg");
    assert_eq!(imports[1].module_path, "pkg.a");
    assert_eq!(imports[2].module_path, "pkg.b");
}

#[test]
fn test_import_in_string_not_detected() {
    let source = r#"
x = "from fake import module"
y = 'import not_a_module'
"#;
    let imports = extract_imports_with_level(source).unwrap();
    assert!(imports.is_empty());
}

#[test]
fn test_import_in_docstring_not_detected() {
    let source = r#"
"""
Example usage:
from example import func
import mylib
"""
def foo():
    pass
"#;
    let imports = extract_imports_with_level(source).unwrap();
    assert!(imports.is_empty());
}

#[test]
fn test_import_in_comment_not_detected() {
    // Note: Comments are stripped by the parser, so this tests that
    // commented imports are not detected as real imports
    let source = r#"
# from commented import module
# import not_real
x = 1
"#;
    let imports = extract_imports_with_level(source).unwrap();
    assert!(imports.is_empty());
}

#[test]
fn test_stdlib_filtering() {
    let source = r#"
import typing
import sys
import os
import re
import json
import mymodule
from typing import List
from mypackage import func
"#;
    let imports = extract_imports_with_level(source).unwrap();
    // Only mymodule and mypackage should be detected
    assert_eq!(imports.len(), 3); // mymodule, mypackage, mypackage.func
    assert!(imports.iter().any(|i| i.module_path == "mymodule"));
    assert!(imports.iter().any(|i| i.module_path == "mypackage"));
    assert!(imports.iter().any(|i| i.module_path == "mypackage.func"));
}

#[test]
fn test_conditional_import_in_if() {
    let source = r#"
if True:
    import conditional_module
"#;
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].module_path, "conditional_module");
}

#[test]
fn test_conditional_import_in_try() {
    let source = r#"
try:
    import optional_module
except ImportError:
    import fallback_module
"#;
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 2);
    assert!(imports.iter().any(|i| i.module_path == "optional_module"));
    assert!(imports.iter().any(|i| i.module_path == "fallback_module"));
}

#[test]
fn test_import_inside_function() {
    let source = r#"
def foo():
    import lazy_module
    from lazy_pkg import item
"#;
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 3);
    assert!(imports.iter().any(|i| i.module_path == "lazy_module"));
    assert!(imports.iter().any(|i| i.module_path == "lazy_pkg"));
    assert!(imports.iter().any(|i| i.module_path == "lazy_pkg.item"));
}

#[test]
fn test_import_inside_class() {
    let source = r#"
class MyClass:
    import class_module
"#;
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].module_path, "class_module");
}

#[test]
fn test_import_with_alias() {
    let source = r#"
import mymodule as mm
from pkg import name as n
"#;
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 3);
    assert_eq!(imports[0].module_path, "mymodule");
    assert_eq!(imports[1].module_path, "pkg");
    assert_eq!(imports[2].module_path, "pkg.name");
}

#[test]
fn test_wildcard_import() {
    let source = "from pkg import *";
    let imports = extract_imports_with_level(source).unwrap();
    // Wildcard should only add the module, not the *
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].module_path, "pkg");
}

#[test]
fn test_parse_error_returns_error() {
    let source = "from import syntax error";
    let result = extract_imports_with_level(source);
    assert!(result.is_err());
}

#[test]
fn test_resolve_relative_import_single_dot() {
    let result = resolve_relative_import("pkg.submodule.module", "utils", 1, false).unwrap();
    assert_eq!(result, "pkg.submodule.utils");
}

#[test]
fn test_resolve_relative_import_double_dot() {
    let result = resolve_relative_import("pkg.submodule.module", "other", 2, false).unwrap();
    assert_eq!(result, "pkg.other");
}

#[test]
fn test_resolve_relative_import_package() {
    let result = resolve_relative_import("pkg.submodule", "utils", 1, true).unwrap();
    assert_eq!(result, "pkg.submodule.utils");
}

#[test]
fn test_deeply_nested_imports() {
    let source = r#"
if True:
    try:
        if True:
            for x in []:
                while False:
                    import deeply_nested
    except:
        pass
"#;
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].module_path, "deeply_nested");
}

// ===== PEP 328 Relative Import Edge Cases =====

#[test]
fn test_resolve_relative_import_triple_dot_error() {
    // from ... import name in pkg.sub.mod - goes above top-level package
    // Level 3 from pkg.sub.mod: parent is pkg.sub, then pkg, then ??? = error
    let result = resolve_relative_import("pkg.sub.mod", "other", 3, false);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("above top-level package"));
}

#[test]
fn test_resolve_relative_import_triple_dot_deep_module() {
    // from ... import name in pkg.sub.deep.mod - goes to pkg
    // Level 3 from pkg.sub.deep.mod: parent is pkg.sub.deep, then pkg.sub, then pkg
    let result = resolve_relative_import("pkg.sub.deep.mod", "utils", 3, false).unwrap();
    assert_eq!(result, "pkg.utils");
}

#[test]
fn test_resolve_relative_import_quadruple_dot() {
    // from .... import name in pkg.sub.deep.very.mod
    // Level 4 from pkg.sub.deep.very.mod: parent is pkg.sub.deep.very, up to pkg
    let result = resolve_relative_import("pkg.sub.deep.very.mod", "utils", 4, false).unwrap();
    assert_eq!(result, "pkg.utils");
}

#[test]
fn test_resolve_relative_single_dot_no_module() {
    // from . import name in pkg.sub.mod -> pkg.sub
    let result = resolve_relative_import("pkg.sub.mod", "", 1, false).unwrap();
    assert_eq!(result, "pkg.sub");
}

#[test]
fn test_resolve_relative_double_dot_no_module() {
    // from .. import name in pkg.sub.mod -> pkg
    let result = resolve_relative_import("pkg.sub.mod", "", 2, false).unwrap();
    assert_eq!(result, "pkg");
}

#[test]
fn test_resolve_relative_import_package_double_dot() {
    // from .. import name in pkg.sub/__init__.py
    let result = resolve_relative_import("pkg.sub", "other", 2, true).unwrap();
    assert_eq!(result, "pkg.other");
}

#[test]
fn test_resolve_relative_import_package_single_dot_no_module() {
    // from . import name in pkg/__init__.py -> pkg
    let result = resolve_relative_import("pkg", "", 1, true).unwrap();
    assert_eq!(result, "pkg");
}

#[test]
fn test_resolve_relative_import_above_top_level() {
    // from .... import name in pkg.sub.mod (too many dots)
    let result = resolve_relative_import("pkg.sub.mod", "other", 4, false);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("above top-level package"));
}

#[test]
fn test_resolve_relative_import_top_level_script() {
    // from . import name in a top-level script (should fail)
    let result = resolve_relative_import("", "other", 1, false);
    assert!(result.is_err());
}

#[test]
fn test_resolve_relative_import_single_module_no_package() {
    // from . import name in a module.py not inside a package
    // This should fail as the module has no parent package
    let result = resolve_relative_import("script", "other", 1, false);
    // After popping, we have empty base - should error
    assert!(result.is_err());
}

#[test]
fn test_extract_relative_triple_dot() {
    let source = "from ...utils import func";
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].module_path, "utils");
    assert_eq!(imports[0].level, 3);
    assert_eq!(imports[1].module_path, "utils.func");
    assert_eq!(imports[1].level, 3);
}

#[test]
fn test_extract_relative_single_dot_no_module() {
    // from . import name
    let source = "from . import name";
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].module_path, "");
    assert_eq!(imports[0].level, 1);
    assert_eq!(imports[1].module_path, "name");
    assert_eq!(imports[1].level, 1);
}

#[test]
fn test_extract_relative_multi_import() {
    // from .utils import a, b, c
    let source = "from .utils import a, b, c";
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 4);
    assert_eq!(imports[0].module_path, "utils");
    assert_eq!(imports[0].level, 1);
    assert!(imports
        .iter()
        .any(|i| i.module_path == "utils.a" && i.level == 1));
    assert!(imports
        .iter()
        .any(|i| i.module_path == "utils.b" && i.level == 1));
    assert!(imports
        .iter()
        .any(|i| i.module_path == "utils.c" && i.level == 1));
}

#[test]
fn test_extract_relative_wildcard() {
    // from .utils import * - wildcard should not add submodule entry
    let source = "from .utils import *";
    let imports = extract_imports_with_level(source).unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].module_path, "utils");
    assert_eq!(imports[0].level, 1);
}

#[test]
fn test_rewrite_relative_single_dot() {
    let source = "from .utils import func";
    let result = rewrite_relative_imports(source, "pkg.sub.mod", false).unwrap();
    assert_eq!(result, "from pkg.sub.utils import func");
}

#[test]
fn test_rewrite_relative_double_dot() {
    let source = "from ..utils import func";
    let result = rewrite_relative_imports(source, "pkg.sub.mod", false).unwrap();
    assert_eq!(result, "from pkg.utils import func");
}

#[test]
fn test_rewrite_relative_single_dot_no_module() {
    let source = "from . import name";
    let result = rewrite_relative_imports(source, "pkg.sub.mod", false).unwrap();
    assert_eq!(result, "from pkg.sub import name");
}

#[test]
fn test_rewrite_relative_in_package_init() {
    let source = "from .utils import func";
    let result = rewrite_relative_imports(source, "pkg", true).unwrap();
    assert_eq!(result, "from pkg.utils import func");
}

#[test]
fn test_rewrite_preserves_indentation() {
    let source = "    from .utils import func";
    let result = rewrite_relative_imports(source, "pkg.mod", false).unwrap();
    assert_eq!(result, "    from pkg.utils import func");
}

#[test]
fn test_rewrite_error_on_invalid() {
    let source = "from .other import func";
    let result = rewrite_relative_imports(source, "script", false);
    assert!(result.is_err());
}
