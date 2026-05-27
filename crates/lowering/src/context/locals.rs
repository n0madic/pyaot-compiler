//! Local variable and basic block allocation methods

use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{BlockId, LocalId};

use super::Lowering;

impl<'a> Lowering<'a> {
    /// Allocate a new local variable ID
    pub(crate) fn alloc_local_id(&mut self) -> LocalId {
        let id = LocalId::new(self.codegen.next_local_id);
        self.codegen.next_local_id += 1;
        id
    }

    /// Demote any `Never` reaching MIR storage to `Any`. Container
    /// parameters use `Type::demote_never_params_to_any` (covariant args,
    /// `Iterator`, `Union` recursion); a bare top-level `Type::Never`
    /// (produced by `extract_iterable_element_type(list[Never])` and
    /// similar element-type lookups for empty containers) becomes
    /// `Type::Any` because `Never` has no runtime representation and
    /// would panic in `type_to_cranelift`.
    fn demote_for_mir_storage(ty: Type) -> Type {
        match ty {
            Type::Never => Type::Any,
            other => other.demote_never_params_to_any(),
        }
    }

    /// Allocate a new local with the default register-level MirType derived
    /// from `ty` (primitives → `Raw(K)`, heap/Tagged → as per
    /// [`pyaot_mir::type_to_mir_type_register`]).
    ///
    /// GC-root status is computed from the resulting `mir_ty` via
    /// [`mir::Local::computed_is_gc_root`]: heap shapes and `Tagged` track,
    /// raw primitives and `FuncPtr`/`Closure` do not. Sites that need an
    /// override (e.g. a Tagged slot holding a non-heap code address) should
    /// use [`Self::alloc_and_add_local_with_mir_ty`] with an explicit
    /// `MirType::FuncPtr(sig)` or `MirType::Raw(I64)` instead.
    pub(crate) fn alloc_and_add_local(
        &mut self,
        ty: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let ty = Self::demote_for_mir_storage(ty);
        let local_id = self.alloc_local_id();
        let mir_ty = Some(mir::type_to_mir_type_register(&ty));
        mir_func.add_local(mir::Local {
            id: local_id,
            name: None,
            ty,
            abi_immutable: false,
            mir_ty,
        });
        local_id
    }

    /// Allocate a new local with an explicit MirType. Used by sites that
    /// need to override the register-level default (e.g. closure tuple
    /// slots, ABI-bound params, container storage, FuncPtr code addresses).
    pub(crate) fn alloc_and_add_local_with_mir_ty(
        &mut self,
        ty: Type,
        mir_ty: mir::MirType,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let ty = Self::demote_for_mir_storage(ty);
        let local_id = self.alloc_local_id();
        mir_func.add_local(mir::Local {
            id: local_id,
            name: None,
            ty,
            abi_immutable: false,
            mir_ty: Some(mir_ty),
        });
        local_id
    }

    /// Create a new basic block
    pub(crate) fn new_block(&mut self) -> mir::BasicBlock {
        let id = BlockId::from(self.codegen.next_block_id);
        self.codegen.next_block_id += 1;
        mir::BasicBlock {
            id,
            instructions: Vec::new(),
            terminator: mir::Terminator::Unreachable,
        }
    }

    /// Get mutable reference to current basic block
    pub(crate) fn current_block_mut(&mut self) -> &mut mir::BasicBlock {
        &mut self.codegen.current_blocks[self.codegen.current_block_idx]
    }

    /// Check if current block already has a terminator
    pub(crate) fn current_block_has_terminator(&self) -> bool {
        !matches!(
            self.codegen.current_blocks[self.codegen.current_block_idx].terminator,
            mir::Terminator::Unreachable
        )
    }

    /// Emit an instruction to the current basic block
    pub(crate) fn emit_instruction(&mut self, kind: mir::InstructionKind) {
        let span = self.codegen.current_span;
        self.current_block_mut()
            .instructions
            .push(mir::Instruction { kind, span });
    }

    /// Emit a runtime call: allocates a result local, emits the RuntimeCall instruction,
    /// and returns the LocalId of the result.
    ///
    /// This consolidates the common pattern of:
    ///   let dest = self.alloc_and_add_local(result_type, mir_func);
    ///   self.emit_instruction(RuntimeCall { dest, func, args });
    ///
    /// For genuinely void runtime helpers (where `runtime_call_is_void`
    /// returns true) prefer [`Self::emit_void_call`]; for non-void helpers
    /// whose result is intentionally discarded use
    /// [`Self::emit_call_discard_result`]. Both alternatives avoid the
    /// misuse pattern of treating an unwritten placeholder dest as a real
    /// SSA value.
    pub(crate) fn emit_runtime_call(
        &mut self,
        func: mir::RuntimeFunc,
        args: Vec<mir::Operand>,
        result_type: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let dest = self.alloc_and_add_local(result_type, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall { dest, func, args });
        dest
    }

