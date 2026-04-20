//! Exception handling lowering (try/except/finally, raise)

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

/// Get the exception type tag for a builtin exception type.
///
/// Uses `BuiltinExceptionKind::tag()` from the unified exception system.
fn get_exc_type_tag(builtin: &hir::Builtin) -> u8 {
    match builtin {
        hir::Builtin::BuiltinException(kind) => kind.tag(),
        _ => 0, // Default to Exception
    }
}

/// Maximum class ID supported for exception handling.
/// Class IDs above this limit will cause a compile-time error.
/// This ensures exception dispatch can use efficient u8 tags.
const MAX_EXCEPTION_CLASS_ID: u32 = 255;

/// Get the exception type tag for a Type (for except handler type matching).
/// Uses the central exception type tag from Type::builtin_exception_type_tag().
/// Returns (tag, is_custom_class).
///
/// # Panics
/// Panics at compile time if a custom exception class has class_id > 255.
pub(crate) fn get_exc_type_tag_from_type(ty: &Type) -> Option<(u8, bool)> {
    // First try built-in exception types
    if let Some(tag) = ty.builtin_exception_type_tag() {
        return Some((tag, false));
    }

    // Handle custom exception classes
    if let Type::Class { class_id, .. } = ty {
        // Validate class_id fits in u8 to prevent silent truncation
        if class_id.0 > MAX_EXCEPTION_CLASS_ID {
            return None;
        }
        // Custom exception class - use class_id as the tag
        return Some((class_id.0 as u8, true));
    }

    // Unknown type - can't check
    None
}

/// Exception info result from extract_exc_info
/// Built-in exceptions: (type_tag, message, None, None)
/// Custom exception classes: (class_id, message, Some(class_id), Some(instance))
/// Instance re-raise: (0, None, None, Some(instance), is_instance_raise=true)
#[derive(Debug)]
pub struct ExcInfo {
    pub type_tag: u8,
    pub message: Option<mir::Operand>,
    pub custom_class_id: Option<u8>, // Some(id) for custom exception classes
    pub instance: Option<mir::Operand>, // Pre-created instance for custom exceptions
    pub is_instance_raise: bool,     // True when raising an existing exception variable
}

impl<'a> Lowering<'a> {
    /// Check if a type refers to an exception class (custom class with is_exception_class).
    fn is_exception_class_type(&self, ty: &Type) -> bool {
        if let Type::Class { class_id, .. } = ty {
            if let Some(info) = self.get_class_info(class_id) {
                return info.is_exception_class;
            }
        }
        false
    }

