//! Generator desugaring pass
//!
//! Transforms generator functions (`is_generator: true`) into regular functions
//! at HIR level. Each generator function is split into:
//! 1. **Creator function** — allocates generator object, stores params, returns it
//! 2. **Resume function** — state machine that dispatches on state, yields values
//!
//! After desugaring, all generator functions have `is_generator = false` and the
//! lowering pipeline processes them as regular functions.

use std::collections::{HashMap, HashSet};

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_hir::cfg_builder::{CfgBuilder, CfgStmt};
use pyaot_types::Type;
use pyaot_utils::{FuncId, Span, StringInterner, VarId, RESUME_FUNC_ID_OFFSET};

use super::for_loop::detect_for_loop_generator;
use super::utils::collect_yield_info;
use super::vars::collect_generator_vars;
use super::while_loop::detect_while_loop_generator;
use super::GeneratorVar;
use crate::context::Lowering;
use crate::utils::get_iterable_info;

/// Offset added to the module's max VarId for synthetic generator variables.
const GEN_VAR_ID_OFFSET: u32 = 20000;

type GenStmt = CfgStmt;

// ============================================================================
// Arena allocation helpers (free functions, avoid closure borrow issues)
// ============================================================================

fn mk_expr(m: &mut hir::Module, kind: hir::ExprKind, ty: Option<Type>, span: Span) -> hir::ExprId {
    m.exprs.alloc(hir::Expr { kind, ty, span })
}

fn mk_stmt(m: &mut hir::Module, kind: hir::StmtKind, span: Span) -> hir::StmtId {
    m.stmts.alloc(hir::Stmt { kind, span })
}

fn mk_leaf_stmt(m: &mut hir::Module, kind: hir::StmtKind, span: Span) -> GenStmt {
    GenStmt::stmt(mk_stmt(m, kind, span))
}

fn wrap_stmt_ids(stmts: &[hir::StmtId]) -> Vec<GenStmt> {
    stmts.iter().copied().map(GenStmt::stmt).collect()
}

/// Allocate a `Var(var_id)` expression referencing the given variable.
fn mk_var(m: &mut hir::Module, var_id: VarId, ty: Type, span: Span) -> hir::ExprId {
    mk_expr(m, hir::ExprKind::Var(var_id), Some(ty), span)
}

/// Module-wide Var → definition map built once per desugar pass.
///
/// Generator desugaring runs **before** type planning, so `Expr.ty` is
/// missing for most expressions and `Var` references have no resolved
/// type. This map indexes every definition site (function params,
/// module-level / function-body Bind statements) so the `shape_infer`
/// walker can follow Var refs to their binding value and recursively
/// reconstruct a type.
///
/// Scope: the whole module. Closure captures in gen-expr functions reuse
/// the outer-scope VarIds, so a single map covers both sides uniformly.
#[derive(Default)]
struct VarTypeMap {
    /// Var ← Bind: the RHS expression assigned to this Var at some point.
    /// If multiple Bind sites exist, the first wins (matches `first-write`
    /// semantics for field-type inference and is usually the declaration).
    by_value: HashMap<VarId, hir::ExprId>,
    /// Var ← typed parameter: explicit type from function signature.
    by_param: HashMap<VarId, Type>,
    /// Var ← ForBind iter: the for-loop iterable expression. Used to
    /// resolve `wo` in `for wo in w: ... zip(wo, x) ...` to the element
    /// type of `w`.
    by_for_iter: HashMap<VarId, hir::ExprId>,
}

impl VarTypeMap {
    fn build(m: &hir::Module) -> Self {
        let mut map = VarTypeMap::default();
        for func in m.func_defs.values() {
            for param in &func.params {
                if let Some(ty) = &param.ty {
                    if *ty != Type::Any {
                        map.by_param.insert(param.var, ty.clone());
                    }
                }
            }
            collect_bind_targets_in_func(m, func, &mut map);
        }
        map
    }
}

fn collect_bind_targets_in_func(m: &hir::Module, func: &hir::Function, map: &mut VarTypeMap) {
    for block in func.blocks.values() {
        for sid in &block.stmts {
            let stmt = &m.stmts[*sid];
            match &stmt.kind {
                hir::StmtKind::Bind { target, value, .. } => {
                    collect_bind_target_vars(target, *value, map);
                }
                hir::StmtKind::IterAdvance { iter, target } => {
                    collect_for_target_vars(target, *iter, map);
                }
                _ => {}
            }
        }
        if let hir::HirTerminator::Branch { cond, .. } = block.terminator {
            let expr = &m.exprs[cond];
            if let hir::ExprKind::IterHasNext(iter) = expr.kind {
                let _ = iter;
            }
        }
    }
}

fn collect_bind_target_vars(t: &hir::BindingTarget, value: hir::ExprId, map: &mut VarTypeMap) {
    // Record only simple `Var` leaves; tuple/attr/index targets don't
    // have a single RHS expression we can assign to their leaves.
    if let hir::BindingTarget::Var(vid) = t {
        map.by_value.entry(*vid).or_insert(value);
    }
}

fn collect_for_target_vars(t: &hir::BindingTarget, iter: hir::ExprId, map: &mut VarTypeMap) {
    // Simple `Var` target: the loop variable's type is iter's element type.
    // For tuple targets, we don't record per-leaf (each leaf is a tuple
    // element); shape_infer reaches them through the iter instead.
    if let hir::BindingTarget::Var(vid) = t {
        map.by_for_iter.entry(*vid).or_insert(iter);
    }
}

/// Recursion guard — caps descent depth and breaks self-referential cycles.
const SHAPE_INFER_MAX_DEPTH: u32 = 12;

/// Element type yielded by `fg.iter_expr` per iteration.
///
/// Generator desugaring runs **before** type planning, so call expressions
/// (`zip`, `enumerate`) have no `Expr.ty` yet. We reconstruct the element
/// shape from their arguments directly — required for the Tuple/Attr/Index
/// target forms of `§C.6`. Falls back to `Type::Int` for opaque sources
/// (e.g. user generators): matches the legacy hardcoded `Int` used before
/// `§C.6`, keeping non-tuple generators unchanged.
fn iter_elem_type(
    m: &hir::Module,
    fg: &super::ForLoopGenerator,
    interner: &StringInterner,
) -> Type {
    let vmap = VarTypeMap::build(m);
    iter_elem_type_with(m, &vmap, fg, interner)
}

