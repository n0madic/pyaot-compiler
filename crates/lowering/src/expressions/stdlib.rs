//! Standard library expression lowering
//!
//! Handles lowering of:
//! - StdlibAttr (sys.argv, os.environ)
//! - StdlibCall (sys.exit, os.path.join, re.search, re.match, re.sub)
//! - StdlibConst (math.pi, math.e)
//!
//! Uses declarative hints from StdlibFunctionDef for special handling:
//! - `hints.variadic_to_list`: Collect variadic args into a list
//! - `hints.auto_box`: Box primitives for Any parameters
//! - `param.default`: Use default values for missing optional args

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_stdlib_defs::{
    ConstValue, StdlibAttrDef, StdlibConstDef, StdlibFunctionDef, StdlibMethodDef, TypeSpec,
};
use pyaot_types::{typespec_to_type, Type};

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a stdlib attribute access (e.g., sys.argv, os.environ)
    /// Uses the definition reference for Single Source of Truth
    pub(crate) fn lower_stdlib_attr(
        &mut self,
        attr_def: &'static StdlibAttrDef,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let result_type = typespec_to_type(&attr_def.ty);
        let result_local = self.alloc_and_add_local(result_type, mir_func);

        // Emit generic StdlibAttrGet - codegen will use attr_def.runtime_getter
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::StdlibAttrGet(attr_def),
            args: vec![],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a stdlib compile-time constant (e.g., math.pi, math.e, string.digits)
    /// Numeric constants are inlined as literal values.
    /// String constants require heap allocation via MakeStr.
    pub(crate) fn lower_stdlib_const(
        &mut self,
        const_def: &'static StdlibConstDef,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // String constants need heap allocation (like string literals)
        if let ConstValue::Str(s) = &const_def.value {
            let interned = self.intern(s);
            return self.lower_str_literal(interned, mir_func);
        }
        Ok(mir::Operand::Constant(
            self.const_value_to_mir(&const_def.value),
        ))
    }

    /// Convert ConstValue to MIR Constant
    fn const_value_to_mir(&mut self, value: &ConstValue) -> mir::Constant {
        match value {
            ConstValue::Int(v) => mir::Constant::Int(*v),
            ConstValue::Float(v) => mir::Constant::Float(*v),
            ConstValue::Bool(v) => mir::Constant::Bool(*v),
            ConstValue::Str(s) => mir::Constant::Str(self.intern(s)),
        }
    }

    /// Lower a stdlib function call using declarative hints
    ///
    /// All special handling is driven by func_def.hints and param definitions:
    /// - variadic_to_list: Collect args into a list
    /// - auto_box: Box primitives for Any parameters
    /// - param.default: Fill missing optional args with defaults
    pub(crate) fn lower_stdlib_call(
        &mut self,
        func_def: &'static StdlibFunctionDef,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let hints = &func_def.hints;

        // Handle variadic_to_list: collect all args into a list
        if hints.variadic_to_list {
            return self.lower_variadic_to_list_call(func_def, args, hir_module, mir_func);
        }

        // Lower arguments with auto-boxing and default value support
        let mir_args = self.lower_stdlib_args(func_def, args, hir_module, mir_func)?;

        let result_type = typespec_to_type(&func_def.return_type);
        let result_local = self.alloc_and_add_local(result_type, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::StdlibCall(func_def),
            args: mir_args,
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower stdlib function arguments with auto-boxing and default values
    fn lower_stdlib_args(
        &mut self,
        func_def: &'static StdlibFunctionDef,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<mir::Operand>> {
        let hints = &func_def.hints;
        let params = func_def.params;
        let mut mir_args = Vec::with_capacity(params.len().max(args.len()));

        for (i, param) in params.iter().enumerate() {
            let operand = if i < args.len() {
                // Argument provided - lower it
                let arg_expr = &hir_module.exprs[args[i]];
                let arg_type = self.get_expr_type(arg_expr, hir_module);
                let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;

                // Auto-box if enabled and parameter is Any
                if hints.auto_box && matches!(param.ty, TypeSpec::Any) {
                    self.box_primitive_if_needed(arg_operand, &arg_type, mir_func)
                } else {
                    arg_operand
                }
            } else if let Some(ref default) = param.default {
                // Use default value
                mir::Operand::Constant(self.const_value_to_mir(default))
            } else if param.optional {
                // Optional with no default - use type-appropriate zero
                mir::Operand::Constant(self.default_for_type(&param.ty))
            } else {
                // This shouldn't happen if arg validation is correct
                mir::Operand::Constant(mir::Constant::None)
            };

            mir_args.push(operand);
        }

        Ok(mir_args)
    }

    /// Get a default value for a TypeSpec (used when no default is specified)
    fn default_for_type(&self, ty: &TypeSpec) -> mir::Constant {
        match ty {
            TypeSpec::Int => mir::Constant::Int(0),
            TypeSpec::Float => mir::Constant::Float(0.0),
            TypeSpec::Bool => mir::Constant::Bool(false),
            TypeSpec::Str => mir::Constant::None, // Empty string would need MakeStr
            _ => mir::Constant::None,
        }
    }

    /// Lower a variadic call by collecting args into a list
    fn lower_variadic_to_list_call(
        &mut self,
        func_def: &'static StdlibFunctionDef,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Create a list to hold the arguments
        let list_local = self.alloc_and_add_local(Type::List(Box::new(Type::Str)), mir_func);

        // Allocate list (assuming string elements - heap objects)
        let capacity = args.len() as i64;
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: list_local,
            func: mir::RuntimeFunc::MakeList,
            args: vec![
                mir::Operand::Constant(mir::Constant::Int(capacity)),
                mir::Operand::Constant(mir::Constant::Int(0)), // ELEM_HEAP_OBJ
            ],
        });

        // Add each argument to the list, boxing primitives as required since the list
        // uses ELEM_HEAP_OBJ storage (float, bool, and None are stored as heap objects).
        for arg_id in args {
            let arg_expr = &hir_module.exprs[*arg_id];
            let arg_type = self.get_expr_type(arg_expr, hir_module);
            let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;

            // Box the argument when it is a primitive that requires heap representation
            // (float → BoxFloat, bool → BoxBool, None → BoxNone)
            let pushed_operand = self.box_primitive_if_needed(arg_operand, &arg_type, mir_func);

            let void_local = self.alloc_and_add_local(Type::None, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: void_local,
                func: mir::RuntimeFunc::ListPush,
                args: vec![mir::Operand::Local(list_local), pushed_operand],
            });
        }

        // Call the runtime function with the list
        let result_type = typespec_to_type(&func_def.return_type);
        let result_local = self.alloc_and_add_local(result_type, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::StdlibCall(func_def),
            args: vec![mir::Operand::Local(list_local)],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower an object method call using the generic ObjectMethodCall pattern
    ///
    /// This handles method calls on objects that have methods defined in ObjectTypeDef,
    /// such as Match.group(), Match.start(), etc.
    ///
    /// Args:
    /// - obj_operand: The object to call the method on (passed as self)
    /// - method_def: The method definition from stdlib-defs
    /// - args: The method arguments (excluding self)
    /// - hir_module: HIR module for type information
    /// - mir_func: The MIR function being built
    pub(crate) fn lower_object_method_call(
        &mut self,
        obj_operand: mir::Operand,
        method_def: &'static StdlibMethodDef,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Build arguments: [self, ...params]
        let mut mir_args = vec![obj_operand];

        // Process method parameters (excluding self, which is already added)
        for (i, param) in method_def.params.iter().enumerate() {
            let operand = if i < args.len() {
                // Argument provided - lower it
                let arg_expr = &hir_module.exprs[args[i]];
                self.lower_expr(arg_expr, hir_module, mir_func)?
            } else if let Some(ref default) = param.default {
                // Use default value
                mir::Operand::Constant(self.const_value_to_mir(default))
            } else if param.optional {
                // Optional with no explicit default - use type-appropriate zero
                mir::Operand::Constant(self.default_for_type(&param.ty))
            } else {
                // This shouldn't happen if arg validation is correct
                mir::Operand::Constant(mir::Constant::None)
            };

            mir_args.push(operand);
        }

        // Allocate result local with correct type
        let result_type = typespec_to_type(&method_def.return_type);
        let result_local = self.alloc_and_add_local(result_type, mir_func);

        // Emit the generic ObjectMethodCall
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::ObjectMethodCall(method_def),
            args: mir_args,
        });

        Ok(mir::Operand::Local(result_local))
    }
}
