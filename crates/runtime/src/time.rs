//! time module runtime support
//!
//! Provides:
//! - time.sleep(seconds): Pause execution for given seconds
//! - time.time(): Return current Unix timestamp as float
//! - time.monotonic(): Return monotonic clock value for measuring intervals
//! - time.perf_counter(): Return high-resolution performance counter
//! - time.ctime([seconds]): Convert seconds to readable local time string
//! - time.localtime([seconds]): Convert seconds to local struct_time
//! - time.gmtime([seconds]): Convert seconds to UTC struct_time
//! - time.mktime(t): Convert struct_time back to seconds

use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::gc;
use crate::object::{Obj, ObjHeader, StructTimeObj, TypeTagKind};
use crate::utils::{make_str_from_rust, str_obj_to_rust_string};
use pyaot_core_defs::Value;

/// Monotonic clock start time (initialized on first call)
static MONOTONIC_START: OnceLock<Instant> = OnceLock::new();

/// Performance counter start time (initialized on first call)
static PERF_COUNTER_START: OnceLock<Instant> = OnceLock::new();

/// time.sleep(seconds) - Pause execution for the given number of seconds
#[no_mangle]
pub extern "C" fn rt_time_sleep(seconds: f64) {
    if seconds <= 0.0 || seconds.is_nan() {
        return;
    }
    std::thread::sleep(Duration::from_secs_f64(seconds));
}

/// time.time() - Return current Unix timestamp as float
#[no_mangle]
pub extern "C" fn rt_time_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// time.monotonic() - Return monotonic clock value for measuring intervals
#[no_mangle]
pub extern "C" fn rt_time_monotonic() -> f64 {
    MONOTONIC_START
        .get_or_init(Instant::now)
        .elapsed()
        .as_secs_f64()
}

/// time.perf_counter() - Return high-resolution performance counter
#[no_mangle]
pub extern "C" fn rt_time_perf_counter() -> f64 {
    PERF_COUNTER_START
        .get_or_init(Instant::now)
        .elapsed()
        .as_secs_f64()
}

/// time.ctime([seconds]) - Convert seconds to readable local time string
/// Format: "Mon Feb  2 14:00:00 2026" (24 characters + newline removed)
/// If seconds is negative, uses current time
pub fn rt_time_ctime(seconds: f64) -> *mut Obj {
    // Get timestamp: use current time if sentinel (-1.0) or negative
    let timestamp = if seconds < 0.0 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    } else {
        seconds as i64
    };

    // Format using libc localtime and strftime
    let formatted = format_ctime(timestamp);

    // Create StrObj
    unsafe { make_str_from_rust(&formatted) }
}
#[export_name = "rt_time_ctime"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_time_ctime_abi(seconds: f64) -> Value {
    Value::from_ptr(rt_time_ctime(seconds))
}


/// Format timestamp as ctime string using libc
fn format_ctime(timestamp: i64) -> String {
    unsafe {
        let time_t = timestamp as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();

        // Convert to local time
        #[cfg(unix)]
        {
            libc::localtime_r(&time_t, &mut tm);
        }
        #[cfg(windows)]
        {
            // Windows uses localtime_s with reversed arguments
            libc::localtime_s(&mut tm, &time_t);
        }

        // Format: "Mon Feb  2 14:00:00 2026"
        let mut buffer = [0i8; 64];
        let format = c"%a %b %e %H:%M:%S %Y";
        libc::strftime(buffer.as_mut_ptr(), buffer.len(), format.as_ptr(), &tm);

        // Convert to Rust string
        std::ffi::CStr::from_ptr(buffer.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

/// Create a StructTimeObj from libc tm struct
unsafe fn create_struct_time_obj(tm: &libc::tm) -> *mut Obj {
    let size = std::mem::size_of::<StructTimeObj>();
    let ptr = gc::gc_alloc(size, TypeTagKind::StructTime as u8) as *mut StructTimeObj;

    (*ptr).header = ObjHeader {
        type_tag: TypeTagKind::StructTime,
        marked: false,
        size,
    };

    // tm_year is years since 1900, Python expects full year
    (*ptr).tm_year = (tm.tm_year + 1900) as i64;
    // tm_mon is 0-11 in libc, Python uses 1-12
    (*ptr).tm_mon = (tm.tm_mon + 1) as i64;
    (*ptr).tm_mday = tm.tm_mday as i64;
    (*ptr).tm_hour = tm.tm_hour as i64;
    (*ptr).tm_min = tm.tm_min as i64;
    (*ptr).tm_sec = tm.tm_sec as i64;
    // tm_wday is 0-6 in libc with Sunday=0, Python uses Monday=0
    (*ptr).tm_wday = ((tm.tm_wday + 6) % 7) as i64;
    // tm_yday is 0-365 in libc, Python uses 1-366
    (*ptr).tm_yday = (tm.tm_yday + 1) as i64;
    (*ptr).tm_isdst = tm.tm_isdst as i64;

    ptr as *mut Obj
}

/// time.localtime([seconds]) - Convert seconds to local struct_time
/// If seconds is negative, uses current time
pub fn rt_time_localtime(seconds: f64) -> *mut Obj {
    let timestamp = if seconds < 0.0 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    } else {
        seconds as i64
    };

    unsafe {
        let time_t = timestamp as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();

        #[cfg(unix)]
        {
            libc::localtime_r(&time_t, &mut tm);
        }
        #[cfg(windows)]
        {
            libc::localtime_s(&mut tm, &time_t);
        }

        create_struct_time_obj(&tm)
    }
}
#[export_name = "rt_time_localtime"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_time_localtime_abi(seconds: f64) -> Value {
    Value::from_ptr(rt_time_localtime(seconds))
}


/// time.gmtime([seconds]) - Convert seconds to UTC struct_time
/// If seconds is negative, uses current time
pub fn rt_time_gmtime(seconds: f64) -> *mut Obj {
    let timestamp = if seconds < 0.0 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    } else {
        seconds as i64
    };

    unsafe {
        let time_t = timestamp as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();

        #[cfg(unix)]
        {
            libc::gmtime_r(&time_t, &mut tm);
        }
        #[cfg(windows)]
        {
            libc::gmtime_s(&mut tm, &time_t);
        }

        create_struct_time_obj(&tm)
    }
}
#[export_name = "rt_time_gmtime"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_time_gmtime_abi(seconds: f64) -> Value {
    Value::from_ptr(rt_time_gmtime(seconds))
}


