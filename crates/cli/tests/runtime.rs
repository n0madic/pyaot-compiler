//! Runtime integration tests.
//!
//! All runtime test cases are declared in this single file so test wiring
//! and expected-output policy stay centralized.

mod common;

/// Generate a standard runtime integration test.
macro_rules! runtime_case {
    ($test_name:ident, $source_file:literal) => {
        #[test]
        fn $test_name() {
            let py_path = crate::common::workspace_root()
                .join("examples")
                .join($source_file);
            crate::common::run_pyaot(stringify!($test_name), &py_path, None);
        }
    };
    ($test_name:ident, $source_file:literal, $expected:expr) => {
        #[test]
        fn $test_name() {
            let py_path = crate::common::workspace_root()
                .join("examples")
                .join($source_file);
            crate::common::run_pyaot(stringify!($test_name), &py_path, Some($expected));
        }
    };
}

/// Declare all runtime test cases in one place.
macro_rules! runtime_cases {
    ($(
        ($name:ident, $source:literal $(, $expected:expr)?)
    ),* $(,)?) => {
        $(
            runtime_case!($name, $source $(, $expected)?);
        )*
    };
}

runtime_cases!(
    // Core language
    (runtime_main, "test_main.py"),
    (runtime_core_types, "test_core_types.py"),
    (runtime_control_flow, "test_control_flow.py"),
    (runtime_functions, "test_functions.py"),
    (runtime_classes, "test_classes.py"),
    (runtime_strings, "test_strings.py"),
    (runtime_types_system, "test_types_system.py"),
    (runtime_exceptions, "test_exceptions.py"),
    (runtime_generators, "test_generators.py"),
    (runtime_iteration, "test_iteration.py"),
    (runtime_match, "test_match.py"),
    (runtime_global_scoping, "test_global_scoping.py"),
    (runtime_gc_simple, "test_gc_simple.py"),
    (runtime_multi_except, "test_multi_except.py"),
    (runtime_dead_code_warnings, "test_dead_code_warnings.py"),
    (runtime_decorator_factory, "test_decorator_factory.py"),
    (runtime_builtin_first_class, "test_builtin_first_class.py"),
    // Collections
    (
        runtime_collections_list_tuple,
        "test_collections_list_tuple.py"
    ),
    (
        runtime_collections_dict_set_bytes,
        "test_collections_dict_set_bytes.py"
    ),
    // Builtins
    (runtime_builtins, "test_builtins.py"),
    // Print output (with expected output verification)
    (
        runtime_print_output,
        "test_print_output.py",
        "42\n\
         -7\n\
         0\n\
         3.14\n\
         1\n\
         0\n\
         True\n\
         False\n\
         None\n\
         hello\n\
         \n\
         \n\
         1 2 3\n\
         a b c\n\
         1 hello True 3.14\n\
         1 None 2\n\
         1-2-3\n\
         a, b, c\n\
         12\n\
         hello world\n\
         line1\n\
         line2\n\
         1-2-3!\n\
         [1, 2, 3]\n\
         ['a', 'b']\n\
         []\n\
         (1, 2, 3)\n\
         (42,)\n\
         ('hello', 'world')\n\
         [[1, 2], [3, 4]]\n\
         {}\n\
         {'x': 1}\n\
         set()\n\
         {42}\n\
         b'hello'\n\
         b''\n\
         30\n\
         200\n\
         0\n\
         1\n\
         2\n\
         Hello World"
    ),
    // Import tests
    (runtime_import, "test_import.py"),
    (runtime_packages, "test_packages.py"),
    // Standard library
    (runtime_stdlib_json, "test_stdlib_json.py"),
    (runtime_stdlib_math, "test_stdlib_math.py"),
    (runtime_stdlib_os, "test_stdlib_os.py"),
    (runtime_stdlib_random, "test_stdlib_random.py"),
    (runtime_stdlib_re, "test_stdlib_re.py"),
    (runtime_stdlib_subprocess, "test_stdlib_subprocess.py"),
    (runtime_stdlib_sys, "test_stdlib_sys.py"),
    (runtime_stdlib_time, "test_stdlib_time.py"),
    (runtime_stdlib_urllib, "test_stdlib_urllib.py"),
    // File I/O
    (runtime_file_io, "test_file_io.py"),
);
