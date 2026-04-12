//! Generator desugaring pass
//!
//! Transforms generator functions (`is_generator: true`) into regular functions
//! at HIR level. Each generator function is split into:
//! 1. **Creator function** — allocates generator object, stores params, returns it
//! 2. **Resume function** — state machine that dispatches on state, yields values
//!
//! After desugaring, all generator functions have `is_generator = false` and the
//! lowering pipeline processes them as regular functions.

use std::collections::HashSet;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::{FuncId, Span, VarId, RESUME_FUNC_ID_OFFSET};

use super::for_loop::detect_for_loop_generator;
use super::utils::collect_yield_info;
use super::vars::collect_generator_vars;
use super::while_loop::detect_while_loop_generator;
use super::GeneratorVar;
use crate::context::Lowering;
use crate::utils::get_iterable_info;

/// Offset added to the module's max VarId for synthetic generator variables.
const GEN_VAR_ID_OFFSET: u32 = 20000;

// ============================================================================
// Arena allocation helpers (free functions, avoid closure borrow issues)
// ============================================================================

fn mk_expr(m: &mut hir::Module, kind: hir::ExprKind, ty: Option<Type>, span: Span) -> hir::ExprId {
    m.exprs.alloc(hir::Expr { kind, ty, span })
}

fn mk_stmt(m: &mut hir::Module, kind: hir::StmtKind, span: Span) -> hir::StmtId {
    m.stmts.alloc(hir::Stmt { kind, span })
}

/// Allocate a `Var(var_id)` expression referencing the given variable.
fn mk_var(m: &mut hir::Module, var_id: VarId, ty: Type, span: Span) -> hir::ExprId {
    mk_expr(m, hir::ExprKind::Var(var_id), Some(ty), span)
}

/// Allocate a `GeneratorIntrinsic::GetLocal` expression.
fn mk_get_local(m: &mut hir::Module, gen_obj_var: VarId, idx: u32, span: Span) -> hir::ExprId {
    let g = mk_var(m, gen_obj_var, Type::HeapAny, span);
    mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::GetLocal { gen: g, idx }),
        Some(Type::Int),
        span,
    )
}

/// Allocate a `GeneratorIntrinsic::SetLocal` expression.
fn mk_set_local(
    m: &mut hir::Module,
    gen_obj_var: VarId,
    idx: u32,
    value: hir::ExprId,
    span: Span,
) -> hir::ExprId {
    let g = mk_var(m, gen_obj_var, Type::HeapAny, span);
    mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::SetLocal { gen: g, idx, value }),
        Some(Type::Int),
        span,
    )
}

/// Allocate a `GeneratorIntrinsic::SetState` expression.
fn mk_set_state(m: &mut hir::Module, gen_obj_var: VarId, state: i64, span: Span) -> hir::ExprId {
    let g = mk_var(m, gen_obj_var, Type::HeapAny, span);
    mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::SetState { gen: g, state }),
        Some(Type::Int),
        span,
    )
}

/// Clone an existing expression into a new arena slot (new ExprId, same content).
fn clone_expr(m: &mut hir::Module, eid: hir::ExprId) -> hir::ExprId {
    let e = m.exprs[eid].clone();
    m.exprs.alloc(e)
}

/// Build: `__gen_set_exhausted(gen_obj); return 0`
fn mk_exhaust_block(m: &mut hir::Module, gen_obj_var: VarId, span: Span) -> Vec<hir::StmtId> {
    let g = mk_var(m, gen_obj_var, Type::HeapAny, span);
    let set_exhausted = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::SetExhausted(g)),
        Some(Type::Int),
        span,
    );
    let s1 = mk_stmt(m, hir::StmtKind::Expr(set_exhausted), span);
    let zero = mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span);
    let s2 = mk_stmt(m, hir::StmtKind::Return(Some(zero)), span);
    vec![s1, s2]
}