    /// Extract exception type tag and message operand from an HIR expression.
    /// Used for both the main exception and the cause in `raise X from Y`.
    fn extract_exc_info(
        &mut self,
        exc_expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<ExcInfo> {
        match &exc_expr.kind {
            hir::ExprKind::Call { func, args, .. } => {
                // Check if calling a custom exception class
                let func_expr = &hir_module.exprs[*func];
                if let hir::ExprKind::ClassRef(class_id) = &func_expr.kind {
                    // Check if this class is an exception class
                    if let Some(class_def) = hir_module.class_defs.get(class_id) {
                        if class_def.is_exception_class {
                            // Validate class_id fits in u8
                            if class_id.0 > MAX_EXCEPTION_CLASS_ID {
                                return Err(pyaot_diagnostics::CompilerError::codegen_error(
                                    format!(
                                        "Exception class {:?} has class_id {} which exceeds the maximum \
                                         supported class_id for exception handling ({}).",
                                        class_def.name, class_id.0, MAX_EXCEPTION_CLASS_ID
                                    ),
                                    Some(func_expr.span),
                                ));
                            }
                            let id = class_id.0 as u8;

                            // Extract message from first arg (for traceback display)
                            // Only use the first arg if it's a string type
                            let msg = if let Some(arg) = args.first() {
                                let arg_id = match arg {
                                    hir::CallArg::Regular(id) => id,
                                    hir::CallArg::Starred(id) => id,
                                };
                                let arg_type = self.get_type_of_expr_id(*arg_id, hir_module);
                                if arg_type == Type::Str {
                                    let arg_expr = &hir_module.exprs[*arg_id];
                                    Some(self.lower_expr(arg_expr, hir_module, mir_func)?)
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            // Create instance eagerly via lower_class_instantiation
                            // This allocates the instance and calls __init__
                            let expanded_args: Vec<crate::expressions::ExpandedArg> = args
                                .iter()
                                .map(|arg| match arg {
                                    hir::CallArg::Regular(id) => {
                                        crate::expressions::ExpandedArg::Regular(*id)
                                    }
                                    hir::CallArg::Starred(id) => {
                                        crate::expressions::ExpandedArg::Regular(*id)
                                    }
                                })
                                .collect();
                            let kwargs_from_call = match &exc_expr.kind {
                                hir::ExprKind::Call { kwargs, .. } => kwargs.clone(),
                                _ => vec![],
                            };
                            let instance = self.lower_class_instantiation(
                                *class_id,
                                &expanded_args,
                                &kwargs_from_call,
                                hir_module,
                                mir_func,
                            )?;

                            return Ok(ExcInfo {
                                type_tag: id,
                                message: msg,
                                custom_class_id: Some(id),
                                instance: Some(instance),
                                is_instance_raise: false,
                            });
                        }
                    }
                }
                // Regular call (not a custom exception class)
                let msg = if let Some(arg) = args.first() {
                    let arg_id = match arg {
                        hir::CallArg::Regular(id) => id,
                        hir::CallArg::Starred(id) => id,
                    };
                    let arg_expr = &hir_module.exprs[*arg_id];
                    Some(self.lower_expr(arg_expr, hir_module, mir_func)?)
                } else {
                    None
                };
                Ok(ExcInfo {
                    type_tag: 0,
                    message: msg,
                    custom_class_id: None,
                    instance: None,
                    is_instance_raise: false,
                })
            }
            hir::ExprKind::BuiltinCall { builtin, args, .. } => {
                let tag = get_exc_type_tag(builtin);
                let msg = if let Some(arg_id) = args.first() {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    Some(self.lower_expr(arg_expr, hir_module, mir_func)?)
                } else {
                    None
                };
                Ok(ExcInfo {
                    type_tag: tag,
                    message: msg,
                    custom_class_id: None,
                    instance: None,
                    is_instance_raise: false,
                })
            }
            hir::ExprKind::Str(s) => {
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::MakeStr,
                    vec![mir::Operand::Constant(mir::Constant::Str(*s))],
                    Type::Str,
                    mir_func,
                );
                Ok(ExcInfo {
                    type_tag: 0,
                    message: Some(mir::Operand::Local(result_local)),
                    custom_class_id: None,
                    instance: None,
                    is_instance_raise: false,
                })
            }
            hir::ExprKind::Var(var_id) => {
                // Check if this variable holds an exception instance (for `raise e`)
                let var_type = self.get_var_type(var_id).cloned().unwrap_or(Type::Any);
                let is_exception = matches!(&var_type, Type::BuiltinException(_))
                    || self.is_exception_class_type(&var_type);
                if is_exception {
                    let var_local = self
                        .get_var_local(var_id)
                        .expect("exception var should have a local");
                    Ok(ExcInfo {
                        type_tag: 0,
                        message: None,
                        custom_class_id: None,
                        instance: Some(mir::Operand::Local(var_local)),
                        is_instance_raise: true,
                    })
                } else {
                    Ok(ExcInfo {
                        type_tag: 0,
                        message: None,
                        custom_class_id: None,
                        instance: None,
                        is_instance_raise: false,
                    })
                }
            }
            _ => Ok(ExcInfo {
                type_tag: 0,
                message: None,
                custom_class_id: None,
                instance: None,
                is_instance_raise: false,
            }),
        }
    }

    /// Lower a raise statement to MIR
    pub(crate) fn lower_raise(
        &mut self,
        exc: &Option<hir::ExprId>,
        cause: &Option<hir::ExprId>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        self.emit_raise_terminator(exc, cause, hir_module, mir_func)?;
        // Create a new unreachable block after raise (for dead code)
        let unreachable_bb = self.new_block();
        self.push_block(unreachable_bb);
        Ok(())
    }

    /// §1.17b-c — emit the `Raise` / `RaiseInstance` / `RaiseCustom` /
    /// `Reraise` MIR terminator on the current block WITHOUT pushing a
    /// dead-code block after it. Used by the CFG walker where each HIR
    /// block maps to exactly one MIR block (no dead-code block needed).
    ///
    /// The tree-walking `lower_raise` wraps this with a push of an
    /// unreachable block because `Raise` is a `StmtKind` inside a
    /// function body that may have subsequent stmts after it (those
    /// stmts are dead code but the tree walker still emits them into
    /// an unreachable block to keep IDs consistent). In CFG form the
    /// bridge lifts Raise to a block terminator, so no dead-code block
    /// is needed.
    pub(crate) fn emit_raise_terminator(
        &mut self,
        exc: &Option<hir::ExprId>,
        cause: &Option<hir::ExprId>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        if let Some(exc_expr_id) = exc {
            let exc_expr = &hir_module.exprs[*exc_expr_id];
            let exc_info = self.extract_exc_info(exc_expr, hir_module, mir_func)?;

            // Check if this is raising an existing exception instance (e.g., `raise e`)
            if exc_info.is_instance_raise {
                if let Some(instance) = exc_info.instance {
                    self.current_block_mut().terminator =
                        mir::Terminator::RaiseInstance { instance };
                } else {
                    // Shouldn't happen, but fall back to generic Exception
                    self.current_block_mut().terminator = mir::Terminator::Raise {
                        exc_type: 0,
                        message: None,
                        cause: None,
                        suppress_context: false,
                    };
                }
            } else if let Some(class_id) = exc_info.custom_class_id {
                // Custom exception - use RaiseCustom terminator
                // Note: cause is not supported for custom exceptions currently
                if cause.is_some() {
                    // For now, we ignore the cause for custom exceptions
                    // A full implementation would need RaiseCustomFrom variant
                }
                self.current_block_mut().terminator = mir::Terminator::RaiseCustom {
                    class_id,
                    message: exc_info.message,
                    instance: exc_info.instance,
                };
            } else {
                // Built-in exception - use Raise terminator
                // Lower cause if present (`raise X from Y`)
                // Also track suppress_context for "raise X from None"
                let (raise_cause, suppress_context) = if let Some(cause_expr_id) = cause {
                    let cause_expr = &hir_module.exprs[*cause_expr_id];
                    // `raise X from None` suppresses context display
                    if matches!(&cause_expr.kind, hir::ExprKind::None) {
                        // No explicit cause, but suppress_context = true
                        (None, true)
                    } else {
                        // Explicit cause - also suppresses context (cause takes precedence)
                        let cause_info = self.extract_exc_info(cause_expr, hir_module, mir_func)?;
                        (
                            Some(mir::RaiseCause {
                                exc_type: cause_info.type_tag,
                                message: cause_info.message,
                            }),
                            true,
                        )
                    }
                } else {
                    // Plain raise without from - don't suppress context
                    (None, false)
                };

                // Emit Raise terminator
                self.current_block_mut().terminator = mir::Terminator::Raise {
                    exc_type: exc_info.type_tag,
                    message: exc_info.message,
                    cause: raise_cause,
                    suppress_context,
                };
            }
        } else {
            // Bare raise - re-raise current exception
            self.current_block_mut().terminator = mir::Terminator::Reraise;
        }

        Ok(())
    }
}
