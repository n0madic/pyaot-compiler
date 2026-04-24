//! os module runtime support
//!
//! Provides:
//! - os.environ: Environment variables as dict[str, str]
//! - os.path.join(path1, path2, ...): Join path components
//! - os.remove(path): Remove a file

use crate::gc::{gc_pop, gc_push, ShadowFrame};
use crate::object::{ListObj, Obj};
use crate::utils::make_str_from_rust;
use pyaot_core_defs::BuiltinExceptionKind;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Convert a Path to a UTF-8 str, raising ValueError on non-UTF-8 paths instead of silently
/// replacing bytes (which `to_string_lossy` does).
fn path_to_utf8_str(path: &Path) -> &str {
    path.to_str().unwrap_or_else(|| unsafe {
        raise_exc!(BuiltinExceptionKind::ValueError, "path is not valid UTF-8")
    })
}

/// Normalize a path by resolving `.` and `..` components without touching the filesystem.
/// This matches Python's os.path.normpath behaviour.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {
                // Skip '.' components — but preserve a leading one for relative paths
                if components.is_empty() {
                    // keep nothing; a completely empty result is fine
                }
            }
            Component::ParentDir => {
                // Pop a Normal component if possible, otherwise keep '..'
                match components.last() {
                    Some(Component::Normal(_)) => {
                        components.pop();
                    }
                    _ => {
                        components.push(component);
                    }
                }
            }
            _ => {
                components.push(component);
            }
        }
    }
    if components.is_empty() {
        PathBuf::from(".")
    } else {
        components.iter().collect()
    }
}

/// Get os.environ as a dict[str, str]
/// Creates a new dict each time (environment could have changed)
#[no_mangle]
pub extern "C" fn rt_os_get_environ() -> *mut Obj {
    let vars: Vec<(String, String)> = env::vars().collect();
    let count = vars.len();

    let dict = crate::dict::rt_make_dict(count as i64);

    // Root dict and an extra slot for the key across the value alloc.
    // Without rooting, every string allocation can trigger a GC that
    // collects `dict` (or a previously allocated key).
    let mut roots: [*mut Obj; 2] = [dict, std::ptr::null_mut()];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 2,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };

    for (key, value) in vars {
        let key_str = unsafe { make_str_from_rust(&key) };
        roots[1] = key_str; // root key across the value alloc
        let value_str = unsafe { make_str_from_rust(&value) };
        crate::dict::rt_dict_set(roots[0], roots[1], value_str);
        roots[1] = std::ptr::null_mut();
    }

    gc_pop();
    roots[0]
}

/// Join path components: os.path.join(path1, path2, ...)
/// Takes a list of string path components and joins them
#[no_mangle]
pub extern "C" fn rt_os_path_join(parts: *mut Obj) -> *mut Obj {
    unsafe {
        if parts.is_null() {
            return make_str_from_rust("");
        }

        let list_obj = parts as *mut ListObj;
        let len = (*list_obj).len;

        if len == 0 {
            return make_str_from_rust("");
        }

        // Start with empty path
        let mut path = PathBuf::new();

        // Join each component
        for i in 0..len {
            let elem = crate::list::list_slot_raw(list_obj, i);
            if let Some(s) = crate::utils::extract_str_checked(elem) {
                path.push(&s);
            }
        }

        // Convert result to string (raises ValueError on non-UTF-8 paths)
        let result = path_to_utf8_str(&path);
        make_str_from_rust(result)
    }
}

