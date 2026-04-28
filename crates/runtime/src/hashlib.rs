//! hashlib module runtime functions

use crate::gc::gc_alloc;
use crate::object::{BytesObj, Obj, ObjHeader, StrObj};
use md5::Md5;
use pyaot_core_defs::TypeTagKind;
use pyaot_core_defs::Value;
use sha1::Sha1;
use sha2::{Digest, Sha256};

/// Hash object storing precomputed digest
#[repr(C)]
pub struct HashObj {
    pub header: ObjHeader,
    pub digest_len: usize,
    pub digest: [u8; 64], // Fixed buffer, enough for SHA512
}

/// Extract byte slice from a str or bytes object
unsafe fn extract_data_slice<'a>(data: *mut Obj) -> &'a [u8] {
    let header = &(*data).header;
    match header.type_tag {
        TypeTagKind::Bytes => {
            let bytes_obj = data as *mut BytesObj;
            std::slice::from_raw_parts((*bytes_obj).data.as_ptr(), (*bytes_obj).len)
        }
        TypeTagKind::Str => {
            let str_obj = data as *mut StrObj;
            std::slice::from_raw_parts((*str_obj).data.as_ptr(), (*str_obj).len)
        }
        _ => {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::TypeError,
                "expected bytes or str"
            );
        }
    }
}

/// Allocate a HashObj and write a digest into it
unsafe fn create_hash_obj(digest_bytes: &[u8]) -> *mut Obj {
    if digest_bytes.len() > 64 {
        eprintln!(
            "FATAL: hash digest size {} exceeds 64-byte buffer capacity",
            digest_bytes.len()
        );
        std::process::abort();
    }
    let size = std::mem::size_of::<HashObj>();
    let hash_obj = gc_alloc(size, TypeTagKind::Hash.tag()) as *mut HashObj;
    (*hash_obj).digest_len = digest_bytes.len();
    let dst = std::ptr::addr_of_mut!((*hash_obj).digest) as *mut u8;
    std::ptr::copy_nonoverlapping(digest_bytes.as_ptr(), dst, digest_bytes.len());
    hash_obj as *mut Obj
}

/// Read digest from a HashObj.
///
/// # Safety
/// The caller must ensure `hash_obj` remains valid and no GC collection
/// occurs between calling this function and consuming the returned slice.
unsafe fn read_digest<'a>(hash_obj: *mut Obj) -> &'a [u8] {
    let hash = hash_obj as *mut HashObj;
    let digest_len = (*hash).digest_len;
    let src = std::ptr::addr_of!((*hash).digest) as *const u8;
    std::slice::from_raw_parts(src, digest_len)
}

/// hashlib.md5(data) -> Hash object
pub unsafe fn rt_hashlib_md5(data: *mut Obj) -> *mut Obj {
    let data_slice = extract_data_slice(data);
    let mut hasher = Md5::new();
    hasher.update(data_slice);
    let digest = hasher.finalize();
    create_hash_obj(&digest[..16])
}
#[export_name = "rt_hashlib_md5"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_hashlib_md5_abi(data: Value) -> Value {
    Value::from_ptr(unsafe { rt_hashlib_md5(data.unwrap_ptr()) })
}


/// hashlib.sha256(data) -> Hash object
pub unsafe fn rt_hashlib_sha256(data: *mut Obj) -> *mut Obj {
    let data_slice = extract_data_slice(data);
    let mut hasher = Sha256::new();
    hasher.update(data_slice);
    let digest = hasher.finalize();
    create_hash_obj(&digest[..32])
}
#[export_name = "rt_hashlib_sha256"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_hashlib_sha256_abi(data: Value) -> Value {
    Value::from_ptr(unsafe { rt_hashlib_sha256(data.unwrap_ptr()) })
}


/// hashlib.sha1(data) -> Hash object
pub unsafe fn rt_hashlib_sha1(data: *mut Obj) -> *mut Obj {
    let data_slice = extract_data_slice(data);
    let mut hasher = Sha1::new();
    hasher.update(data_slice);
    let digest = hasher.finalize();
    create_hash_obj(&digest[..20])
}
#[export_name = "rt_hashlib_sha1"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_hashlib_sha1_abi(data: Value) -> Value {
    Value::from_ptr(unsafe { rt_hashlib_sha1(data.unwrap_ptr()) })
}


/// Hash.hexdigest() -> str
pub unsafe fn rt_hash_hexdigest(hash_obj: *mut Obj) -> *mut Obj {
    crate::debug_assert_type_tag!(hash_obj, TypeTagKind::Hash, "rt_hash_hexdigest");
    let digest_slice = read_digest(hash_obj);
    let hex_string = digest_slice
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    crate::string::rt_make_str(hex_string.as_ptr(), hex_string.len())
}
#[export_name = "rt_hash_hexdigest"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_hash_hexdigest_abi(hash_obj: Value) -> Value {
    Value::from_ptr(unsafe { rt_hash_hexdigest(hash_obj.unwrap_ptr()) })
}


/// Hash.digest() -> bytes
pub unsafe fn rt_hash_digest(hash_obj: *mut Obj) -> *mut Obj {
    crate::debug_assert_type_tag!(hash_obj, TypeTagKind::Hash, "rt_hash_digest");
    let digest_slice = read_digest(hash_obj);
    crate::bytes::rt_make_bytes(digest_slice.as_ptr(), digest_slice.len())
}
#[export_name = "rt_hash_digest"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_hash_digest_abi(hash_obj: Value) -> Value {
    Value::from_ptr(unsafe { rt_hash_digest(hash_obj.unwrap_ptr()) })
}