/// time.mktime(t) - Convert struct_time to seconds since epoch
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_time_mktime(t: *mut Obj) -> f64 {
    if t.is_null() {
        return 0.0;
    }

    unsafe {
        if (*t).header.type_tag != TypeTagKind::StructTime {
            return 0.0;
        }

        let st = t as *const StructTimeObj;

        // Convert Python struct_time back to libc tm
        let tm = libc::tm {
            tm_year: ((*st).tm_year - 1900) as i32,
            tm_mon: ((*st).tm_mon - 1) as i32,
            tm_mday: (*st).tm_mday as i32,
            tm_hour: (*st).tm_hour as i32,
            tm_min: (*st).tm_min as i32,
            tm_sec: (*st).tm_sec as i32,
            // Python wday is Monday=0, libc is Sunday=0
            tm_wday: (((*st).tm_wday + 1) % 7) as i32,
            // Python yday is 1-366, libc is 0-365
            tm_yday: ((*st).tm_yday - 1) as i32,
            tm_isdst: (*st).tm_isdst as i32,
            #[cfg(unix)]
            tm_gmtoff: 0,
            #[cfg(unix)]
            tm_zone: std::ptr::null_mut(),
        };

        let result = libc::mktime(&tm as *const libc::tm as *mut libc::tm);
        result as f64
    }
}
#[export_name = "rt_time_mktime"]
pub extern "C" fn rt_time_mktime_abi(t: Value) -> f64 {
    rt_time_mktime(t.unwrap_ptr())
}