/// Remove a file: os.remove(path)
/// Raises FileNotFoundError, PermissionError, or IOError on failure
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_remove(path: *mut Obj) {
    unsafe {
        if path.is_null() {
            raise_exc!(BuiltinExceptionKind::IOError, "os.remove: path is None");
        }

        if let Some(path_str) = crate::utils::extract_str_checked(path) {
            match fs::remove_file(&path_str) {
                Ok(()) => {
                    // Success - no return value needed
                }
                Err(e) => match e.kind() {
                    std::io::ErrorKind::NotFound => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::FileNotFoundError,
                            "No such file or directory: '{}'",
                            path_str
                        );
                    }
                    std::io::ErrorKind::PermissionDenied => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::PermissionError,
                            "Permission denied: '{}'",
                            path_str
                        );
                    }
                    std::io::ErrorKind::IsADirectory => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::IOError,
                            "Is a directory: '{}'",
                            path_str
                        );
                    }
                    _ => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::IOError,
                            "Error removing file '{}': {}",
                            path_str,
                            e
                        );
                    }
                },
            }
        } else {
            raise_exc!(BuiltinExceptionKind::IOError, "os.remove: invalid path");
        }
    }
}

/// Check if a path exists: os.path.exists(path)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_path_exists(path: *mut Obj) -> i8 {
    unsafe {
        if path.is_null() {
            return 0;
        }
        if let Some(path_str) = crate::utils::extract_str_checked(path) {
            if std::path::Path::new(&path_str).exists() {
                1
            } else {
                0
            }
        } else {
            0
        }
    }
}

// ============= Working with current directory =============

/// Get current working directory: os.getcwd()
#[no_mangle]
pub extern "C" fn rt_os_getcwd() -> *mut Obj {
    unsafe {
        match env::current_dir() {
            Ok(path) => {
                let path_str = path_to_utf8_str(&path);
                make_str_from_rust(path_str)
            }
            Err(e) => {
                crate::raise_exc!(
                    crate::exceptions::ExceptionType::IOError,
                    "Error getting current directory: {}",
                    e
                );
            }
        }
    }
}

/// Change current working directory: os.chdir(path)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_chdir(path: *mut Obj) {
    unsafe {
        if path.is_null() {
            raise_exc!(BuiltinExceptionKind::IOError, "os.chdir: path is None");
        }

        if let Some(path_str) = crate::utils::extract_str_checked(path) {
            match env::set_current_dir(&path_str) {
                Ok(()) => {
                    // Success
                }
                Err(e) => match e.kind() {
                    std::io::ErrorKind::NotFound => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::FileNotFoundError,
                            "No such directory: '{}'",
                            path_str
                        );
                    }
                    std::io::ErrorKind::PermissionDenied => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::PermissionError,
                            "Permission denied: '{}'",
                            path_str
                        );
                    }
                    _ => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::IOError,
                            "Error changing directory '{}': {}",
                            path_str,
                            e
                        );
                    }
                },
            }
        } else {
            raise_exc!(BuiltinExceptionKind::IOError, "os.chdir: invalid path");
        }
    }
}

// ============= Listing files =============

/// List files in directory: os.listdir(path='.')
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_listdir(path: *mut Obj) -> *mut Obj {
    unsafe {
        let path_str = if path.is_null() {
            ".".to_string()
        } else if let Some(s) = crate::utils::extract_str_checked(path) {
            s
        } else {
            raise_exc!(BuiltinExceptionKind::IOError, "os.listdir: invalid path");
        };

        match fs::read_dir(&path_str) {
            Ok(entries) => {
                // Collect all entry names first
                let mut names = Vec::new();
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        names.push(name.to_string());
                    }
                }

                // Build list using rt_make_list + rt_list_push so GC ownership
                // and finalization are handled correctly by list_finalize.
                let count = names.len() as i64;
                let list_ptr = crate::list::rt_make_list(count, 0 /* ELEM_HEAP_OBJ */);

                // Root list_ptr so GC triggered by make_str_from_rust does not collect it.
                let mut roots: [*mut Obj; 1] = [list_ptr];
                let mut frame = ShadowFrame {
                    prev: std::ptr::null_mut(),
                    nroots: 1,
                    roots: roots.as_mut_ptr(),
                };
                gc_push(&mut frame);

                for name in &names {
                    let str_obj = make_str_from_rust(name);
                    crate::list::rt_list_push(roots[0], str_obj);
                }

                gc_pop();
                roots[0]
            }
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => {
                    crate::raise_exc!(
                        crate::exceptions::ExceptionType::FileNotFoundError,
                        "No such directory: '{}'",
                        path_str
                    );
                }
                std::io::ErrorKind::PermissionDenied => {
                    crate::raise_exc!(
                        crate::exceptions::ExceptionType::PermissionError,
                        "Permission denied: '{}'",
                        path_str
                    );
                }
                _ => {
                    crate::raise_exc!(
                        crate::exceptions::ExceptionType::IOError,
                        "Error listing directory '{}': {}",
                        path_str,
                        e
                    );
                }
            },
        }
    }
}