/// Build the standard preamble for every resume function:
///   state = __gen_get_state(gen_obj)
///   if __gen_is_exhausted(gen_obj): return 0
fn mk_resume_preamble(
    m: &mut hir::Module,
    gen_obj_var: VarId,
    state_var: VarId,
    span: Span,
) -> Vec<hir::StmtId> {
    let mut stmts = Vec::new();
    // state = __gen_get_state(gen_obj)
    let g1 = mk_var(m, gen_obj_var, Type::HeapAny, span);
    let get_state = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::GetState(g1)),
        Some(Type::Int),
        span,
    );
    stmts.push(mk_stmt(
        m,
        hir::StmtKind::Assign {
            target: state_var,
            value: get_state,
            type_hint: Some(Type::Int),
        },
        span,
    ));

    // if __gen_is_exhausted(gen_obj): return 0
    let g2 = mk_var(m, gen_obj_var, Type::HeapAny, span);
    let is_exhausted = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::IsExhausted(g2)),
        Some(Type::Bool),
        span,
    );
    let zero = mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span);
    let ret = mk_stmt(m, hir::StmtKind::Return(Some(zero)), span);
    stmts.push(mk_stmt(
        m,
        hir::StmtKind::If {
            cond: is_exhausted,
            then_block: vec![ret],
            else_block: vec![],
        },
        span,
    ));

    stmts
}

/// Build: `if state == N: [then_block] else: [else_block]`
fn mk_state_check(
    m: &mut hir::Module,
    state_var: VarId,
    state_val: i64,
    then_block: Vec<hir::StmtId>,
    else_block: Vec<hir::StmtId>,
    span: Span,
) -> hir::StmtId {
    let sr = mk_var(m, state_var, Type::Int, span);
    let sc = mk_expr(m, hir::ExprKind::Int(state_val), Some(Type::Int), span);
    let cmp = mk_expr(
        m,
        hir::ExprKind::Compare {
            left: sr,
            op: hir::CmpOp::Eq,
            right: sc,
        },
        Some(Type::Bool),
        span,
    );
    mk_stmt(
        m,
        hir::StmtKind::If {
            cond: cmp,
            then_block,
            else_block,
        },
        span,
    )
}

/// Emit: load all gen_vars from generator locals into HIR variables.
fn emit_load_all_vars(
    m: &mut hir::Module,
    gen_vars: &[GeneratorVar],
    gen_obj_var: VarId,
    body: &mut Vec<hir::StmtId>,
    span: Span,
) {
    for gv in gen_vars {
        let get = mk_get_local(m, gen_obj_var, gv.gen_local_idx, span);
        body.push(mk_stmt(
            m,
            hir::StmtKind::Assign {
                target: gv.var_id,
                value: get,
                type_hint: Some(gv.ty.clone()),
            },
            span,
        ));
    }
}

/// Emit: save all gen_vars from HIR variables into generator locals.
fn emit_save_all_vars(
    m: &mut hir::Module,
    gen_vars: &[GeneratorVar],
    gen_obj_var: VarId,
    body: &mut Vec<hir::StmtId>,
    span: Span,
) {
    for gv in gen_vars {
        let vr = mk_var(m, gv.var_id, gv.ty.clone(), span);
        let set = mk_set_local(m, gen_obj_var, gv.gen_local_idx, vr, span);
        body.push(mk_stmt(m, hir::StmtKind::Expr(set), span));
    }
}

// ============================================================================
// Main entry point
// ============================================================================

impl<'a> Lowering<'a> {
    /// Desugar all generator functions in the module into regular functions.
    pub(crate) fn desugar_generators(&mut self, hir_module: &mut hir::Module) -> Result<()> {
        let gen_func_ids: Vec<FuncId> = hir_module
            .func_defs
            .iter()
            .filter(|(_, f)| f.is_generator)
            .map(|(id, _)| *id)
            .collect();

        if gen_func_ids.is_empty() {
            return Ok(());
        }

        // Find max VarId in the module to allocate fresh VarIds above it
        let mut max_var_id: u32 = 0;
        for func in hir_module.func_defs.values() {
            for param in &func.params {
                max_var_id = max_var_id.max(param.var.0);
            }
        }
        let mut next_var_id = max_var_id + GEN_VAR_ID_OFFSET;

        for func_id in gen_func_ids {
            self.desugar_one_generator(func_id, hir_module, &mut next_var_id)?;
        }

        Ok(())
    }