fn iter_elem_type_with(
    m: &hir::Module,
    vmap: &VarTypeMap,
    fg: &super::ForLoopGenerator,
    interner: &StringInterner,
) -> Type {
    let iter_ty = m.exprs[fg.iter_expr].ty.clone().unwrap_or(Type::Any);
    if let Some((_k, ty)) = get_iterable_info(&iter_ty) {
        if ty != Type::Any {
            return ty;
        }
    }
    let fallback = if matches!(fg.target, hir::BindingTarget::Var(_)) {
        Type::Int
    } else {
        Type::Any
    };
    arg_elem_type(m, vmap, fg.iter_expr, 0, interner).unwrap_or(fallback)
}

/// Element type of an arbitrary iterable expression.
fn arg_elem_type(
    m: &hir::Module,
    vmap: &VarTypeMap,
    expr_id: hir::ExprId,
    depth: u32,
    interner: &StringInterner,
) -> Option<Type> {
    if depth > SHAPE_INFER_MAX_DEPTH {
        return None;
    }
    // Prefer frontend-annotated type if present.
    if let Some(ty) = &m.exprs[expr_id].ty {
        if let Some((_k, elem)) = get_iterable_info(ty) {
            if elem != Type::Any {
                return Some(elem);
            }
        }
    }
    // Shape-infer the iterable's full type, then extract its element type.
    let inferred = shape_infer_type(m, vmap, expr_id, depth, interner)?;
    get_iterable_info(&inferred).map(|(_k, ty)| ty)
}

/// Best-effort type inference without the memoised `TypeEnvironment`.
///
/// Handles the shapes that commonly appear as gen-expr / comprehension
/// iterables and yield expressions at desugar time:
///   - literal primitives, tuples, lists, sets
///   - `Var` references via `VarTypeMap` (Bind RHS, ForBind iter, params)
///   - `BuiltinCall` for `zip` / `enumerate` (synthesise iterator types)
///   - `MethodCall` for `dict.items/keys/values`
///   - `Attribute` lookup when obj is class-typed
fn shape_infer_type(
    m: &hir::Module,
    vmap: &VarTypeMap,
    expr_id: hir::ExprId,
    depth: u32,
    interner: &StringInterner,
) -> Option<Type> {
    if depth > SHAPE_INFER_MAX_DEPTH {
        return None;
    }
    let expr = &m.exprs[expr_id];
    if let Some(ty) = &expr.ty {
        if !matches!(ty, Type::Any) {
            return Some(ty.clone());
        }
    }
    match &expr.kind {
        hir::ExprKind::Int(_) => Some(Type::Int),
        hir::ExprKind::Float(_) => Some(Type::Float),
        hir::ExprKind::Bool(_) => Some(Type::Bool),
        hir::ExprKind::Str(_) => Some(Type::Str),
        hir::ExprKind::None => Some(Type::None),

        hir::ExprKind::BinOp { left, op, right } => {
            let left_ty = shape_infer_type(m, vmap, *left, depth + 1, interner)?;
            let right_ty = shape_infer_type(m, vmap, *right, depth + 1, interner)?;
            crate::type_planning::helpers::resolve_binop_type(op, &left_ty, &right_ty)
        }
        hir::ExprKind::UnOp { op, operand, .. } => {
            let operand_ty = shape_infer_type(m, vmap, *operand, depth + 1, interner)?;
            let ty = match op {
                hir::UnOp::Not => Type::Bool,
                hir::UnOp::Neg | hir::UnOp::Pos | hir::UnOp::Invert => operand_ty,
            };
            Some(ty)
        }
        hir::ExprKind::FuncRef(func_id) => {
            m.func_defs.get(func_id).and_then(|f| f.return_type.clone())
        }
        hir::ExprKind::Closure { func, .. } => {
            m.func_defs.get(func).and_then(|f| f.return_type.clone())
        }
        hir::ExprKind::Call { func, .. } => match &m.exprs[*func].kind {
            hir::ExprKind::FuncRef(func_id) => {
                m.func_defs.get(func_id).and_then(|f| f.return_type.clone())
            }
            hir::ExprKind::Closure { func, .. } => {
                m.func_defs.get(func).and_then(|f| f.return_type.clone())
            }
            _ => None,
        },

        hir::ExprKind::Tuple(items) => {
            let elem_types: Vec<Type> = items
                .iter()
                .map(|e| shape_infer_type(m, vmap, *e, depth + 1, interner).unwrap_or(Type::Any))
                .collect();
            Some(Type::Tuple(elem_types))
        }
        hir::ExprKind::List(items) => {
            let elem_ty = items
                .first()
                .and_then(|first| shape_infer_type(m, vmap, *first, depth + 1, interner))
                .unwrap_or(Type::Any);
            Some(Type::List(Box::new(elem_ty)))
        }
        hir::ExprKind::Set(items) => {
            let elem_ty = items
                .first()
                .and_then(|first| shape_infer_type(m, vmap, *first, depth + 1, interner))
                .unwrap_or(Type::Any);
            Some(Type::Set(Box::new(elem_ty)))
        }
        hir::ExprKind::Dict(pairs) => {
            let (key_ty, val_ty) = pairs
                .first()
                .map(|(k, v)| {
                    (
                        shape_infer_type(m, vmap, *k, depth + 1, interner).unwrap_or(Type::Any),
                        shape_infer_type(m, vmap, *v, depth + 1, interner).unwrap_or(Type::Any),
                    )
                })
                .unwrap_or((Type::Any, Type::Any));
            Some(Type::Dict(Box::new(key_ty), Box::new(val_ty)))
        }

        hir::ExprKind::Var(vid) => resolve_var_type(m, vmap, *vid, depth + 1, interner),

        hir::ExprKind::IfExpr {
            then_val, else_val, ..
        } => {
            // Ternary: union the two branches' types — if either resolves to
            // a primitive (Int/Bool/Float/Str) and they agree, propagate it
            // so the enclosing yield's return type is concrete.
            let then_ty = shape_infer_type(m, vmap, *then_val, depth + 1, interner);
            let else_ty = shape_infer_type(m, vmap, *else_val, depth + 1, interner);
            match (then_ty, else_ty) {
                (Some(a), Some(b)) if a == b => Some(a),
                (Some(a), Some(b)) => Some(Type::normalize_union(vec![a, b])),
                (Some(a), None) | (None, Some(a)) => Some(a),
                (None, None) => None,
            }
        }

        hir::ExprKind::BuiltinCall {
            builtin: hir::Builtin::Zip,
            args,
            ..
        } => {
            let tuple_elems: Vec<Type> = args
                .iter()
                .map(|a| arg_elem_type(m, vmap, *a, depth + 1, interner).unwrap_or(Type::Any))
                .collect();
            Some(Type::Iterator(Box::new(Type::Tuple(tuple_elems))))
        }
        hir::ExprKind::BuiltinCall {
            builtin: hir::Builtin::Enumerate,
            args,
            ..
        } => {
            let inner = args
                .first()
                .and_then(|a| arg_elem_type(m, vmap, *a, depth + 1, interner))
                .unwrap_or(Type::Any);
            Some(Type::Iterator(Box::new(Type::Tuple(vec![
                Type::Int,
                inner,
            ]))))
        }
        hir::ExprKind::BuiltinCall {
            builtin: hir::Builtin::Range,
            ..
        } => Some(Type::Iterator(Box::new(Type::Int))),

        hir::ExprKind::MethodCall { obj, method, .. } => {
            // Resolve `.items() / .keys() / .values()` on dict-typed
            // receivers. Returns the *container* type the method yields —
            // `arg_elem_type` then extracts per-iteration elements from it.
            let method_name = interner.resolve(*method);
            let obj_ty = shape_infer_type(m, vmap, *obj, depth + 1, interner)?;
            let (kt, vt) = match &obj_ty {
                Type::Dict(k, v) | Type::DefaultDict(k, v) => ((**k).clone(), (**v).clone()),
                _ => return None,
            };
            match method_name {
                "items" => Some(Type::List(Box::new(Type::Tuple(vec![kt, vt])))),
                "keys" => Some(Type::List(Box::new(kt))),
                "values" => Some(Type::List(Box::new(vt))),
                _ => None,
            }
        }

        hir::ExprKind::Attribute { obj, attr } => {
            // Walk the module's class_defs to find the field type when obj
            // shape-infers to a Class. Covers `self.field` / `inst.field`
            // references whose type isn't yet attached via type-planning.
            if let Some(Type::Class { class_id, .. }) =
                shape_infer_type(m, vmap, *obj, depth + 1, interner)
            {
                for cdef in m.class_defs.values() {
                    if cdef.id == class_id {
                        if let Some(fdef) = cdef.fields.iter().find(|f| f.name == *attr) {
                            return Some(fdef.ty.clone());
                        }
                    }
                }
            }
            None
        }

        _ => None,
    }
}