// ============= Path operations =============

/// Get absolute path: os.path.abspath(path)
///
/// Like CPython's os.path.abspath, this normalizes the path and makes it
/// absolute but does NOT resolve symlinks (use os.path.realpath for that).
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_path_abspath(path: *mut Obj) -> *mut Obj {
    unsafe {
        if path.is_null() {
            raise_exc!(
                BuiltinExceptionKind::IOError,
                "os.path.abspath: path is None"
            );
        }

        if let Some(path_str) = crate::utils::extract_str_checked(path) {
            let path_obj = std::path::Path::new(&path_str);

            // Python's os.path.abspath does NOT resolve symlinks (unlike realpath).
            // It only makes the path absolute by joining with CWD if needed,
            // then normalizes away '.' and '..' components.
            let abs_path = if path_obj.is_absolute() {
                path_obj.to_path_buf()
            } else {
                match env::current_dir() {
                    Ok(cwd) => cwd.join(path_obj),
                    Err(e) => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::IOError,
                            "Error getting current directory: {}",
                            e
                        );
                    }
                }
            };

            let normalized = normalize_path(&abs_path);
            // Convert to string (raises ValueError on non-UTF-8 paths)
            let result = path_to_utf8_str(&normalized);
            make_str_from_rust(result)
        } else {
            raise_exc!(
                BuiltinExceptionKind::IOError,
                "os.path.abspath: invalid path"
            );
        }
    }
}

/// Check if path is a directory: os.path.isdir(path)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_path_isdir(path: *mut Obj) -> i8 {
    unsafe {
        if path.is_null() {
            return 0;
        }
        if let Some(path_str) = crate::utils::extract_str_checked(path) {
            std::path::Path::new(&path_str).is_dir() as i8
        } else {
            0
        }
    }
}

/// Check if path is a file: os.path.isfile(path)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_path_isfile(path: *mut Obj) -> i8 {
    unsafe {
        if path.is_null() {
            return 0;
        }
        if let Some(path_str) = crate::utils::extract_str_checked(path) {
            std::path::Path::new(&path_str).is_file() as i8
        } else {
            0
        }
    }
}

/// Get basename of path: os.path.basename(path)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_path_basename(path: *mut Obj) -> *mut Obj {
    unsafe {
        if path.is_null() {
            return make_str_from_rust("");
        }

        let result = if let Some(path_str) = crate::utils::extract_str_checked(path) {
            let path_buf = std::path::Path::new(&path_str);
            path_buf
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        };

        make_str_from_rust(&result)
    }
}

/// Get dirname of path: os.path.dirname(path)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_path_dirname(path: *mut Obj) -> *mut Obj {
    unsafe {
        if path.is_null() {
            return make_str_from_rust("");
        }

        let result = if let Some(path_str) = crate::utils::extract_str_checked(path) {
            let path_buf = std::path::Path::new(&path_str);
            path_buf
                .parent()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        };

        make_str_from_rust(&result)
    }
}