/// time.strftime(format, t) - Format struct_time to string
/// Common format codes: %Y (year), %m (month), %d (day), %H (hour), %M (minute), %S (second)
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_time_strftime(format: *mut Obj, t: *mut Obj) -> *mut Obj {
    if format.is_null() {
        return unsafe { make_str_from_rust("") };
    }

    unsafe {
        // Get format string early so we can check for empty / embedded null.
        let format_str = str_obj_to_rust_string(format);

        // M15: empty format string — return empty string immediately to avoid
        // the ambiguity in the doubling-buffer loop (strftime returns 0 for
        // both truncation and genuinely empty output).
        if format_str.is_empty() {
            return crate::string::rt_make_str(std::ptr::null(), 0);
        }

        // H13: reject format strings with embedded null bytes — CString::new
        // would silently truncate at the first null, producing wrong output.
        let fmt_c = match std::ffi::CString::new(format_str) {
            Ok(s) => s,
            Err(_) => {
                raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "embedded null character"
                );
            }
        };

        // When t is null (omitted), use current local time like CPython
        let tm = if t.is_null() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as libc::time_t)
                .unwrap_or(0);
            let mut local_tm: libc::tm = std::mem::zeroed();
            #[cfg(unix)]
            {
                libc::localtime_r(&now, &mut local_tm);
            }
            #[cfg(windows)]
            {
                libc::localtime_s(&mut local_tm, &now);
            }
            local_tm
        } else {
            if (*t).header.type_tag != TypeTagKind::StructTime {
                return make_str_from_rust("");
            }

            let st = t as *const StructTimeObj;

            libc::tm {
                tm_year: ((*st).tm_year - 1900) as i32,
                tm_mon: ((*st).tm_mon - 1) as i32,
                tm_mday: (*st).tm_mday as i32,
                tm_hour: (*st).tm_hour as i32,
                tm_min: (*st).tm_min as i32,
                tm_sec: (*st).tm_sec as i32,
                tm_wday: (((*st).tm_wday + 1) % 7) as i32,
                tm_yday: ((*st).tm_yday - 1) as i32,
                tm_isdst: (*st).tm_isdst as i32,
                #[cfg(unix)]
                tm_gmtoff: 0,
                #[cfg(unix)]
                tm_zone: std::ptr::null_mut(),
            }
        };

        // Use a retry loop to handle format strings that produce long output.
        // Double the buffer on each failed attempt, up to a 1 MB hard limit.
        // The POSIX spec states strftime returns 0 on truncation (not error),
        // so we keep doubling until the output fits or we exceed the limit.
        const MAX_STRFTIME_BUF: usize = 1 << 20; // 1 MB
        let mut buf_size = 256usize;
        let result = loop {
            let mut buffer = vec![0i8; buf_size];
            let written = libc::strftime(buffer.as_mut_ptr(), buf_size, fmt_c.as_ptr(), &tm);
            if written > 0 {
                let s = std::ffi::CStr::from_ptr(buffer.as_ptr())
                    .to_string_lossy()
                    .into_owned();
                break s;
            }
            if buf_size >= MAX_STRFTIME_BUF {
                // Output still doesn't fit after 1 MB — raise an error.
                raise_exc!(
                    pyaot_core_defs::BuiltinExceptionKind::ValueError,
                    "strftime: formatted string too long"
                );
            }
            buf_size *= 2;
        };
        make_str_from_rust(&result)
    }
}
#[export_name = "rt_time_strftime"]
pub extern "C" fn rt_time_strftime_abi(format: Value, t: Value) -> Value {
    Value::from_ptr(rt_time_strftime(format.unwrap_ptr(), t.unwrap_ptr()))
}


/// time.strptime(string, format) - Parse string to struct_time
/// Common format codes: %Y (year), %m (month), %d (day), %H (hour), %M (minute), %S (second)
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_time_strptime(string: *mut Obj, format: *mut Obj) -> *mut Obj {
    if string.is_null() || format.is_null() {
        unsafe {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::ValueError,
                "strptime: string and format must not be None"
            );
        }
    }

    unsafe {
        let string_str = str_obj_to_rust_string(string);
        let format_str = str_obj_to_rust_string(format);

        let string_cstr = std::ffi::CString::new(string_str)
            .unwrap_or_else(|_| std::ffi::CString::new("").unwrap());
        let format_cstr = std::ffi::CString::new(format_str)
            .unwrap_or_else(|_| std::ffi::CString::new("").unwrap());

        let mut tm: libc::tm = std::mem::zeroed();
        // Initialize with reasonable defaults
        tm.tm_mday = 1; // Day 1 (strptime may not set this if not in format)
        tm.tm_isdst = -1; // Let system determine DST

        let result = libc::strptime(string_cstr.as_ptr(), format_cstr.as_ptr(), &mut tm);

        if result.is_null() {
            // Parse failed — raise ValueError matching CPython's behaviour
            crate::raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "time data '{}' does not match format '{}'",
                crate::utils::str_obj_to_rust_string(string),
                crate::utils::str_obj_to_rust_string(format)
            );
        }

        create_struct_time_obj(&tm)
    }
}
#[export_name = "rt_time_strptime"]
pub extern "C" fn rt_time_strptime_abi(string: Value, format: Value) -> Value {
    Value::from_ptr(rt_time_strptime(string.unwrap_ptr(), format.unwrap_ptr()))
}


// ============= struct_time field getters =============

/// Generic struct_time field accessor.
/// field_index: 0=year, 1=mon, 2=mday, 3=hour, 4=min, 5=sec, 6=wday, 7=yday, 8=isdst
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_struct_time_get_field(t: *mut Obj, field_index: u8) -> i64 {
    if t.is_null() {
        return 0;
    }
    unsafe {
        if (*t).header.type_tag != TypeTagKind::StructTime {
            return 0;
        }
        let st = t as *const StructTimeObj;
        match field_index {
            0 => (*st).tm_year,
            1 => (*st).tm_mon,
            2 => (*st).tm_mday,
            3 => (*st).tm_hour,
            4 => (*st).tm_min,
            5 => (*st).tm_sec,
            6 => (*st).tm_wday,
            7 => (*st).tm_yday,
            8 => (*st).tm_isdst,
            _ => 0,
        }
    }
}
#[export_name = "rt_struct_time_get_field"]
pub extern "C" fn rt_struct_time_get_field_abi(t: Value, field_index: u8) -> i64 {
    rt_struct_time_get_field(t.unwrap_ptr(), field_index)
}

