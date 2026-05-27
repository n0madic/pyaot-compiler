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
    /// **Misuse note**: many existing call sites pass a void runtime helper
    /// (e.g. `rt_list_append`, `rt_dict_set`) through this entry point with
    /// `result_type = Type::None` and return the unwritten dest as the
    /// expression's value. The dest is never written by codegen — DCE
    /// later evicts it from `func.locals`. Reading the returned `Operand`
    /// would observe Cranelift's default zero, NOT a tagged `Value::None`.
    /// This is silently safe today only because callers discard the result
    /// (statement-form `xs.append(x)`). Future work: migrate those sites to
    /// [`Self::emit_runtime_call_void`] + an explicit `Const(None)` operand.
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

    /// Emit a runtime call whose result is discarded.
    ///
    /// The name is historical — this helper covers BOTH genuinely void
    /// runtime helpers (`runtime_call_is_void(func) == true`, e.g.
    /// `rt_list_append`, `rt_print_newline`) AND non-void helpers whose
    /// caller doesn't need the result (e.g. `del xs[i]` lowers as
    /// `rt_list_pop(...)` with result thrown away). The allocated dest is
    /// always a `Type::None` placeholder.
    ///
    /// **Void-dest invariant** — for genuinely void helpers,
    /// `InstructionKind::def()` reports `None` (since Phase 2 / commit
    /// 561ddec), so DCE's `eliminate_dead_locals` may evict this local from
    /// `func.locals`. Codegen is safe because `compile_runtime_func_def`
    /// only reads `var_map[&dest]` when `def.returns.is_some()` (see
    /// `codegen-cranelift/src/runtime_calls/mod.rs`). Any new pass added
    /// downstream MUST preserve that invariant — do not look up
    /// `var_map[&dest]` for a `RuntimeCall` whose runtime def is void.
    ///
    /// For the non-void / discarded-result case the local stays alive
    /// (`.def()` returns `Some(dest)`); the discarded result is simply
    /// never read.
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
