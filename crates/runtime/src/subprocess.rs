//! subprocess module runtime support
//!
//! Provides subprocess.run() for spawning and managing processes.

use crate::gc;
use crate::object::{CompletedProcessObj, ListObj, Obj, ObjHeader, TypeTagKind};
use crate::utils::make_str_from_rust;
use std::process::Command;

/// Extract list of strings from ListObj
unsafe fn extract_string_list(obj: *mut Obj) -> Vec<String> {
    if obj.is_null() {
        return Vec::new();
    }

    let list_obj = obj as *const ListObj;
    let len = (*list_obj).len;
    let data = (*list_obj).data;

    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        let item = *data.add(i);
        if let Some(s) = crate::utils::extract_str_checked(item) {
            result.push(s);
        }
    }
    result
}

/// subprocess.run(args, capture_output=False, check=False) -> CompletedProcess
///
/// Executes a command and returns a CompletedProcess instance.
///
/// Parameters:
/// - args: list[str] - Command and arguments
/// - capture_output: bool - Whether to capture stdout/stderr (default: False)
/// - check: bool - Whether to raise RuntimeError on non-zero exit (default: False)
///
/// Returns CompletedProcess with:
/// - args: list[str] - The command that was run
/// - returncode: int - Exit status
/// - stdout: Optional[str] - Captured stdout if capture_output=True
/// - stderr: Optional[str] - Captured stderr if capture_output=True
#[no_mangle]
pub extern "C" fn rt_subprocess_run(args: *mut Obj, capture_output: i8, check: i8) -> *mut Obj {
    unsafe {
        // Extract command and arguments
        let cmd_args = extract_string_list(args);

        if cmd_args.is_empty() {
            crate::utils::raise_runtime_error("subprocess.run: args must not be empty");
        }

        let program = &cmd_args[0];
        let arguments = &cmd_args[1..];

        // Build command
        let mut command = Command::new(program);
        command.args(arguments);

        // Execute command
        let output = if capture_output != 0 {
            // Capture stdout and stderr
            match command.output() {
                Ok(output) => output,
                Err(e) => {
                    let msg = format!("subprocess.run: failed to execute command: {}", e);
                    crate::utils::raise_runtime_error(&msg);
                }
            }
        } else {
            // Don't capture output - inherit stdio
            match command.status() {
                Ok(status) => std::process::Output {
                    status,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                },
                Err(e) => {
                    let msg = format!("subprocess.run: failed to execute command: {}", e);
                    crate::utils::raise_runtime_error(&msg);
                }
            }
        };

        // Get exit code
        let returncode = output.status.code().unwrap_or(-1) as i64;

        // Check if we should raise on non-zero exit
        if check != 0 && returncode != 0 {
            let msg = format!(
                "subprocess.run: command '{}' returned non-zero exit status {}",
                cmd_args.join(" "),
                returncode
            );
            crate::utils::raise_runtime_error(&msg);
        }

        // Create stdout string if captured
        let stdout_obj = if capture_output != 0 {
            let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
            make_str_from_rust(&stdout_str)
        } else {
            crate::object::none_obj()
        };

        // Create stderr string if captured
        let stderr_obj = if capture_output != 0 {
            let stderr_str = String::from_utf8_lossy(&output.stderr).to_string();
            make_str_from_rust(&stderr_str)
        } else {
            crate::object::none_obj()
        };

        // Copy args list for the result object using the GC-managed list copy, which
        // allocates the data array via the system allocator in a way that list_finalize
        // will correctly free when the list is GC-collected.
        let args_copy = crate::list::rt_list_copy(args);

        // Create CompletedProcess object
        let size = std::mem::size_of::<CompletedProcessObj>();
        let ptr =
            gc::gc_alloc(size, TypeTagKind::CompletedProcess as u8) as *mut CompletedProcessObj;

        (*ptr).header = ObjHeader {
            type_tag: TypeTagKind::CompletedProcess,
            marked: false,
            size,
        };
        (*ptr).args = args_copy;
        (*ptr).returncode = returncode;
        (*ptr).stdout = stdout_obj;
        (*ptr).stderr = stderr_obj;

        ptr as *mut Obj
    }
}

// ============= CompletedProcess getter methods =============

/// Get args field from CompletedProcess object
#[no_mangle]
pub extern "C" fn rt_completed_process_get_args(obj: *mut Obj) -> *mut Obj {
    unsafe {
        debug_assert_type_tag!(
            obj,
            TypeTagKind::CompletedProcess,
            "rt_completed_process_get_args"
        );
        let cp_obj = obj as *mut CompletedProcessObj;
        (*cp_obj).args
    }
}

/// Get returncode field from CompletedProcess object
#[no_mangle]
pub extern "C" fn rt_completed_process_get_returncode(obj: *mut Obj) -> i64 {
    unsafe {
        debug_assert_type_tag!(
            obj,
            TypeTagKind::CompletedProcess,
            "rt_completed_process_get_returncode"
        );
        let cp_obj = obj as *mut CompletedProcessObj;
        (*cp_obj).returncode
    }
}

/// Get stdout field from CompletedProcess object
#[no_mangle]
pub extern "C" fn rt_completed_process_get_stdout(obj: *mut Obj) -> *mut Obj {
    unsafe {
        debug_assert_type_tag!(
            obj,
            TypeTagKind::CompletedProcess,
            "rt_completed_process_get_stdout"
        );
        let cp_obj = obj as *mut CompletedProcessObj;
        (*cp_obj).stdout
    }
}

/// Get stderr field from CompletedProcess object
#[no_mangle]
pub extern "C" fn rt_completed_process_get_stderr(obj: *mut Obj) -> *mut Obj {
    unsafe {
        debug_assert_type_tag!(
            obj,
            TypeTagKind::CompletedProcess,
            "rt_completed_process_get_stderr"
        );
        let cp_obj = obj as *mut CompletedProcessObj;
        (*cp_obj).stderr
    }
}
