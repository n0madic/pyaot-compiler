//! Runtime integration tests.
//!
//! All runtime test cases are declared in this single file so test wiring
//! and expected-output policy stay centralized.

mod common;

/// Generate a standard runtime integration test.
macro_rules! runtime_case {
    // No expected output, no diffs
    ($test_name:ident, $source_file:literal) => {
        #[test]
        fn $test_name() {
            let py_path = crate::common::workspace_root()
                .join("examples")
                .join($source_file);
            crate::common::run_pyaot(stringify!($test_name), &py_path, None, &[]);
        }
    };
    // With allowed_diffs only
    ($test_name:ident, $source_file:literal, allowed_diffs: [$( ($pyaot:expr, $cpython:expr, $reason:expr) ),* $(,)?]) => {
        #[test]
        fn $test_name() {
            let py_path = crate::common::workspace_root()
                .join("examples")
                .join($source_file);
            crate::common::run_pyaot(stringify!($test_name), &py_path, None, &[
                $(crate::common::AllowedDiff {
                    pyaot_contains: $pyaot,
                    cpython_contains: $cpython,
                    reason: $reason,
                }),*
            ]);
        }
    };
    // With expected output only
    ($test_name:ident, $source_file:literal, $expected:expr) => {
        #[test]
        fn $test_name() {
            let py_path = crate::common::workspace_root()
                .join("examples")
                .join($source_file);
            crate::common::run_pyaot(stringify!($test_name), &py_path, Some($expected), &[]);
        }
    };
    // With expected output and allowed_diffs
    ($test_name:ident, $source_file:literal, $expected:expr, allowed_diffs: [$( ($pyaot:expr, $cpython:expr, $reason:expr) ),* $(,)?]) => {
        #[test]
        fn $test_name() {
            let py_path = crate::common::workspace_root()
                .join("examples")
                .join($source_file);
            crate::common::run_pyaot(stringify!($test_name), &py_path, Some($expected), &[
                $(crate::common::AllowedDiff {
                    pyaot_contains: $pyaot,
                    cpython_contains: $cpython,
                    reason: $reason,
                }),*
            ]);
        }
    };
}

/// Declare all runtime test cases in one place.
macro_rules! runtime_cases {
    ($(
        ($name:ident, $source:literal $($rest:tt)*)
    ),* $(,)?) => {
        $(
            runtime_case!($name, $source $($rest)*);
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
    (runtime_generics, "test_generics.py"),
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
        "test_collections_dict_set_bytes.py",
        allowed_diffs: [
            ("{'", "{'", "set print order is non-deterministic"),
        ]
    ),
    (runtime_collections, "test_collections.py"),
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
         1.0\n\
         0.0\n\
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
    (runtime_stdlib_os, "test_stdlib_os.py", allowed_diffs: [
        ("environ keys count:", "environ keys count:", "environ count differs between runtimes"),
        ("Current directory:", "Current directory:", "cwd may differ between runtimes"),
    ]),
    (runtime_stdlib_random, "test_stdlib_random.py"),
    (runtime_stdlib_re, "test_stdlib_re.py"),
    (runtime_stdlib_subprocess, "test_stdlib_subprocess.py"),
    (runtime_stdlib_sys, "test_stdlib_sys.py", allowed_diffs: [
        ("pyaot_test_", ".py", "sys.argv[0] is executable path vs script path"),
    ]),
    (runtime_stdlib_time, "test_stdlib_time.py"),
    (runtime_stdlib_urllib, "test_stdlib_urllib.py"),
    // File I/O
    (runtime_file_io, "test_file_io.py"),
    // Tracebacks
    (
        runtime_traceback,
        "test_traceback.py",
        "caught ZeroDivisionError\n\
         skip zero\n\
         100\n\
         50\n\
         inner caught\n\
         after inner try\n\
         outer ok\n\
         done"
    ),
);

// -O (full optimisation) regression cases. Default-opts run reaches every
// codegen path; these add coverage for optimizer/codegen interaction —
// especially the `--inline` pass, which the default suite does not
// exercise. Stage E (closure ABI) introduced an inline-pass bug where
// the moved CallDirect terminator's target Phi sources kept referring
// to the old call_block_id, causing
// "phi has no source for predecessor block — arity violation" at
// codegen time. Re-running the full decorator-factory suite under -O
// catches that regression.
#[test]
fn runtime_decorator_factory_optimized() {
    let py_path = crate::common::workspace_root()
        .join("examples")
        .join("test_decorator_factory.py");
    crate::common::run_pyaot_optimized("runtime_decorator_factory_optimized", &py_path, None, &[]);
}

#[test]
fn runtime_functions_optimized() {
    let py_path = crate::common::workspace_root()
        .join("examples")
        .join("test_functions.py");
    crate::common::run_pyaot_optimized("runtime_functions_optimized", &py_path, None, &[]);
}

#[test]
fn runtime_iteration_optimized() {
    let py_path = crate::common::workspace_root()
        .join("examples")
        .join("test_iteration.py");
    crate::common::run_pyaot_optimized("runtime_iteration_optimized", &py_path, None, &[]);
}

#[test]
fn runtime_builtins_optimized() {
    let py_path = crate::common::workspace_root()
        .join("examples")
        .join("test_builtins.py");
    crate::common::run_pyaot_optimized("runtime_builtins_optimized", &py_path, None, &[]);
}
