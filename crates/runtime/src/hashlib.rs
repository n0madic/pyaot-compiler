//! hashlib module runtime functions

use crate::gc::gc_alloc;
use crate::object::{BytesObj, Obj, ObjHeader, StrObj};
use md5::Md5;
use pyaot_core_defs::TypeTagKind;
use sha2::{Digest, Sha256};

/// Hash object storing precomputed digest
#[repr(C)]
pub struct HashObj {
    pub header: ObjHeader,
    pub digest_len: usize,
    pub digest: [u8; 32], // Fixed buffer, enough for SHA256
}

/// Extract byte slice from a str or bytes object
unsafe fn extract_data_slice<'a>(data: *mut Obj, func_name: &str) -> &'a [u8] {
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
        _ => panic!(
            "{}: expected bytes or str, got {:?}",
            func_name, header.type_tag
        ),
    }
}

/// Allocate a HashObj and write a digest into it
unsafe fn create_hash_obj(digest_bytes: &[u8]) -> *mut Obj {
    let size = std::mem::size_of::<HashObj>();
    let hash_obj = gc_alloc(size, TypeTagKind::Hash.tag()) as *mut HashObj;
    (*hash_obj).digest_len = digest_bytes.len();
    let dst = std::ptr::addr_of_mut!((*hash_obj).digest) as *mut u8;
    std::ptr::copy_nonoverlapping(digest_bytes.as_ptr(), dst, digest_bytes.len());
    hash_obj as *mut Obj
}

/// Read digest from a HashObj
unsafe fn read_digest(hash_obj: *mut Obj) -> &'static [u8] {
    let hash = hash_obj as *mut HashObj;
    let digest_len = (*hash).digest_len;
    let src = std::ptr::addr_of!((*hash).digest) as *const u8;
    std::slice::from_raw_parts(src, digest_len)
}

/// hashlib.md5(data) -> Hash object
#[no_mangle]
pub unsafe extern "C" fn rt_hashlib_md5(data: *mut Obj) -> *mut Obj {
    let data_slice = extract_data_slice(data, "rt_hashlib_md5");
    let mut hasher = Md5::new();
    hasher.update(data_slice);
    let digest = hasher.finalize();
    create_hash_obj(&digest[..16])
}

/// hashlib.sha256(data) -> Hash object
#[no_mangle]
pub unsafe extern "C" fn rt_hashlib_sha256(data: *mut Obj) -> *mut Obj {
    let data_slice = extract_data_slice(data, "rt_hashlib_sha256");
    let mut hasher = Sha256::new();
    hasher.update(data_slice);
    let digest = hasher.finalize();
    create_hash_obj(&digest[..32])
}

/// hashlib.sha1(data) -> Hash object
#[no_mangle]
pub unsafe extern "C" fn rt_hashlib_sha1(data: *mut Obj) -> *mut Obj {
    let data_slice = extract_data_slice(data, "rt_hashlib_sha1");
    let digest = sha1_hash(data_slice);
    create_hash_obj(&digest)
}

/// Hash.hexdigest() -> str
#[no_mangle]
pub unsafe extern "C" fn rt_hash_hexdigest(hash_obj: *mut Obj) -> *mut Obj {
    crate::debug_assert_type_tag!(hash_obj, TypeTagKind::Hash, "rt_hash_hexdigest");
    let digest_slice = read_digest(hash_obj);
    let hex_string = digest_slice
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    crate::string::rt_make_str(hex_string.as_ptr(), hex_string.len())
}

/// Hash.digest() -> bytes
#[no_mangle]
pub unsafe extern "C" fn rt_hash_digest(hash_obj: *mut Obj) -> *mut Obj {
    crate::debug_assert_type_tag!(hash_obj, TypeTagKind::Hash, "rt_hash_digest");
    let digest_slice = read_digest(hash_obj);
    crate::bytes::rt_make_bytes(digest_slice.as_ptr(), digest_slice.len())
}

/// Simple SHA1 implementation (FIPS 180-4)
fn sha1_hash(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let ml = (data.len() as u64) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() * 8) % 512 != 448 {
        padded.push(0);
    }
    padded.extend_from_slice(&ml.to_be_bytes());

    for chunk in padded.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                60..=79 => (b ^ c ^ d, 0xCA62C1D6u32),
                _ => unreachable!(),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut digest = [0u8; 20];
    digest[0..4].copy_from_slice(&h0.to_be_bytes());
    digest[4..8].copy_from_slice(&h1.to_be_bytes());
    digest[8..12].copy_from_slice(&h2.to_be_bytes());
    digest[12..16].copy_from_slice(&h3.to_be_bytes());
    digest[16..20].copy_from_slice(&h4.to_be_bytes());
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha1_hash() {
        let digest = sha1_hash(b"");
        let hex = digest
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();
        assert_eq!(hex, "da39a3ee5e6b4b0d3255bfef95601890afd80709");

        let digest = sha1_hash(b"abc");
        let hex = digest
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();
        assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");

        let digest = sha1_hash(b"The quick brown fox jumps over the lazy dog");
        let hex = digest
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();
        assert_eq!(hex, "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12");
    }
}