    fn desugar_one_generator(
        &mut self,
        func_id: FuncId,
        m: &mut hir::Module,
        next_var_id: &mut u32,
    ) -> Result<()> {
        let func = m
            .func_defs
            .get(&func_id)
            .expect("internal error: generator func_id not found in HIR module")
            .clone();
        let span = func.span;

        // 1. Collect persistent variables
        let gen_vars = collect_generator_vars(&func, m);
        let num_locals = gen_vars.len() as u32 + 5;

        // 2. Infer yield type
        let yield_elem_type = self.infer_generator_yield_type_for_desugar(&func, m);
        self.func_return_types
            .inner
            .insert(func_id, Type::Iterator(Box::new(yield_elem_type)));

        // 3. Allocate VarIds for resume function
        let gen_obj_var = VarId(*next_var_id);
        *next_var_id += 1;
        let state_var = VarId(*next_var_id);
        *next_var_id += 1;

        // 4. Create resume function
        let resume_func_id = FuncId(func_id.0 + RESUME_FUNC_ID_OFFSET);
        let resume_name = {
            let orig = self.interner.resolve(func.name).to_string();
            self.interner.intern(&format!("{orig}$resume"))
        };

        let resume_body = if let Some(for_gen) = detect_for_loop_generator(&func.body, m) {
            build_for_loop_resume(m, &for_gen, gen_obj_var, state_var, next_var_id, span)
        } else if let Some(while_gen) = detect_while_loop_generator(&func.body, m) {
            build_while_loop_resume(
                m,
                &gen_vars,
                &while_gen,
                gen_obj_var,
                state_var,
                next_var_id,
                span,
            )
        } else {
            build_generic_resume(m, &gen_vars, &func, gen_obj_var, state_var, span)
        };

        let gen_obj_name = self.interner.intern("__gen_obj");
        let resume_func = hir::Function {
            id: resume_func_id,
            name: resume_name,
            params: vec![hir::Param {
                name: gen_obj_name,
                var: gen_obj_var,
                ty: Some(Type::HeapAny),
                default: None,
                kind: hir::ParamKind::Regular,
                span,
            }],
            return_type: Some(Type::Int),
            body: resume_body,
            span,
            cell_vars: HashSet::new(),
            nonlocal_vars: HashSet::new(),
            is_generator: false,
            method_kind: hir::MethodKind::default(),
            is_abstract: false,
        };
        m.func_defs.insert(resume_func_id, resume_func);
        m.functions.push(resume_func_id);

        // 5. Replace original function body with creator logic
        let creator_body = build_creator_body(m, &func, &gen_vars, num_locals, span);
        // Retrieve the already-stored return type (Iterator[elem_type])
        let creator_return_type = self
            .func_return_types
            .inner
            .get(&func_id)
            .cloned()
            .unwrap_or_else(|| Type::Iterator(Box::new(Type::Any)));
        let original = m
            .func_defs
            .get_mut(&func_id)
            .expect("internal error: generator func_id not found in HIR module");
        original.body = creator_body;
        original.is_generator = false;
        // Set return type so callers know this returns an iterator
        original.return_type = Some(creator_return_type);

        Ok(())
    }

    /// Simplified yield type inference for the desugaring pass.
    fn infer_generator_yield_type_for_desugar(
        &self,
        func: &hir::Function,
        m: &hir::Module,
    ) -> Type {
        let ty = self.infer_yield_type_raw(func, m);
        match ty {
            Type::Bool => Type::Int,
            other => other,
        }
    }

