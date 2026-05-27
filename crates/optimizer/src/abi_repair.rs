//! MIR ABI repair after type materialization.
//!
//! Phase 1's production pipeline materializes whole-program types back into
//! MIR locals, params, and returns before codegen. This pass makes the
//! already-lowered MIR agree with those materialized function signatures by
//! inserting explicit coercions at internal call sites and rewriting
//! singleton-target internal calls to `CallDirect`.

use std::collections::HashMap;

use indexmap::{IndexMap, IndexSet};
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_mir::{
    ClassMetadata, Constant, Function, Instruction, InstructionKind, MirType, Module, Operand,
    RawKind, RuntimeFunc,
};
use pyaot_types::Type;
use pyaot_utils::{BlockId, ClassId, FuncId, LocalId, Span};

/// Build a boxing `InstructionKind` that wraps a primitive `src` (of
/// `src_ty`) into a tagged `Value` at `dest`. Used in two contexts:
///
/// - `coerce_operand` where lowering already allocated `dest`.
/// - `emit_call_direct_with_bridge` where the boxed `dest` carries the
///   widened-return-type slot.
///
/// `src_ty` must be one of `Int`, `Bool`, `Float`, `None` — primitives that
/// have a defined tagged representation. Heap types do not need boxing
/// (they're already pointer-shaped); callers must filter before calling.
///
/// Panics on non-primitive `src_ty` (programmer error).
fn box_primitive_inst(src: Operand, src_ty: &Type, dest: LocalId) -> InstructionKind {
    match src_ty {
        Type::Int => InstructionKind::BoxValue {
            dest,
            src,
            src_type: Type::Int,
        },
        Type::Bool => InstructionKind::BoxValue {
            dest,
            src,
            src_type: Type::Bool,
        },
        Type::Float => InstructionKind::RuntimeCall {
            dest,
            func: RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT),
            args: vec![src],
        },
        Type::None => InstructionKind::RuntimeCall {
            dest,
            func: RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_NONE),
            // rt_box_none() is a zero-arg singleton accessor — the boxed
            // `None` value is the same regardless of source. Verifier
            // catches arity mismatches.
            args: vec![],
        },
        _ => unreachable!("box_primitive_inst: non-primitive src_ty={:?}", src_ty),
    }
}

/// Allocate a fresh temp local and emit a boxing instruction wrapping
/// `operand` (of `actual_ty`) into a tagged Value slot typed `dest_ty`.
/// Returns the boxed Operand.
fn emit_box_primitive(
    func: &mut Function,
    next_local_id: &mut u32,
    operand: Operand,
    actual_ty: &Type,
    dest_ty: Type,
    span: Option<Span>,
    emitted: &mut Vec<Instruction>,
) -> Operand {
    let dest = alloc_temp_local(func, next_local_id, dest_ty);
    emitted.push(Instruction {
        kind: box_primitive_inst(operand, actual_ty, dest),
        span,
    });
    Operand::Local(dest)
}

fn operand_type(operand: &Operand, func: &Function) -> Type {
    match operand {
        Operand::Local(id) => func
            .locals
            .get(id)
            .map(|local| local.ty.clone())
            .unwrap_or(Type::Any),
        Operand::Constant(Constant::Int(_)) => Type::Int,
        Operand::Constant(Constant::Float(_)) => Type::Float,
        Operand::Constant(Constant::Bool(_)) => Type::Bool,
        Operand::Constant(Constant::Str(_)) => Type::Str,
        Operand::Constant(Constant::Bytes(_)) => Type::Bytes,
        Operand::Constant(Constant::None) => Type::None,
    }
}

fn alloc_temp_local(func: &mut Function, next_local_id: &mut u32, ty: Type) -> LocalId {
    let id = LocalId::from(*next_local_id);
    *next_local_id += 1;
    let mir_ty = pyaot_mir::type_to_mir_type_register(&ty);
    func.locals.insert(
        id,
        pyaot_mir::Local {
            id,
            name: None,
            ty: ty.clone(),
            abi_immutable: false,
            // Phase 3e: derive mir_ty so abi_repair-introduced temps have a
            // precise MirType signature for the verifier and downstream
            // optimiser passes (was `mir_ty: None` which forced fallback
            // register-level translation of `ty`).
            mir_ty: Some(mir_ty),
        },
    );
    id
}

fn next_local_seed(func: &Function) -> u32 {
    func.locals
        .keys()
        .map(|id| id.0)
        .chain(func.params.iter().map(|local| local.id.0))
        .max()
        .map(|max| max.saturating_add(1))
        .unwrap_or(0)
}

fn build_def_map(func: &Function) -> HashMap<LocalId, InstructionKind> {
    let mut defs = HashMap::new();
    for block in func.blocks.values() {
        for inst in &block.instructions {
            let dest = match &inst.kind {
                InstructionKind::Const { dest, .. }
                | InstructionKind::BinOp { dest, .. }
                | InstructionKind::UnOp { dest, .. }
                | InstructionKind::Call { dest, .. }
                | InstructionKind::CallDirect { dest, .. }
                | InstructionKind::CallNamed { dest, .. }
                | InstructionKind::CallVirtual { dest, .. }
                | InstructionKind::CallVirtualNamed { dest, .. }
                | InstructionKind::FuncAddr { dest, .. }
                | InstructionKind::BuiltinAddr { dest, .. }
                | InstructionKind::RuntimeCall { dest, .. }
                | InstructionKind::Copy { dest, .. }
                | InstructionKind::GcAlloc { dest, .. }
                | InstructionKind::FloatToInt { dest, .. }
                | InstructionKind::BoolToInt { dest, .. }
                | InstructionKind::IntToFloat { dest, .. }
                | InstructionKind::FloatBits { dest, .. }
                | InstructionKind::IntBitsToFloat { dest, .. }
                | InstructionKind::BoxValue { dest, .. }
                | InstructionKind::UnboxValue { dest, .. }
                | InstructionKind::FloatAbs { dest, .. }
                | InstructionKind::ExcGetType { dest }
                | InstructionKind::ExcHasException { dest }
                | InstructionKind::ExcGetCurrent { dest }
                | InstructionKind::ExcCheckType { dest, .. }
                | InstructionKind::ExcCheckClass { dest, .. }
                | InstructionKind::Phi { dest, .. }
                | InstructionKind::Refine { dest, .. } => Some(*dest),
                _ => None,
            };
            if let Some(dest) = dest {
                defs.insert(dest, inst.kind.clone());
            }
        }
    }
    defs
}