    /// Emit a runtime call to a genuinely void helper — one where
    /// `pyaot_mir::runtime_call_is_void` returns true (e.g. `rt_list_append`,
    /// `rt_dict_set`, all `rt_global_set_*`, all `rt_print_*`, etc.).
    ///
    /// The allocated dest is a `Type::None` placeholder that codegen never
    /// writes through. The runtime descriptor's `returns: None` is what
    /// drives codegen to skip the result-extraction path
    /// (`codegen-cranelift/src/runtime_calls/mod.rs` only consults
    /// `var_map[&dest]` when `def.returns.is_some()`); since
    /// `InstructionKind::def()` also reports `None` for these calls, DCE is
    /// free to evict the placeholder from `func.locals`. Any downstream
    /// pass that reads `var_map[&dest]` for a void RuntimeCall would
    /// violate this contract.
    pub(crate) fn emit_void_call(
        &mut self,
        func: mir::RuntimeFunc,
        args: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) {
        debug_assert!(
            mir::runtime_call_is_void(&func),
            "emit_void_call called with non-void function {:?}",
            func
        );
        let dest = self.alloc_and_add_local(Type::None, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall { dest, func, args });
    }

    /// Emit a runtime call whose result is intentionally discarded.
    ///
    /// Unlike [`Self::emit_void_call`], this is for non-void runtime
    /// helpers (`runtime_call_is_void` returns false) where the caller
    /// doesn't need the return value. Example: `del xs[i]` lowers as
    /// `rt_list_pop(xs, i)` (returns the popped element) but the popped
    /// element is thrown away. The allocated dest is a `Type::None`
    /// placeholder that codegen still writes through; `InstructionKind::def()`
    /// reports `Some(dest)` so the local stays live but is simply never read.
    pub(crate) fn emit_call_discard_result(
        &mut self,
        func: mir::RuntimeFunc,
        args: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) {
        debug_assert!(
            !mir::runtime_call_is_void(&func),
            "emit_call_discard_result called with void function {:?} \
             (use emit_void_call instead)",
            func
        );
        let dest = self.alloc_and_add_local(Type::None, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall { dest, func, args });
    }

    /// Emit a runtime call whose result is a GC-tracked heap object.
    ///
    /// Like `emit_runtime_call` but marks the result local as a GC root
    /// (`is_gc_root: true`). Use for calls that allocate heap objects (strings,
    /// lists, dicts, etc.) that must be kept alive across GC collection points.
    pub(crate) fn emit_runtime_call_gc(
        &mut self,
        func: mir::RuntimeFunc,
        args: Vec<mir::Operand>,
        result_type: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let dest = self.alloc_and_add_local(result_type, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall { dest, func, args });
        dest
    }

    /// Emit `RT_TUPLE_GET(tuple, index)` and unbox the result if the declared
    /// element type is a primitive. After §F.4, `rt_tuple_get` always returns
    /// the raw tagged `Value` bits; this helper applies `UnwrapValueInt` /
    /// `UnwrapValueBool` / `rt_unbox_float` based on `elem_type`.
    /// Other types pass through as `HeapAny` / pointer.
    pub(crate) fn emit_tuple_get(
        &mut self,
        tuple_operand: mir::Operand,
        index_operand: mir::Operand,
        elem_type: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let needs_unbox = matches!(elem_type, Type::Int | Type::Bool | Type::Float);
        // For primitives, emit the raw call into HeapAny then unbox.
        // For `Any` elements: RT_TUPLE_GET returns a tagged Value (storage-uniform
        // invariant post §F.7c BigBang), so the result is guaranteed tagged —
        // use HeapAny rather than Any to signal this to downstream dispatch.
        // For all other heap/Union types, label with the declared element type.
        let raw_result_type = if needs_unbox || matches!(elem_type, Type::Any) {
            Type::Any
        } else {
            elem_type.clone()
        };
        let tagged_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET),
            vec![tuple_operand, index_operand],
            raw_result_type,
            mir_func,
        );
        if needs_unbox {
            let unboxed =
                self.unbox_if_needed(mir::Operand::Local(tagged_local), &elem_type, mir_func);
            match unboxed {
                mir::Operand::Local(id) => id,
                other => panic!("unexpected non-local unbox result: {other:?}"),
            }
        } else {
            tagged_local
        }
    }

    /// Emit `RT_INSTANCE_GET_FIELD(obj, offset)` and recover the typed
    /// value. Storage is uniform tagged `Value` for every field:
    /// `Float` slots hold `Value::from_ptr(*FloatObj)` recovered via
    /// `rt_unbox_float`; `Int` slots hold `Value::from_int(n)` recovered
    /// via `UnwrapValueInt`; `Bool` slots hold `Value::from_bool(b)`
    /// recovered via `UnwrapValueBool`; heap/dynamic shapes are loaded
    /// directly with `read_type` as the result label.
    pub(crate) fn emit_instance_get_field(
        &mut self,
        obj_operand: mir::Operand,
        offset: usize,
        read_type: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let load_label = match read_type {
            Type::Int | Type::Bool | Type::Float => Type::Any,
            _ => read_type.clone(),
        };
        let boxed_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INSTANCE_GET_FIELD),
            vec![
                obj_operand,
                mir::Operand::Constant(mir::Constant::Int(offset as i64)),
            ],
            load_label,
            mir_func,
        );
        match read_type {
            Type::Int | Type::Bool | Type::Float => {
                let dest = self.alloc_and_add_local(read_type.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::UnboxValue {
                    dest,
                    src: mir::Operand::Local(boxed_local),
                    dest_type: read_type.clone(),
                });
                dest
            }
            _ => boxed_local,
        }
    }
}