    fn infer_yield_type_raw(&self, func: &hir::Function, m: &hir::Module) -> Type {
        if let Some(for_gen) = detect_for_loop_generator(&func.body, m) {
            // Use expr.ty from frontend (not get_type_of_expr_id, which needs &mut self
            // and type planning that hasn't run yet).
            let iter_type = m.exprs[for_gen.iter_expr].ty.clone().unwrap_or(Type::Any);
            let elem_ty = get_iterable_info(&iter_type).map(|(_k, ty)| ty);

            if let Some(yield_eid) = for_gen.yield_expr {
                let yield_ty = m.exprs[yield_eid].ty.clone().unwrap_or(Type::Any);
                if yield_ty != Type::Any {
                    return yield_ty;
                }
                // Attribute access: yield v.field
                let ye = &m.exprs[yield_eid];
                if let hir::ExprKind::Attribute { obj, attr } = &ye.kind {
                    if let hir::ExprKind::Var(vid) = &m.exprs[*obj].kind {
                        if *vid == for_gen.target_var {
                            if let Some(Type::Class { class_id, .. }) = &elem_ty {
                                if let Some(ci) = self.classes.class_info.get(class_id) {
                                    if let Some(ft) = ci.field_types.get(attr) {
                                        return ft.clone();
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(ty) = elem_ty {
                if ty != Type::Any {
                    return ty;
                }
            }
        }
        Type::Any
    }
}

// ============================================================================
// Creator body
// ============================================================================

fn build_creator_body(
    m: &mut hir::Module,
    func: &hir::Function,
    gen_vars: &[GeneratorVar],
    num_locals: u32,
    span: Span,
) -> Vec<hir::StmtId> {
    let mut stmts = Vec::new();
    let creator_gen_var = VarId(GEN_VAR_ID_OFFSET + 30000 + func.id.0);

    // gen_obj = GeneratorIntrinsic::Create { func_id, num_locals }
    let create = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::Create {
            func_id: func.id.0,
            num_locals,
        }),
        Some(Type::Iterator(Box::new(Type::Any))),
        span,
    );
    stmts.push(mk_stmt(
        m,
        hir::StmtKind::Assign {
            target: creator_gen_var,
            value: create,
            type_hint: Some(Type::Iterator(Box::new(Type::Any))),
        },
        span,
    ));

    // Save each parameter to generator locals
    for gv in gen_vars {
        if gv.is_param {
            let pv = mk_var(m, gv.var_id, gv.ty.clone(), span);
            let set = mk_set_local(m, creator_gen_var, gv.gen_local_idx, pv, span);
            stmts.push(mk_stmt(m, hir::StmtKind::Expr(set), span));
        }
    }

    // Initialize constant assignments before first yield
    for stmt_id in &func.body {
        let stmt = m.stmts[*stmt_id].clone();
        match &stmt.kind {
            hir::StmtKind::Assign { target, value, .. } => {
                let ve = m.exprs[*value].clone();
                let is_const = matches!(
                    ve.kind,
                    hir::ExprKind::Int(_) | hir::ExprKind::Float(_) | hir::ExprKind::Bool(_)
                );
                if is_const {
                    if let Some(gv) = gen_vars.iter().find(|v| v.var_id == *target) {
                        let val = clone_expr(m, *value);
                        let set = mk_set_local(m, creator_gen_var, gv.gen_local_idx, val, span);
                        stmts.push(mk_stmt(m, hir::StmtKind::Expr(set), span));
                    }
                }
            }
            hir::StmtKind::Expr(eid) => {
                let e = &m.exprs[*eid];
                if matches!(e.kind, hir::ExprKind::Yield(_)) {
                    break;
                }
            }
            _ => {}
        }
    }

    // For for-loop generators, evaluate the iterable and create an iterator,
    // then store it at slot 0. We use `Builtin::Iter` to create the iterator;
    // the lowering's `lower_builtin_call` handles type dispatch (list → rt_iter_list, etc.).
    if let Some(for_gen) = detect_for_loop_generator(&func.body, m) {
        let iter_expr_clone = clone_expr(m, for_gen.iter_expr);
        let iter_call = mk_expr(
            m,
            hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Iter,
                args: vec![iter_expr_clone],
                kwargs: vec![],
            },
            Some(Type::Iterator(Box::new(Type::Any))),
            span,
        );
        let set_iter = mk_set_local(m, creator_gen_var, 0, iter_call, span);
        stmts.push(mk_stmt(m, hir::StmtKind::Expr(set_iter), span));

        // Mark slot 0 as heap pointer for GC
        let g = mk_var(m, creator_gen_var, Type::HeapAny, span);
        let set_type = mk_expr(
            m,
            hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::SetLocalType {
                gen: g,
                idx: 0,
                type_tag: 3, // LOCAL_TYPE_PTR
            }),
            Some(Type::Int),
            span,
        );
        stmts.push(mk_stmt(m, hir::StmtKind::Expr(set_type), span));
    }

    // return gen_obj
    let ret_val = mk_var(
        m,
        creator_gen_var,
        Type::Iterator(Box::new(Type::Any)),
        span,
    );
    stmts.push(mk_stmt(m, hir::StmtKind::Return(Some(ret_val)), span));

    stmts
}

// ============================================================================
// Generic sequential resume
// ============================================================================

fn build_generic_resume(
    m: &mut hir::Module,
    gen_vars: &[GeneratorVar],
    func: &hir::Function,
    gen_obj_var: VarId,
    state_var: VarId,
    span: Span,
) -> Vec<hir::StmtId> {
    let yield_infos = collect_yield_info(&func.body, m);
    let n = yield_infos.len();
    let mut stmts = mk_resume_preamble(m, gen_obj_var, state_var, span);

    // Build state dispatch as nested if/elif, from last state backwards
    let mut else_block = mk_exhaust_block(m, gen_obj_var, span);

    for i in (0..n).rev() {
        let yi = &yield_infos[i];
        let mut body = Vec::new();

        // 1. Load all generator variables from generator locals.
        //    This restores the state from previous yield.
        emit_load_all_vars(m, gen_vars, gen_obj_var, &mut body, span);

        // 2. For states > 0, handle sent value from previous yield.
        //    The sent value goes to the assignment target variable
        //    (e.g., `received = yield 1` → received gets the sent value).
        if i > 0 {
            let prev = &yield_infos[i - 1];
            if let Some(target) = prev.assignment_target {
                // Get sent value and assign directly to the target VarId
                let g = mk_var(m, gen_obj_var, Type::HeapAny, span);
                let get_sent = mk_expr(
                    m,
                    hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::GetSentValue(g)),
                    Some(Type::Int),
                    span,
                );
                body.push(mk_stmt(
                    m,
                    hir::StmtKind::Assign {
                        target,
                        value: get_sent,
                        type_hint: Some(Type::Int),
                    },
                    span,
                ));
                // Also save sent value to the generator local for persistence
                if let Some(gv) = gen_vars.iter().find(|v| v.var_id == target) {
                    let tv = mk_var(m, target, Type::Int, span);
                    let set = mk_set_local(m, gen_obj_var, gv.gen_local_idx, tv, span);
                    body.push(mk_stmt(m, hir::StmtKind::Expr(set), span));
                }
            }
        }

        // 3. Compute yield value (clone the original expression).
        //    Original expressions reference VarIds that were loaded in step 1.
        let yield_value = yi
            .yield_value
            .map(|eid| clone_expr(m, eid))
            .unwrap_or_else(|| mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span));

        // 4. Save all variables back to generator locals
        emit_save_all_vars(m, gen_vars, gen_obj_var, &mut body, span);

        // 5. Set next state
        let set_state = mk_set_state(m, gen_obj_var, (i + 1) as i64, span);
        body.push(mk_stmt(m, hir::StmtKind::Expr(set_state), span));

        // 6. Return yield value
        body.push(mk_stmt(m, hir::StmtKind::Return(Some(yield_value)), span));

        // Wrap in if state == i
        let if_stmt = mk_state_check(m, state_var, i as i64, body, else_block, span);
        else_block = vec![if_stmt];
    }

    stmts.extend(else_block);
    stmts
}

// ============================================================================
// While-loop resume
// ============================================================================

fn build_while_loop_resume(
    m: &mut hir::Module,
    gen_vars: &[GeneratorVar],
    wg: &super::WhileLoopGenerator,
    gen_obj_var: VarId,
    state_var: VarId,
    next_var_id: &mut u32,
    span: Span,
) -> Vec<hir::StmtId> {
    let num_yields = wg.yield_sections.len();
    let mut stmts = mk_resume_preamble(m, gen_obj_var, state_var, span);

    // State numbering:
    //   State 0 (init): load params, init stmts, cond → yield section[0], set state=1
    //   State 1..N-1 (yields for sections 1..N-1): load, stmts, yield, save, set state
    //   State N (update): load, update stmts, save, cond → yield section[0], set state=1
    //
    // For single-yield (N=1): State 0=init, State 1=update (no intermediate yield states)
    let update_state = if num_yields == 1 {
        1i64
    } else {
        num_yields as i64
    };
    let mut else_block = mk_exhaust_block(m, gen_obj_var, span);

    // Update state
    let update_body = build_while_update(m, gen_vars, wg, gen_obj_var, num_yields, span);
    let update_if = mk_state_check(m, state_var, update_state, update_body, else_block, span);
    else_block = vec![update_if];

    // Yield states for sections 1..N-1 (only if N > 1)
    if num_yields > 1 {
        for yi in (1..num_yields).rev() {
            let section = wg.yield_sections[yi].clone();
            // State yi: yields section[yi], sets state = yi+1 (or update)
            let next_state = if yi < num_yields - 1 {
                (yi + 1) as i64
            } else {
                update_state
            };
            let yield_body = build_while_yield_with_next_state(
                m,
                gen_vars,
                &section,
                gen_obj_var,
                next_state,
                span,
            );
            let yield_if = mk_state_check(m, state_var, yi as i64, yield_body, else_block, span);
            else_block = vec![yield_if];
        }
    }

    // State 0: init (yields section[0], sets state=1)
    let init_body = build_while_init(m, gen_vars, wg, gen_obj_var, next_var_id, span);
    let init_if = mk_state_check(m, state_var, 0, init_body, else_block, span);
    stmts.push(init_if);

    stmts
}

fn build_while_init(
    m: &mut hir::Module,
    gen_vars: &[GeneratorVar],
    wg: &super::WhileLoopGenerator,
    gen_obj_var: VarId,
    _next_var_id: &mut u32,
    span: Span,
) -> Vec<hir::StmtId> {
    let mut body = Vec::new();

    // Load parameters
    for gv in gen_vars {
        if gv.is_param {
            let get = mk_get_local(m, gen_obj_var, gv.gen_local_idx, span);
            body.push(mk_stmt(
                m,
                hir::StmtKind::Assign {
                    target: gv.var_id,
                    value: get,
                    type_hint: Some(gv.ty.clone()),
                },
                span,
            ));
        }
    }

    // Execute init statements (reuse original HIR)
    body.extend_from_slice(&wg.init_stmts);

    // Save variables
    emit_save_all_vars(m, gen_vars, gen_obj_var, &mut body, span);

    // Check condition
    let yield_val = wg.yield_sections[0]
        .yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span));

    let set_state = mk_set_state(m, gen_obj_var, 1, span);
    let ss = mk_stmt(m, hir::StmtKind::Expr(set_state), span);
    let ret = mk_stmt(m, hir::StmtKind::Return(Some(yield_val)), span);

    let exhaust = mk_exhaust_block(m, gen_obj_var, span);
    let cond_check = mk_stmt(
        m,
        hir::StmtKind::If {
            cond: wg.cond,
            then_block: vec![ss, ret],
            else_block: exhaust,
        },
        span,
    );
    body.push(cond_check);

    body
}

