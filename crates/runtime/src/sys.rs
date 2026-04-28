//! sys module runtime support
//!
//! Provides:
//! - sys.argv: Command line arguments as list[str]
//! - sys.path: Module search path as list[str]
//! - sys.exit(code): Exit program with given code

use crate::gc::{self, gc_pop, gc_push, ShadowFrame};
use crate::object::{ListObj, Obj, ObjHeader, StrObj, TypeTagKind};
use pyaot_core_defs::Value;
use std::cell::UnsafeCell;
use std::ffi::CStr;

/// Lock-free global storage for sys.argv list
struct SysArgvStorage(UnsafeCell<*mut Obj>);

// Safety: The runtime is single-threaded (AOT-compiled Python has no threading)
unsafe impl Sync for SysArgvStorage {}

static SYS_ARGV: SysArgvStorage = SysArgvStorage(UnsafeCell::new(std::ptr::null_mut()));

/// Lock-free global storage for sys.path list.
/// Lazily initialised on first read so we don't pay the cost (allocating a
/// list + N strings, walking `PYTHONPATH`) on programs that never read it.
struct SysPathStorage(UnsafeCell<*mut Obj>);

// Safety: The runtime is single-threaded (AOT-compiled Python has no threading)
unsafe impl Sync for SysPathStorage {}

static SYS_PATH: SysPathStorage = SysPathStorage(UnsafeCell::new(std::ptr::null_mut()));

/// Initialize sys.argv from main's argc/argv
///
/// # Safety
/// `argv` must be a valid pointer to an array of at least `argc` null-terminated C strings.
pub unsafe fn init_sys_argv(argc: i32, argv: *const *const i8) {
    // Create a list to hold the arguments
    let list_ptr = create_argv_list(argc, argv);

    // Store in global
    *SYS_ARGV.0.get() = list_ptr;
}

/// Create the argv list from C argc/argv
///
/// # Safety
/// `argv` must be a valid pointer to an array of at least `argc` null-terminated C strings.
unsafe fn create_argv_list(argc: i32, argv: *const *const i8) -> *mut Obj {
    // Allocate list with capacity for argc elements
    let capacity = if argc > 0 { argc as usize } else { 0 };

    // Allocate list object
    let list_size = std::mem::size_of::<ListObj>();
    let list_ptr = gc::gc_alloc(list_size, TypeTagKind::List as u8) as *mut ListObj;

    (*list_ptr).header = ObjHeader {
        type_tag: TypeTagKind::List,
        marked: false,
        size: list_size,
    };
    (*list_ptr).len = 0;
    (*list_ptr).capacity = capacity;

    // Allocate data array if needed
    if capacity > 0 {
        let data_layout = std::alloc::Layout::array::<pyaot_core_defs::Value>(capacity)
            .expect("Allocation size overflow - capacity too large");
        (*list_ptr).data = std::alloc::alloc(data_layout) as *mut pyaot_core_defs::Value;

        // Initialize all slots to Value(0) (null pointer, skipped by GC).
        for i in 0..capacity {
            *(*list_ptr).data.add(i) = pyaot_core_defs::Value(0);
        }
    } else {
        (*list_ptr).data = std::ptr::null_mut();
    }

    // Root list_ptr before the loop: each gc_alloc for a string may trigger a
    // GC collection, and list_ptr would be freed if it is not on the shadow stack.
    let mut roots: [*mut Obj; 1] = [list_ptr as *mut Obj];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);

    // Convert each argv element to a StrObj and add to list
    for i in 0..argc {
        let c_str_ptr = *argv.add(i as usize);
        if c_str_ptr.is_null() {
            continue;
        }

        let c_str = CStr::from_ptr(c_str_ptr);
        let bytes = c_str.to_bytes();
        let len = bytes.len();

        // Allocate string object (GC may run; list_ptr stays alive via frame)
        let str_size = std::mem::size_of::<StrObj>() + len;
        let str_ptr = gc::gc_alloc(str_size, TypeTagKind::Str as u8) as *mut StrObj;

        (*str_ptr).header = ObjHeader {
            type_tag: TypeTagKind::Str,
            marked: false,
            size: str_size,
        };
        (*str_ptr).len = len;

        // Copy string data
        if len > 0 {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*str_ptr).data.as_mut_ptr(), len);
        }

        // Re-derive list_ptr after gc_alloc: the GC is non-moving, so the
        // address is unchanged, but using the rooted pointer makes ownership
        // explicit and avoids any confusion about which pointer is authoritative.
        let list_ptr = roots[0] as *mut ListObj;

        // Add to list (wrap the string pointer as a tagged pointer Value).
        *(*list_ptr).data.add(i as usize) = pyaot_core_defs::Value::from_ptr(str_ptr as *mut Obj);
        (*list_ptr).len += 1;
    }

    gc_pop();
    roots[0]
}

/// Pointers held in the `sys` module's static slots. Returned to the GC
/// mark phase so the cached `sys.argv` and `sys.path` lists (which live in
/// process-wide statics, not in the shadow stack or globals map) survive
/// collection cycles regardless of how often user code holds references.
pub fn get_sys_module_roots() -> [*mut Obj; 2] {
    unsafe { [*SYS_ARGV.0.get(), *SYS_PATH.0.get()] }
}