/// Split path into (dirname, basename): os.path.split(path)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_path_split(path: *mut Obj) -> *mut Obj {
    unsafe {
        let (dirname, basename) = if path.is_null() {
            (String::new(), String::new())
        } else if let Some(path_str) = crate::utils::extract_str_checked(path) {
            let path_buf = std::path::Path::new(&path_str);
            let dirname = path_buf
                .parent()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let basename = path_buf
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            (dirname, basename)
        } else {
            (String::new(), String::new())
        };

        // Create tuple with 2 string elements.
        // Root tuple and dirname_obj so they survive subsequent allocs.
        let tuple = crate::tuple::rt_make_tuple(2, 0); // ELEM_HEAP_OBJ
        let mut roots: [*mut Obj; 2] = [tuple, std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);
        let dirname_obj = make_str_from_rust(&dirname);
        roots[1] = dirname_obj; // root dirname_obj across the basename alloc
        let basename_obj = make_str_from_rust(&basename);
        gc_pop();

        crate::tuple::rt_tuple_set(roots[0], 0, roots[1]);
        crate::tuple::rt_tuple_set(roots[0], 1, basename_obj);

        roots[0]
    }
}

// ============= Creating and deleting directories =============

/// Create a directory: os.mkdir(path)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_mkdir(path: *mut Obj) {
    unsafe {
        if path.is_null() {
            raise_exc!(BuiltinExceptionKind::IOError, "os.mkdir: path is None");
        }

        if let Some(path_str) = crate::utils::extract_str_checked(path) {
            match fs::create_dir(&path_str) {
                Ok(()) => {
                    // Success
                }
                Err(e) => match e.kind() {
                    std::io::ErrorKind::AlreadyExists => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::FileExistsError,
                            "File exists: '{}'",
                            path_str
                        );
                    }
                    std::io::ErrorKind::PermissionDenied => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::PermissionError,
                            "Permission denied: '{}'",
                            path_str
                        );
                    }
                    std::io::ErrorKind::NotFound => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::FileNotFoundError,
                            "No such file or directory: '{}'",
                            path_str
                        );
                    }
                    _ => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::IOError,
                            "Error creating directory '{}': {}",
                            path_str,
                            e
                        );
                    }
                },
            }
        } else {
            raise_exc!(BuiltinExceptionKind::IOError, "os.mkdir: invalid path");
        }
    }
}

/// Create directories recursively: os.makedirs(path, exist_ok=False)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_makedirs(path: *mut Obj, exist_ok: i8) {
    unsafe {
        if path.is_null() {
            raise_exc!(BuiltinExceptionKind::IOError, "os.makedirs: path is None");
        }

        if let Some(path_str) = crate::utils::extract_str_checked(path) {
            match fs::create_dir_all(&path_str) {
                Ok(()) => {
                    // Success
                }
                Err(e) => {
                    // If exist_ok is true and error is AlreadyExists, check if it's a directory
                    if exist_ok != 0
                        && e.kind() == std::io::ErrorKind::AlreadyExists
                        && std::path::Path::new(&path_str).is_dir()
                    {
                        return; // It's OK, directory already exists
                    }

                    match e.kind() {
                        std::io::ErrorKind::AlreadyExists => {
                            crate::raise_exc!(
                                crate::exceptions::ExceptionType::FileExistsError,
                                "File exists: '{}'",
                                path_str
                            );
                        }
                        std::io::ErrorKind::PermissionDenied => {
                            crate::raise_exc!(
                                crate::exceptions::ExceptionType::PermissionError,
                                "Permission denied: '{}'",
                                path_str
                            );
                        }
                        _ => {
                            crate::raise_exc!(
                                crate::exceptions::ExceptionType::IOError,
                                "Error creating directories '{}': {}",
                                path_str,
                                e
                            );
                        }
                    }
                }
            }
        } else {
            raise_exc!(BuiltinExceptionKind::IOError, "os.makedirs: invalid path");
        }
    }
}