/// Build a yield state block with an explicit next state value.
fn build_while_yield_with_next_state(
    m: &mut hir::Module,
    gen_vars: &[GeneratorVar],
    section: &super::YieldSection,
    gen_obj_var: VarId,
    next_state: i64,
    span: Span,
) -> Vec<hir::StmtId> {
    let mut body = Vec::new();

    // Load all variables
    emit_load_all_vars(m, gen_vars, gen_obj_var, &mut body, span);

    // Execute statements before this yield (reuse original HIR)
    body.extend_from_slice(&section.stmts_before);

    // Compute yield value
    let yield_val = section
        .yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span));

    // Save all variables
    emit_save_all_vars(m, gen_vars, gen_obj_var, &mut body, span);

    // Set next state
    let ss = mk_set_state(m, gen_obj_var, next_state, span);
    body.push(mk_stmt(m, hir::StmtKind::Expr(ss), span));

    // return yield_value
    body.push(mk_stmt(m, hir::StmtKind::Return(Some(yield_val)), span));

    body
}

fn build_while_update(
    m: &mut hir::Module,
    gen_vars: &[GeneratorVar],
    wg: &super::WhileLoopGenerator,
    gen_obj_var: VarId,
    num_yields: usize,
    span: Span,
) -> Vec<hir::StmtId> {
    let mut body = Vec::new();

    // Load all variables
    emit_load_all_vars(m, gen_vars, gen_obj_var, &mut body, span);

    // Execute update statements (reuse original HIR)
    body.extend_from_slice(&wg.update_stmts);

    // Save variables
    emit_save_all_vars(m, gen_vars, gen_obj_var, &mut body, span);

    // Re-check condition: if true → state=1 + yield first value; if false → exhaust
    let yield_val = wg.yield_sections[0]
        .yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span));

    let set_state = mk_set_state(m, gen_obj_var, 1, span);
    let ss = mk_stmt(m, hir::StmtKind::Expr(set_state), span);
    let ret = mk_stmt(m, hir::StmtKind::Return(Some(yield_val)), span);

    let exhaust = mk_exhaust_block(m, gen_obj_var, span);
    let _ = num_yields;

    let cond_check = mk_stmt(
        m,
        hir::StmtKind::If {
            cond: wg.cond,
            then_block: vec![ss, ret],
            else_block: exhaust,
        },
        span,
    );
    body.push(cond_check);

    body
}

