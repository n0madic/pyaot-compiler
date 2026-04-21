//! Introspection functions lowering: isinstance(), hash(), id(), repr(), type(), callable(), hasattr(), getattr(), setattr()

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

/// CPython-compatible hash value for None
/// This is a fixed non-zero value to match CPython behavior
const HASH_NONE: i64 = 270898368;

impl<'a> Lowering<'a> {
    /// Lower isinstance(obj, type) -> bool
    pub(super) fn lower_isinstance(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 2, "isinstance", self.call_span())?;

        let obj_expr = &hir_module.exprs[args[0]];
        let type_expr = &hir_module.exprs[args[1]];

        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.seed_expr_type(args[0], hir_module);

        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        match &type_expr.kind {
            // User-defined class - runtime check via class_id with inheritance support
            hir::ExprKind::ClassRef(class_id) => {
                // Primitives (int, float, bool, None) can never be class instances
                // Check at compile-time to avoid passing non-pointer to runtime
                let is_primitive =
                    matches!(obj_type, Type::Int | Type::Float | Type::Bool | Type::None);

                if is_primitive {
                    // Compile-time: primitives are never class instances
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: result_local,
                        src: mir::Operand::Constant(mir::Constant::Bool(false)),
                    });
                } else {
                    // For class isinstance with heap types, need runtime check
                    // Use inheritance-aware version that walks the parent chain
                    // This correctly handles isinstance(Dog(), Animal) -> True
                    // Use offset-adjusted class_id for multi-module support
                    let effective_class_id = self.get_effective_class_id(*class_id);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_ISINSTANCE_CLASS_INHERITED,
                        ),
                        args: vec![
                            obj_operand,
                            mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        ],
                    });
                }
            }
            // Built-in type - can resolve statically for primitives
            hir::ExprKind::TypeRef(check_type) => {
                // Tuple-of-types `isinstance(x, (int, float))` arrives as a
                // Union-valued TypeRef. Dispatch per member and OR the results.
                if let Type::Union(members) = check_type {
                    let mut running = mir::Operand::Constant(mir::Constant::Bool(false));
                    for member in members {
                        let member_result = self.lower_isinstance_single(
                            obj_operand.clone(),
                            &obj_type,
                            member,
                            mir_func,
                        );
                        let combined = self.alloc_and_add_local(Type::Bool, mir_func);
                        self.emit_instruction(mir::InstructionKind::BinOp {
                            dest: combined,
                            op: mir::BinOp::Or,
                            left: running,
                            right: member_result,
                        });
                        running = mir::Operand::Local(combined);
                    }
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: result_local,
                        src: running,
                    });
                    return Ok(mir::Operand::Local(result_local));
                }

                // Check if we can resolve at compile time
                // (primitives have known types, heap types need runtime check)
                let can_resolve = matches!(
                    (&obj_type, check_type),
                    (Type::Int, _) | (Type::Float, _) | (Type::Bool, _) | (Type::None, _)
                );

                if can_resolve {
                    // Compile-time resolution for primitives
                    let result = self.types_match_isinstance(&obj_type, check_type);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: result_local,
                        src: mir::Operand::Constant(mir::Constant::Bool(result)),
                    });
                } else {
                    // Runtime check via type tag
                    if let Some(type_tag) = self.get_type_tag_for_isinstance_check(check_type) {
                        // Get type tag at runtime and compare
                        let tag_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_GET_TYPE_TAG,
                            ),
                            vec![obj_operand],
                            Type::Int,
                            mir_func,
                        );

                        // Compare with expected type tag
                        self.emit_instruction(mir::InstructionKind::BinOp {
                            dest: result_local,
                            op: mir::BinOp::Eq,
                            left: mir::Operand::Local(tag_local),
                            right: mir::Operand::Constant(mir::Constant::Int(type_tag)),
                        });
                    } else {
                        // Unknown type - return false
                        self.emit_instruction(mir::InstructionKind::Copy {
                            dest: result_local,
                            src: mir::Operand::Constant(mir::Constant::Bool(false)),
                        });
                    }
                }
            }
            _ => {
                // Invalid type argument (shouldn't happen if frontend works correctly)
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: mir::Operand::Constant(mir::Constant::Bool(false)),
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a single-type `isinstance` check (primitive, container, or
    /// Class) against an object operand. Returns the bool result operand.
    /// Used by the Union-target (tuple-of-types) path in `lower_isinstance`.
    fn lower_isinstance_single(
        &mut self,
        obj_operand: mir::Operand,
        obj_type: &Type,
        check_type: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        // Class member: runtime inheritance check.
        if let Type::Class { class_id, .. } = check_type {
            let is_primitive =
                matches!(obj_type, Type::Int | Type::Float | Type::Bool | Type::None);
            if is_primitive {
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: mir::Operand::Constant(mir::Constant::Bool(false)),
                });
            } else {
                let effective_class_id = self.get_effective_class_id(*class_id);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_ISINSTANCE_CLASS_INHERITED,
                    ),
                    args: vec![
                        obj_operand,
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                    ],
                });
            }
            return mir::Operand::Local(result_local);
        }

        // Compile-time resolution for primitive object types.
        let can_resolve = matches!(obj_type, Type::Int | Type::Float | Type::Bool | Type::None);
        if can_resolve {
            let result = self.types_match_isinstance(obj_type, check_type);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: mir::Operand::Constant(mir::Constant::Bool(result)),
            });
            return mir::Operand::Local(result_local);
        }

        // Runtime type-tag check.
        if let Some(type_tag) = self.get_type_tag_for_isinstance_check(check_type) {
            let tag_local = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GET_TYPE_TAG),
                vec![obj_operand],
                Type::Int,
                mir_func,
            );
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: result_local,
                op: mir::BinOp::Eq,
                left: mir::Operand::Local(tag_local),
                right: mir::Operand::Constant(mir::Constant::Int(type_tag)),
            });
        } else {
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: mir::Operand::Constant(mir::Constant::Bool(false)),
            });
        }
        mir::Operand::Local(result_local)
    }

    /// Lower issubclass(class, classinfo) -> bool
    pub(super) fn lower_issubclass(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 2, "issubclass", self.call_span())?;

        let class_expr = &hir_module.exprs[args[0]];
        let parent_expr = &hir_module.exprs[args[1]];

        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        // Extract class IDs from ClassRef expressions
        match (&class_expr.kind, &parent_expr.kind) {
            (hir::ExprKind::ClassRef(class_id), hir::ExprKind::ClassRef(parent_id)) => {
                // Use effective class IDs for multi-module support
                let effective_class_id = self.get_effective_class_id(*class_id);
                let effective_parent_id = self.get_effective_class_id(*parent_id);

                // Call runtime function to check inheritance
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ISSUBCLASS),
                    args: vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(effective_parent_id)),
                    ],
                });
            }
            _ => {
                // Invalid arguments - return False
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: mir::Operand::Constant(mir::Constant::Bool(false)),
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower hash(x) -> int
    pub(super) fn lower_hash(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "hash", self.call_span())?;

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.seed_expr_type(args[0], hir_module);

        let result_local = self.alloc_and_add_local(Type::Int, mir_func);

        use pyaot_core_defs::runtime_func_def::*;
        match arg_type {
            Type::Int => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&RT_HASH_INT),
                    args: vec![arg_operand],
                });
            }
            Type::Str => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&RT_HASH_STR),
                    args: vec![arg_operand],
                });
            }
            Type::Bool => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&RT_HASH_BOOL),
                    args: vec![arg_operand],
                });
            }
            Type::None => {
                // hash(None) returns a constant non-zero value for CPython compatibility
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: result_local,
                    value: mir::Constant::Int(HASH_NONE),
                });
            }
            Type::Tuple(_) => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&RT_HASH_TUPLE),
                    args: vec![arg_operand],
                });
            }
            Type::List(_) => {
                // Lists are not hashable - raise TypeError
                let type_name = self.intern("unhashable type: 'list'");
                self.current_block_mut().terminator = mir::Terminator::Raise {
                    exc_type: 5, // TypeError
                    message: Some(mir::Operand::Constant(mir::Constant::Str(type_name))),
                    cause: None,
                    suppress_context: false,
                };
                // Create unreachable block for dead code
                let unreachable_bb = self.new_block();
                self.push_block(unreachable_bb);
            }
            Type::Dict(_, _) => {
                // Dicts are not hashable - raise TypeError
                let type_name = self.intern("unhashable type: 'dict'");
                self.current_block_mut().terminator = mir::Terminator::Raise {
                    exc_type: 5, // TypeError
                    message: Some(mir::Operand::Constant(mir::Constant::Str(type_name))),
                    cause: None,
                    suppress_context: false,
                };
                // Create unreachable block for dead code
                let unreachable_bb = self.new_block();
                self.push_block(unreachable_bb);
            }
            Type::Set(_) => {
                // Sets are not hashable - raise TypeError
                let type_name = self.intern("unhashable type: 'set'");
                self.current_block_mut().terminator = mir::Terminator::Raise {
                    exc_type: 5, // TypeError
                    message: Some(mir::Operand::Constant(mir::Constant::Str(type_name))),
                    cause: None,
                    suppress_context: false,
                };
                // Create unreachable block for dead code
                let unreachable_bb = self.new_block();
                self.push_block(unreachable_bb);
            }
            Type::Class { class_id, .. } => {
                // Check for __hash__ method
                if let Some(class_info) = self.get_class_info(&class_id) {
                    if let Some(hash_func) = class_info.get_dunder_func("__hash__") {
                        // Call __hash__ method
                        self.emit_instruction(mir::InstructionKind::CallDirect {
                            dest: result_local,
                            func: hash_func,
                            args: vec![arg_operand],
                        });
                    } else {
                        // No __hash__ defined - instances are unhashable by default
                        let type_name =
                            self.intern("unhashable type: class instance without __hash__");
                        self.current_block_mut().terminator = mir::Terminator::Raise {
                            exc_type: 5, // TypeError
                            message: Some(mir::Operand::Constant(mir::Constant::Str(type_name))),
                            cause: None,
                            suppress_context: false,
                        };
                        // Create unreachable block for dead code
                        let unreachable_bb = self.new_block();
                        self.push_block(unreachable_bb);
                    }
                }
            }
            _ => {
                // For other types, return 0 for now
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: result_local,
                    value: mir::Constant::Int(0),
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower id(x) -> int
    /// Returns the identity of an object (memory address for heap objects, value for primitives)
    pub(super) fn lower_id(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "id", self.call_span())?;

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.seed_expr_type(args[0], hir_module);

        let result_local = self.alloc_and_add_local(Type::Int, mir_func);

        match arg_type {
            // For primitives, return the value itself as the "id"
            // This matches CPython behavior for small integers
            Type::Int => {
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: arg_operand,
                });
            }
            Type::Bool => {
                // Bool is i8, extend to i64
                let extend_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BoolToInt {
                    dest: extend_local,
                    src: arg_operand,
                });
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: mir::Operand::Local(extend_local),
                });
            }
            Type::Float => {
                // For float, we bitcast to i64 to get a unique representation
                self.emit_instruction(mir::InstructionKind::FloatBits {
                    dest: result_local,
                    src: arg_operand,
                });
            }
            Type::None => {
                // None always has the same id (0)
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: result_local,
                    value: mir::Constant::Int(0),
                });
            }
            // For heap types, return the pointer address
            Type::Str | Type::List(_) | Type::Dict(_, _) | Type::Tuple(_) | Type::Class { .. } => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ID_OBJ),
                    args: vec![arg_operand],
                });
            }
            _ => {
                // For any other type, try to use as pointer
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ID_OBJ),
                    args: vec![arg_operand],
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower repr(obj) -> str
    pub(super) fn lower_repr(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "repr", self.call_span())?;

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.seed_expr_type(args[0], hir_module);

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        // Check for Class types with __repr__ method
        if let Type::Class { class_id, .. } = &arg_type {
            if let Some(class_info) = self.get_class_info(class_id) {
                // Try __repr__ method
                if let Some(repr_func) = class_info.get_dunder_func("__repr__") {
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: repr_func,
                        args: vec![arg_operand],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
                // Fallback to default repr
                else {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_OBJ_DEFAULT_REPR,
                        ),
                        args: vec![arg_operand],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
            }
        }

        // Select the appropriate repr target kind based on type
        let target_kind = match &arg_type {
            Type::Int => mir::ReprTargetKind::Int,
            Type::Float => mir::ReprTargetKind::Float,
            Type::Bool => mir::ReprTargetKind::Bool,
            Type::None => mir::ReprTargetKind::None,
            _ => mir::ReprTargetKind::Collection, // Runtime type-dispatched for str, bytes, containers, and unknown types
        };
        let def = target_kind.runtime_func_def(mir::StringFormat::Repr);
        // For nullary repr (None), pass no args; for others, pass the operand
        let call_args = if target_kind.is_nullary() {
            vec![]
        } else {
            vec![arg_operand]
        };

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(def),
            args: call_args,
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower ascii(obj) -> str (like repr but escapes non-ASCII characters)
    pub(super) fn lower_ascii(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "ascii", self.call_span())?;

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.seed_expr_type(args[0], hir_module);

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        // Select the appropriate target kind and format based on type
        // For types without non-ASCII chars (int, float, bool, None), we can use repr
        // For strings, bytes, and collections, we need special ascii handling
        let (target_kind, format) = match &arg_type {
            Type::Int => (mir::ReprTargetKind::Int, mir::StringFormat::Repr), // No non-ASCII possible
            Type::Float => (mir::ReprTargetKind::Float, mir::StringFormat::Repr), // No non-ASCII possible
            Type::Bool => (mir::ReprTargetKind::Bool, mir::StringFormat::Repr), // No non-ASCII possible
            Type::None => (mir::ReprTargetKind::None, mir::StringFormat::Repr), // No non-ASCII possible
            Type::Bytes => (mir::ReprTargetKind::Collection, mir::StringFormat::Repr), // Bytes repr already escapes
            _ => (mir::ReprTargetKind::Collection, mir::StringFormat::Ascii), // Runtime type-dispatched for str, containers, and unknown types
        };
        let def = target_kind.runtime_func_def(format);
        // For nullary (None), pass no args; for others, pass the operand
        let call_args = if target_kind.is_nullary() {
            vec![]
        } else {
            vec![arg_operand]
        };

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(def),
            args: call_args,
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower type(obj) -> type object
    /// Returns a "type object" (represented as Type::Any) that supports .__name__ attribute
    /// Note: The type() builtin in Python returns a type object, not a string.
    /// Printing type(x) shows "<class 'int'>" because that's the __repr__ of the type object.
    pub(super) fn lower_type(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "type", self.call_span())?;

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.seed_expr_type(args[0], hir_module);

        // type() returns a type object, represented internally as a string containing
        // the full type representation "<class 'typename'>".
        // When printed, this shows the class representation.
        // When accessing .__name__, we extract just the type name.
        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        // For known types at compile time, we can generate a constant string
        // For heap types, use runtime dispatch
        let type_name = match &arg_type {
            Type::Int => Some("<class 'int'>"),
            Type::Float => Some("<class 'float'>"),
            Type::Bool => Some("<class 'bool'>"),
            Type::None => Some("<class 'NoneType'>"),
            Type::Str => Some("<class 'str'>"),
            Type::List(_) => Some("<class 'list'>"),
            Type::Tuple(_) => Some("<class 'tuple'>"),
            Type::Dict(_, _) => Some("<class 'dict'>"),
            Type::Set(_) => Some("<class 'set'>"),
            Type::Bytes => Some("<class 'bytes'>"),
            _ => None, // Need runtime dispatch
        };

        if let Some(name) = type_name {
            // Create string constant
            let name_interned = self.intern(name);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::MakeStr,
                args: vec![mir::Operand::Constant(mir::Constant::Str(name_interned))],
            });
        } else {
            // Runtime dispatch
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TYPE_NAME),
                args: vec![arg_operand],
            });
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower callable(obj) -> bool
    pub(super) fn lower_callable(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "callable", self.call_span())?;

        let arg_expr = &hir_module.exprs[args[0]];
        let _arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.seed_expr_type(args[0], hir_module);

        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        // Check if type is callable at compile time
        // Classes are callable (constructors), functions/lambdas are callable
        let is_callable = matches!(&arg_type, Type::Class { .. } | Type::Function { .. });
        self.emit_instruction(mir::InstructionKind::Const {
            dest: result_local,
            value: mir::Constant::Bool(is_callable),
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower hasattr(obj, name) -> bool
    pub(super) fn lower_hasattr(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 2, "hasattr", self.call_span())?;

        let obj_expr = &hir_module.exprs[args[0]];
        let _obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.seed_expr_type(args[0], hir_module);

        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        // Check if the attribute name is a constant string
        let name_expr = &hir_module.exprs[args[1]];
        if let hir::ExprKind::Str(attr_name) = &name_expr.kind {
            let attr_str = self.resolve(*attr_name);

            // For class instances, check if the attribute exists in class_info
            if let Type::Class { class_id, .. } = &obj_type {
                if let Some(class_info) = self.get_class_info(class_id) {
                    // Use lookup to find the interned string
                    let has_attr = if let Some(attr_interned) = self.lookup_interned(attr_str) {
                        class_info.field_offsets.contains_key(&attr_interned)
                            || class_info.method_funcs.contains_key(&attr_interned)
                            || class_info.properties.contains_key(&attr_interned)
                    } else {
                        false
                    };

                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: result_local,
                        value: mir::Constant::Bool(has_attr),
                    });

                    return Ok(mir::Operand::Local(result_local));
                }
            }
        }

        // Default: return False for unknown types/attributes
        self.emit_instruction(mir::InstructionKind::Const {
            dest: result_local,
            value: mir::Constant::Bool(false),
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower getattr(obj, name[, default]) -> value
    pub(super) fn lower_getattr(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_min_args(args, 2, "getattr", self.call_span())?;

        let obj_expr = &hir_module.exprs[args[0]];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.seed_expr_type(args[0], hir_module);

        let name_expr = &hir_module.exprs[args[1]];

        // Get default value if provided
        let default_operand = if args.len() > 2 {
            let default_expr = &hir_module.exprs[args[2]];
            Some(self.lower_expr(default_expr, hir_module, mir_func)?)
        } else {
            None
        };

        // Check if the attribute name is a constant string
        if let hir::ExprKind::Str(attr_name) = &name_expr.kind {
            let attr_str = self.resolve(*attr_name).to_string();

            // For class instances, get the attribute value
            if let Type::Class { class_id, .. } = &obj_type {
                if let Some(class_info) = self.get_class_info(class_id).cloned() {
                    // Use lookup to find the interned string
                    if let Some(attr_interned) = self.lookup_interned(&attr_str) {
                        if let Some(&offset) = class_info.field_offsets.get(&attr_interned) {
                            let field_type = class_info
                                .field_types
                                .get(&attr_interned)
                                .cloned()
                                .unwrap_or(Type::Any);

                            let result_local = self.emit_runtime_call(
                                mir::RuntimeFunc::Call(
                                    &pyaot_core_defs::runtime_func_def::RT_INSTANCE_GET_FIELD,
                                ),
                                vec![
                                    obj_operand,
                                    mir::Operand::Constant(mir::Constant::Int(offset as i64)),
                                ],
                                field_type,
                                mir_func,
                            );

                            return Ok(mir::Operand::Local(result_local));
                        }
                    }
                }
            }
        }

        // If we have a default, return it; otherwise return None
        if let Some(default) = default_operand {
            Ok(default)
        } else {
            // Should raise AttributeError, but for simplicity return None
            Ok(mir::Operand::Constant(mir::Constant::None))
        }
    }

    /// Lower format(value, format_spec='') -> str
    /// Calls rt_format_value(boxed_value, spec_str) -> *mut Obj
    pub(super) fn lower_format(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() || args.len() > 2 {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                format!("format() requires 1 or 2 argument(s), got {}", args.len()),
                self.call_span(),
            ));
        }

        let value_expr = &hir_module.exprs[args[0]];
        let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;
        let value_type = self.seed_expr_type(args[0], hir_module);

        // Check for class with __format__ dunder — call it directly instead of runtime dispatch
        if let Type::Class { class_id, .. } = &value_type {
            if let Some(format_func_id) = self
                .get_class_info(class_id)
                .and_then(|info| info.get_dunder_func("__format__"))
            {
                let spec_operand = if args.len() > 1 {
                    let spec_expr = &hir_module.exprs[args[1]];
                    self.lower_expr(spec_expr, hir_module, mir_func)?
                } else {
                    let empty = self.intern("");
                    mir::Operand::Constant(mir::Constant::Str(empty))
                };

                let result_local = self.alloc_and_add_local(Type::Str, mir_func);
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func: format_func_id,
                    args: vec![value_operand, spec_operand],
                });
                return Ok(mir::Operand::Local(result_local));
            }
        }

        // Box the value so it becomes *mut Obj
        let boxed_value = self.box_primitive_if_needed(value_operand, &value_type, mir_func);

        // Get format spec (default to empty string if not provided)
        let spec_operand = if args.len() > 1 {
            let spec_expr = &hir_module.exprs[args[1]];
            self.lower_expr(spec_expr, hir_module, mir_func)?
        } else {
            // Pass null pointer for empty spec (runtime handles null as "")
            mir::Operand::Constant(mir::Constant::Int(0))
        };

        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FORMAT_VALUE),
            vec![boxed_value, spec_operand],
            Type::Str,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower object.__new__(cls) -> allocate instance by class_id
    pub(super) fn lower_object_new(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "object.__new__", self.call_span())?;

        let cls_expr = &hir_module.exprs[args[0]];
        let cls_operand = self.lower_expr(cls_expr, hir_module, mir_func)?;

        // Result type is Any (instance pointer) since we don't know the class statically here
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_OBJECT_NEW),
            vec![cls_operand],
            Type::Any,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower setattr(obj, name, value) -> None
    pub(super) fn lower_setattr(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 3, "setattr", self.call_span())?;

        let obj_expr = &hir_module.exprs[args[0]];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.seed_expr_type(args[0], hir_module);

        let name_expr = &hir_module.exprs[args[1]];
        let value_expr = &hir_module.exprs[args[2]];
        let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;

        // Check if the attribute name is a constant string
        if let hir::ExprKind::Str(attr_name) = &name_expr.kind {
            let attr_str = self.resolve(*attr_name).to_string();

            // For class instances, set the attribute value
            if let Type::Class { class_id, .. } = &obj_type {
                if let Some(class_info) = self.get_class_info(class_id).cloned() {
                    // Use lookup to find the interned string
                    if let Some(attr_interned) = self.lookup_interned(&attr_str) {
                        if let Some(&offset) = class_info.field_offsets.get(&attr_interned) {
                            let _dummy_local = self.emit_runtime_call(
                                mir::RuntimeFunc::Call(
                                    &pyaot_core_defs::runtime_func_def::RT_INSTANCE_SET_FIELD,
                                ),
                                vec![
                                    obj_operand,
                                    mir::Operand::Constant(mir::Constant::Int(offset as i64)),
                                    value_operand,
                                ],
                                Type::None,
                                mir_func,
                            );

                            return Ok(mir::Operand::Constant(mir::Constant::None));
                        }
                    }
                }
            }
        }

        // For dynamic attribute setting, we'd need runtime support
        // For now, just return None
        Ok(mir::Operand::Constant(mir::Constant::None))
    }
}