/// Remove a directory: os.rmdir(path)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_rmdir(path: *mut Obj) {
    unsafe {
        if path.is_null() {
            raise_exc!(BuiltinExceptionKind::IOError, "os.rmdir: path is None");
        }

        if let Some(path_str) = crate::utils::extract_str_checked(path) {
            match fs::remove_dir(&path_str) {
                Ok(()) => {
                    // Success
                }
                Err(e) => match e.kind() {
                    std::io::ErrorKind::NotFound => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::FileNotFoundError,
                            "No such directory: '{}'",
                            path_str
                        );
                    }
                    std::io::ErrorKind::PermissionDenied => {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::PermissionError,
                            "Permission denied: '{}'",
                            path_str
                        );
                    }
                    _ => {
                        // Check if directory is not empty
                        let is_not_empty = fs::read_dir(&path_str)
                            .ok()
                            .is_some_and(|mut entries| entries.next().is_some());
                        if is_not_empty {
                            crate::raise_exc!(
                                crate::exceptions::ExceptionType::IOError,
                                "Directory not empty: '{}'",
                                path_str
                            );
                        } else {
                            crate::raise_exc!(
                                crate::exceptions::ExceptionType::IOError,
                                "Error removing directory '{}': {}",
                                path_str,
                                e
                            );
                        }
                    }
                },
            }
        } else {
            raise_exc!(BuiltinExceptionKind::IOError, "os.rmdir: invalid path");
        }
    }
}

// ============= Renaming and moving =============

/// Rename or move a file/directory: os.rename(src, dst)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_rename(src: *mut Obj, dst: *mut Obj) {
    unsafe {
        if src.is_null() || dst.is_null() {
            raise_exc!(BuiltinExceptionKind::IOError, "os.rename: path is None");
        }

        let src_str = crate::utils::extract_str_checked(src);
        let dst_str = crate::utils::extract_str_checked(dst);

        if src_str.is_none() || dst_str.is_none() {
            raise_exc!(BuiltinExceptionKind::IOError, "os.rename: invalid path");
        }

        let src_str = src_str.expect("src_str is Some");
        let dst_str = dst_str.expect("dst_str is Some");

        match fs::rename(&src_str, &dst_str) {
            Ok(()) => {
                // Success
            }
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => {
                    crate::raise_exc!(
                        crate::exceptions::ExceptionType::FileNotFoundError,
                        "No such file or directory: '{}'",
                        src_str
                    );
                }
                std::io::ErrorKind::PermissionDenied => {
                    crate::raise_exc!(
                        crate::exceptions::ExceptionType::PermissionError,
                        "Permission denied"
                    );
                }
                _ => {
                    crate::raise_exc!(
                        crate::exceptions::ExceptionType::IOError,
                        "Error renaming '{}' to '{}': {}",
                        src_str,
                        dst_str,
                        e
                    );
                }
            },
        }
    }
}

/// Replace file/directory: os.replace(src, dst) - same as rename but overwrites dst
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_replace(src: *mut Obj, dst: *mut Obj) {
    // On most platforms, fs::rename already has replace semantics
    // Call the same implementation as rename
    rt_os_rename(src, dst);
}

// ============= Environment variables =============

/// Get environment variable: os.getenv(key, default=None)
/// Returns string value or default (or None if default not provided)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_os_getenv(key: *mut Obj, default: *mut Obj) -> *mut Obj {
    unsafe {
        if key.is_null() {
            // If default is null pointer, return None singleton
            return if default.is_null() {
                crate::object::none_obj()
            } else {
                default
            };
        }

        if let Some(key_str) = crate::utils::extract_str_checked(key) {
            match env::var(&key_str) {
                Ok(value) => make_str_from_rust(&value),
                Err(_) => {
                    // If default is null pointer, return None singleton
                    if default.is_null() {
                        crate::object::none_obj()
                    } else {
                        default
                    }
                }
            }
        } else {
            // If default is null pointer, return None singleton
            if default.is_null() {
                crate::object::none_obj()
            } else {
                default
            }
        }
    }
}

// ============= OS information =============

/// Get OS name: os.name
/// Returns 'posix' on Unix/Linux/macOS, 'nt' on Windows
#[no_mangle]
pub extern "C" fn rt_os_get_name() -> *mut Obj {
    unsafe {
        #[cfg(unix)]
        let name = "posix";

        #[cfg(windows)]
        let name = "nt";

        #[cfg(not(any(unix, windows)))]
        let name = "unknown";

        make_str_from_rust(name)
    }
}