// ============================================================================
// For-loop resume
// ============================================================================

fn build_for_loop_resume(
    m: &mut hir::Module,
    fg: &super::ForLoopGenerator,
    gen_obj_var: VarId,
    state_var: VarId,
    next_var_id: &mut u32,
    span: Span,
) -> Vec<hir::StmtId> {
    let num_trailing = fg.trailing_yields.len();
    let mut stmts = mk_resume_preamble(m, gen_obj_var, state_var, span);

    // if state == 0: set state = 1 (first call initialization)
    {
        let set_s1 = mk_set_state(m, gen_obj_var, 1, span);
        let ss = mk_stmt(m, hir::StmtKind::Expr(set_s1), span);

        // Build trailing yield state dispatch (else branch of state==0)
        let mut trailing_else = mk_exhaust_block(m, gen_obj_var, span);
        for ti in (0..num_trailing).rev() {
            let trail_body = build_trailing_yield(
                m,
                gen_obj_var,
                &fg.trailing_yields[ti],
                ti,
                num_trailing,
                span,
            );
            let trail_if = mk_state_check(
                m,
                state_var,
                (ti + 2) as i64,
                trail_body,
                trailing_else,
                span,
            );
            trailing_else = vec![trail_if];
        }

        // State 1 — falls through to common iter-next below
        let state1_if = mk_state_check(m, state_var, 1, vec![], trailing_else, span);
        let state0_if = mk_state_check(m, state_var, 0, vec![ss], vec![state1_if], span);
        stmts.push(state0_if);
    }

    // Common iter-next logic
    let iter_var = VarId(*next_var_id);
    *next_var_id += 1;
    let next_val_var = VarId(*next_var_id);
    *next_var_id += 1;
    let iter_done_var = VarId(*next_var_id);
    *next_var_id += 1;

    // iter = __gen_get_local(gen_obj, 0)
    let get_iter = mk_get_local(m, gen_obj_var, 0, span);
    stmts.push(mk_stmt(
        m,
        hir::StmtKind::Assign {
            target: iter_var,
            value: get_iter,
            type_hint: Some(Type::Iterator(Box::new(Type::Any))),
        },
        span,
    ));

    if fg.filter_cond.is_some() {
        // Filtered for-loop: wrap in while True loop that retries until filter passes
        build_for_loop_filtered(
            m,
            fg,
            gen_obj_var,
            iter_var,
            next_val_var,
            iter_done_var,
            num_trailing,
            span,
            &mut stmts,
        );
    } else {
        // Non-filtered: straight iter-next + yield
        build_for_loop_direct(
            m,
            fg,
            gen_obj_var,
            iter_var,
            next_val_var,
            iter_done_var,
            num_trailing,
            span,
            &mut stmts,
        );
    }

    stmts
}