fn operand_is_exception_word_source(
    operand: &Operand,
    def_map: &HashMap<LocalId, InstructionKind>,
) -> bool {
    fn local_is_exception_word_source(
        local: LocalId,
        def_map: &HashMap<LocalId, InstructionKind>,
        visiting: &mut IndexSet<LocalId>,
    ) -> bool {
        if !visiting.insert(local) {
            return false;
        }
        let result = match def_map.get(&local) {
            Some(InstructionKind::Copy { src, .. } | InstructionKind::Refine { src, .. }) => {
                operand_is_exception_word_source_inner(src, def_map, visiting)
            }
            Some(InstructionKind::Phi { sources, .. }) => sources
                .iter()
                .all(|(_, src)| operand_is_exception_word_source_inner(src, def_map, visiting)),
            Some(InstructionKind::ExcGetCurrent { .. }) => true,
            _ => false,
        };
        visiting.shift_remove(&local);
        result
    }

    fn operand_is_exception_word_source_inner(
        operand: &Operand,
        def_map: &HashMap<LocalId, InstructionKind>,
        visiting: &mut IndexSet<LocalId>,
    ) -> bool {
        match operand {
            Operand::Local(local) => local_is_exception_word_source(*local, def_map, visiting),
            Operand::Constant(_) => false,
        }
    }

    operand_is_exception_word_source_inner(operand, def_map, &mut IndexSet::new())
}

fn boxed_value_hint(
    operand: &Operand,
    def_map: &HashMap<LocalId, InstructionKind>,
) -> Option<Type> {
    fn inner(local: LocalId, def_map: &HashMap<LocalId, InstructionKind>) -> Option<Type> {
        match def_map.get(&local)?.clone() {
            InstructionKind::Copy { src, .. } | InstructionKind::Refine { src, .. } => {
                boxed_value_hint(&src, def_map)
            }
            InstructionKind::BoolToInt { .. } | InstructionKind::FloatToInt { .. } => {
                Some(Type::Int)
            }
            InstructionKind::IntToFloat { .. } | InstructionKind::IntBitsToFloat { .. } => {
                Some(Type::Float)
            }
            kind => kind.boxed_primitive_type(),
        }
    }

    match operand {
        Operand::Constant(Constant::Int(_)) => Some(Type::Int),
        Operand::Constant(Constant::Float(_)) => Some(Type::Float),
        Operand::Constant(Constant::Bool(_)) => Some(Type::Bool),
        Operand::Constant(Constant::None) => Some(Type::None),
        Operand::Constant(Constant::Str(_)) => Some(Type::Str),
        Operand::Constant(Constant::Bytes(_)) => Some(Type::Bytes),
        Operand::Local(local) => inner(*local, def_map),
    }
}

fn is_runtime_unbox_source(ty: &Type) -> bool {
    matches!(ty, Type::Any | Type::Union(_))
}

/// Emit `CallDirect { dest: temp(callee_return_ty) }` and then bridge
/// `temp → original_dest` with either a BoxValue (when narrowing a wide dest
/// to a primitive callee return) or a Refine (narrower callee into wider dest).
///
/// This mirrors the `Call → CallDirect` singleton-narrowing bridge used for
/// indirect calls and is now also applied to `CallVirtual → CallDirect` and
/// `CallVirtualNamed → CallDirect` narrowing to eliminate Source-2 raw-bit
/// leaks (post-devirt `Type::Any` dest locals whose callee is NOT
/// return-flipped yet returns raw primitive bits).
///
/// When `callee_return_ty == original_dest_ty` no bridge is needed; the
/// helper emits a plain `CallDirect { dest: original_dest }` directly.
#[allow(clippy::too_many_arguments)]
fn emit_call_direct_with_bridge(
    dest: pyaot_utils::LocalId,
    callee: pyaot_utils::FuncId,
    args: Vec<Operand>,
    callee_return_ty: &Type,
    original_dest_ty: &Type,
    func: &mut Function,
    next_local_id: &mut u32,
    repaired: &mut Vec<Instruction>,
    span: Option<pyaot_utils::Span>,
) {
    if callee_return_ty == original_dest_ty {
        repaired.push(Instruction {
            kind: InstructionKind::CallDirect {
                dest,
                func: callee,
                args,
            },
            span,
        });
        return;
    }

    let temp_dest = alloc_temp_local(func, next_local_id, callee_return_ty.clone());
    repaired.push(Instruction {
        kind: InstructionKind::CallDirect {
            dest: temp_dest,
            func: callee,
            args,
        },
        span,
    });

    // Source-2 fix: when the original dest is a wide type (Any/HeapAny/Union)
    // and the callee returns a raw primitive (non-return-flipped callee), the
    // raw bits must be boxed into a tagged Value before being stored into the
    // Any-typed dest.  Without this boxing, the dest local carries raw
    // primitive bits under a Type::Any label — after the F.1 HeapAny→Any
    // collapse, such slots become indistinguishable from guaranteed-tagged
    // Any slots, causing the rt_obj_* routing guard in binary_ops.rs to
    // route raw-bit operands through tagged-Value runtime helpers → SIGSEGV.
    //
    // Return-flipped callees are identified by having `return_type == Any`
    // (the invariant maintained by `materialize_function_return_types`), so
    // their `callee_return_ty` will be `Any` — not a primitive — and this
    // path is not taken.  Non-flipped primitive-returning callees have
    // `callee_return_ty` = Int / Bool / Float / None, triggering boxing.
    let widening_to_heap = matches!(original_dest_ty, Type::Any | Type::Union(_))
        && matches!(
            callee_return_ty,
            Type::Int | Type::Bool | Type::Float | Type::None
        );

    if widening_to_heap {
        repaired.push(Instruction {
            kind: box_primitive_inst(Operand::Local(temp_dest), callee_return_ty, dest),
            span,
        });
    } else {
        repaired.push(Instruction {
            kind: InstructionKind::Refine {
                dest,
                src: Operand::Local(temp_dest),
                ty: original_dest_ty.clone(),
            },
            span,
        });
    }
}