/// Resolve a Var's type via the pre-built map. Priority: param type,
/// for-loop iter element type, Bind RHS shape.
fn resolve_var_type(
    m: &hir::Module,
    vmap: &VarTypeMap,
    vid: VarId,
    depth: u32,
    interner: &StringInterner,
) -> Option<Type> {
    if let Some(ty) = vmap.by_param.get(&vid) {
        return Some(ty.clone());
    }
    if let Some(iter_eid) = vmap.by_for_iter.get(&vid) {
        // Element type of the for-loop iterable — that's the loop var's type.
        return arg_elem_type(m, vmap, *iter_eid, depth, interner);
    }
    if let Some(value_eid) = vmap.by_value.get(&vid) {
        return shape_infer_type(m, vmap, *value_eid, depth, interner);
    }
    None
}

/// Allocate a `GeneratorIntrinsic::GetLocal` expression.
///
/// `ty` declares the logical Python type stored in the slot (e.g. `Int`, `Str`,
/// `Iterator[Any]`). The runtime always returns raw i64 bits; the HIR-level
/// type is used for bidirectional coercion and type validation of the
/// surrounding assignment.
fn mk_get_local(
    m: &mut hir::Module,
    gen_obj_var: VarId,
    idx: u32,
    ty: Type,
    span: Span,
) -> hir::ExprId {
    let g = mk_var(m, gen_obj_var, Type::HeapAny, span);
    mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::GetLocal { gen: g, idx }),
        Some(ty),
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