#[allow(clippy::too_many_arguments)]
fn build_for_loop_direct(
    m: &mut hir::Module,
    fg: &super::ForLoopGenerator,
    gen_obj_var: VarId,
    iter_var: VarId,
    next_val_var: VarId,
    iter_done_var: VarId,
    num_trailing: usize,
    span: Span,
    stmts: &mut Vec<hir::StmtId>,
) {
    // next_val = __iter_next_no_exc(iter)
    let ir = mk_var(m, iter_var, Type::Iterator(Box::new(Type::Any)), span);
    let nv = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::IterNextNoExc(ir)),
        Some(Type::Int),
        span,
    );
    stmts.push(mk_stmt(
        m,
        hir::StmtKind::Assign {
            target: next_val_var,
            value: nv,
            type_hint: Some(Type::Int),
        },
        span,
    ));

    // iter_done = __iter_is_exhausted(iter)
    let ir2 = mk_var(m, iter_var, Type::Iterator(Box::new(Type::Any)), span);
    let id = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::IterIsExhausted(ir2)),
        Some(Type::Bool),
        span,
    );
    stmts.push(mk_stmt(
        m,
        hir::StmtKind::Assign {
            target: iter_done_var,
            value: id,
            type_hint: Some(Type::Bool),
        },
        span,
    ));

    // if iter_done: go to first trailing yield or exhaust
    let done_ref = mk_var(m, iter_done_var, Type::Bool, span);
    let done_target = if num_trailing > 0 {
        build_trailing_yield(
            m,
            gen_obj_var,
            &fg.trailing_yields[0],
            0,
            num_trailing,
            span,
        )
    } else {
        mk_exhaust_block(m, gen_obj_var, span)
    };
    stmts.push(mk_stmt(
        m,
        hir::StmtKind::If {
            cond: done_ref,
            then_block: done_target,
            else_block: vec![],
        },
        span,
    ));

    // Assign loop variable
    let nvr = mk_var(m, next_val_var, Type::Int, span);
    stmts.push(mk_stmt(
        m,
        hir::StmtKind::Assign {
            target: fg.target_var,
            value: nvr,
            type_hint: None,
        },
        span,
    ));

    // Save iterator back
    let ir3 = mk_var(m, iter_var, Type::Iterator(Box::new(Type::Any)), span);
    let save = mk_set_local(m, gen_obj_var, 0, ir3, span);
    stmts.push(mk_stmt(m, hir::StmtKind::Expr(save), span));

    // Compute and return yield value
    let yv = fg
        .yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::None, Some(Type::None), span));
    stmts.push(mk_stmt(m, hir::StmtKind::Return(Some(yv)), span));
}