fn coerce_error_with_site(
    err: CompilerError,
    kind: &str,
    func_name: &str,
    block: BlockId,
) -> CompilerError {
    CompilerError::codegen_error(
        format!("{err} in {kind} call at {func_name} block {block:?}"),
        None,
    )
}

fn lookup_field_type(
    class_info: &IndexMap<ClassId, ClassMetadata>,
    func: &Function,
    obj: &Operand,
    offset: &Operand,
) -> Option<Type> {
    let class_id = match operand_type(obj, func) {
        Type::Class { class_id, .. } => class_id,
        _ => return None,
    };
    let Operand::Constant(Constant::Int(offset)) = offset else {
        return None;
    };
    let meta = class_info.get(&class_id)?;
    let field_name = meta
        .field_offsets
        .iter()
        .find_map(|(name, off)| (*off == *offset as usize).then_some(*name))?;
    meta.field_types.get(&field_name).cloned()
}

fn function_operand_abi(operand: &Operand, func: &Function) -> Option<UnifiedCallAbi> {
    match operand_type(operand, func) {
        Type::Function { params, ret } => Some(UnifiedCallAbi {
            params,
            return_type: *ret,
            singleton_target: None,
        }),
        _ => None,
    }
}

fn callable_operand_is_runtime_erased(
    operand: &Operand,
    def_map: &HashMap<LocalId, InstructionKind>,
) -> bool {
    fn local_is_runtime_erased(
        local: LocalId,
        def_map: &HashMap<LocalId, InstructionKind>,
        visiting: &mut IndexSet<LocalId>,
    ) -> bool {
        if !visiting.insert(local) {
            return false;
        }
        let result = match def_map.get(&local) {
            Some(InstructionKind::Copy { src, .. } | InstructionKind::Refine { src, .. }) => {
                operand_is_runtime_erased(src, def_map, visiting)
            }
            Some(InstructionKind::Phi { sources, .. }) => sources
                .iter()
                .all(|(_, src)| operand_is_runtime_erased(src, def_map, visiting)),
            Some(InstructionKind::RuntimeCall {
                func: RuntimeFunc::Call(def),
                ..
            }) => std::ptr::eq(*def, &pyaot_core_defs::runtime_func_def::RT_TUPLE_GET),
            // Parameters / opaque locals without a local def are runtime-erased from
            // ABI repair's point of view: there is no exact target contract to repair against.
            None => true,
            _ => false,
        };
        visiting.shift_remove(&local);
        result
    }

    fn operand_is_runtime_erased(
        operand: &Operand,
        def_map: &HashMap<LocalId, InstructionKind>,
        visiting: &mut IndexSet<LocalId>,
    ) -> bool {
        match operand {
            Operand::Local(local) => local_is_runtime_erased(*local, def_map, visiting),
            Operand::Constant(_) => false,
        }
    }

    operand_is_runtime_erased(operand, def_map, &mut IndexSet::new())
}

#[derive(Debug, Clone)]
struct UnifiedCallAbi {
    params: Vec<Type>,
    return_type: Type,
    singleton_target: Option<FuncId>,
}

fn unify_internal_call_abi(
    signatures: &HashMap<FuncId, (Vec<Type>, Type)>,
    targets: &IndexSet<FuncId>,
    kind: &str,
    func_name: &str,
    block: BlockId,
) -> Result<Option<UnifiedCallAbi>> {
    let mut targets = targets.iter().copied();
    let Some(first_target) = targets.next() else {
        return Ok(None);
    };
    let Some((params, return_type)) = signatures.get(&first_target) else {
        return Err(CompilerError::codegen_error(
            format!(
                "missing callee {:?} while repairing {kind} call in {func_name} block {block:?}",
                first_target
            ),
            None,
        ));
    };
    let params = params.clone();
    let return_type = return_type.clone();
    let mut singleton_target = Some(first_target);

    for target in targets {
        let Some((other_params, other_return_type)) = signatures.get(&target) else {
            return Err(CompilerError::codegen_error(
                format!(
                    "missing callee {:?} while repairing {kind} call in {func_name} block {block:?}",
                    target
                ),
                None,
            ));
        };
        if *other_params != params || *other_return_type != return_type {
            return Err(CompilerError::codegen_error(
                format!(
                    "divergent ABI across internal {kind} targets in {func_name} block {block:?}"
                ),
                None,
            ));
        }
        singleton_target = None;
    }

    Ok(Some(UnifiedCallAbi {
        params,
        return_type,
        singleton_target,
    }))
}

