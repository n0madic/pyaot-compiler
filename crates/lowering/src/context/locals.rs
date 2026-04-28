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

    /// Allocate a new local variable and add it to the function.
    /// This is a helper to reduce boilerplate when creating locals.
    pub(crate) fn alloc_and_add_local(
        &mut self,
        ty: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let local_id = self.alloc_local_id();
        mir_func.add_local(mir::Local {
            id: local_id,
            name: None,
            ty: ty.clone(),
            is_gc_root: ty.is_heap(),
        });
        local_id
    }

    /// Allocate a new local variable with explicit gc_root flag.
    /// Use this when you need to override the default is_heap_type behavior.
    fn alloc_and_add_local_with_gc(
        &mut self,
        ty: Type,
        is_gc_root: bool,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        let local_id = self.alloc_local_id();
        mir_func.add_local(mir::Local {
            id: local_id,
            name: None,
            ty,
            is_gc_root,
        });
        local_id
    }

    /// Allocate a local that is a GC root (for heap-allocated types like str, list, dict).
    /// Use this for values that live on the heap and need garbage collection tracking.
    pub(crate) fn alloc_gc_local(&mut self, ty: Type, mir_func: &mut mir::Function) -> LocalId {
        self.alloc_and_add_local_with_gc(ty, true, mir_func)
    }

    /// Allocate a local that is NOT a GC root (for stack values or transient pointers).
    /// Use this for temporary values that don't need GC tracking, such as:
    /// - Static string pointers (compile-time constants)
    /// - Transient pointers used only for immediate unboxing
    pub(crate) fn alloc_stack_local(&mut self, ty: Type, mir_func: &mut mir::Function) -> LocalId {
        self.alloc_and_add_local_with_gc(ty, false, mir_func)
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

    /// Emit a void runtime call (no meaningful return value).
    ///
    /// Allocates a throwaway `Type::None` local as the required dest slot, then emits
    /// the RuntimeCall. Use this for calls whose return value is never used (e.g. print,
    /// gc_push/gc_pop, GC root registration).
    pub(crate) fn emit_runtime_call_void(
        &mut self,
        func: mir::RuntimeFunc,
        args: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) {
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
        let dest = self.alloc_gc_local(result_type, mir_func);
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
        // For other types, label the result with the declared element type so
        // downstream comparison/dispatch paths see the right type (e.g. Union or
        // HeapAny for `Any` elements) rather than losing the type information.
        let raw_result_type = if needs_unbox {
            Type::HeapAny
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
    /// value. After Phase 2 §F.7c, fields are uniform tagged `Value`s:
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
        if matches!(read_type, Type::Float) {
            return self.emit_runtime_call(
                mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_INSTANCE_GET_FIELD_F64,
                ),
                vec![
                    obj_operand,
                    mir::Operand::Constant(mir::Constant::Int(offset as i64)),
                ],
                Type::Float,
                mir_func,
            );
        }
        if matches!(read_type, Type::Int | Type::Bool) {
            let boxed_local = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INSTANCE_GET_FIELD),
                vec![
                    obj_operand,
                    mir::Operand::Constant(mir::Constant::Int(offset as i64)),
                ],
                Type::HeapAny,
                mir_func,
            );
            let dest = self.alloc_and_add_local(read_type.clone(), mir_func);
            let src = mir::Operand::Local(boxed_local);
            self.emit_instruction(if matches!(read_type, Type::Int) {
                mir::InstructionKind::UnwrapValueInt { dest, src }
            } else {
                mir::InstructionKind::UnwrapValueBool { dest, src }
            });
            return dest;
        }
        self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INSTANCE_GET_FIELD),
            vec![
                obj_operand,
                mir::Operand::Constant(mir::Constant::Int(offset as i64)),
            ],
            read_type,
            mir_func,
        )
    }
}