#[allow(clippy::too_many_arguments)]
fn build_for_loop_filtered(
    m: &mut hir::Module,
    fg: &super::ForLoopGenerator,
    gen_obj_var: VarId,
    iter_var: VarId,
    next_val_var: VarId,
    iter_done_var: VarId,
    num_trailing: usize,
    span: Span,
    stmts: &mut Vec<hir::StmtId>,
) {
    let filter_cond_id = fg
        .filter_cond
        .expect("internal error: filter_cond is Some, guaranteed by caller's is_some() check");
    let true_expr = mk_expr(m, hir::ExprKind::Bool(true), Some(Type::Bool), span);

    let mut loop_body = Vec::new();

    // next_val = __iter_next_no_exc(iter)
    let ir = mk_var(m, iter_var, Type::Iterator(Box::new(Type::Any)), span);
    let nv = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::IterNextNoExc(ir)),
        Some(Type::Int),
        span,
    );
    loop_body.push(mk_stmt(
        m,
        hir::StmtKind::Assign {
            target: next_val_var,
            value: nv,
            type_hint: Some(Type::Int),
        },
        span,
    ));

    // iter_done = __iter_is_exhausted(iter)
    let ir2 = mk_var(m, iter_var, Type::Iterator(Box::new(Type::Any)), span);
    let id = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::IterIsExhausted(ir2)),
        Some(Type::Bool),
        span,
    );
    loop_body.push(mk_stmt(
        m,
        hir::StmtKind::Assign {
            target: iter_done_var,
            value: id,
            type_hint: Some(Type::Bool),
        },
        span,
    ));

    // if iter_done: exhaust/trailing
    let done_ref = mk_var(m, iter_done_var, Type::Bool, span);
    let done_target = if num_trailing > 0 {
        build_trailing_yield(
            m,
            gen_obj_var,
            &fg.trailing_yields[0],
            0,
            num_trailing,
            span,
        )
    } else {
        mk_exhaust_block(m, gen_obj_var, span)
    };
    loop_body.push(mk_stmt(
        m,
        hir::StmtKind::If {
            cond: done_ref,
            then_block: done_target,
            else_block: vec![],
        },
        span,
    ));

    // Assign target var
    let nvr = mk_var(m, next_val_var, Type::Int, span);
    loop_body.push(mk_stmt(
        m,
        hir::StmtKind::Assign {
            target: fg.target_var,
            value: nvr,
            type_hint: None,
        },
        span,
    ));

    // if filter_cond: save iter, return yield value
    let mut yield_body = Vec::new();
    let ir3 = mk_var(m, iter_var, Type::Iterator(Box::new(Type::Any)), span);
    let save = mk_set_local(m, gen_obj_var, 0, ir3, span);
    yield_body.push(mk_stmt(m, hir::StmtKind::Expr(save), span));

    let yv = fg
        .yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::None, Some(Type::None), span));
    yield_body.push(mk_stmt(m, hir::StmtKind::Return(Some(yv)), span));

    loop_body.push(mk_stmt(
        m,
        hir::StmtKind::If {
            cond: filter_cond_id,
            then_block: yield_body,
            else_block: vec![], // continue loop: filter didn't match
        },
        span,
    ));

    // while True: [loop_body]
    stmts.push(mk_stmt(
        m,
        hir::StmtKind::While {
            cond: true_expr,
            body: loop_body,
            else_block: vec![],
        },
        span,
    ));
}

fn build_trailing_yield(
    m: &mut hir::Module,
    gen_obj_var: VarId,
    trailing_yield_expr: &Option<hir::ExprId>,
    trailing_idx: usize,
    _num_trailing: usize,
    span: Span,
) -> Vec<hir::StmtId> {
    let mut body = Vec::new();

    // Set state to next trailing yield
    let next_state = (trailing_idx + 2 + 1) as i64;
    let ss = mk_set_state(m, gen_obj_var, next_state, span);
    body.push(mk_stmt(m, hir::StmtKind::Expr(ss), span));

    // Return trailing yield value
    let value = trailing_yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span));
    body.push(mk_stmt(m, hir::StmtKind::Return(Some(value)), span));

    body
}