fn filter_targets_by_arity(
    signatures: &HashMap<FuncId, (Vec<Type>, Type)>,
    targets: &IndexSet<FuncId>,
    arity: usize,
) -> IndexSet<FuncId> {
    targets
        .iter()
        .copied()
        .filter(|target| {
            signatures
                .get(target)
                .is_some_and(|(params, _)| params.len() == arity)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn repair_positional_args(
    args: Vec<Operand>,
    expected_params: &[Type],
    func: &mut Function,
    def_map: &HashMap<LocalId, InstructionKind>,
    emitted: &mut Vec<Instruction>,
    next_local_id: &mut u32,
    kind: &str,
    func_name: &str,
    block: BlockId,
) -> Result<Vec<Operand>> {
    if args.len() != expected_params.len() {
        return Err(CompilerError::codegen_error(
            format!(
                "{kind} call at {func_name} block {block:?} has {} args but ABI expects {}",
                args.len(),
                expected_params.len()
            ),
            None,
        ));
    }

    let mut new_args = Vec::with_capacity(args.len());
    for (arg, expected) in args.into_iter().zip(expected_params.iter()) {
        let repaired_arg = coerce_operand(arg, expected, func, def_map, emitted, next_local_id)
            .map_err(|err| coerce_error_with_site(err, kind, func_name, block))?;
        new_args.push(repaired_arg);
    }
    Ok(new_args)
}

fn coerce_operand(
    operand: Operand,
    expected: &Type,
    func: &mut Function,
    def_map: &HashMap<LocalId, InstructionKind>,
    emitted: &mut Vec<Instruction>,
    next_local_id: &mut u32,
) -> Result<Operand> {
    let actual = operand_type(&operand, func);
    let boxed_hint = boxed_value_hint(&operand, def_map);
    if actual == *expected {
        return Ok(operand);
    }

    let span = None;
    match expected {
        Type::Int => match actual {
            Type::Int => Ok(operand),
            Type::Bool => {
                if let Operand::Constant(Constant::Bool(value)) = operand {
                    Ok(Operand::Constant(Constant::Int(i64::from(value))))
                } else {
                    let dest = alloc_temp_local(func, next_local_id, Type::Int);
                    emitted.push(Instruction {
                        kind: InstructionKind::BoolToInt { dest, src: operand },
                        span,
                    });
                    Ok(Operand::Local(dest))
                }
            }
            Type::None => match operand {
                Operand::Constant(Constant::None) => Ok(Operand::Constant(Constant::Int(0))),
                _ => {
                    let dest = alloc_temp_local(func, next_local_id, Type::Int);
                    emitted.push(Instruction {
                        kind: InstructionKind::BoolToInt { dest, src: operand },
                        span,
                    });
                    Ok(Operand::Local(dest))
                }
            },
            Type::Float => {
                let dest = alloc_temp_local(func, next_local_id, Type::Int);
                emitted.push(Instruction {
                    kind: InstructionKind::FloatToInt { dest, src: operand },
                    span,
                });
                Ok(Operand::Local(dest))
            }
            _ if is_runtime_unbox_source(&actual) => match boxed_hint {
                Some(Type::Float) => {
                    let float_dest = alloc_temp_local(func, next_local_id, Type::Float);
                    emitted.push(Instruction {
                        kind: InstructionKind::RuntimeCall {
                            dest: float_dest,
                            func: RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT,
                            ),
                            args: vec![operand],
                        },
                        span,
                    });
                    let int_dest = alloc_temp_local(func, next_local_id, Type::Int);
                    emitted.push(Instruction {
                        kind: InstructionKind::FloatToInt {
                            dest: int_dest,
                            src: Operand::Local(float_dest),
                        },
                        span,
                    });
                    Ok(Operand::Local(int_dest))
                }
                Some(Type::Bool) => {
                    let bool_dest = alloc_temp_local(func, next_local_id, Type::Bool);
                    emitted.push(Instruction {
                        kind: InstructionKind::UnboxValue {
                            dest: bool_dest,
                            src: operand,
                            dest_type: Type::Bool,
                        },
                        span,
                    });
                    let int_dest = alloc_temp_local(func, next_local_id, Type::Int);
                    emitted.push(Instruction {
                        kind: InstructionKind::BoolToInt {
                            dest: int_dest,
                            src: Operand::Local(bool_dest),
                        },
                        span,
                    });
                    Ok(Operand::Local(int_dest))
                }
                None if operand_is_exception_word_source(&operand, def_map) => Ok(operand),
                _ => {
                    let dest = alloc_temp_local(func, next_local_id, Type::Int);
                    emitted.push(Instruction {
                        kind: InstructionKind::UnboxValue {
                            dest,
                            src: operand,
                            dest_type: Type::Int,
                        },
                        span,
                    });
                    Ok(Operand::Local(dest))
                }
            },
            _ => Err(CompilerError::codegen_error(
                format!("cannot coerce {:?} to Int at call site", actual),
                None,
            )),
        },
        Type::Float => match actual {
            Type::Float => Ok(operand),
            Type::Int => {
                let dest = alloc_temp_local(func, next_local_id, Type::Float);
                emitted.push(Instruction {
                    kind: InstructionKind::IntToFloat { dest, src: operand },
                    span,
                });
                Ok(Operand::Local(dest))
            }
            Type::Bool => {
                let int_dest = alloc_temp_local(func, next_local_id, Type::Int);
                emitted.push(Instruction {
                    kind: InstructionKind::BoolToInt {
                        dest: int_dest,
                        src: operand,
                    },
                    span,
                });
                let float_dest = alloc_temp_local(func, next_local_id, Type::Float);
                emitted.push(Instruction {
                    kind: InstructionKind::IntToFloat {
                        dest: float_dest,
                        src: Operand::Local(int_dest),
                    },
                    span,
                });
                Ok(Operand::Local(float_dest))
            }
            _ if is_runtime_unbox_source(&actual) => match boxed_hint {
                Some(Type::Int) => {
                    let int_dest = alloc_temp_local(func, next_local_id, Type::Int);
                    emitted.push(Instruction {
                        kind: InstructionKind::UnboxValue {
                            dest: int_dest,
                            src: operand,
                            dest_type: Type::Int,
                        },
                        span,
                    });
                    let float_dest = alloc_temp_local(func, next_local_id, Type::Float);
                    emitted.push(Instruction {
                        kind: InstructionKind::IntToFloat {
                            dest: float_dest,
                            src: Operand::Local(int_dest),
                        },
                        span,
                    });
                    Ok(Operand::Local(float_dest))
                }
                Some(Type::Bool) => {
                    let bool_dest = alloc_temp_local(func, next_local_id, Type::Bool);
                    emitted.push(Instruction {
                        kind: InstructionKind::UnboxValue {
                            dest: bool_dest,
                            src: operand,
                            dest_type: Type::Bool,
                        },
                        span,
                    });
                    let int_dest = alloc_temp_local(func, next_local_id, Type::Int);
                    emitted.push(Instruction {
                        kind: InstructionKind::BoolToInt {
                            dest: int_dest,
                            src: Operand::Local(bool_dest),
                        },
                        span,
                    });
                    let float_dest = alloc_temp_local(func, next_local_id, Type::Float);
                    emitted.push(Instruction {
                        kind: InstructionKind::IntToFloat {
                            dest: float_dest,
                            src: Operand::Local(int_dest),
                        },
                        span,
                    });
                    Ok(Operand::Local(float_dest))
                }
                _ => {
                    let dest = alloc_temp_local(func, next_local_id, Type::Float);
                    emitted.push(Instruction {
                        kind: InstructionKind::RuntimeCall {
                            dest,
                            func: RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT,
                            ),
                            args: vec![operand],
                        },
                        span,
                    });
                    Ok(Operand::Local(dest))
                }
            },
            _ => Err(CompilerError::codegen_error(
                format!("cannot coerce {:?} to Float at call site", actual),
                None,
            )),
        },
        Type::Bool => match actual {
            Type::Bool => Ok(operand),
            _ if is_runtime_unbox_source(&actual) => {
                let dest = alloc_temp_local(func, next_local_id, Type::Bool);
                emitted.push(Instruction {
                    kind: InstructionKind::UnboxValue {
                        dest,
                        src: operand,
                        dest_type: Type::Bool,
                    },
                    span,
                });
                Ok(Operand::Local(dest))
            }
            _ => Err(CompilerError::codegen_error(
                format!("cannot coerce {:?} to Bool at call site", actual),
                None,
            )),
        },
        Type::None => match actual {
            Type::None => Ok(operand),
            _ => Err(CompilerError::codegen_error(
                format!("cannot coerce {:?} to None at call site", actual),
                None,
            )),
        },
        Type::Any | Type::Union(_) => match actual {
            Type::Int | Type::Bool | Type::Float | Type::None => Ok(emit_box_primitive(
                func,
                next_local_id,
                operand,
                &actual,
                expected.clone(),
                span,
                emitted,
            )),
            _ => Ok(operand),
        },
        _ if expected.is_heap() => match actual {
            Type::None => match operand {
                Operand::Constant(Constant::None) => Ok(Operand::Constant(Constant::Int(0))),
                _ => {
                    let dest = alloc_temp_local(func, next_local_id, expected.clone());
                    emitted.push(Instruction {
                        kind: InstructionKind::BoolToInt { dest, src: operand },
                        span,
                    });
                    Ok(Operand::Local(dest))
                }
            },
            _ if actual.is_heap() => Ok(operand),
            _ => Err(CompilerError::codegen_error(
                format!(
                    "cannot coerce {:?} to heap type {:?} at call site",
                    actual, expected
                ),
                None,
            )),
        },
        _ => Ok(operand),
    }
}

pub fn repair_mir_abi_from_types(module: &mut Module) -> Result<()> {
    let call_graph = crate::call_graph::CallGraph::build(module);
    let signatures_by_func: HashMap<FuncId, (Vec<Type>, Type)> = module
        .functions
        .iter()
        .map(|(&func_id, func)| {
            (
                func_id,
                (
                    func.params.iter().map(|param| param.ty.clone()).collect(),
                    func.return_type.clone(),
                ),
            )
        })
        .collect();
    let param_types_by_func: HashMap<FuncId, Vec<Type>> = module
        .functions
        .iter()
        .map(|(&func_id, func)| {
            (
                func_id,
                func.params.iter().map(|local| local.ty.clone()).collect(),
            )
        })
        .collect();

    let module_field_types = module.class_info.clone();
    for func in module.functions.values_mut() {
        let mut next_local_id = next_local_seed(func);
        let func_name = func.name.clone();
        let block_ids: Vec<_> = func.blocks.keys().copied().collect();
        let def_map = build_def_map(func);

        for block_id in block_ids {
            let old = {
                let block = func
                    .blocks
                    .get_mut(&block_id)
                    .expect("block id collected from function must exist");
                std::mem::take(&mut block.instructions)
            };
            let mut repaired = Vec::with_capacity(old.len());

            for (inst_idx, inst) in old.into_iter().enumerate() {
                match inst.kind {
                    InstructionKind::CallDirect {
                        dest,
                        func: callee,
                        args,
                    } => {
                        let Some(expected_params) = param_types_by_func.get(&callee) else {
                            repaired.push(Instruction {
                                kind: InstructionKind::CallDirect {
                                    dest,
                                    func: callee,
                                    args,
                                },
                                span: inst.span,
                            });
                            continue;
                        };

                        let new_args = repair_positional_args(
                            args,
                            expected_params,
                            func,
                            &def_map,
                            &mut repaired,
                            &mut next_local_id,
                            "direct",
                            &func_name,
                            block_id,
                        )?;

                        repaired.push(Instruction {
                            kind: InstructionKind::CallDirect {
                                dest,
                                func: callee,
                                args: new_args,
                            },
                            span: inst.span,
                        });
                    }
                    InstructionKind::CallNamed { dest, name, args } => {
                        let targets = call_graph.targets_at(func.id, block_id, inst_idx);
                        let Some(abi) = unify_internal_call_abi(
                            &signatures_by_func,
                            &targets,
                            "named",
                            &func_name,
                            block_id,
                        )?
                        else {
                            repaired.push(Instruction {
                                kind: InstructionKind::CallNamed { dest, name, args },
                                span: inst.span,
                            });
                            continue;
                        };
                        debug_assert!(
                            abi.return_type == func.locals.get(&dest).map(|local| local.ty.clone()).unwrap_or(Type::Any)
                                || matches!(abi.return_type, Type::Any | Type::Union(_)),
                            "named call return type mismatch should have been materialized before ABI repair"
                        );
                        let new_args = repair_positional_args(
                            args,
                            &abi.params,
                            func,
                            &def_map,
                            &mut repaired,
                            &mut next_local_id,
                            "named",
                            &func_name,
                            block_id,
                        )?;
                        match abi.singleton_target {
                            Some(callee) => repaired.push(Instruction {
                                kind: InstructionKind::CallDirect {
                                    dest,
                                    func: callee,
                                    args: new_args,
                                },
                                span: inst.span,
                            }),
                            None => repaired.push(Instruction {
                                kind: InstructionKind::CallNamed {
                                    dest,
                                    name,
                                    args: new_args,
                                },
                                span: inst.span,
                            }),
                        }
                    }
                    InstructionKind::Call {
                        dest,
                        func: callee_operand,
                        args,
                    } => {
                        let raw_targets = call_graph.targets_at(func.id, block_id, inst_idx);
                        let site_exact =
                            call_graph.site_targets_are_exact(func.id, block_id, inst_idx);
                        let targets =
                            filter_targets_by_arity(&signatures_by_func, &raw_targets, args.len());
                        let abi = if targets.len() == 1 {
                            unify_internal_call_abi(
                                &signatures_by_func,
                                &targets,
                                "indirect",
                                &func_name,
                                block_id,
                            )?
                        } else if let Some(function_abi) =
                            function_operand_abi(&callee_operand, func)
                        {
                            Some(function_abi)
                        } else if !site_exact
                            || callable_operand_is_runtime_erased(&callee_operand, &def_map)
                        {
                            None
                        } else {
                            unify_internal_call_abi(
                                &signatures_by_func,
                                &targets,
                                "indirect",
                                &func_name,
                                block_id,
                            )?
                        };
                        let Some(abi) = abi else {
                            repaired.push(Instruction {
                                kind: InstructionKind::Call {
                                    dest,
                                    func: callee_operand,
                                    args,
                                },
                                span: inst.span,
                            });
                            continue;
                        };
                        let new_args = repair_positional_args(
                            args,
                            &abi.params,
                            func,
                            &def_map,
                            &mut repaired,
                            &mut next_local_id,
                            "indirect",
                            &func_name,
                            block_id,
                        )?;
                        match abi.singleton_target {
                            Some(callee) => {
                                // Preserve the dispatch's original dest type when
                                // narrowing `Call(func_ptr, ...)` to `CallDirect(callee, ...)`.
                                //
                                // emit_capture_dispatch in lowering allocates each
                                // branch's `branch_result` with a uniform `result_ty`
                                // (the user-declared return type, e.g. `Int`). At
                                // runtime only one branch matches `n_captures` and
                                // executes — the others are dead. But the static
                                // call-graph + arity filter narrows each Call to
                                // whichever address-taken function matches that arity,
                                // which may have a different return type (e.g. the
                                // outer closure factory returns `Any`/`HeapAny`).
                                //
                                // Without the bridge below, `type_inference`'s
                                // `CallDirect` rule overwrites the dest's type with
                                // the callee's return type, which then widens through
                                // the merge-block Phi to `Union[Int, Any]`. The Phi
                                // codegen then mishandles the join, and downstream
                                // raw-int consumers (`rt_global_set_int`,
                                // `rt_print_int_value`) receive tagged `Value` bits
                                // instead of raw int — printing `(payload << 3) | 1`
                                // (e.g. 49 for payload 6) or SEGV-ing when raw bits
                                // are misread as a heap pointer.
                                //
                                // Fix: emit `CallDirect` into a fresh temp typed by
                                // the callee, then bridge to the original dest:
                                //
                                //   - Narrowing (callee returns wider, dest narrower —
                                //     e.g. `Any → Int`): use `Refine`. This is a
                                //     compile-time narrowing hint with no runtime
                                //     conversion. Live branches by construction
                                //     return the matching type, and dead branches
                                //     (statically unreachable for the runtime
                                //     `n_captures` value) simply carry a stale type
                                //     label whose bits are never observed. Avoids
                                //     `UnwrapValueInt` in inlined branches that
                                //     produce raw bits directly.
                                //
                                //   - Widening (callee returns narrower, dest wider
                                //     — e.g. `Int → Any`): emit `ValueFromInt` /
                                //     `ValueFromBool` (MIR inline boxing) or
                                //     `RT_BOX_FLOAT` / `RT_BOX_NONE` so the bits
                                //     are properly tagged. The Phi merge stays
                                //     uniform under the dest's wider type, and
                                //     consumers (`rt_print_obj`, `rt_global_set_ptr`)
                                //     see well-formed `Value`-tagged bits rather than
                                //     raw scalars that would mis-decode as invalid
                                //     pointers.
                                let original_dest_ty = func
                                    .locals
                                    .get(&dest)
                                    .map(|local| local.ty.clone())
                                    .unwrap_or(Type::Any);
                                emit_call_direct_with_bridge(
                                    dest,
                                    callee,
                                    new_args,
                                    &abi.return_type,
                                    &original_dest_ty,
                                    func,
                                    &mut next_local_id,
                                    &mut repaired,
                                    inst.span,
                                );
                            }
                            None => repaired.push(Instruction {
                                kind: InstructionKind::Call {
                                    dest,
                                    func: callee_operand,
                                    args: new_args,
                                },
                                span: inst.span,
                            }),
                        }
                    }
                    InstructionKind::CallVirtual {
                        dest,
                        obj,
                        slot,
                        args,
                    } => {
                        let raw_targets = call_graph.targets_at(func.id, block_id, inst_idx);
                        let site_exact =
                            call_graph.site_targets_are_exact(func.id, block_id, inst_idx);
                        let targets = filter_targets_by_arity(
                            &signatures_by_func,
                            &raw_targets,
                            args.len() + 1,
                        );
                        let abi = if !site_exact {
                            unify_internal_call_abi(
                                &signatures_by_func,
                                &targets,
                                "virtual",
                                &func_name,
                                block_id,
                            )
                            .ok()
                            .flatten()
                        } else {
                            unify_internal_call_abi(
                                &signatures_by_func,
                                &targets,
                                "virtual",
                                &func_name,
                                block_id,
                            )?
                        };
                        let Some(abi) = abi else {
                            repaired.push(Instruction {
                                kind: InstructionKind::CallVirtual {
                                    dest,
                                    obj,
                                    slot,
                                    args,
                                },
                                span: inst.span,
                            });
                            continue;
                        };
                        if abi.params.is_empty() {
                            return Err(CompilerError::codegen_error(
                                format!(
                                    "virtual call at {func_name} block {block_id:?} resolved to zero-arg callee"
                                ),
                                None,
                            ));
                        }
                        if args.len() + 1 != abi.params.len() {
                            return Err(CompilerError::codegen_error(
                                format!(
                                    "virtual call at {func_name} block {block_id:?} has {} args but ABI expects {}",
                                    args.len() + 1,
                                    abi.params.len()
                                ),
                                None,
                            ));
                        }
                        let repaired_obj = coerce_operand(
                            obj,
                            &abi.params[0],
                            func,
                            &def_map,
                            &mut repaired,
                            &mut next_local_id,
                        )
                        .map_err(|err| {
                            coerce_error_with_site(err, "virtual", &func_name, block_id)
                        })?;
                        let new_args = repair_positional_args(
                            args,
                            &abi.params[1..],
                            func,
                            &def_map,
                            &mut repaired,
                            &mut next_local_id,
                            "virtual",
                            &func_name,
                            block_id,
                        )?;
                        match abi.singleton_target {
                            Some(callee) => {
                                let mut direct_args = Vec::with_capacity(new_args.len() + 1);
                                direct_args.push(repaired_obj);
                                direct_args.extend(new_args);
                                // Source-2 fix: apply the same dest-type bridge as
                                // the `Call → CallDirect` singleton path. When the
                                // original dest is wide (Any/HeapAny/Union) but the
                                // callee returns a raw primitive, insert a BoxValue
                                // so the Any-typed slot carries a tagged Value rather
                                // than raw bits.
                                let original_dest_ty = func
                                    .locals
                                    .get(&dest)
                                    .map(|local| local.ty.clone())
                                    .unwrap_or(Type::Any);
                                emit_call_direct_with_bridge(
                                    dest,
                                    callee,
                                    direct_args,
                                    &abi.return_type,
                                    &original_dest_ty,
                                    func,
                                    &mut next_local_id,
                                    &mut repaired,
                                    inst.span,
                                );
                            }
                            None => repaired.push(Instruction {
                                kind: InstructionKind::CallVirtual {
                                    dest,
                                    obj: repaired_obj,
                                    slot,
                                    args: new_args,
                                },
                                span: inst.span,
                            }),
                        }
                    }
                    InstructionKind::CallVirtualNamed {
                        dest,
                        obj,
                        name_hash,
                        args,
                    } => {
                        let raw_targets = call_graph.targets_at(func.id, block_id, inst_idx);
                        let site_exact =
                            call_graph.site_targets_are_exact(func.id, block_id, inst_idx);
                        let targets = filter_targets_by_arity(
                            &signatures_by_func,
                            &raw_targets,
                            args.len() + 1,
                        );
                        let abi = if !site_exact {
                            unify_internal_call_abi(
                                &signatures_by_func,
                                &targets,
                                "virtual",
                                &func_name,
                                block_id,
                            )
                            .ok()
                            .flatten()
                        } else {
                            unify_internal_call_abi(
                                &signatures_by_func,
                                &targets,
                                "virtual",
                                &func_name,
                                block_id,
                            )?
                        };
                        let Some(abi) = abi else {
                            repaired.push(Instruction {
                                kind: InstructionKind::CallVirtualNamed {
                                    dest,
                                    obj,
                                    name_hash,
                                    args,
                                },
                                span: inst.span,
                            });
                            continue;
                        };
                        if abi.params.is_empty() {
                            return Err(CompilerError::codegen_error(
                                format!(
                                    "virtual call at {func_name} block {block_id:?} resolved to zero-arg callee"
                                ),
                                None,
                            ));
                        }
                        if args.len() + 1 != abi.params.len() {
                            return Err(CompilerError::codegen_error(
                                format!(
                                    "virtual call at {func_name} block {block_id:?} has {} args but ABI expects {}",
                                    args.len() + 1,
                                    abi.params.len()
                                ),
                                None,
                            ));
                        }
                        let repaired_obj = coerce_operand(
                            obj,
                            &abi.params[0],
                            func,
                            &def_map,
                            &mut repaired,
                            &mut next_local_id,
                        )
                        .map_err(|err| {
                            coerce_error_with_site(err, "virtual", &func_name, block_id)
                        })?;
                        let new_args = repair_positional_args(
                            args,
                            &abi.params[1..],
                            func,
                            &def_map,
                            &mut repaired,
                            &mut next_local_id,
                            "virtual",
                            &func_name,
                            block_id,
                        )?;
                        match abi.singleton_target {
                            Some(callee) => {
                                let mut direct_args = Vec::with_capacity(new_args.len() + 1);
                                direct_args.push(repaired_obj);
                                direct_args.extend(new_args);
                                // Source-2 fix: mirror the `Call → CallDirect` and
                                // `CallVirtual → CallDirect` dest-type bridge.
                                let original_dest_ty = func
                                    .locals
                                    .get(&dest)
                                    .map(|local| local.ty.clone())
                                    .unwrap_or(Type::Any);
                                emit_call_direct_with_bridge(
                                    dest,
                                    callee,
                                    direct_args,
                                    &abi.return_type,
                                    &original_dest_ty,
                                    func,
                                    &mut next_local_id,
                                    &mut repaired,
                                    inst.span,
                                );
                            }
                            None => repaired.push(Instruction {
                                kind: InstructionKind::CallVirtualNamed {
                                    dest,
                                    obj: repaired_obj,
                                    name_hash,
                                    args: new_args,
                                },
                                span: inst.span,
                            }),
                        }
                    }
                    InstructionKind::RuntimeCall {
                        dest,
                        func: runtime_func,
                        args,
                    } if matches!(
                        runtime_func,
                        RuntimeFunc::Call(def)
                            if std::ptr::eq(
                                def,
                                &pyaot_core_defs::runtime_func_def::RT_INSTANCE_SET_FIELD
                            )
                    ) =>
                    {
                        if args.len() == 3 {
                            if let Some(field_ty) =
                                lookup_field_type(&module_field_types, func, &args[0], &args[1])
                            {
                                // §F.7c: `InstanceObj.fields` stores uniform
                                // tagged `Value` words. For every primitive
                                // field type (Float/Int/Bool/None) the value
                                // reaching `rt_instance_set_field` is already
                                // a `Value` (`Value::from_ptr` for Float,
                                // `Value::from_int` for Int, `Value::from_bool`
                                // for Bool, `Value::NONE` for None). Coerce
                                // to `HeapAny` so abi_repair does not try to
                                // unwrap the freshly-wrapped value back to a
                                // raw scalar.
                                let expected_arg_ty = match field_ty {
                                    Type::Float | Type::Int | Type::Bool | Type::None => Type::Any,
                                    other => other,
                                };
                                let repaired_value = coerce_operand(
                                    args[2].clone(),
                                    &expected_arg_ty,
                                    func,
                                    &def_map,
                                    &mut repaired,
                                    &mut next_local_id,
                                )
                                .map_err(|err| {
                                    coerce_error_with_site(err, "field-write", &func_name, block_id)
                                })?;
                                let mut new_args = args;
                                new_args[2] = repaired_value;
                                repaired.push(Instruction {
                                    kind: InstructionKind::RuntimeCall {
                                        dest,
                                        func: runtime_func,
                                        args: new_args,
                                    },
                                    span: inst.span,
                                });
                                continue;
                            }
                        }
                        repaired.push(Instruction {
                            kind: InstructionKind::RuntimeCall {
                                dest,
                                func: runtime_func,
                                args,
                            },
                            span: inst.span,
                        });
                    }
                    // Fix raw-int args to `rt_obj_*` comparison functions.
                    //
                    // `rt_obj_eq` and `rt_obj_cmp` expect genuinely tagged
                    // Values (they dereference their args to read the type
                    // tag). When a local is allocated as `Any` during
                    // lowering (seed return type before WPA), and WPA later
                    // narrows it to `Int` with `mir_ty: Raw(I64)`, the
                    // already-emitted `rt_obj_eq(raw_local, tagged_int)`
                    // crashes with EXC_BAD_ACCESS (address=6 etc.) because
                    // the raw i64 is dereferenced as a heap pointer.
                    //
                    // We limit this repair to `RT_CMP_OBJ_EQ` /
                    // `RT_CMP_OBJ_ORD` because these are the only comparison
                    // functions that dereference ALL their args as heap
                    // objects. Other functions with `Tagged` semantics (e.g.
                    // `rt_str_slice`, `rt_list_eq`, `rt_hash_int`) either
                    // handle raw ints at the Cranelift ABI boundary or
                    // receive genuinely tagged values from their call sites.
                    InstructionKind::RuntimeCall {
                        dest,
                        func: runtime_func,
                        args,
                    } if matches!(
                        &runtime_func,
                        RuntimeFunc::Call(def)
                            if std::ptr::eq(
                                *def,
                                &pyaot_core_defs::runtime_func_def::RT_CMP_OBJ_EQ,
                            ) || std::ptr::eq(
                                *def,
                                &pyaot_core_defs::runtime_func_def::RT_CMP_OBJ_ORD,
                            )
                    ) =>
                    {
                        let mut new_args = args;
                        for arg in new_args.iter_mut() {
                            // Only box Local args whose resolved MirType is
                            // Raw(_). Constants are already handled by
                            // emit_value_slot during lowering.
                            let raw_kind = match arg {
                                Operand::Local(id) => {
                                    match func.locals.get(id).map(|l| l.resolved_mir_type()) {
                                        Some(MirType::Raw(k)) => k,
                                        _ => continue,
                                    }
                                }
                                Operand::Constant(_) => continue,
                            };
                            let src_type = match raw_kind {
                                RawKind::I64 => Type::Int,
                                RawKind::F64 => Type::Float,
                                RawKind::I8 => Type::Bool,
                                RawKind::I32 => continue, // global slot ID — skip
                            };
                            let box_dest = alloc_temp_local(func, &mut next_local_id, Type::Any);
                            repaired.push(Instruction {
                                kind: InstructionKind::BoxValue {
                                    dest: box_dest,
                                    src: arg.clone(),
                                    src_type,
                                },
                                span: inst.span,
                            });
                            *arg = Operand::Local(box_dest);
                        }
                        repaired.push(Instruction {
                            kind: InstructionKind::RuntimeCall {
                                dest,
                                func: runtime_func,
                                args: new_args,
                            },
                            span: inst.span,
                        });
                    }
                    other => repaired.push(Instruction {
                        kind: other,
                        span: inst.span,
                    }),
                }
            }

            func.blocks
                .get_mut(&block_id)
                .expect("block id collected from function must exist")
                .instructions = repaired;
        }
    }

    Ok(())
}
