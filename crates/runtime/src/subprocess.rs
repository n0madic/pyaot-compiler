//! subprocess module runtime support
//!
//! Provides subprocess.run() for spawning and managing processes.
//!
//! # Security note
//! Uses exec-style invocation (not shell), so shell injection is not possible.
//! However, compiled programs can execute arbitrary system binaries with the same
//! privileges as the process. This is by design — compiled Python programs have
//! full OS access, matching CPython's behavior.

use crate::gc::{self, ShadowFrame};
use crate::object::{CompletedProcessObj, ListObj, Obj, ObjHeader, TypeTagKind};
use crate::utils::make_str_from_rust;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Default maximum wall-clock time allowed for a subprocess before it is killed.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Polling interval used while waiting for a subprocess to exit.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Extract list of strings from ListObj
unsafe fn extract_string_list(obj: *mut Obj) -> Vec<String> {
    if obj.is_null() {
        return Vec::new();
    }

    let list_obj = obj as *mut ListObj;
    let len = (*list_obj).len;

    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        let item = crate::list::list_slot_raw(list_obj, i);
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
///
/// Raises TimeoutError if the subprocess does not exit within DEFAULT_TIMEOUT (300 s).
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

        // Configure stdio redirection based on capture_output
        if capture_output != 0 {
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
        }

        // Spawn child process
        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                crate::raise_exc!(
                    crate::exceptions::ExceptionType::RuntimeError,
                    "subprocess.run: failed to execute command: {}",
                    e
                );
            }
        };

        // Poll for completion, enforcing DEFAULT_TIMEOUT to prevent hangs.
        let start = Instant::now();
        let output = loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    // Process finished; collect output.
                    match child.wait_with_output() {
                        Ok(out) => break out,
                        Err(e) => {
                            crate::raise_exc!(
                                crate::exceptions::ExceptionType::RuntimeError,
                                "subprocess.run: failed to collect output: {}",
                                e
                            );
                        }
                    }
                }
                Ok(None) => {
                    // Process still running; check timeout.
                    if start.elapsed() >= DEFAULT_TIMEOUT {
                        // Kill the child before raising so it doesn't become a zombie.
                        child.kill().ok();
                        // Reap the child to release OS resources.
                        child.wait().ok();
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::TimeoutError,
                            "subprocess.run: command '{}' timed out after {} seconds",
                            cmd_args.join(" "),
                            DEFAULT_TIMEOUT.as_secs()
                        );
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(e) => {
                    crate::raise_exc!(
                        crate::exceptions::ExceptionType::RuntimeError,
                        "subprocess.run: wait failed: {}",
                        e
                    );
                }
            }
        };

        // Get exit code
        let returncode = output.status.code().unwrap_or(-1) as i64;

        // Check if we should raise on non-zero exit
        if check != 0 && returncode != 0 {
            crate::raise_exc!(
                crate::exceptions::ExceptionType::RuntimeError,
                "Command '{}' returned non-zero exit status {}",
                cmd_args.join(" "),
                returncode
            );
        }

        // Allocate stdout/stderr/args_copy and the CompletedProcess object.
        // Each of these is a GC allocation, so each can trigger a collection that
        // would free any previously allocated heap object not on the shadow stack.
        // We root stdout_obj, stderr_obj, and args_copy in a frame to keep them live.
        // roots[0] = stdout_obj, roots[1] = stderr_obj, roots[2] = args_copy
        let mut roots: [*mut Obj; 3] = [
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 3,
            roots: roots.as_mut_ptr(),
        };
        gc::gc_push(&mut frame);

        // Create stdout string if captured
        roots[0] = if capture_output != 0 {
            match String::from_utf8(output.stdout) {
                Ok(s) => make_str_from_rust(&s),
                Err(e) => {
                    gc::gc_pop();
                    crate::raise_exc!(
                        crate::exceptions::ExceptionType::ValueError,
                        "subprocess stdout is not valid UTF-8: {}",
                        e.utf8_error()
                    );
                }
            }
        } else {
            crate::object::none_obj()
        };

        // Create stderr string if captured — stdout_obj is now rooted at roots[0].
        roots[1] = if capture_output != 0 {
            match String::from_utf8(output.stderr) {
                Ok(s) => make_str_from_rust(&s),
                Err(e) => {
                    gc::gc_pop();
                    crate::raise_exc!(
                        crate::exceptions::ExceptionType::ValueError,
                        "subprocess stderr is not valid UTF-8: {}",
                        e.utf8_error()
                    );
                }
            }
        } else {
            crate::object::none_obj()
        };

        // Copy args list — stdout and stderr are rooted at roots[0..1].
        roots[2] = crate::list::rt_list_copy(args);

        // Create CompletedProcess object — stdout, stderr, and args_copy all rooted.
        let size = std::mem::size_of::<CompletedProcessObj>();
        let ptr =
            gc::gc_alloc(size, TypeTagKind::CompletedProcess as u8) as *mut CompletedProcessObj;

        gc::gc_pop();

        (*ptr).header = ObjHeader {
            type_tag: TypeTagKind::CompletedProcess,
            marked: false,
            size,
        };
        (*ptr).args = roots[2];
        (*ptr).returncode = returncode;
        (*ptr).stdout = roots[0];
        (*ptr).stderr = roots[1];

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
