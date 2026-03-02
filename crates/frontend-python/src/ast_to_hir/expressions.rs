use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_stdlib_defs::{self as stdlib, StdlibItem as RegistryItem};
use pyaot_types::Type;
use pyaot_utils::{Span, VarId};
use rustpython_parser::ast as py;

impl AstToHir {
    /// Helper to convert call arguments, handling starred arguments (*args)
    pub(crate) fn convert_call_args(&mut self, args: Vec<py::Expr>) -> Result<Vec<CallArg>> {
        let mut call_args = Vec::new();
        for arg in args {
            match arg {
                py::Expr::Starred(starred) => {
                    // *args unpacking
                    let expr_id = self.convert_expr(*starred.value)?;
                    call_args.push(CallArg::Starred(expr_id));
                }
                other => {
                    // Regular positional argument
                    let expr_id = self.convert_expr(other)?;
                    call_args.push(CallArg::Regular(expr_id));
                }
            }
        }
        Ok(call_args)
    }

    pub(crate) fn convert_expr(&mut self, expr: py::Expr) -> Result<ExprId> {
        let expr_span = Self::span_from(&expr);
        let kind = match expr {
            py::Expr::Constant(c) => self.convert_constant(&c.value, expr_span)?,

            py::Expr::Name(name) => {
                let name_str = self.interner.intern(&name.id);

                // First check if it's a stdlib name (from X import Y)
                if let Some(stdlib_item) = self.stdlib_names.get(&name_str).cloned() {
                    match stdlib_item {
                        super::StdlibItem::Attr(attr) => ExprKind::StdlibAttr(attr),
                        super::StdlibItem::Func(_) => {
                            // Function references need to be handled at call site
                            // Return a placeholder - will be caught in Call handler
                            return Err(CompilerError::parse_error(
                                format!(
                                    "Stdlib function '{}' must be called, cannot be used as value",
                                    self.interner.resolve(name_str)
                                ),
                                expr_span,
                            ));
                        }
                        super::StdlibItem::Const(const_def) => {
                            // Compile-time constant - will be inlined as literal
                            ExprKind::StdlibConst(const_def)
                        }
                    }
                // Check if it's a variable
                } else if let Some(&var_id) = self.var_map.get(&name_str) {
                    ExprKind::Var(var_id)
                // Then check if it's a function reference (for passing functions as values)
                } else if let Some(&func_id) = self.func_map.get(&name_str) {
                    ExprKind::FuncRef(func_id)
                // Check if it's a class reference
                } else if let Some(&class_id) = self.class_map.get(&name_str) {
                    ExprKind::ClassRef(class_id)
                // Check if it's an imported name
                } else if let Some(imported) = self.imported_names.get(&name_str) {
                    match &imported.kind {
                        super::ImportedNameKind::Function(func_id) => ExprKind::FuncRef(*func_id),
                        super::ImportedNameKind::Class(class_id) => ExprKind::ClassRef(*class_id),
                        super::ImportedNameKind::Variable(var_id) => ExprKind::Var(*var_id),
                        super::ImportedNameKind::Unresolved => {
                            // Import not yet resolved - emit ImportedRef for later resolution
                            ExprKind::ImportedRef {
                                module: imported.module.clone(),
                                name: imported.original_name.clone(),
                            }
                        }
                    }
                // Check if it's a stdlib module (import sys, etc.)
                } else if self.stdlib_imports.contains(&name_str) {
                    return Err(CompilerError::parse_error(
                        format!(
                            "Module '{}' cannot be used as a value; use 'module.name' to access its members",
                            self.interner.resolve(name_str)
                        ),
                        expr_span,
                    ));
                // Check if it's an imported module (for module.attr access)
                } else if self.imported_modules.contains_key(&name_str) {
                    // This will be handled by Attribute access - for now, just return a placeholder
                    // that will be caught in the Attribute handler
                    return Err(CompilerError::parse_error(
                        format!(
                            "Module '{}' cannot be used as a value; use 'module.name' to access its members",
                            self.interner.resolve(name_str)
                        ),
                        expr_span,
                    ));
                // Check module-level variables (decorated functions, module-level assignments)
                } else if let Some(&var_id) = self.module_var_map.get(&name_str) {
                    ExprKind::Var(var_id)
                // Handle __name__ built-in - always "__main__" for direct script execution
                } else if name.id.as_str() == "__name__" {
                    ExprKind::Str(self.interner.intern("__main__"))
                // Check if it's a first-class builtin function (len, str, int, etc.)
                // This must come AFTER checking local variables to allow shadowing
                } else if let Some(builtin_kind) = BuiltinFunctionKind::from_name(&name.id) {
                    ExprKind::BuiltinRef(builtin_kind)
                } else {
                    return Err(CompilerError::name_error(name.id.clone(), expr_span));
                }
            }

            py::Expr::BinOp(binop) => {
                let left = self.convert_expr(*binop.left)?;
                let right = self.convert_expr(*binop.right)?;
                let op = self.convert_binop(&binop.op, expr_span)?;
                ExprKind::BinOp { op, left, right }
            }

            py::Expr::UnaryOp(unop) => {
                let operand = self.convert_expr(*unop.operand)?;
                let op = self.convert_unop(&unop.op, expr_span)?;
                ExprKind::UnOp { op, operand }
            }

            py::Expr::Compare(cmp) => {
                // Python allows chained comparisons (a < b < c)
                if cmp.ops.len() == 1 && cmp.comparators.len() == 1 {
                    // Detect type(x) == <type_name> pattern before normal conversion.
                    // type() returns a string like "<class 'int'>", so we replace the
                    // type name with the corresponding string constant for comparison.
                    if matches!(cmp.ops[0], py::CmpOp::Eq | py::CmpOp::NotEq) {
                        // type(x) == NAME
                        let fwd = Self::detect_type_comparison(&cmp.left, &cmp.comparators[0]);
                        if let Some(class_str) = fwd {
                            let left = self.convert_expr(*cmp.left)?;
                            let op = self.convert_cmpop(&cmp.ops[0], expr_span)?;
                            let right = self.module.exprs.alloc(Expr {
                                kind: ExprKind::Str(self.interner.intern(class_str)),
                                ty: Some(Type::Str),
                                span: expr_span,
                            });
                            return Ok(self.module.exprs.alloc(Expr {
                                kind: ExprKind::Compare { left, op, right },
                                ty: Some(Type::Bool),
                                span: expr_span,
                            }));
                        }
                        // NAME == type(x) (reversed)
                        let rev = Self::detect_type_comparison(&cmp.comparators[0], &cmp.left);
                        if let Some(class_str) = rev {
                            let comparator = cmp
                                .comparators
                                .into_iter()
                                .next()
                                .expect("comparison must have at least one comparator");
                            let left = self.convert_expr(comparator)?;
                            let op = self.convert_cmpop(&cmp.ops[0], expr_span)?;
                            let right = self.module.exprs.alloc(Expr {
                                kind: ExprKind::Str(self.interner.intern(class_str)),
                                ty: Some(Type::Str),
                                span: expr_span,
                            });
                            return Ok(self.module.exprs.alloc(Expr {
                                kind: ExprKind::Compare { left, op, right },
                                ty: Some(Type::Bool),
                                span: expr_span,
                            }));
                        }
                    }

                    // Simple binary comparison
                    let left = self.convert_expr(*cmp.left)?;
                    let right = self.convert_expr(
                        cmp.comparators
                            .into_iter()
                            .next()
                            .expect("comparison must have at least one comparator"),
                    )?;
                    let op = self.convert_cmpop(&cmp.ops[0], expr_span)?;

                    ExprKind::Compare { left, op, right }
                } else {
                    // Chained comparison: desugar to AND chain
                    // a < b < c  =>  (a < b) and (b < c)
                    // Middle operands need temp vars if they have side effects
                    return self.desugar_chained_comparison(cmp, expr_span);
                }
            }

            py::Expr::Call(call) => {
                // Convert keyword arguments first (they need to be converted before being moved)
                let (kwargs, kwargs_unpack) = self.convert_keywords(call.keywords.clone())?;

                // Check for stdlib function calls first
                // Handle direct calls from "from X import Y" style
                if let py::Expr::Name(name) = &*call.func {
                    let name_str = self.interner.intern(&name.id);
                    if let Some(super::StdlibItem::Func(stdlib_func)) =
                        self.stdlib_names.get(&name_str).cloned()
                    {
                        // Intercept functools.reduce (from functools import reduce)
                        // reduce takes a callable arg, can't be a StdlibCall
                        if stdlib_func.runtime_name == "rt_reduce" {
                            let mut hir_args = Vec::new();
                            for arg in call.args.clone() {
                                hir_args.push(self.convert_expr(arg)?);
                            }
                            return Ok(self.module.exprs.alloc(Expr {
                                kind: ExprKind::BuiltinCall {
                                    builtin: Builtin::Reduce,
                                    args: hir_args,
                                    kwargs,
                                },
                                ty: None,
                                span: expr_span,
                            }));
                        }

                        // Intercept itertools.chain/islice (from itertools import chain/islice)
                        if stdlib_func.runtime_name == "rt_chain_new" {
                            let mut hir_args = Vec::new();
                            for arg in call.args.clone() {
                                hir_args.push(self.convert_expr(arg)?);
                            }
                            return Ok(self.module.exprs.alloc(Expr {
                                kind: ExprKind::BuiltinCall {
                                    builtin: Builtin::Chain,
                                    args: hir_args,
                                    kwargs,
                                },
                                ty: None,
                                span: expr_span,
                            }));
                        }
                        if stdlib_func.runtime_name == "rt_islice_new" {
                            let mut hir_args = Vec::new();
                            for arg in call.args.clone() {
                                hir_args.push(self.convert_expr(arg)?);
                            }
                            return Ok(self.module.exprs.alloc(Expr {
                                kind: ExprKind::BuiltinCall {
                                    builtin: Builtin::ISlice,
                                    args: hir_args,
                                    kwargs,
                                },
                                ty: None,
                                span: expr_span,
                            }));
                        }

                        let mut args = Vec::new();
                        for arg in call.args.clone() {
                            args.push(self.convert_expr(arg)?);
                        }
                        return Ok(self.module.exprs.alloc(Expr {
                            kind: ExprKind::StdlibCall {
                                func: stdlib_func,
                                args,
                            },
                            ty: None,
                            span: expr_span,
                        }));
                    }
                }

                // Check for super().method(args) pattern
                if let py::Expr::Attribute(attr) = &*call.func {
                    // Check if this is super().method(...)
                    if let py::Expr::Call(super_call) = &*attr.value {
                        if let py::Expr::Name(name) = &*super_call.func {
                            if name.id.as_str() == "super" {
                                // This is super().method(args)
                                let method = self.interner.intern(&attr.attr);
                                let mut args = Vec::new();
                                for arg in call.args {
                                    args.push(self.convert_expr(arg)?);
                                }
                                return Ok(self.module.exprs.alloc(Expr {
                                    kind: ExprKind::SuperCall { method, args },
                                    ty: None,
                                    span: expr_span,
                                }));
                            }
                        }
                    }

                    // Check for os.path.join(...) pattern
                    if let py::Expr::Attribute(outer_attr) = &*attr.value {
                        if let py::Expr::Name(module_name) = &*outer_attr.value {
                            let module_str = self.interner.intern(&module_name.id);
                            if self.stdlib_imports.contains(&module_str) {
                                let module = self.interner.resolve(module_str);
                                if module == "os"
                                    && outer_attr.attr.as_str() == "path"
                                    && attr.attr.as_str() == "join"
                                {
                                    // This is os.path.join(...)
                                    let mut args = Vec::new();
                                    for arg in call.args.clone() {
                                        args.push(self.convert_expr(arg)?);
                                    }
                                    return Ok(self.module.exprs.alloc(Expr {
                                        kind: ExprKind::StdlibCall {
                                            func: &pyaot_stdlib_defs::modules::os::OS_PATH_JOIN,
                                            args,
                                        },
                                        ty: None,
                                        span: expr_span,
                                    }));
                                }

                                if module == "os"
                                    && outer_attr.attr.as_str() == "path"
                                    && attr.attr.as_str() == "exists"
                                {
                                    // This is os.path.exists(...)
                                    let mut args = Vec::new();
                                    for arg in call.args.clone() {
                                        args.push(self.convert_expr(arg)?);
                                    }
                                    return Ok(self.module.exprs.alloc(Expr {
                                        kind: ExprKind::StdlibCall {
                                            func: &pyaot_stdlib_defs::modules::os::OS_PATH_EXISTS,
                                            args,
                                        },
                                        ty: None,
                                        span: expr_span,
                                    }));
                                }
                            }
                        }
                    }

                    // Check for stdlib module.func(...) pattern (e.g., sys.exit(), re.search())
                    if let py::Expr::Name(module_name) = &*attr.value {
                        let module_str = self.interner.intern(&module_name.id);
                        if self.stdlib_imports.contains(&module_str) {
                            let module = self.interner.resolve(module_str);
                            let func_name = attr.attr.as_str();

                            // Intercept functools.reduce -> Builtin::Reduce
                            // (reduce takes a callable arg, can't be a StdlibCall)
                            if module == "functools" && func_name == "reduce" {
                                let mut hir_args = Vec::new();
                                for arg in call.args.clone() {
                                    hir_args.push(self.convert_expr(arg)?);
                                }
                                return Ok(self.module.exprs.alloc(Expr {
                                    kind: ExprKind::BuiltinCall {
                                        builtin: Builtin::Reduce,
                                        args: hir_args,
                                        kwargs,
                                    },
                                    ty: None,
                                    span: expr_span,
                                }));
                            }

                            // Intercept itertools.chain/islice -> Builtin::Chain/ISlice
                            if module == "itertools" {
                                let builtin = match func_name {
                                    "chain" => Some(Builtin::Chain),
                                    "islice" => Some(Builtin::ISlice),
                                    _ => None,
                                };
                                if let Some(builtin) = builtin {
                                    let mut hir_args = Vec::new();
                                    for arg in call.args.clone() {
                                        hir_args.push(self.convert_expr(arg)?);
                                    }
                                    return Ok(self.module.exprs.alloc(Expr {
                                        kind: ExprKind::BuiltinCall {
                                            builtin,
                                            args: hir_args,
                                            kwargs,
                                        },
                                        ty: None,
                                        span: expr_span,
                                    }));
                                }
                            }

                            // Use registry to check if this is a valid stdlib function
                            if let Some(RegistryItem::Function(func_def)) =
                                stdlib::get_item(module, func_name)
                            {
                                // Map positional and keyword args to parameter slots
                                let mut arg_slots: Vec<Option<ExprId>> =
                                    vec![None; func_def.params.len()];

                                // Fill from positional args
                                for (i, arg) in call.args.iter().enumerate() {
                                    if i < func_def.params.len() {
                                        arg_slots[i] = Some(self.convert_expr((*arg).clone())?);
                                    }
                                }

                                // Map keyword args to positions by matching param names
                                for kw in &call.keywords {
                                    if let Some(ref kw_name) = kw.arg {
                                        if let Some(pos) = func_def
                                            .params
                                            .iter()
                                            .position(|p| p.name == kw_name.as_str())
                                        {
                                            arg_slots[pos] =
                                                Some(self.convert_expr(kw.value.clone())?);
                                        }
                                    }
                                }

                                // Collect contiguous args (lowering handles trailing defaults)
                                let mut args = Vec::new();
                                for slot in &arg_slots {
                                    if let Some(expr_id) = slot {
                                        args.push(*expr_id);
                                    } else {
                                        break;
                                    }
                                }

                                return Ok(self.module.exprs.alloc(Expr {
                                    kind: ExprKind::StdlibCall {
                                        func: func_def,
                                        args,
                                    },
                                    ty: None,
                                    span: expr_span,
                                }));
                            }
                        }
                    }
                }

                // Check if this is a method call (obj.method(...))
                if let py::Expr::Attribute(attr) = &*call.func {
                    // Check for chained package access: pkg.sub.func()
                    // This handles `import pkg.sub` then calling `pkg.sub.func()`
                    if let Some(module_path) =
                        self.try_resolve_chained_module_path(&attr.value, &attr.attr)
                    {
                        let attr_name = self.interner.intern(&attr.attr);
                        let module_attr_expr = self.module.exprs.alloc(Expr {
                            kind: ExprKind::ModuleAttr {
                                module: module_path,
                                attr: attr_name,
                            },
                            ty: None,
                            span: expr_span,
                        });

                        let args = self.convert_call_args(call.args)?;

                        return Ok(self.module.exprs.alloc(Expr {
                            kind: ExprKind::Call {
                                func: module_attr_expr,
                                args,
                                kwargs,
                                kwargs_unpack,
                            },
                            ty: None,
                            span: expr_span,
                        }));
                    }

                    // Check if this is module.func() - calling a function from an imported module
                    if let py::Expr::Name(name) = &*attr.value {
                        let name_str = self.interner.intern(&name.id);
                        if let Some(module_path) = self.imported_modules.get(&name_str).cloned() {
                            // This is module.func(args) - emit Call with ModuleAttr as func
                            let attr_name = self.interner.intern(&attr.attr);
                            let module_attr_expr = self.module.exprs.alloc(Expr {
                                kind: ExprKind::ModuleAttr {
                                    module: module_path,
                                    attr: attr_name,
                                },
                                ty: None,
                                span: expr_span,
                            });

                            let args = self.convert_call_args(call.args)?;

                            return Ok(self.module.exprs.alloc(Expr {
                                kind: ExprKind::Call {
                                    func: module_attr_expr,
                                    args,
                                    kwargs,
                                    kwargs_unpack,
                                },
                                ty: None,
                                span: expr_span,
                            }));
                        }
                    }

                    // Special handling for str.format() on string literals
                    if attr.attr.as_str() == "format" {
                        if let py::Expr::Constant(c) = &*attr.value {
                            if let py::Constant::Str(format_str) = &c.value {
                                // Desugar "pattern".format(args, kwargs) to string concatenation
                                let mut format_args = Vec::new();
                                for arg in call.args.clone() {
                                    format_args.push(self.convert_expr(arg)?);
                                }

                                // Collect keyword arguments for named placeholders
                                let mut format_kwargs = Vec::new();
                                for kw in call.keywords.clone() {
                                    if let Some(arg_name) = kw.arg {
                                        let name = self.interner.intern(&arg_name);
                                        let value = self.convert_expr(kw.value)?;
                                        format_kwargs.push((name, value));
                                    }
                                }

                                return self.desugar_format_string(
                                    format_str,
                                    &format_args,
                                    &format_kwargs,
                                    expr_span,
                                );
                            }
                        }
                    }

                    let obj = self.convert_expr(*attr.value.clone())?;
                    let method = self.interner.intern(&attr.attr);

                    let mut args = Vec::new();
                    for arg in call.args {
                        args.push(self.convert_expr(arg)?);
                    }

                    // Extract keyword arguments for method calls (e.g., list.sort(reverse=True))
                    let method_kwargs = self.convert_method_keywords(call.keywords)?;

                    return Ok(self.module.exprs.alloc(Expr {
                        kind: ExprKind::MethodCall {
                            obj,
                            method,
                            args,
                            kwargs: method_kwargs,
                        },
                        ty: None, // Type will be inferred based on method
                        span: expr_span,
                    }));
                }

                // Check if the func is a simple Name
                if let py::Expr::Name(name) = &*call.func {
                    let name_str = self.interner.intern(&name.id);

                    // Check for built-in functions first
                    if let Some(builtin_expr) = self.handle_builtin_call(
                        &name.id,
                        call.clone(),
                        kwargs.clone(),
                        kwargs_unpack,
                        expr_span,
                    )? {
                        return Ok(builtin_expr);
                    }

                    // Check if it's a class instantiation
                    if let Some(&class_id) = self.class_map.get(&name_str) {
                        // Create ClassRef expression for class instantiation
                        let class_ref_expr = Expr {
                            kind: ExprKind::ClassRef(class_id),
                            ty: None,
                            span: expr_span,
                        };
                        let func = self.module.exprs.alloc(class_ref_expr);

                        let args = self.convert_call_args(call.args)?;
                        ExprKind::Call {
                            func,
                            args,
                            kwargs,
                            kwargs_unpack,
                        }
                    // Check if it's a user-defined function reference
                    } else if let Some(&func_id) = self.func_map.get(&name_str) {
                        // Create FuncRef expression
                        let func_ref_expr = Expr {
                            kind: ExprKind::FuncRef(func_id),
                            ty: None, // Function types not yet implemented
                            span: expr_span,
                        };
                        let func = self.module.exprs.alloc(func_ref_expr);

                        let args = self.convert_call_args(call.args)?;
                        ExprKind::Call {
                            func,
                            args,
                            kwargs,
                            kwargs_unpack,
                        }
                    // Check if it's an imported name
                    } else if let Some(imported) = self.imported_names.get(&name_str).cloned() {
                        let func_expr = match imported.kind {
                            super::ImportedNameKind::Function(func_id) => {
                                self.module.exprs.alloc(Expr {
                                    kind: ExprKind::FuncRef(func_id),
                                    ty: None,
                                    span: expr_span,
                                })
                            }
                            super::ImportedNameKind::Class(class_id) => {
                                self.module.exprs.alloc(Expr {
                                    kind: ExprKind::ClassRef(class_id),
                                    ty: None,
                                    span: expr_span,
                                })
                            }
                            super::ImportedNameKind::Variable(_) => {
                                // Calling a variable - convert normally
                                self.convert_expr(*call.func)?
                            }
                            super::ImportedNameKind::Unresolved => {
                                // Import not yet resolved - emit ImportedRef
                                self.module.exprs.alloc(Expr {
                                    kind: ExprKind::ImportedRef {
                                        module: imported.module.clone(),
                                        name: imported.original_name.clone(),
                                    },
                                    ty: None,
                                    span: expr_span,
                                })
                            }
                        };

                        let args = self.convert_call_args(call.args)?;
                        ExprKind::Call {
                            func: func_expr,
                            args,
                            kwargs,
                            kwargs_unpack,
                        }
                    } else {
                        // Not a class or function, convert as normal expression
                        let func = self.convert_expr(*call.func)?;

                        let args = self.convert_call_args(call.args)?;
                        ExprKind::Call {
                            func,
                            args,
                            kwargs,
                            kwargs_unpack,
                        }
                    }
                } else {
                    // Complex expression, convert as normal
                    let func = self.convert_expr(*call.func)?;
                    let args = self.convert_call_args(call.args)?;
                    ExprKind::Call {
                        func,
                        args,
                        kwargs,
                        kwargs_unpack,
                    }
                }
            }

            py::Expr::IfExp(if_exp) => {
                let cond = self.convert_expr(*if_exp.test)?;
                let then_val = self.convert_expr(*if_exp.body)?;
                let else_val = self.convert_expr(*if_exp.orelse)?;
                ExprKind::IfExpr {
                    cond,
                    then_val,
                    else_val,
                }
            }

            py::Expr::List(list) => {
                let mut elements = Vec::new();
                for elem in list.elts {
                    elements.push(self.convert_expr(elem)?);
                }
                ExprKind::List(elements)
            }

            py::Expr::Tuple(tuple) => {
                let mut elements = Vec::new();
                for elem in tuple.elts {
                    elements.push(self.convert_expr(elem)?);
                }
                ExprKind::Tuple(elements)
            }

            py::Expr::Dict(dict) => {
                let has_unpacking = dict.keys.iter().any(|k| k.is_none());

                if !has_unpacking {
                    // Fast path: no unpacking, convert directly
                    let mut pairs = Vec::new();
                    for (key, value) in dict.keys.into_iter().zip(dict.values.into_iter()) {
                        let key_expr =
                            self.convert_expr(key.expect("checked: no unpacking in fast path"))?;
                        let value_expr = self.convert_expr(value)?;
                        pairs.push((key_expr, value_expr));
                    }
                    ExprKind::Dict(pairs)
                } else {
                    // Desugar dict unpacking:
                    // {"a": 1, **d1, "b": 2, **d2} becomes:
                    //   __dict_N = {"a": 1}      (leading regular pairs)
                    //   __dict_N.update(d1)
                    //   __dict_N["b"] = 2
                    //   __dict_N.update(d2)
                    //   result: __dict_N

                    // 1. Generate unique temp var
                    let temp_name = format!("__dict_{}", self.next_comp_id);
                    self.next_comp_id += 1;
                    let temp_var_id = self.alloc_var_id();
                    let temp_interned = self.interner.intern(&temp_name);
                    self.var_map.insert(temp_interned, temp_var_id);

                    // 2. Collect leading regular pairs for the initial dict
                    let mut init_pairs = Vec::new();
                    let mut items: Vec<(Option<py::Expr>, py::Expr)> =
                        dict.keys.into_iter().zip(dict.values).collect();
                    let mut start_idx = 0;
                    for (key, value) in &items {
                        if key.is_some() {
                            let key_expr = self.convert_expr(key.clone().unwrap())?;
                            let value_expr = self.convert_expr(value.clone())?;
                            init_pairs.push((key_expr, value_expr));
                            start_idx += 1;
                        } else {
                            break;
                        }
                    }

                    // 3. Create init: __dict_N = {leading pairs...}
                    let init_dict = self.module.exprs.alloc(Expr {
                        kind: ExprKind::Dict(init_pairs),
                        ty: None,
                        span: expr_span,
                    });
                    let init_stmt = self.module.stmts.alloc(Stmt {
                        kind: StmtKind::Assign {
                            target: temp_var_id,
                            value: init_dict,
                            type_hint: None,
                        },
                        span: expr_span,
                    });
                    self.pending_stmts.push(init_stmt);

                    // 4. Process remaining items
                    let remaining = items.split_off(start_idx);
                    let update_str = self.interner.intern("update");
                    for (key, value) in remaining {
                        let dict_ref = self.module.exprs.alloc(Expr {
                            kind: ExprKind::Var(temp_var_id),
                            ty: None,
                            span: expr_span,
                        });

                        if let Some(k) = key {
                            // Regular pair: __dict_N[key] = value
                            let key_expr = self.convert_expr(k)?;
                            let value_expr = self.convert_expr(value)?;
                            let assign_stmt = self.module.stmts.alloc(Stmt {
                                kind: StmtKind::IndexAssign {
                                    obj: dict_ref,
                                    index: key_expr,
                                    value: value_expr,
                                },
                                span: expr_span,
                            });
                            self.pending_stmts.push(assign_stmt);
                        } else {
                            // Unpacking: __dict_N.update(value)
                            let value_expr = self.convert_expr(value)?;
                            let call_expr = self.module.exprs.alloc(Expr {
                                kind: ExprKind::MethodCall {
                                    obj: dict_ref,
                                    method: update_str,
                                    args: vec![value_expr],
                                    kwargs: vec![],
                                },
                                ty: None,
                                span: expr_span,
                            });
                            let call_stmt = self.module.stmts.alloc(Stmt {
                                kind: StmtKind::Expr(call_expr),
                                span: expr_span,
                            });
                            self.pending_stmts.push(call_stmt);
                        }
                    }

                    // 5. Return reference to temp variable
                    ExprKind::Var(temp_var_id)
                }
            }

            py::Expr::Set(set_expr) => {
                let mut elements = Vec::new();
                for elem in set_expr.elts {
                    elements.push(self.convert_expr(elem)?);
                }
                ExprKind::Set(elements)
            }

            py::Expr::Subscript(sub) => {
                let obj = self.convert_expr(*sub.value)?;

                // Check if it's a slice or a simple index
                match *sub.slice {
                    py::Expr::Slice(slice) => {
                        let start = if let Some(lower) = slice.lower {
                            Some(self.convert_expr(*lower)?)
                        } else {
                            None
                        };
                        let end = if let Some(upper) = slice.upper {
                            Some(self.convert_expr(*upper)?)
                        } else {
                            None
                        };
                        let step = if let Some(step_expr) = slice.step {
                            Some(self.convert_expr(*step_expr)?)
                        } else {
                            None
                        };
                        ExprKind::Slice {
                            obj,
                            start,
                            end,
                            step,
                        }
                    }
                    other => {
                        let index = self.convert_expr(other)?;
                        ExprKind::Index { obj, index }
                    }
                }
            }

            py::Expr::BoolOp(bool_op) => {
                // Convert boolean operations to nested binary ops
                if bool_op.values.len() < 2 {
                    return Err(CompilerError::parse_error(
                        "BoolOp must have at least 2 values",
                        expr_span,
                    ));
                }

                let op = match bool_op.op {
                    py::BoolOp::And => LogicalOp::And,
                    py::BoolOp::Or => LogicalOp::Or,
                };

                let mut iter = bool_op.values.into_iter();
                let first =
                    self.convert_expr(iter.next().expect("BoolOp must have at least two values"))?;
                let second =
                    self.convert_expr(iter.next().expect("BoolOp must have at least two values"))?;

                let mut result_id = self.module.exprs.alloc(Expr {
                    kind: ExprKind::LogicalOp {
                        op,
                        left: first,
                        right: second,
                    },
                    ty: None,
                    span: expr_span,
                });

                for val in iter {
                    let next_val = self.convert_expr(val)?;
                    result_id = self.module.exprs.alloc(Expr {
                        kind: ExprKind::LogicalOp {
                            op,
                            left: result_id,
                            right: next_val,
                        },
                        ty: None,
                        span: expr_span,
                    });
                }

                return Ok(result_id);
            }

            py::Expr::JoinedStr(joined) => {
                // F-string: desugar f"Hello {name}" to "Hello " + str(name)
                return self.desugar_fstring(&joined.values, expr_span);
            }

            py::Expr::FormattedValue(_) => {
                // FormattedValue should only appear inside JoinedStr
                return Err(CompilerError::parse_error(
                    "FormattedValue outside f-string",
                    expr_span,
                ));
            }

            py::Expr::Attribute(attr) => {
                // Check if this is stdlib module.attr, class.attr, or module.attr access
                if let py::Expr::Name(name) = &*attr.value {
                    let name_str = self.interner.intern(&name.id);

                    // Check if this is a class attribute access: ClassName.attr
                    if let Some(&class_id) = self.class_map.get(&name_str) {
                        let attr_name = self.interner.intern(&attr.attr);
                        return Ok(self.module.exprs.alloc(Expr {
                            kind: ExprKind::ClassAttrRef {
                                class_id,
                                attr: attr_name,
                            },
                            ty: None,
                            span: expr_span,
                        }));
                    }

                    // Handle stdlib module attribute access
                    if self.stdlib_imports.contains(&name_str) {
                        let module_name = self.interner.resolve(name_str);
                        let attr_name = attr.attr.as_str();

                        // Handle os.path as a submodule
                        if module_name == "os" && attr_name == "path" {
                            // This is accessing os.path - will be handled as os.path.join() etc.
                            // Return a placeholder that will be caught in Call handler
                            return Err(CompilerError::parse_error(
                                "os.path cannot be used as a value; use 'os.path.join()' etc.",
                                expr_span,
                            ));
                        }

                        // Use registry to determine what kind of item this is
                        match stdlib::get_item(module_name, attr_name) {
                            Some(RegistryItem::Attr(attr_def)) => {
                                // Pass definition reference (Single Source of Truth)
                                return Ok(self.module.exprs.alloc(Expr {
                                    kind: ExprKind::StdlibAttr(attr_def),
                                    ty: None,
                                    span: expr_span,
                                }));
                            }
                            Some(RegistryItem::Function(_)) => {
                                // Functions must be called, cannot be used as values
                                return Err(CompilerError::parse_error(
                                    format!(
                                        "{}.{} must be called, cannot be used as value",
                                        module_name, attr_name
                                    ),
                                    expr_span,
                                ));
                            }
                            Some(RegistryItem::Constant(const_def)) => {
                                // Pass definition reference (Single Source of Truth)
                                // Constants are inlined at compile time
                                return Ok(self.module.exprs.alloc(Expr {
                                    kind: ExprKind::StdlibConst(const_def),
                                    ty: None,
                                    span: expr_span,
                                }));
                            }
                            Some(RegistryItem::Class(_)) => {
                                // Classes cannot be used as values directly
                                return Err(CompilerError::parse_error(
                                    format!(
                                        "Stdlib class '{}.{}' cannot be used as value",
                                        module_name, attr_name
                                    ),
                                    expr_span,
                                ));
                            }
                            None => {
                                let available = stdlib::list_all_names(module_name);
                                return Err(CompilerError::parse_error(
                                    format!(
                                        "Unknown attribute '{}.{}'. Available: {}",
                                        module_name,
                                        attr_name,
                                        available.join(", ")
                                    ),
                                    expr_span,
                                ));
                            }
                        }
                    }

                    // Check if this is user module.attr access
                    if let Some(module_path) = self.imported_modules.get(&name_str).cloned() {
                        // This is a module attribute access: module.attr
                        let attr_name = self.interner.intern(&attr.attr);
                        return Ok(self.module.exprs.alloc(Expr {
                            kind: ExprKind::ModuleAttr {
                                module: module_path,
                                attr: attr_name,
                            },
                            ty: None,
                            span: expr_span,
                        }));
                    }
                }

                // Check for chained module access: pkg.sub.VAR
                // This handles `import pkg.sub` then accessing `pkg.sub.VAR`
                if let Some(module_path) =
                    self.try_resolve_chained_module_attr(&attr.value, &attr.attr)
                {
                    let attr_name = self.interner.intern(&attr.attr);
                    return Ok(self.module.exprs.alloc(Expr {
                        kind: ExprKind::ModuleAttr {
                            module: module_path,
                            attr: attr_name,
                        },
                        ty: None,
                        span: expr_span,
                    }));
                }

                // Field/attribute access: obj.field
                let obj = self.convert_expr(*attr.value)?;
                let attr_name = self.interner.intern(&attr.attr);
                ExprKind::Attribute {
                    obj,
                    attr: attr_name,
                }
            }

            py::Expr::Lambda(lambda) => {
                return self.convert_lambda(lambda);
            }

            py::Expr::ListComp(list_comp) => {
                return self.desugar_list_comprehension(list_comp);
            }

            py::Expr::DictComp(dict_comp) => {
                return self.desugar_dict_comprehension(dict_comp);
            }

            py::Expr::SetComp(set_comp) => {
                return self.desugar_set_comprehension(set_comp);
            }

            py::Expr::GeneratorExp(gen_exp) => {
                return self.desugar_generator_expression(gen_exp);
            }

            py::Expr::Yield(yield_expr) => {
                // Mark the current function as a generator
                self.current_func_is_generator = true;

                // Convert the yield value if present
                let value = if let Some(value_expr) = yield_expr.value {
                    Some(self.convert_expr(*value_expr)?)
                } else {
                    None
                };

                ExprKind::Yield(value)
            }

            py::Expr::YieldFrom(yield_from) => {
                // Mark the current function as a generator
                self.current_func_is_generator = true;

                // Desugar: yield from expr  →  for __v in expr: yield __v
                // (v1: no send/throw forwarding, result is None)

                // 1. Convert the iterable expression
                let iter_expr_id = self.convert_expr(*yield_from.value)?;

                // 2. Create a temp variable for the loop target
                let temp_var = self.alloc_var_id();

                // 3. Create yield expression: yield __v
                let var_ref = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Var(temp_var),
                    ty: None,
                    span: expr_span,
                });
                let yield_expr_id = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Yield(Some(var_ref)),
                    ty: None,
                    span: expr_span,
                });

                // 4. Wrap yield in an expression statement
                let yield_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Expr(yield_expr_id),
                    span: expr_span,
                });

                // 5. Create the for loop: for __v in expr: yield __v
                let for_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::For {
                        target: temp_var,
                        iter: iter_expr_id,
                        body: vec![yield_stmt],
                        else_block: vec![],
                    },
                    span: expr_span,
                });

                // 6. Push for loop as pending statement
                self.pending_stmts.push(for_stmt);

                // 7. The result of yield from is None (v1)
                ExprKind::None
            }

            py::Expr::NamedExpr(named) => {
                // Walrus operator: (target := value)
                // Desugar into: assignment + variable reference
                // 1. Convert the value expression
                let value_id = self.convert_expr(*named.value)?;

                // 2. Get or create the target variable
                let target_var = self.get_or_create_var_from_expr(&named.target)?;
                self.mark_var_initialized(&named.target);

                // 3. Emit assignment as pending statement
                let assign_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Assign {
                        target: target_var,
                        value: value_id,
                        type_hint: None,
                    },
                    span: expr_span,
                });
                self.pending_stmts.push(assign_stmt);

                // 4. Return variable reference as the expression value
                ExprKind::Var(target_var)
            }

            _ => {
                return Err(CompilerError::parse_error(
                    format!("Unsupported expression: {:?}", expr),
                    expr_span,
                ))
            }
        };

        // Infer types for literal expressions
        let ty = match &kind {
            ExprKind::Int(_) => Some(Type::Int),
            ExprKind::Float(_) => Some(Type::Float),
            ExprKind::Bool(_) => Some(Type::Bool),
            ExprKind::Str(_) => Some(Type::Str),
            ExprKind::Bytes(_) => Some(Type::Bytes),
            ExprKind::None => Some(Type::None),
            _ => None,
        };

        let expr_id = self.module.exprs.alloc(Expr {
            kind,
            ty,
            span: expr_span,
        });
        Ok(expr_id)
    }

    /// Try to resolve a chained attribute access like `pkg.sub` to a module path.
    /// This handles `import pkg.sub` then accessing `pkg.sub.func`.
    /// Returns Some(module_path) if this is a dotted import, None otherwise.
    fn try_resolve_chained_module_path(
        &self,
        expr: &py::Expr,
        _final_attr: &str,
    ) -> Option<String> {
        self.build_module_path_from_expr(expr)
    }

    /// Try to resolve a chained attribute access for variable access.
    /// This handles `import pkg.sub` then accessing `pkg.sub.VAR`.
    fn try_resolve_chained_module_attr(
        &self,
        expr: &py::Expr,
        _final_attr: &str,
    ) -> Option<String> {
        self.build_module_path_from_expr(expr)
    }

    /// Build a module path from a chained attribute expression.
    /// For `pkg.sub`, returns Some("pkg.sub") if it matches a dotted import.
    fn build_module_path_from_expr(&self, expr: &py::Expr) -> Option<String> {
        // Build the full dotted path from the expression
        let mut parts = Vec::new();
        let mut current = expr;

        loop {
            match current {
                py::Expr::Attribute(attr) => {
                    parts.push(attr.attr.as_str());
                    current = &attr.value;
                }
                py::Expr::Name(name) => {
                    parts.push(&name.id);
                    break;
                }
                _ => return None,
            }
        }

        // Reverse to get the path in order (root to leaf)
        parts.reverse();
        let full_path = parts.join(".");

        // Check if this full path matches a dotted import
        if self.dotted_imports.contains_key(&full_path) {
            return Some(full_path);
        }

        None
    }

    /// Detect `type(x) == <type_name>` pattern.
    /// Returns the type class string (e.g., `"<class 'tuple'>"`) if `type_call` is a
    /// `type(arg)` call and `type_name` is a known built-in type name.
    fn detect_type_comparison(type_call: &py::Expr, type_name: &py::Expr) -> Option<&'static str> {
        // Check if type_call is type(x)
        let py::Expr::Call(call) = type_call else {
            return None;
        };
        let py::Expr::Name(func_name) = &*call.func else {
            return None;
        };
        if func_name.id.as_str() != "type" || call.args.len() != 1 || !call.keywords.is_empty() {
            return None;
        }
        // Check if type_name is a known builtin type name
        let py::Expr::Name(name) = type_name else {
            return None;
        };
        match name.id.as_str() {
            "int" => Some("<class 'int'>"),
            "float" => Some("<class 'float'>"),
            "bool" => Some("<class 'bool'>"),
            "str" => Some("<class 'str'>"),
            "tuple" => Some("<class 'tuple'>"),
            "list" => Some("<class 'list'>"),
            "dict" => Some("<class 'dict'>"),
            "set" => Some("<class 'set'>"),
            "bytes" => Some("<class 'bytes'>"),
            _ => None,
        }
    }

    /// Desugar a chained comparison like `a < b < c` into `(a < b) and (b < c)`.
    /// Middle operands that may have side effects are stored in temp variables
    /// to ensure single evaluation.
    ///
    /// Python semantics:
    /// - `a < b < c` is equivalent to `(a < b) and (b < c)`
    /// - `b` is evaluated only once
    /// - Short-circuit evaluation applies (if `a < b` is false, `b < c` is not evaluated)
    fn desugar_chained_comparison(
        &mut self,
        cmp: py::ExprCompare,
        expr_span: Span,
    ) -> Result<ExprId> {
        // Build a list of all operands: [left, comparator0, comparator1, ...]
        // And the corresponding operators: [op0, op1, ...]
        let mut operands: Vec<py::Expr> = Vec::with_capacity(cmp.comparators.len() + 1);
        operands.push(*cmp.left);
        operands.extend(cmp.comparators);

        let ops = cmp.ops;

        // We need to create comparisons: (operands[i] op[i] operands[i+1]) for all i
        // For middle operands (indices 1..len-1), we need temp vars if they have side effects

        // First pass: convert all operands and create temp vars for middle ones that need them
        // middle_exprs[i] holds (expr_id, optional_temp_var_id) for operand i
        let mut converted_operands: Vec<(ExprId, Option<VarId>)> =
            Vec::with_capacity(operands.len());

        for (i, operand) in operands.iter().enumerate() {
            let expr_id = self.convert_expr(operand.clone())?;

            // Middle operands (not first or last) that have side effects need temp vars
            let is_middle = i > 0 && i < ops.len();
            if is_middle && self.expr_needs_temp_var(operand) {
                // Create temp variable for this middle operand
                let temp_name = format!("__chain_{}", self.next_comp_id);
                self.next_comp_id += 1;

                let temp_var_id = self.alloc_var_id();
                let temp_interned = self.interner.intern(&temp_name);
                self.var_map.insert(temp_interned, temp_var_id);

                // Create assignment: __chain_N = expr
                let assign_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Assign {
                        target: temp_var_id,
                        value: expr_id,
                        type_hint: None,
                    },
                    span: expr_span,
                });
                self.pending_stmts.push(assign_stmt);

                converted_operands.push((expr_id, Some(temp_var_id)));
            } else {
                converted_operands.push((expr_id, None));
            }
        }

        // Second pass: create comparisons and chain them with AND
        let mut comparisons: Vec<ExprId> = Vec::with_capacity(ops.len());

        for (i, op) in ops.iter().enumerate() {
            let hir_op = self.convert_cmpop(op, expr_span)?;

            // Left side: use temp var from previous operand if available, else the expr
            let left = if i > 0 {
                if let Some(temp_var) = converted_operands[i].1 {
                    // Use the temp var reference
                    self.module.exprs.alloc(Expr {
                        kind: ExprKind::Var(temp_var),
                        ty: None,
                        span: expr_span,
                    })
                } else {
                    converted_operands[i].0
                }
            } else {
                converted_operands[i].0
            };

            // Right side: use temp var if the operand has one (for middle operands),
            // but the actual temp var assignment uses the original expr
            // For the comparison, we use the temp var if present
            let right = if let Some(temp_var) = converted_operands[i + 1].1 {
                // Use temp var reference
                self.module.exprs.alloc(Expr {
                    kind: ExprKind::Var(temp_var),
                    ty: None,
                    span: expr_span,
                })
            } else {
                converted_operands[i + 1].0
            };

            // Create the comparison expression
            let cmp_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::Compare {
                    left,
                    op: hir_op,
                    right,
                },
                ty: Some(Type::Bool),
                span: expr_span,
            });

            comparisons.push(cmp_expr);
        }

        // Chain all comparisons with AND
        // (cmp0) and (cmp1) and (cmp2) ...
        let mut result = comparisons[0];
        for cmp_expr in comparisons.into_iter().skip(1) {
            result = self.module.exprs.alloc(Expr {
                kind: ExprKind::LogicalOp {
                    op: LogicalOp::And,
                    left: result,
                    right: cmp_expr,
                },
                ty: Some(Type::Bool),
                span: expr_span,
            });
        }

        Ok(result)
    }

    /// Check if an expression needs a temp variable to avoid multiple evaluation.
    /// Simple expressions like variables and literals don't need temps.
    fn expr_needs_temp_var(&self, expr: &py::Expr) -> bool {
        match expr {
            // Variables and constants are safe to evaluate multiple times
            py::Expr::Name(_) | py::Expr::Constant(_) => false,
            // Everything else might have side effects or be expensive
            _ => true,
        }
    }
}