// §F.7b: mk_set_local_type_ptr removed — per-slot tag side-array deleted.
// SetLocal now boxes primitives via box_primitive_if_needed so GC can walk
// locals uniformly via Value::is_ptr() without a separate type tag array.

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
fn mk_exhaust_block(m: &mut hir::Module, gen_obj_var: VarId, span: Span) -> Vec<GenStmt> {
    let g = mk_var(m, gen_obj_var, Type::HeapAny, span);
    let set_exhausted = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::SetExhausted(g)),
        Some(Type::Int),
        span,
    );
    let s1 = mk_leaf_stmt(m, hir::StmtKind::Expr(set_exhausted), span);
    let zero = mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span);
    let s2 = mk_leaf_stmt(m, hir::StmtKind::Return(Some(zero)), span);
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
) -> Vec<GenStmt> {
    let mut stmts = Vec::new();
    // state = __gen_get_state(gen_obj)
    let g1 = mk_var(m, gen_obj_var, Type::HeapAny, span);
    let get_state = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::GetState(g1)),
        Some(Type::Int),
        span,
    );
    stmts.push(mk_leaf_stmt(
        m,
        hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(state_var),
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
    let ret = mk_leaf_stmt(m, hir::StmtKind::Return(Some(zero)), span);
    stmts.push(GenStmt::If {
        cond: is_exhausted,
        then_body: vec![ret],
        else_body: vec![],
        span,
    });

    stmts
}

/// Build: `if state == N: [then_block] else: [else_block]`
fn mk_state_check(
    m: &mut hir::Module,
    state_var: VarId,
    state_val: i64,
    then_body: Vec<GenStmt>,
    else_body: Vec<GenStmt>,
    span: Span,
) -> GenStmt {
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
    let _ = m;
    GenStmt::If {
        cond: cmp,
        then_body,
        else_body,
        span,
    }
}

/// Emit: load all gen_vars from generator locals into HIR variables.
fn emit_load_all_vars(
    m: &mut hir::Module,
    gen_vars: &[GeneratorVar],
    gen_obj_var: VarId,
    body: &mut Vec<GenStmt>,
    span: Span,
) {
    for gv in gen_vars {
        let get = mk_get_local(m, gen_obj_var, gv.gen_local_idx, gv.ty.clone(), span);
        body.push(mk_leaf_stmt(
            m,
            hir::StmtKind::Bind {
                target: hir::BindingTarget::Var(gv.var_id),
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
    body: &mut Vec<GenStmt>,
    span: Span,
) {
    emit_save_vars_where(m, gen_vars, gen_obj_var, body, span, |_| true);
}

/// Emit save_local calls for the subset of `gen_vars` that pass `keep`.
///
/// Used by `build_while_init` (S1.6e-gen fix): init-state saves MUST
/// skip gen_vars that aren't assigned before the first yield, because
/// referencing such a var in init reads a LocalId only defined in
/// later state blocks — the def doesn't dominate the use and
/// `construct_ssa` reports `UseNotDominated`. The init state's slot
/// for that var stays at Cranelift's default zero; the first block
/// that actually assigns the var also saves it before yielding, so
/// subsequent resumes see the right value.
fn emit_save_vars_where(
    m: &mut hir::Module,
    gen_vars: &[GeneratorVar],
    gen_obj_var: VarId,
    body: &mut Vec<GenStmt>,
    span: Span,
    keep: impl Fn(&GeneratorVar) -> bool,
) {
    for gv in gen_vars {
        if !keep(gv) {
            continue;
        }
        let vr = mk_var(m, gv.var_id, gv.ty.clone(), span);
        let set = mk_set_local(m, gen_obj_var, gv.gen_local_idx, vr, span);
        body.push(mk_leaf_stmt(m, hir::StmtKind::Expr(set), span));
        // §F.7b: No SetLocalType needed — SetLocal now boxes primitives via
        // box_primitive_if_needed so GC walks locals via Value::is_ptr().
    }
}

/// Collect every VarId bound by a statement list (recursive through
/// control-flow bodies). Used to decide which gen_vars are safe to
/// save in the init state.
fn collect_defined_vars(stmts: &[GenStmt], module: &hir::Module, out: &mut HashSet<VarId>) {
    for stmt in stmts {
        match stmt {
            GenStmt::Stmt(stmt_id) => {
                let stmt = &module.stmts[*stmt_id];
                if let hir::StmtKind::Bind { target, .. } = &stmt.kind {
                    target.for_each_var(&mut |v| {
                        out.insert(v);
                    });
                }
            }
            GenStmt::For {
                target,
                body,
                else_body,
                ..
            } => {
                target.for_each_var(&mut |v| {
                    out.insert(v);
                });
                collect_defined_vars(body, module, out);
                collect_defined_vars(else_body, module, out);
            }
            GenStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_defined_vars(then_body, module, out);
                collect_defined_vars(else_body, module, out);
            }
            GenStmt::While {
                body, else_body, ..
            } => {
                collect_defined_vars(body, module, out);
                collect_defined_vars(else_body, module, out);
            }
            GenStmt::Try {
                body,
                handlers,
                else_body,
                finally_body,
                ..
            } => {
                collect_defined_vars(body, module, out);
                for h in handlers {
                    if let Some(v) = h.name {
                        out.insert(v);
                    }
                    collect_defined_vars(&h.body, module, out);
                }
                collect_defined_vars(else_body, module, out);
                collect_defined_vars(finally_body, module, out);
            }
            GenStmt::Match { cases, .. } => {
                for case in cases {
                    collect_defined_vars(&case.body, module, out);
                }
            }
        }
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

        // Area G §G.10: propagate capture types onto gen-expr creator
        // params before desugaring. Gen-exprs receive their free variables
        // through `ExprKind::Closure { func, captures }` (see
        // comprehensions.rs::desugar_generator_expression). At call time,
        // `lower_closure_call` prepends capture values to the call args,
        // so the creator's first N params *are* the captures. The frontend
        // creates those params with `ty: None`, but the resume function's
        // for-loop element type (used to select the right unbox step via
        // `emit_tuple_get`) is inferred here at desugar time via
        // `VarTypeMap`, which only sees typed params. Without this pass a
        // nested gen-expr like
        //     [sum(wi * xi for wi, xi in zip(wo, x)) for wo in w]
        // leaves the inner zip's tuple elements as `Any` and the
        // `wi * xi` multiplication overflows on raw pointers.
        //
        // Iterate up to a small bound so nested captures (inner gen-expr
        // capturing an outer gen-expr's capture param) converge.
        let gen_func_set: std::collections::HashSet<FuncId> =
            gen_func_ids.iter().copied().collect();
        for _ in 0..3 {
            if !self.propagate_genexp_capture_types(hir_module, &gen_func_set) {
                break;
            }
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

    /// For every `ExprKind::Closure { func, captures }` whose `func` is a
    /// gen-expr creator, resolve each capture's type via `VarTypeMap` and
    /// write it onto the corresponding `func.params[i].ty`. Returns
    /// `true` if any param type was updated — callers can iterate to
    /// fixed-point for nested gen-expr chains.
    fn propagate_genexp_capture_types(
        &self,
        m: &mut hir::Module,
        gen_func_set: &std::collections::HashSet<FuncId>,
    ) -> bool {
        let vmap = VarTypeMap::build(m);
        // Collect (func_id, param_idx -> type) updates before mutating to
        // satisfy the borrow checker.
        let mut updates: Vec<(FuncId, Vec<(usize, Type)>)> = Vec::new();
        for (_eid, expr) in m.exprs.iter() {
            if let hir::ExprKind::Closure { func, captures } = &expr.kind {
                if !gen_func_set.contains(func) {
                    continue;
                }
                let Some(func_def) = m.func_defs.get(func) else {
                    continue;
                };
                let mut param_updates = Vec::new();
                for (i, cap_id) in captures.iter().enumerate() {
                    if i >= func_def.params.len() {
                        break;
                    }
                    if func_def.params[i].ty.is_some() {
                        continue;
                    }
                    let cap_expr = &m.exprs[*cap_id];
                    let ty = shape_infer_type(m, &vmap, *cap_id, 0, self.interner)
                        .or_else(|| cap_expr.ty.clone())
                        .unwrap_or(Type::Any);
                    if !matches!(ty, Type::Any) {
                        param_updates.push((i, ty));
                    }
                }
                if !param_updates.is_empty() {
                    updates.push((*func, param_updates));
                }
            }
        }

        let changed = !updates.is_empty();
        for (func_id, param_updates) in updates {
            if let Some(func_def) = m.func_defs.get_mut(&func_id) {
                for (idx, ty) in param_updates {
                    if let Some(param) = func_def.params.get_mut(idx) {
                        if param.ty.is_none() {
                            param.ty = Some(ty);
                        }
                    }
                }
            }
        }
        changed
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

        let resume_body = if let Some(for_gen) = detect_for_loop_generator(&func, m) {
            build_for_loop_resume(
                m,
                &for_gen,
                gen_obj_var,
                state_var,
                next_var_id,
                span,
                self.interner,
            )
        } else if let Some(while_gen) = detect_while_loop_generator(&func, m) {
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
        let mut resume_cfg = CfgBuilder::new();
        let resume_entry_block = resume_cfg.new_block();
        resume_cfg.enter(resume_entry_block);
        resume_cfg.lower_cfg_stmts(&resume_body, m);
        resume_cfg.terminate_if_open(hir::HirTerminator::Return(None));
        let (resume_blocks, resume_entry_block, resume_try_scopes) =
            resume_cfg.finish(resume_entry_block);
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
            return_type: Some(Type::Any),
            span,
            cell_vars: HashSet::new(),
            nonlocal_vars: HashSet::new(),
            is_generator: false,
            method_kind: hir::MethodKind::default(),
            is_abstract: false,
            blocks: resume_blocks,
            entry_block: resume_entry_block,
            try_scopes: resume_try_scopes,
        };
        m.func_defs.insert(resume_func_id, resume_func);
        m.functions.push(resume_func_id);

        // 5. Replace original function body with creator logic
        // Retrieve the already-stored return type (Iterator[elem_type])
        let creator_return_type = self
            .func_return_types
            .inner
            .get(&func_id)
            .cloned()
            .unwrap_or_else(|| Type::Iterator(Box::new(Type::Any)));
        let creator_body =
            build_creator_body(m, &func, &gen_vars, num_locals, &creator_return_type, span);
        let mut creator_cfg = CfgBuilder::new();
        let creator_entry_block = creator_cfg.new_block();
        creator_cfg.enter(creator_entry_block);
        creator_cfg.lower_cfg_stmts(&creator_body, m);
        creator_cfg.terminate_if_open(hir::HirTerminator::Return(None));
        let (creator_blocks, creator_entry_block, creator_try_scopes) =
            creator_cfg.finish(creator_entry_block);
        let original = m
            .func_defs
            .get_mut(&func_id)
            .expect("internal error: generator func_id not found in HIR module");
        original.is_generator = false;
        // Set return type so callers know this returns an iterator
        original.return_type = Some(creator_return_type);
        original.blocks = creator_blocks;
        original.entry_block = creator_entry_block;
        original.try_scopes = creator_try_scopes;

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
        if let Some(for_gen) = detect_for_loop_generator(func, m) {
            let vmap = VarTypeMap::build(m);
            // Use the module-wide Var/param index so Var refs in yield expressions
            // (`yield (v, i)` where v and i are for-loop targets) resolve to
            // their inferred types even though type planning hasn't run yet.
            let iter_type = m.exprs[for_gen.iter_expr].ty.clone().unwrap_or(Type::Any);
            let elem_ty = get_iterable_info(&iter_type).map(|(_k, ty)| ty);

            if let Some(yield_eid) = for_gen.yield_expr {
                let yield_ty = m.exprs[yield_eid].ty.clone().unwrap_or(Type::Any);
                if yield_ty != Type::Any {
                    return yield_ty;
                }
                // Shape-infer the yield expression: handles `yield (v, i)`
                // by reconstructing `Tuple([Int, Int])` from the leaves.
                if let Some(ty) =
                    shape_infer_yield_expr(m, &vmap, yield_eid, &for_gen, self.interner)
                {
                    if ty != Type::Any {
                        return ty;
                    }
                }
                // Attribute access: yield v.field — only valid when the
                // for-loop target is a single Var leaf (tuple targets expose
                // multiple names, so this shortcut can't apply).
                if let hir::BindingTarget::Var(target_var) = for_gen.target {
                    let ye = &m.exprs[yield_eid];
                    if let hir::ExprKind::Attribute { obj, attr } = &ye.kind {
                        if let hir::ExprKind::Var(vid) = &m.exprs[*obj].kind {
                            if *vid == target_var {
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
            }
            if let Some(ty) = elem_ty {
                if ty != Type::Any {
                    return ty;
                }
            }
        }
        let vmap = VarTypeMap::build(m);
        let mut joined: Option<Type> = None;
        let mut saw_any = false;
        for yi in collect_yield_info(func, m) {
            let yield_ty = match yi.yield_value {
                Some(expr_id) => m.exprs[expr_id].ty.clone().unwrap_or_else(|| {
                    shape_infer_type(m, &vmap, expr_id, 0, self.interner).unwrap_or(Type::Any)
                }),
                None => Type::None,
            };
            if matches!(yield_ty, Type::Any) {
                saw_any = true;
                continue;
            }
            joined = Some(match joined {
                None => yield_ty,
                Some(prev) => Type::unify_field_type(&prev, &yield_ty),
            });
        }
        joined.unwrap_or(if saw_any { Type::Any } else { Type::None })
    }
}

/// Shape-infer a yield expression, tracking the for-loop's binding target so
/// that Var leaves that are tuple-target components (e.g. `v`, `i` in
/// `for i, v in ...`) resolve to the iter's element-type components.
fn shape_infer_yield_expr(
    m: &hir::Module,
    vmap: &VarTypeMap,
    yield_eid: hir::ExprId,
    for_gen: &super::ForLoopGenerator,
    interner: &StringInterner,
) -> Option<Type> {
    // Seed a secondary map where tuple-target Var leaves are resolved to the
    // corresponding element of the iter's computed tuple shape. This augments
    // the static VarTypeMap (which only knows param/Bind/ForBind-Var targets).
    let mut augmented = VarTypeMap {
        by_param: vmap.by_param.clone(),
        by_value: vmap.by_value.clone(),
        by_for_iter: vmap.by_for_iter.clone(),
    };

    // Compute the iter's element type once.
    let iter_elem = Some(iter_elem_type_with(m, vmap, for_gen, interner));
    if let Some(iter_elem) = &iter_elem {
        augment_target_var_types(&for_gen.target, iter_elem, &mut augmented);
    }

    shape_infer_type(m, &augmented, yield_eid, 0, interner)
}

fn augment_target_var_types(target: &hir::BindingTarget, ty: &Type, augmented: &mut VarTypeMap) {
    match (target, ty) {
        (hir::BindingTarget::Var(vid), _) => {
            augmented.by_param.insert(*vid, ty.clone());
        }
        (hir::BindingTarget::Tuple { elts, .. }, Type::Tuple(elem_tys)) => {
            for (elt, elem_ty) in elts.iter().zip(elem_tys.iter()) {
                augment_target_var_types(elt, elem_ty, augmented);
            }
        }
        (hir::BindingTarget::Starred { inner, .. }, Type::List(elem_ty))
        | (hir::BindingTarget::Starred { inner, .. }, Type::TupleVar(elem_ty))
        | (hir::BindingTarget::Starred { inner, .. }, Type::Iterator(elem_ty)) => {
            augment_target_var_types(inner, &Type::List(elem_ty.clone()), augmented);
        }
        _ => {}
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
    iterator_type: &Type,
    span: Span,
) -> Vec<GenStmt> {
    let mut stmts = Vec::new();
    let creator_gen_var = VarId(GEN_VAR_ID_OFFSET + 30000 + func.id.0);

    // gen_obj = GeneratorIntrinsic::Create { func_id, num_locals }
    let create = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::Create {
            func_id: func.id.0,
            num_locals,
        }),
        Some(iterator_type.clone()),
        span,
    );
    stmts.push(mk_leaf_stmt(
        m,
        hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(creator_gen_var),
            value: create,
            type_hint: Some(iterator_type.clone()),
        },
        span,
    ));

    // Save each parameter to generator locals
    for gv in gen_vars {
        if gv.is_param {
            let pv = mk_var(m, gv.var_id, gv.ty.clone(), span);
            let set = mk_set_local(m, creator_gen_var, gv.gen_local_idx, pv, span);
            stmts.push(mk_leaf_stmt(m, hir::StmtKind::Expr(set), span));
            // §F.7b: No SetLocalType needed — SetLocal now boxes primitives via
            // box_primitive_if_needed so GC walks locals via Value::is_ptr().
        }
    }

    // Initialize constant assignments before first yield
    'scan_consts: for block in func.blocks.values() {
        for stmt_id in &block.stmts {
            let stmt = m.stmts[*stmt_id].clone();
            // Only scalar Var bindings are candidates for constant initialization;
            // tuple-pattern bindings (e.g., `a, b = 1, 2`) are not constants.
            let var_assign = match &stmt.kind {
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    value,
                    ..
                } => Some((*target_var, *value)),
                _ => None,
            };
            if let Some((target, value)) = var_assign {
                let ve = m.exprs[value].clone();
                let is_const = matches!(
                    ve.kind,
                    hir::ExprKind::Int(_) | hir::ExprKind::Float(_) | hir::ExprKind::Bool(_)
                );
                if is_const {
                    if let Some(gv) = gen_vars.iter().find(|v| v.var_id == target) {
                        let val = clone_expr(m, value);
                        let set = mk_set_local(m, creator_gen_var, gv.gen_local_idx, val, span);
                        stmts.push(mk_leaf_stmt(m, hir::StmtKind::Expr(set), span));
                    }
                }
            } else if let hir::StmtKind::Expr(eid) = &stmt.kind {
                let e = &m.exprs[*eid];
                if matches!(e.kind, hir::ExprKind::Yield(_)) {
                    break 'scan_consts;
                }
            }
        }
    }

    // For for-loop generators, evaluate the iterable and create an iterator,
    // then store it at slot 0. We use `Builtin::Iter` to create the iterator;
    // the lowering's `lower_builtin_call` handles type dispatch (list → rt_iter_list, etc.).
    if let Some(for_gen) = detect_for_loop_generator(func, m) {
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
        stmts.push(mk_leaf_stmt(m, hir::StmtKind::Expr(set_iter), span));
        // §F.7b: No SetLocalType needed — SetLocal boxes the iterator pointer
        // via box_primitive_if_needed; GC follows it via Value::is_ptr().
    }

    // return gen_obj
    let ret_val = mk_var(
        m,
        creator_gen_var,
        Type::Iterator(Box::new(Type::Any)),
        span,
    );
    stmts.push(mk_leaf_stmt(m, hir::StmtKind::Return(Some(ret_val)), span));

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
) -> Vec<GenStmt> {
    let yield_infos = collect_yield_info(func, m);
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
                body.push(mk_leaf_stmt(
                    m,
                    hir::StmtKind::Bind {
                        target: hir::BindingTarget::Var(target),
                        value: get_sent,
                        type_hint: Some(Type::Int),
                    },
                    span,
                ));
                // Also save sent value to the generator local for persistence
                if let Some(gv) = gen_vars.iter().find(|v| v.var_id == target) {
                    let tv = mk_var(m, target, Type::Int, span);
                    let set = mk_set_local(m, gen_obj_var, gv.gen_local_idx, tv, span);
                    body.push(mk_leaf_stmt(m, hir::StmtKind::Expr(set), span));
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
        body.push(mk_leaf_stmt(m, hir::StmtKind::Expr(set_state), span));

        // 6. Return yield value
        body.push(mk_leaf_stmt(
            m,
            hir::StmtKind::Return(Some(yield_value)),
            span,
        ));

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
) -> Vec<GenStmt> {
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
) -> Vec<GenStmt> {
    let mut body = Vec::new();

    // Load parameters
    for gv in gen_vars {
        if gv.is_param {
            let get = mk_get_local(m, gen_obj_var, gv.gen_local_idx, gv.ty.clone(), span);
            body.push(mk_leaf_stmt(
                m,
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(gv.var_id),
                    value: get,
                    type_hint: Some(gv.ty.clone()),
                },
                span,
            ));
        }
    }

    // Execute init statements (reuse original HIR)
    body.extend(wrap_stmt_ids(&wg.init_stmts));

    // Save only the variables that are actually defined at this point:
    // parameters (loaded above) and anything bound in init_stmts. Vars
    // that are only assigned inside later yield sections (e.g. `x`, `y`
    // in `while i < n: yield i; x = i*2; yield x; ...`) MUST NOT be
    // saved here — their HIR var is unbound in init, so the lowering
    // reads a LocalId that gets defined only in a later state block,
    // producing an SSA `UseNotDominated` violation. Skip them; the
    // generator state slot stays at Cranelift's zero default, and the
    // first state block that actually assigns the var saves it before
    // yielding.
    let mut defined_in_init: HashSet<VarId> = HashSet::new();
    for gv in gen_vars {
        if gv.is_param {
            defined_in_init.insert(gv.var_id);
        }
    }
    let init_stmts = wrap_stmt_ids(&wg.init_stmts);
    collect_defined_vars(&init_stmts, m, &mut defined_in_init);
    emit_save_vars_where(m, gen_vars, gen_obj_var, &mut body, span, |gv| {
        defined_in_init.contains(&gv.var_id)
    });

    // Check condition
    let yield_val = wg.yield_sections[0]
        .yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span));

    let set_state = mk_set_state(m, gen_obj_var, 1, span);
    let ss = mk_leaf_stmt(m, hir::StmtKind::Expr(set_state), span);
    let ret = mk_leaf_stmt(m, hir::StmtKind::Return(Some(yield_val)), span);

    let exhaust = mk_exhaust_block(m, gen_obj_var, span);
    let cond_check = GenStmt::If {
        cond: wg.cond,
        then_body: vec![ss, ret],
        else_body: exhaust,
        span,
    };
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
) -> Vec<GenStmt> {
    let mut body = Vec::new();

    // Load all variables
    emit_load_all_vars(m, gen_vars, gen_obj_var, &mut body, span);

    // Execute statements before this yield (reuse original HIR)
    body.extend(wrap_stmt_ids(&section.stmts_before));

    // Compute yield value
    let yield_val = section
        .yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span));

    // Save all variables
    emit_save_all_vars(m, gen_vars, gen_obj_var, &mut body, span);

    // Set next state
    let ss = mk_set_state(m, gen_obj_var, next_state, span);
    body.push(mk_leaf_stmt(m, hir::StmtKind::Expr(ss), span));

    // return yield_value
    body.push(mk_leaf_stmt(
        m,
        hir::StmtKind::Return(Some(yield_val)),
        span,
    ));

    body
}

fn build_while_update(
    m: &mut hir::Module,
    gen_vars: &[GeneratorVar],
    wg: &super::WhileLoopGenerator,
    gen_obj_var: VarId,
    num_yields: usize,
    span: Span,
) -> Vec<GenStmt> {
    let mut body = Vec::new();

    // Load all variables
    emit_load_all_vars(m, gen_vars, gen_obj_var, &mut body, span);

    // Execute update statements (reuse original HIR)
    body.extend(wrap_stmt_ids(&wg.update_stmts));

    // Save variables
    emit_save_all_vars(m, gen_vars, gen_obj_var, &mut body, span);

    // Re-check condition: if true → state=1 + yield first value; if false → exhaust
    let yield_val = wg.yield_sections[0]
        .yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span));

    let set_state = mk_set_state(m, gen_obj_var, 1, span);
    let ss = mk_leaf_stmt(m, hir::StmtKind::Expr(set_state), span);
    let ret = mk_leaf_stmt(m, hir::StmtKind::Return(Some(yield_val)), span);

    let exhaust = mk_exhaust_block(m, gen_obj_var, span);
    let _ = num_yields;

    let cond_check = GenStmt::If {
        cond: wg.cond,
        then_body: vec![ss, ret],
        else_body: exhaust,
        span,
    };
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
    interner: &StringInterner,
) -> Vec<GenStmt> {
    let num_trailing = fg.trailing_yields.len();
    let mut stmts = mk_resume_preamble(m, gen_obj_var, state_var, span);

    // if state == 0: set state = 1 (first call initialization)
    {
        let set_s1 = mk_set_state(m, gen_obj_var, 1, span);
        let ss = mk_leaf_stmt(m, hir::StmtKind::Expr(set_s1), span);

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
    // After §F.7c BigBang: carry the actual elem type through so IterNextNoExc
    // lowering picks the correct UnwrapValue path for typed Int/Bool.
    let elem_ty_for_iter = iter_elem_type(m, fg, interner);
    let iter_ty = Type::Iterator(Box::new(elem_ty_for_iter));
    let get_iter = mk_get_local(m, gen_obj_var, 0, iter_ty.clone(), span);
    stmts.push(mk_leaf_stmt(
        m,
        hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(iter_var),
            value: get_iter,
            type_hint: Some(iter_ty),
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
            interner,
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
            interner,
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
    stmts: &mut Vec<GenStmt>,
    interner: &StringInterner,
) {
    // Element type yielded per iteration. Required so `lower_binding_target`
    // picks the right unbox step (via `emit_tuple_get`) vs `RT_LIST_GET` when
    // the target is a Tuple. Falls back to `Any` for iterables whose element
    // type is only known after type planning.
    let elem_ty = iter_elem_type(m, fg, interner);

    // next_val = __iter_next_no_exc(iter)
    let ir = mk_var(m, iter_var, Type::Iterator(Box::new(elem_ty.clone())), span);
    let nv = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::IterNextNoExc(ir)),
        Some(elem_ty.clone()),
        span,
    );
    stmts.push(mk_leaf_stmt(
        m,
        hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(next_val_var),
            value: nv,
            type_hint: Some(elem_ty.clone()),
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
    stmts.push(mk_leaf_stmt(
        m,
        hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(iter_done_var),
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
    stmts.push(GenStmt::If {
        cond: done_ref,
        then_body: done_target,
        else_body: vec![],
        span,
    });

    // Assign loop target — may be Var, Tuple, Attr, Index, etc. The Bind
    // statement's handler lowers the full shape recursively. `elem_ty` flows
    // through `type_hint` so tuple-target unpack picks the right
    // `RT_TUPLE_GET_*` runtime call.
    let nvr = mk_var(m, next_val_var, elem_ty.clone(), span);
    stmts.push(mk_leaf_stmt(
        m,
        hir::StmtKind::Bind {
            target: fg.target.clone(),
            value: nvr,
            type_hint: Some(elem_ty),
        },
        span,
    ));

    // Save iterator back
    let ir3 = mk_var(m, iter_var, Type::Iterator(Box::new(Type::Any)), span);
    let save = mk_set_local(m, gen_obj_var, 0, ir3, span);
    stmts.push(mk_leaf_stmt(m, hir::StmtKind::Expr(save), span));

    // Compute and return yield value
    let yv = fg
        .yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::None, Some(Type::None), span));
    stmts.push(mk_leaf_stmt(m, hir::StmtKind::Return(Some(yv)), span));
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
    stmts: &mut Vec<GenStmt>,
    interner: &StringInterner,
) {
    let filter_cond_id = fg
        .filter_cond
        .expect("internal error: filter_cond is Some, guaranteed by caller's is_some() check");
    let true_expr = mk_expr(m, hir::ExprKind::Bool(true), Some(Type::Bool), span);

    // See `build_for_loop_direct` — propagate iter element type to the Bind
    // so recursive unpack picks the correct runtime call.
    let elem_ty = iter_elem_type(m, fg, interner);

    let mut loop_body = Vec::new();

    // next_val = __iter_next_no_exc(iter)
    let ir = mk_var(m, iter_var, Type::Iterator(Box::new(elem_ty.clone())), span);
    let nv = mk_expr(
        m,
        hir::ExprKind::GeneratorIntrinsic(hir::GeneratorIntrinsic::IterNextNoExc(ir)),
        Some(elem_ty.clone()),
        span,
    );
    loop_body.push(mk_leaf_stmt(
        m,
        hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(next_val_var),
            value: nv,
            type_hint: Some(elem_ty.clone()),
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
    loop_body.push(mk_leaf_stmt(
        m,
        hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(iter_done_var),
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
    loop_body.push(GenStmt::If {
        cond: done_ref,
        then_body: done_target,
        else_body: vec![],
        span,
    });

    // Assign loop target (recursive unpack for Tuple/Attr/Index shapes).
    let nvr = mk_var(m, next_val_var, elem_ty.clone(), span);
    loop_body.push(mk_leaf_stmt(
        m,
        hir::StmtKind::Bind {
            target: fg.target.clone(),
            value: nvr,
            type_hint: Some(elem_ty),
        },
        span,
    ));

    // if filter_cond: save iter, return yield value
    let mut yield_body = Vec::new();
    let ir3 = mk_var(m, iter_var, Type::Iterator(Box::new(Type::Any)), span);
    let save = mk_set_local(m, gen_obj_var, 0, ir3, span);
    yield_body.push(mk_leaf_stmt(m, hir::StmtKind::Expr(save), span));

    let yv = fg
        .yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::None, Some(Type::None), span));
    yield_body.push(mk_leaf_stmt(m, hir::StmtKind::Return(Some(yv)), span));

    loop_body.push(GenStmt::If {
        cond: filter_cond_id,
        then_body: yield_body,
        else_body: vec![],
        span,
    });

    // while True: [loop_body]
    stmts.push(GenStmt::While {
        cond: true_expr,
        body: loop_body,
        else_body: vec![],
        span,
    });
}

fn build_trailing_yield(
    m: &mut hir::Module,
    gen_obj_var: VarId,
    trailing_yield_expr: &Option<hir::ExprId>,
    trailing_idx: usize,
    _num_trailing: usize,
    span: Span,
) -> Vec<GenStmt> {
    let mut body = Vec::new();

    // Set state to next trailing yield
    let next_state = (trailing_idx + 2 + 1) as i64;
    let ss = mk_set_state(m, gen_obj_var, next_state, span);
    body.push(mk_leaf_stmt(m, hir::StmtKind::Expr(ss), span));

    // Return trailing yield value
    let value = trailing_yield_expr
        .map(|eid| clone_expr(m, eid))
        .unwrap_or_else(|| mk_expr(m, hir::ExprKind::Int(0), Some(Type::Int), span));
    body.push(mk_leaf_stmt(m, hir::StmtKind::Return(Some(value)), span));

    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_utils::{HirBlockId, StringInterner};

    #[test]
    fn desugaring_marks_resume_return_type_as_any() {
        let mut interner = StringInterner::default();
        let module_name = interner.intern("gen_test");
        let func_name = interner.intern("tuple_gen");
        let span = Span::dummy();

        let mut module = hir::Module::new(module_name);
        let one = module.exprs.alloc(hir::Expr {
            kind: hir::ExprKind::Int(1),
            ty: Some(Type::Int),
            span,
        });
        let two = module.exprs.alloc(hir::Expr {
            kind: hir::ExprKind::Int(2),
            ty: Some(Type::Int),
            span,
        });
        let tuple_expr = module.exprs.alloc(hir::Expr {
            kind: hir::ExprKind::Tuple(vec![one, two]),
            ty: Some(Type::Tuple(vec![Type::Int, Type::Int])),
            span,
        });
        let yield_expr = module.exprs.alloc(hir::Expr {
            kind: hir::ExprKind::Yield(Some(tuple_expr)),
            ty: Some(Type::Tuple(vec![Type::Int, Type::Int])),
            span,
        });
        let yield_stmt = module.stmts.alloc(hir::Stmt {
            kind: hir::StmtKind::Expr(yield_expr),
            span,
        });

        let func_id = FuncId::new(0);
        let entry_block = HirBlockId::new(0);
        module.func_defs.insert(
            func_id,
            hir::Function {
                id: func_id,
                name: func_name,
                params: vec![],
                return_type: None,
                span,
                cell_vars: HashSet::new(),
                nonlocal_vars: HashSet::new(),
                is_generator: true,
                method_kind: hir::MethodKind::default(),
                is_abstract: false,
                blocks: indexmap::indexmap! {
                    entry_block => hir::HirBlock {
                        id: entry_block,
                        stmts: vec![yield_stmt],
                        terminator: hir::HirTerminator::Return(None),
                        loop_depth: 0,
                        handler_depth: 0,
                    }
                },
                entry_block,
                try_scopes: vec![],
            },
        );
        module.functions.push(func_id);

        let mut lowering = Lowering::new(&mut interner);
        lowering.desugar_generators(&mut module).unwrap();

        let resume_func_id = FuncId(func_id.0 + RESUME_FUNC_ID_OFFSET);
        let resume_func = module
            .func_defs
            .get(&resume_func_id)
            .expect("resume function should be created during desugaring");
        assert_eq!(resume_func.return_type, Some(Type::Any));
    }
}
