//! Exception handling lowering (try/except/finally, raise)

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;
use indexmap::IndexSet;
use pyaot_utils::VarId;

/// Recursively collect all variables assigned in a list of statements.
/// This traverses into nested control flow structures (if/while/for/try).
fn collect_assigned_vars(
    stmts: &[hir::StmtId],
    hir_module: &hir::Module,
    assigned: &mut IndexSet<VarId>,
) {
    for stmt_id in stmts {
        let stmt = &hir_module.stmts[*stmt_id];
        match &stmt.kind {
            hir::StmtKind::Assign { target, .. } => {
                assigned.insert(*target);
            }
            hir::StmtKind::UnpackAssign {
                before_star,
                starred,
                after_star,
                ..
            } => {
                for target in before_star {
                    assigned.insert(*target);
                }
                if let Some(starred_var) = starred {
                    assigned.insert(*starred_var);
                }
                for target in after_star {
                    assigned.insert(*target);
                }
            }
            hir::StmtKind::NestedUnpackAssign { targets, .. } => {
                // Recursively extract variables from nested unpacking patterns
                fn extract_vars(target: &hir::UnpackTarget, assigned: &mut IndexSet<VarId>) {
                    match target {
                        hir::UnpackTarget::Var(var_id) => {
                            assigned.insert(*var_id);
                        }
                        hir::UnpackTarget::Nested(nested) => {
                            for t in nested {
                                extract_vars(t, assigned);
                            }
                        }
                    }
                }
                for target in targets {
                    extract_vars(target, assigned);
                }
            }
            hir::StmtKind::For { target, body, .. } => {
                assigned.insert(*target);
                collect_assigned_vars(body, hir_module, assigned);
            }
            hir::StmtKind::ForUnpack { targets, body, .. } => {
                for target in targets {
                    assigned.insert(*target);
                }
                collect_assigned_vars(body, hir_module, assigned);
            }
            hir::StmtKind::If {
                then_block,
                else_block,
                ..
            } => {
                collect_assigned_vars(then_block, hir_module, assigned);
                collect_assigned_vars(else_block, hir_module, assigned);
            }
            hir::StmtKind::While { body, .. } => {
                collect_assigned_vars(body, hir_module, assigned);
            }
            hir::StmtKind::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                collect_assigned_vars(body, hir_module, assigned);
                for handler in handlers {
                    if let Some(var_id) = handler.name {
                        assigned.insert(var_id);
                    }
                    collect_assigned_vars(&handler.body, hir_module, assigned);
                }
                collect_assigned_vars(else_block, hir_module, assigned);
                collect_assigned_vars(finally_block, hir_module, assigned);
            }
            _ => {}
        }
    }
}

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
fn get_exc_type_tag_from_type(ty: &Type) -> Option<(u8, bool)> {
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

    /// Collect variables assigned in try block that need cell wrapping.
    ///
    /// Due to setjmp/longjmp semantics, variables modified after setjmp may lose their
    /// values when longjmp is called. We wrap these variables in heap-allocated cells
    /// to preserve their values across exception unwinding.
    ///
    /// Filters out:
    /// - Variables already in cells (cell_vars, nonlocal_cells)
    /// - Global variables (handled by global storage)
    /// - Function references (tracked in var_to_func)
    /// - Closure references (tracked in var_to_closure)
    fn collect_try_assigned_vars(
        &self,
        body: &[hir::StmtId],
        hir_module: &hir::Module,
    ) -> IndexSet<VarId> {
        let mut assigned = IndexSet::new();
        collect_assigned_vars(body, hir_module, &mut assigned);

        // Filter out variables that don't need cell wrapping
        assigned.retain(|var_id| {
            // Skip if already in a cell
            if self.is_cell_var(var_id) || self.has_nonlocal_cell(var_id) {
                return false;
            }
            // Skip if it's a global variable
            if self.is_global(var_id) {
                return false;
            }
            // Skip if it's a function or closure reference
            if self.has_var_func(var_id) || self.has_var_closure(var_id) {
                return false;
            }
            true
        });

        assigned
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
                                return Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                                    format!(
                                        "Exception class {:?} has class_id {} which exceeds the maximum \
                                         supported class_id for exception handling ({}).",
                                        class_def.name, class_id.0, MAX_EXCEPTION_CLASS_ID
                                    ),
                                    func_expr.span,
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
                let result_local = self.alloc_and_add_local(Type::Str, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::MakeStr,
                    args: vec![mir::Operand::Constant(mir::Constant::Str(*s))],
                });
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

        // Create a new unreachable block after raise (for dead code)
        let unreachable_bb = self.new_block();
        self.push_block(unreachable_bb);

        Ok(())
    }

    /// Lower a try/except/else/finally statement to MIR
    ///
    /// # Exception Frame Ownership Model (UPDATED)
    ///
    /// **INVARIANT:** Exception frame is pushed EXACTLY ONCE at entry and popped EXACTLY ONCE before exit.
    ///
    /// **RULE 1 (Exception Path - exception in try or else):**
    /// - When exception is raised, `rt_exc_raise()` pops the frame before longjmp (runtime/exceptions.rs:238-240)
    /// - Handler code runs with NO active frame for this try block
    /// - Finally block runs with NO active frame (already popped)
    /// - Reraise path does NOT pop frame (already popped by rt_exc_raise)
    ///
    /// **RULE 2 (Normal Path - no exception):**
    /// - Frame remains active through try body and else block (if present)
    /// - Frame is popped at finally exit before normal exit
    /// - Else block keeps frame active so exceptions trigger finally before propagating
    /// - Handler dispatch checks exception types; if no match, finally runs then reraise
    ///
    /// # CFG Structure
    /// - [current] -> ExcPushFrame -> TrySetjmp
    /// - TrySetjmp branches to [try_body] (normal) or [handler_dispatch] (exception)
    /// - [try_body] -> [else_block] -> [finally] (on normal exit, if else exists)
    /// - [try_body] -> [finally] (on normal exit, if no else)
    /// - [handler_dispatch] -> [handler] -> ExcClear -> [finally] (always skips else)
    /// - [finally] -> [exit] (normal) or Reraise (propagating)
    ///
    /// # Control Flow Paths (7 total) - UPDATED
    /// 1. **try -> else -> finally -> exit** (normal, has else): Pop at finally exit
    /// 2. **try -> finally -> exit** (normal, no else): Pop at finally exit
    /// 3. **try -> exception -> handler -> finally -> exit** (caught): Already popped by rt_exc_raise
    /// 4. **try -> exception -> finally -> reraise** (uncaught): Already popped by rt_exc_raise
    /// 5. **else -> exception -> finally -> reraise** (exception in else): Frame active, rt_exc_raise pops, finally runs, reraise
    /// 6. **finally -> exception -> propagate** (exception in finally): Complex, depends on entry path
    /// 7. **handler -> exception -> finally -> reraise** (exception in handler): Frame already popped, finally runs, reraise
    pub(crate) fn lower_try(
        &mut self,
        body: &[hir::StmtId],
        handlers: &[hir::ExceptHandler],
        else_block: &[hir::StmtId],
        finally_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // Allocate exception frame local (opaque pointer type, 8 bytes)
        // The actual ExceptionFrame struct is allocated in codegen
        let frame_local = self.alloc_and_add_local(Type::Int, mir_func);

        // Allocate propagating flag local (tracks if we need to reraise after finally)
        let propagating_local = self.alloc_and_add_local(Type::Bool, mir_func);

        // Initialize propagating = false
        self.emit_instruction(mir::InstructionKind::Const {
            dest: propagating_local,
            value: mir::Constant::Bool(false),
        });

        // Create basic blocks
        let try_body_bb = self.new_block();
        let handler_dispatch_bb = self.new_block();
        // Create else block only if there are else statements
        let else_bb = if !else_block.is_empty() {
            Some(self.new_block())
        } else {
            None
        };
        let finally_bb = self.new_block();
        let exit_bb = self.new_block();

        let try_body_id = try_body_bb.id;
        let handler_dispatch_id = handler_dispatch_bb.id;
        let else_id = else_bb.as_ref().map(|b| b.id);
        let finally_id = finally_bb.id;
        let exit_id = exit_bb.id;

        // After try body success: go to else block if exists, otherwise go to finally
        let try_success_target = else_id.unwrap_or(finally_id);

        // Push exception frame
        self.emit_instruction(mir::InstructionKind::ExcPushFrame { frame_local });

        // Collect variables assigned in try block that need cell wrapping
        // to preserve their values across setjmp/longjmp exception unwinding
        let try_assigned_vars = self.collect_try_assigned_vars(body, hir_module);

        // For variables that already exist, create cells immediately
        // For variables first assigned in the try block, just mark them to use cells
        let mut try_cell_locals = indexmap::IndexMap::new();
        for var_id in &try_assigned_vars {
            // Check if variable already has a value (was assigned before the try block)
            if let Some(existing_local) = self.get_var_local(var_id) {
                // Variable exists - create cell with current value
                let var_type = self.get_var_type(var_id).cloned().unwrap_or(Type::Int);

                // Allocate a local for the cell (cells are heap objects, need GC tracking)
                let cell_local = self.alloc_local_id();
                mir_func.add_local(mir::Local {
                    id: cell_local,
                    name: None,
                    ty: Type::HeapAny, // Cell pointer (heap object)
                    is_gc_root: true,  // Cells are heap objects
                });

                // Create cell with current value
                let make_func = self.get_make_cell_func(&var_type);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: cell_local,
                    func: make_func,
                    args: vec![mir::Operand::Local(existing_local)],
                });

                try_cell_locals.insert(*var_id, cell_local);
            }
        }

        // Save original nonlocal_cells
        let saved_nonlocal_cells = self.clone_nonlocal_cells();

        // Install mappings for variables that already have cells
        for (var_id, cell_local) in &try_cell_locals {
            self.insert_nonlocal_cell(*var_id, *cell_local);
        }

        // TrySetjmp terminator: branch to try_body (0) or handler_dispatch (non-zero)
        self.current_block_mut().terminator = mir::Terminator::TrySetjmp {
            frame_local,
            try_body: try_body_id,
            handler_entry: handler_dispatch_id,
        };

        // ===== Try body block =====
        self.push_block(try_body_bb);

        // Lower try body statements
        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        // Pop the exception frame on normal exit path.
        // Must happen even if the block already has a terminator (return/break inside try),
        // because ExcPopFrame is an instruction that executes before the terminator.
        self.emit_instruction(mir::InstructionKind::ExcPopFrame);
        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(try_success_target);
        }

        // ===== Handler dispatch block =====
        self.push_block(handler_dispatch_bb);

        // Exception dispatch:
        // - For handlers with types, check exception type and branch appropriately
        // - For bare except (no type), catch all
        if handlers.is_empty() {
            // No handlers - just set propagating = true and go to finally
            self.emit_instruction(mir::InstructionKind::Const {
                dest: propagating_local,
                value: mir::Constant::Bool(true),
            });
            self.current_block_mut().terminator = mir::Terminator::Goto(finally_id);
        } else {
            // Create handler blocks and optional type-check blocks
            // Each handler has: block, type_tag (Option<u8>), is_custom_class (bool)
            let mut handler_info: Vec<(mir::BasicBlock, Option<u8>, bool)> = Vec::new();
            for handler in handlers {
                let handler_bb = self.new_block();
                // Get exception type tag if handler has a type
                let (type_tag, is_custom) = if let Some(ty) = handler.ty.as_ref() {
                    if let Some((tag, is_custom_class)) = get_exc_type_tag_from_type(ty) {
                        (Some(tag), is_custom_class)
                    } else {
                        (None, false)
                    }
                } else {
                    (None, false) // Bare except
                };
                handler_info.push((handler_bb, type_tag, is_custom));
            }

            // Build dispatch chain: check each handler's type in order
            // If type matches, go to handler; otherwise, try next handler
            // If no handler matches, set propagating=true and go to finally
            self.build_exception_dispatch(
                &handler_info,
                0,
                propagating_local,
                finally_id,
                mir_func,
            );

            // Lower each handler
            for (i, (handler, (handler_bb, _, _))) in handlers.iter().zip(handler_info).enumerate()
            {
                self.push_block(handler_bb);

                // Mark start of exception handling - preserves exception for __context__
                // This must be done BEFORE ExcGetCurrent so the exception is captured
                self.emit_instruction(mir::InstructionKind::ExcStartHandling);

                // If handler has a name binding (except Exception as e:), bind it
                if let Some(var_id) = handler.name {
                    // Use the handler's exception type for the bound variable.
                    // For built-in exceptions: Type::BuiltinException(kind)
                    // For custom exception classes: Type::Class { class_id, name }
                    // For bare except: defaults to Type::BuiltinException(Exception)
                    let exc_type = handler.ty.clone().unwrap_or(Type::BuiltinException(
                        pyaot_core_defs::BuiltinExceptionKind::Exception,
                    ));
                    let exc_local = self.alloc_and_add_local(exc_type.clone(), mir_func);
                    // Get current exception instance
                    self.emit_instruction(mir::InstructionKind::ExcGetCurrent { dest: exc_local });
                    // Map VarId to LocalId and register the type for type inference
                    self.insert_var_local(var_id, exc_local);
                    self.insert_var_type(var_id, exc_type);
                }

                // Clear exception (it's been handled)
                self.emit_instruction(mir::InstructionKind::ExcClear);

                // Lower handler body normally - if it raises, the exception will propagate
                // and the outer finally block will run
                // NOTE: We do NOT wrap the handler in try/finally because that would cause
                // the finally block to run twice (once here, once at the outer level)
                for stmt_id in &handler.body {
                    let stmt = &hir_module.stmts[*stmt_id];
                    self.lower_stmt(stmt, hir_module, mir_func)?;
                }

                // After handler completes normally, mark end of exception handling
                // This clears the saved exception since we're done handling
                // NOTE: We do NOT set propagating=true here, because the handler may have
                // suppressed the exception (by not reraising). propagating only gets set to true
                // when NO handler matches and we go directly to finally.
                if !self.current_block_has_terminator() {
                    self.emit_instruction(mir::InstructionKind::ExcEndHandling);
                    self.current_block_mut().terminator = mir::Terminator::Goto(finally_id);
                }

                let _ = i; // suppress unused warning
            }
        }

        // ===== Else block (runs only if no exception in try body) =====
        // Important: Exceptions in else are NOT caught by this try's handlers,
        // but they MUST still execute the finally block before propagating.
        //
        // Strategy: If there's a finally block, recursively lower else+finally as
        // a nested try/finally with NO handlers. This ensures:
        // 1. Main frame is popped before else (so handlers don't catch exceptions from else)
        // 2. Helper try/finally ensures finally runs if else raises
        // 3. Exception then propagates to outer handlers
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);

            // Frame was already popped at end of try body, so else runs with no active frame
            // This ensures exceptions in else are NOT caught by this try's handlers

            // If there's a finally block, wrap else in a try/finally with no handlers
            // to ensure finally runs even if else raises
            if !finally_block.is_empty() {
                // Lower as: try { else_body } finally { finally_block }
                // This is a recursive call to lower_try with no handlers and no else
                // The recursive call will create its own finally block
                self.lower_try(
                    else_block,    // try body = else block
                    &[],           // no handlers
                    &[],           // no else
                    finally_block, // finally block
                    hir_module,
                    mir_func,
                )?;
                // After the helper try/finally completes (which already ran finally),
                // jump directly to exit, SKIPPING the main finally block
                // (otherwise finally would run twice!)
                if !self.current_block_has_terminator() {
                    self.current_block_mut().terminator = mir::Terminator::Goto(exit_id);
                }
            } else {
                // No finally block - just lower else block statements normally
                for stmt_id in else_block {
                    let stmt = &hir_module.stmts[*stmt_id];
                    self.lower_stmt(stmt, hir_module, mir_func)?;
                }

                if !self.current_block_has_terminator() {
                    self.current_block_mut().terminator = mir::Terminator::Goto(finally_id);
                }
            }
        }

        // ===== Finally block with conditional frame pop =====
        // We need TWO entry points to finally:
        // 1. From normal path: pop frame first (to protect against exceptions in finally)
        // 2. From exception path: don't pop (frame already popped by rt_exc_raise)
        //
        // Strategy: Create a dispatch block that checks propagating and branches
        let finally_dispatch_bb = finally_bb;
        let finally_with_pop_bb = self.new_block();
        let finally_no_pop_bb = self.new_block();
        let finally_body_bb = self.new_block();

        let _finally_with_pop_id = finally_with_pop_bb.id;
        let _finally_no_pop_id = finally_no_pop_bb.id;
        let finally_body_id = finally_body_bb.id;

        // Finally dispatch - just jump directly to finally body
        // Frame is ALWAYS already popped before reaching here:
        // - Normal path: popped at end of try body
        // - Exception path: popped by rt_exc_raise
        // - After handler: popped by rt_exc_raise (even if handler suppressed)
        self.push_block(finally_dispatch_bb);
        self.current_block_mut().terminator = mir::Terminator::Goto(finally_body_id);

        // Finally body (shared by both paths)
        self.push_block(finally_body_bb);

        // Lower finally statements if any
        for stmt_id in finally_block {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        // ===== Finally block exit: Frame is ALREADY popped at entry =====
        // Frame was popped at finally entry (see above), so we just need to decide
        // whether to exit normally or reraise based on the propagating flag.
        let has_else = !else_block.is_empty();
        if !self.current_block_has_terminator() {
            if finally_block.is_empty() && handlers.is_empty() && !has_else {
                // Simple try/finally: just exit (frame was popped before else or at finally entry)
                self.current_block_mut().terminator = mir::Terminator::Goto(exit_id);
            } else if handlers.is_empty() && has_else {
                // try/else/finally with no handlers: just exit (frame was popped before else)
                self.current_block_mut().terminator = mir::Terminator::Goto(exit_id);
            } else {
                // Has handlers - check propagating flag to decide whether to reraise
                let reraise_bb = self.new_block();
                let normal_exit_bb = self.new_block();
                let reraise_id = reraise_bb.id;
                let normal_exit_id = normal_exit_bb.id;

                // Branch on propagating flag
                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: mir::Operand::Local(propagating_local),
                    then_block: reraise_id,
                    else_block: normal_exit_id,
                };

                // Normal exit block
                self.push_block(normal_exit_bb);
                self.current_block_mut().terminator = mir::Terminator::Goto(exit_id);

                // Reraise block
                self.push_block(reraise_bb);
                self.current_block_mut().terminator = mir::Terminator::Reraise;
            }
        }

        // ===== Exit block =====
        self.push_block(exit_bb);

        // Extract final values from cells back to normal locals before restoring mappings
        // This ensures variables retain their cell-preserved values after the try block
        for (var_id, cell_local) in &try_cell_locals {
            let var_type = self.get_var_type(var_id).cloned().unwrap_or(Type::Int);

            // Get the value from the cell
            let get_func = self.get_cell_get_func(&var_type);

            // Get or create the normal local for this variable
            let normal_local = if let Some(local) = self.get_var_local(var_id) {
                local
            } else {
                let local = self.alloc_local_id();
                let is_ptr_type = matches!(
                    var_type,
                    Type::Str
                        | Type::List(_)
                        | Type::Dict(_, _)
                        | Type::Tuple(_)
                        | Type::Set(_)
                        | Type::Bytes
                        | Type::Class { .. }
                        | Type::Iterator(_)
                        | Type::Union(_)
                );
                mir_func.add_local(mir::Local {
                    id: local,
                    name: None,
                    ty: var_type.clone(),
                    is_gc_root: is_ptr_type,
                });
                self.insert_var_local(*var_id, local);
                local
            };

            // Read from cell into normal local
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: normal_local,
                func: get_func,
                args: vec![mir::Operand::Local(*cell_local)],
            });
        }

        // Restore original nonlocal_cells (remove temporary try-block cell mappings)
        self.restore_nonlocal_cells(saved_nonlocal_cells);

        Ok(())
    }

    /// Build exception dispatch chain for type checking
    /// Checks handlers in order, branching to the first matching handler
    /// handler_info: (block, type_tag, is_custom_class)
    fn build_exception_dispatch(
        &mut self,
        handler_info: &[(mir::BasicBlock, Option<u8>, bool)],
        index: usize,
        propagating_local: pyaot_utils::LocalId,
        finally_id: pyaot_utils::BlockId,
        mir_func: &mut mir::Function,
    ) {
        if index >= handler_info.len() {
            // No more handlers - set propagating=true and go to finally
            self.emit_instruction(mir::InstructionKind::Const {
                dest: propagating_local,
                value: mir::Constant::Bool(true),
            });
            self.current_block_mut().terminator = mir::Terminator::Goto(finally_id);
            return;
        }

        let (handler_bb, type_tag, is_custom_class) = &handler_info[index];
        let handler_id = handler_bb.id;

        match type_tag {
            None => {
                // Bare except - catches all, go directly to handler
                self.current_block_mut().terminator = mir::Terminator::Goto(handler_id);
            }
            Some(tag)
                if !*is_custom_class
                    && *tag == pyaot_core_defs::BuiltinExceptionKind::BaseException.tag() =>
            {
                // BaseException - catches all, go directly to handler
                self.current_block_mut().terminator = mir::Terminator::Goto(handler_id);
            }
            Some(tag) => {
                // Typed handler - check if exception matches
                let check_local = self.alloc_and_add_local(Type::Bool, mir_func);

                // Always use ExcCheckClass for inheritance-aware exception matching.
                // This handles both:
                // - Built-in exceptions: except ValueError catches ValueError
                // - Custom exceptions inheriting from built-ins: except ValueError catches ValidationError(ValueError)
                // - Custom exceptions: except MyError catches MyError and its subclasses
                self.emit_instruction(mir::InstructionKind::ExcCheckClass {
                    dest: check_local,
                    class_id: *tag,
                });

                let _ = is_custom_class; // Suppress unused warning - we now use ExcCheckClass for all

                // Create block for next handler check
                let next_check_bb = self.new_block();
                let next_check_id = next_check_bb.id;

                // Branch: if matches, go to handler; otherwise, try next
                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: mir::Operand::Local(check_local),
                    then_block: handler_id,
                    else_block: next_check_id,
                };

                // Continue dispatch in the next check block
                self.push_block(next_check_bb);
                self.build_exception_dispatch(
                    handler_info,
                    index + 1,
                    propagating_local,
                    finally_id,
                    mir_func,
                );
            }
        }
    }
}