/// Get sys.argv list
/// Returns a pointer to the list of command-line arguments
pub fn rt_sys_get_argv() -> *mut Obj {
    let argv_ptr = unsafe { *SYS_ARGV.0.get() };
    if !argv_ptr.is_null() {
        return argv_ptr;
    }
    // Return empty list if not initialized (shouldn't happen in normal usage)
    unsafe { build_str_list::<&str>(&[]) }
}
#[export_name = "rt_sys_get_argv"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_sys_get_argv_abi() -> Value {
    Value::from_ptr(rt_sys_get_argv())
}


/// Get sys.path list — module search paths.
///
/// On first call, builds the list from:
/// 1. Directory of the executable (for code installed next to the binary)
/// 2. Current working directory
/// 3. `:`-separated entries of `PYTHONPATH` (Unix; `;` on Windows)
///
/// Subsequent calls return the cached list, so user code can do
/// `sys.path.append("...")` and have the change persist within the
/// process. Imports are resolved at compile time and ignore mutations
/// to the runtime list.
pub fn rt_sys_get_path() -> *mut Obj {
    let cached = unsafe { *SYS_PATH.0.get() };
    if !cached.is_null() {
        return cached;
    }

    let mut entries: Vec<String> = Vec::new();

    // 1. Executable's directory.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            entries.push(dir.to_string_lossy().into_owned());
        }
    }

    // 2. Current working directory.
    if let Ok(cwd) = std::env::current_dir() {
        entries.push(cwd.to_string_lossy().into_owned());
    }

    // 3. PYTHONPATH entries.
    if let Ok(pythonpath) = std::env::var("PYTHONPATH") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for entry in pythonpath.split(sep) {
            if !entry.is_empty() {
                entries.push(entry.to_string());
            }
        }
    }

    let list_ptr = unsafe { build_str_list(&entries) };
    unsafe {
        *SYS_PATH.0.get() = list_ptr;
    }
    list_ptr
}
#[export_name = "rt_sys_get_path"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_sys_get_path_abi() -> Value {
    Value::from_ptr(rt_sys_get_path())
}


/// Build a `ListObj` of `StrObj` from a slice of Rust strings.
///
/// # Safety
/// The caller must ensure the GC root for the returned list is established
/// if the caller plans to allocate after this call. Internally, the list
/// pointer is rooted across each per-string allocation since `gc_alloc`
/// for the strings can trigger a collection.
unsafe fn build_str_list<S: AsRef<str>>(entries: &[S]) -> *mut Obj {
    let capacity = entries.len();
    let list_size = std::mem::size_of::<ListObj>();
    let list_ptr = gc::gc_alloc(list_size, TypeTagKind::List as u8) as *mut ListObj;

    (*list_ptr).header = ObjHeader {
        type_tag: TypeTagKind::List,
        marked: false,
        size: list_size,
    };
    (*list_ptr).len = 0;
    (*list_ptr).capacity = capacity;

    if capacity > 0 {
        let data_layout = std::alloc::Layout::array::<pyaot_core_defs::Value>(capacity)
            .expect("Allocation size overflow - capacity too large");
        (*list_ptr).data = std::alloc::alloc(data_layout) as *mut pyaot_core_defs::Value;
        for i in 0..capacity {
            *(*list_ptr).data.add(i) = pyaot_core_defs::Value(0);
        }
    } else {
        (*list_ptr).data = std::ptr::null_mut();
        return list_ptr as *mut Obj;
    }

    // Root the list while we allocate the per-entry strings.
    let mut roots: [*mut Obj; 1] = [list_ptr as *mut Obj];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);

    for (i, entry) in entries.iter().enumerate() {
        let bytes = entry.as_ref().as_bytes();
        let len = bytes.len();
        let str_size = std::mem::size_of::<StrObj>() + len;
        let str_ptr = gc::gc_alloc(str_size, TypeTagKind::Str as u8) as *mut StrObj;

        (*str_ptr).header = ObjHeader {
            type_tag: TypeTagKind::Str,
            marked: false,
            size: str_size,
        };
        (*str_ptr).len = len;
        if len > 0 {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*str_ptr).data.as_mut_ptr(), len);
        }

        let list_ptr = roots[0] as *mut ListObj;
        *(*list_ptr).data.add(i) = pyaot_core_defs::Value::from_ptr(str_ptr as *mut Obj);
        (*list_ptr).len += 1;
    }

    gc_pop();
    roots[0]
}

/// Exit the program with the given exit code
/// This function never returns (diverging)
#[no_mangle]
pub extern "C" fn rt_sys_exit(code: i64) -> ! {
    // Call rt_shutdown to clean up
    crate::rt_shutdown();
    std::process::exit(code as i32)
}

/// Intern a string - returns an interned version of the string
/// If the string is already interned, returns the same object.
/// This is equivalent to Python's sys.intern(string).
///
/// # Safety
/// `str_obj` must be a valid pointer to a StrObj.
pub unsafe fn rt_sys_intern(str_obj: *mut Obj) -> *mut Obj {
    use crate::object::StrObj;
    use crate::string::rt_make_str_interned;

    if str_obj.is_null() {
        return str_obj;
    }

    let str_ptr = str_obj as *mut StrObj;
    let len = (*str_ptr).len;
    let data = (*str_ptr).data.as_ptr();

    // Use the string interning function to get or create interned version
    rt_make_str_interned(data, len)
}
#[export_name = "rt_sys_intern"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_sys_intern_abi(str_obj: Value) -> Value {
    Value::from_ptr(unsafe { rt_sys_intern(str_obj.unwrap_ptr()) })
}

