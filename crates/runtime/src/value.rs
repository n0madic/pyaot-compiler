//! Runtime-side extensions for the tagged [`Value`] type.
//!
//! `core-defs::Value` owns the bit layout and the primitive APIs (see
//! `ARCHITECTURE_REFACTOR.md` §2.1/§2.2). Operations that require
//! dereferencing a heap [`Obj`] — notably the full runtime-type lookup —
//! live here because `core-defs` is a leaf crate that forbids `unsafe`.
//!
//! Phase 2 §F.7d.3 deleted the four primitive box/unbox externs;
//! lowering and codegen now emit inline `ValueFromInt` /
//! `ValueFromBool` / `UnwrapValueInt` / `UnwrapValueBool` MIR
//! instructions for Int and Bool. Float still uses the heap-boxed
//! `rt_box_float` / `rt_unbox_float` extern shims.

pub use pyaot_core_defs::Value;

use pyaot_core_defs::TypeTagKind;

use crate::object::Obj;

/// Full runtime type lookup.
///
/// For immediate tags (Int / Bool / None) this returns the primitive
/// `TypeTagKind` directly. For pointer tags it dereferences the
/// [`ObjHeader`](crate::object::ObjHeader) and reads the heap object's
/// tag.
///
/// # Safety
///
/// If `v.is_ptr()`, the caller must guarantee that the wrapped pointer
/// is either null (rejected by the debug assertion) or points to a
/// live heap object whose `ObjHeader` has been initialized by the
/// allocator. Passing a dangling or uninitialized pointer is undefined
/// behavior.
#[inline]
pub unsafe fn type_of(v: Value) -> TypeTagKind {
    if let Some(t) = v.primitive_type() {
        return t;
    }
    let ptr = v.unwrap_ptr::<Obj>();
    debug_assert!(!ptr.is_null(), "runtime::type_of on null pointer");
    (*ptr).header.type_tag
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ObjHeader;

    fn heap_value_for_tag(tag: TypeTagKind) -> (Box<ObjHeader>, Value) {
        // Keep the header owned by the caller; Value only borrows its
        // address for the duration of the test. Box keeps the heap
        // allocation 8-byte aligned so the tag-bit invariant holds.
        let header = Box::new(ObjHeader {
            type_tag: tag,
            marked: false,
            size: 0,
        });
        let obj = Box::as_ref(&header) as *const ObjHeader as *mut Obj;
        let value = Value::from_ptr(obj);
        assert!(
            value.is_ptr(),
            "boxed header must be 8-byte aligned for Value::from_ptr"
        );
        (header, value)
    }

    #[test]
    fn type_of_handles_primitives() {
        unsafe {
            assert_eq!(type_of(Value::NONE), TypeTagKind::None);
            assert_eq!(type_of(Value::TRUE), TypeTagKind::Bool);
            assert_eq!(type_of(Value::FALSE), TypeTagKind::Bool);
            assert_eq!(type_of(Value::from_int(0)), TypeTagKind::Int);
            assert_eq!(type_of(Value::from_int(-1)), TypeTagKind::Int);
            assert_eq!(type_of(Value::from_int(1 << 40)), TypeTagKind::Int);
        }
    }

    #[test]
    fn type_of_reads_obj_header_for_pointers() {
        let heap_tags = [
            TypeTagKind::Str,
            TypeTagKind::List,
            TypeTagKind::Tuple,
            TypeTagKind::Dict,
            TypeTagKind::Set,
            TypeTagKind::Float,
            TypeTagKind::Instance,
            TypeTagKind::Iterator,
            TypeTagKind::Bytes,
            TypeTagKind::Generator,
            TypeTagKind::Cell,
            TypeTagKind::File,
        ];
        for tag in heap_tags {
            let (_keepalive, v) = heap_value_for_tag(tag);
            let got = unsafe { type_of(v) };
            assert_eq!(got, tag, "type_of pointer with tag {tag:?}");
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "runtime::type_of on null pointer")]
    fn type_of_null_pointer_panics_in_debug() {
        let null = Value::from_ptr::<Obj>(core::ptr::null_mut());
        unsafe {
            let _ = type_of(null);
        }
    }
}
