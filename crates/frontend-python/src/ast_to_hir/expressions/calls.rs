//! Function and method call handling: builtins, stdlib, user-defined, attribute calls.

use super::super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_stdlib_defs::{self as stdlib, StdlibItem as RegistryItem};
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    /// Convert a Call expression, dispatching to specialized handlers based on
    /// whether the callee is a Name (stdlib/builtin/user-defined) or an
    /// Attribute (method call, module.func, super().method, etc.).
    pub(crate) fn convert_call_expr(
        &mut self,
        call: py::ExprCall,
        expr_span: Span,
    ) -> Result<ExprId> {
        // Convert keyword arguments first (they need to be converted before being moved)
        let (kwargs, kwargs_unpack) = self.convert_keywords(call.keywords.clone())?;

        // Check for stdlib function calls first (from X import Y)
        if let py::Expr::Name(name) = &*call.func {
            let name_str = self.interner.intern(&name.id);
            if let Some(super::super::StdlibItem::Func(stdlib_func)) =
                self.imports.stdlib_names.get(&name_str).cloned()
            {
                return self.convert_stdlib_name_call(stdlib_func, &call, kwargs, expr_span);
            }
        }

        // Check for attribute-based calls (method calls, module.func, super().method, etc.)
        if let py::Expr::Attribute(_) = &*call.func {
            return self.convert_attribute_call(call, kwargs, kwargs_unpack, expr_span);
        }

        // Check for name-based calls (builtins, classes, user functions, imported names)
        if let py::Expr::Name(_) = &*call.func {
            return self.convert_name_call(call, kwargs, kwargs_unpack, expr_span);
        }

        // Complex expression call (e.g., closures, subscript results)
        let func = self.convert_expr(*call.func)?;
        let args = self.convert_call_args(call.args)?;
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::Call {
                func,
                args,
                kwargs,
                kwargs_unpack,
            },
            ty: None,
            span: expr_span,
        }))
    }

    /// Handle calls to stdlib functions imported via `from X import Y`.
    /// Intercepts special cases (reduce, chain, defaultdict, Counter, deque)
    /// and falls back to a generic StdlibCall.
    fn convert_stdlib_name_call(
        &mut self,
        stdlib_func: &'static pyaot_stdlib_defs::StdlibFunctionDef,
        call: &py::ExprCall,
        kwargs: Vec<KeywordArg>,
        expr_span: Span,
    ) -> Result<ExprId> {
        // Intercept functools.reduce (from functools import reduce)
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

        // Intercept collections.defaultdict -- convert factory name to tag
        if stdlib_func.runtime_name == "rt_make_defaultdict" {
            let mut hir_args = Vec::new();
            if !call.args.is_empty() {
                let factory_tag = self.resolve_defaultdict_factory(&call.args[0], expr_span)?;
                hir_args.push(self.module.exprs.alloc(Expr {
                    kind: ExprKind::Int(factory_tag),
                    ty: None,
                    span: expr_span,
                }));
            }
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::BuiltinCall {
                    builtin: Builtin::DefaultDict,
                    args: hir_args,
                    kwargs,
                },
                ty: None,
                span: expr_span,
            }));
        }

        // Intercept collections.Counter
        if stdlib_func.runtime_name == "rt_make_counter" {
            let mut hir_args = Vec::new();
            for arg in call.args.clone() {
                hir_args.push(self.convert_expr(arg)?);
            }
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::BuiltinCall {
                    builtin: Builtin::Counter,
                    args: hir_args,
                    kwargs,
                },
                ty: None,
                span: expr_span,
            }));
        }

        // Intercept collections.deque -- resolve maxlen kwarg
        if stdlib_func.runtime_name == "rt_make_deque" {
            let mut hir_args = Vec::new();
            for arg in call.args.clone() {
                hir_args.push(self.convert_expr(arg)?);
            }
            if hir_args.len() < 2 {
                for kw in &call.keywords {
                    if kw.arg.as_ref().map(|s| s.as_str()) == Some("maxlen") {
                        while hir_args.is_empty() {
                            hir_args.push(self.module.exprs.alloc(Expr {
                                kind: ExprKind::None,
                                ty: None,
                                span: expr_span,
                            }));
                        }
                        hir_args.push(self.convert_expr(kw.value.clone())?);
                        break;
                    }
                }
            }
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::BuiltinCall {
                    builtin: Builtin::Deque,
                    args: hir_args,
                    kwargs: vec![],
                },
                ty: None,
                span: expr_span,
            }));
        }

        // Generic stdlib call. Variadic functions (e.g. `os.path.join`)
        // keep the simple pass-through so their trailing `*args` param
        // receives every positional arg. Fixed-arity functions get the
        // richer slot-filling path so callers can mix positional and
        // keyword arguments (e.g. `Request(url, method="POST")` after
        // `from urllib.request import Request`).
        if stdlib_func.params.iter().any(|p| p.variadic) {
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
        let mut arg_slots: Vec<Option<ExprId>> = vec![None; stdlib_func.params.len()];
        for (i, arg) in call.args.iter().enumerate() {
            if i < stdlib_func.params.len() {
                arg_slots[i] = Some(self.convert_expr(arg.clone())?);
            }
        }
        for kw in &call.keywords {
            if let Some(ref kw_name) = kw.arg {
                if let Some(pos) = stdlib_func
                    .params
                    .iter()
                    .position(|p| p.name == kw_name.as_str())
                {
                    arg_slots[pos] = Some(self.convert_expr(kw.value.clone())?);
                }
            }
        }
        let last_filled = arg_slots.iter().rposition(|s| s.is_some());
        let collect_len = last_filled.map_or(0, |i| i + 1);
        let mut args = Vec::with_capacity(collect_len);
        for (i, slot) in arg_slots.iter().enumerate().take(collect_len) {
            if let Some(expr_id) = slot {
                args.push(*expr_id);
            } else {
                let param = &stdlib_func.params[i];
                let default_expr = if let Some(ref dv) = param.default {
                    match dv {
                        pyaot_stdlib_defs::ConstValue::Int(v) => ExprKind::Int(*v),
                        pyaot_stdlib_defs::ConstValue::Float(v) => ExprKind::Float(*v),
                        pyaot_stdlib_defs::ConstValue::Bool(v) => ExprKind::Bool(*v),
                        pyaot_stdlib_defs::ConstValue::Str(s) => {
                            ExprKind::Str(self.interner.intern(s))
                        }
                    }
                } else {
                    ExprKind::None
                };
                let filler = self.module.exprs.alloc(Expr {
                    kind: default_expr,
                    ty: None,
                    span: expr_span,
                });
                args.push(filler);
            }
        }
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::StdlibCall {
                func: stdlib_func,
                args,
            },
            ty: None,
            span: expr_span,
        }))
    }

    /// Handle calls where the callee is an attribute expression (`obj.method(...)`,
    /// `module.func(...)`, `super().method(...)`, `os.path.join(...)`, etc.).
    fn convert_attribute_call(
        &mut self,
        call: py::ExprCall,
        kwargs: Vec<KeywordArg>,
        kwargs_unpack: Option<ExprId>,
        expr_span: Span,
    ) -> Result<ExprId> {
        let attr = match &*call.func {
            py::Expr::Attribute(a) => a,
            _ => unreachable!("convert_attribute_call called with non-Attribute func"),
        };

        // Check if this is super().method(...)
        if let py::Expr::Call(super_call) = &*attr.value {
            if let py::Expr::Name(name) = &*super_call.func {
                if name.id.as_str() == "super" {
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

        // Check for os.path.join(...) / os.path.exists(...) pattern
        if let py::Expr::Attribute(outer_attr) = &*attr.value {
            if let py::Expr::Name(module_name) = &*outer_attr.value {
                let module_str = self.interner.intern(&module_name.id);
                if self.imports.stdlib_imports.contains(&module_str) {
                    let module = self.interner.resolve(module_str);
                    if module == "os"
                        && outer_attr.attr.as_str() == "path"
                        && attr.attr.as_str() == "join"
                    {
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
            if self.imports.stdlib_imports.contains(&module_str) {
                let module = self.interner.resolve(module_str);
                let func_name = attr.attr.as_str();

                // Intercept functools.reduce -> Builtin::Reduce
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

                // Intercept collections.defaultdict/Counter/deque
                if module == "collections" {
                    if func_name == "defaultdict" {
                        let mut hir_args = Vec::new();
                        if !call.args.is_empty() {
                            let factory_tag =
                                self.resolve_defaultdict_factory(&call.args[0], expr_span)?;
                            hir_args.push(self.module.exprs.alloc(Expr {
                                kind: ExprKind::Int(factory_tag),
                                ty: None,
                                span: expr_span,
                            }));
                        }
                        return Ok(self.module.exprs.alloc(Expr {
                            kind: ExprKind::BuiltinCall {
                                builtin: Builtin::DefaultDict,
                                args: hir_args,
                                kwargs,
                            },
                            ty: None,
                            span: expr_span,
                        }));
                    }
                    if func_name == "Counter" {
                        let mut hir_args = Vec::new();
                        for arg in call.args.clone() {
                            hir_args.push(self.convert_expr(arg)?);
                        }
                        return Ok(self.module.exprs.alloc(Expr {
                            kind: ExprKind::BuiltinCall {
                                builtin: Builtin::Counter,
                                args: hir_args,
                                kwargs,
                            },
                            ty: None,
                            span: expr_span,
                        }));
                    }
                    if func_name == "deque" {
                        let mut hir_args = Vec::new();
                        for arg in call.args.clone() {
                            hir_args.push(self.convert_expr(arg)?);
                        }
                        if hir_args.len() < 2 {
                            for kw in &call.keywords {
                                if kw.arg.as_ref().map(|s| s.as_str()) == Some("maxlen") {
                                    while hir_args.is_empty() {
                                        hir_args.push(self.module.exprs.alloc(Expr {
                                            kind: ExprKind::None,
                                            ty: None,
                                            span: expr_span,
                                        }));
                                    }
                                    hir_args.push(self.convert_expr(kw.value.clone())?);
                                    break;
                                }
                            }
                        }
                        return Ok(self.module.exprs.alloc(Expr {
                            kind: ExprKind::BuiltinCall {
                                builtin: Builtin::Deque,
                                args: hir_args,
                                kwargs: vec![],
                            },
                            ty: None,
                            span: expr_span,
                        }));
                    }
                }

                // Use registry to check if this is a valid stdlib function,
                // falling back to the package registry so registered
                // third-party packages reuse the same call lowering.
                let item = stdlib::get_item(module, func_name)
                    .or_else(|| pyaot_pkg_defs::get_item(module, func_name));
                if let Some(RegistryItem::Function(func_def)) = item {
                    // Map positional and keyword args to parameter slots
                    let mut arg_slots: Vec<Option<ExprId>> = vec![None; func_def.params.len()];

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
                                arg_slots[pos] = Some(self.convert_expr(kw.value.clone())?);
                            }
                        }
                    }

                    // Find the last filled slot to determine how many args to pass.
                    let last_filled = arg_slots.iter().rposition(|s| s.is_some());
                    let collect_len = last_filled.map_or(0, |i| i + 1);

                    let mut args = Vec::with_capacity(collect_len);
                    for (i, slot) in arg_slots.iter().enumerate().take(collect_len) {
                        if let Some(expr_id) = slot {
                            args.push(*expr_id);
                        } else {
                            // Fill interior gap with a default-value expression
                            let param = &func_def.params[i];
                            let default_expr = if let Some(ref dv) = param.default {
                                match dv {
                                    pyaot_stdlib_defs::ConstValue::Int(v) => ExprKind::Int(*v),
                                    pyaot_stdlib_defs::ConstValue::Float(v) => ExprKind::Float(*v),
                                    pyaot_stdlib_defs::ConstValue::Bool(v) => ExprKind::Bool(*v),
                                    pyaot_stdlib_defs::ConstValue::Str(s) => {
                                        ExprKind::Str(self.interner.intern(s))
                                    }
                                }
                            } else {
                                ExprKind::None
                            };
                            let filler = self.module.exprs.alloc(Expr {
                                kind: default_expr,
                                ty: None,
                                span: expr_span,
                            });
                            args.push(filler);
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

        // Check for chained package access: pkg.sub.func()
        if let Some(module_path) = self.try_resolve_chained_module_path(&attr.value, &attr.attr) {
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
            if let Some(module_path) = self.imports.imported_modules.get(&name_str).cloned() {
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

        // Special handling for object.__new__(cls) -> allocate instance
        if attr.attr.as_str() == "__new__" {
            if let py::Expr::Name(name) = &*attr.value {
                if name.id.as_str() == "object" {
                    let mut hir_args = Vec::new();
                    for arg in call.args {
                        hir_args.push(self.convert_expr(arg)?);
                    }
                    return Ok(self.module.exprs.alloc(Expr {
                        kind: ExprKind::BuiltinCall {
                            builtin: Builtin::ObjectNew,
                            args: hir_args,
                            kwargs: vec![],
                        },
                        ty: None,
                        span: expr_span,
                    }));
                }
            }
        }

        // Special handling for str.format() on string literals
        if attr.attr.as_str() == "format" {
            if let py::Expr::Constant(c) = &*attr.value {
                if let py::Constant::Str(format_str) = &c.value {
                    let mut format_args = Vec::new();
                    for arg in call.args.clone() {
                        format_args.push(self.convert_expr(arg)?);
                    }

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

        // General method call: obj.method(args)
        let obj = self.convert_expr(*attr.value.clone())?;
        let method = self.interner.intern(&attr.attr);

        let mut args = Vec::new();
        for arg in call.args {
            args.push(self.convert_expr(arg)?);
        }

        let method_kwargs = self.convert_method_keywords(call.keywords)?;

        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::MethodCall {
                obj,
                method,
                args,
                kwargs: method_kwargs,
            },
            ty: None,
            span: expr_span,
        }))
    }

    /// Handle calls where the callee is a simple Name expression
    /// (builtins, class instantiation, user-defined functions, imported names).
    fn convert_name_call(
        &mut self,
        call: py::ExprCall,
        kwargs: Vec<KeywordArg>,
        kwargs_unpack: Option<ExprId>,
        expr_span: Span,
    ) -> Result<ExprId> {
        let name = match &*call.func {
            py::Expr::Name(n) => n,
            _ => unreachable!("convert_name_call called with non-Name func"),
        };
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
        if let Some(&class_id) = self.symbols.class_map.get(&name_str) {
            let class_ref_expr = Expr {
                kind: ExprKind::ClassRef(class_id),
                ty: None,
                span: expr_span,
            };
            let func = self.module.exprs.alloc(class_ref_expr);

            let args = self.convert_call_args(call.args)?;
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::Call {
                    func,
                    args,
                    kwargs,
                    kwargs_unpack,
                },
                ty: None,
                span: expr_span,
            }));
        }

        // Check if it's a user-defined function reference
        if let Some(&func_id) = self.symbols.func_map.get(&name_str) {
            let func_ref_expr = Expr {
                kind: ExprKind::FuncRef(func_id),
                ty: None,
                span: expr_span,
            };
            let func = self.module.exprs.alloc(func_ref_expr);

            let args = self.convert_call_args(call.args)?;
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::Call {
                    func,
                    args,
                    kwargs,
                    kwargs_unpack,
                },
                ty: None,
                span: expr_span,
            }));
        }

        // Check if it's an imported name
        if let Some(imported) = self.imports.imported_names.get(&name_str).cloned() {
            let func_expr = match imported.kind {
                super::super::ImportedNameKind::Function(func_id) => {
                    self.module.exprs.alloc(Expr {
                        kind: ExprKind::FuncRef(func_id),
                        ty: None,
                        span: expr_span,
                    })
                }
                super::super::ImportedNameKind::Class(class_id) => self.module.exprs.alloc(Expr {
                    kind: ExprKind::ClassRef(class_id),
                    ty: None,
                    span: expr_span,
                }),
                super::super::ImportedNameKind::Variable(_) => self.convert_expr(*call.func)?,
                super::super::ImportedNameKind::Unresolved => self.module.exprs.alloc(Expr {
                    kind: ExprKind::ImportedRef {
                        module: imported.module.clone(),
                        name: imported.original_name.clone(),
                    },
                    ty: None,
                    span: expr_span,
                }),
            };

            let args = self.convert_call_args(call.args)?;
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::Call {
                    func: func_expr,
                    args,
                    kwargs,
                    kwargs_unpack,
                },
                ty: None,
                span: expr_span,
            }));
        }

        // Not a class or function, convert as normal expression
        let func = self.convert_expr(*call.func)?;
        let args = self.convert_call_args(call.args)?;
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::Call {
                func,
                args,
                kwargs,
                kwargs_unpack,
            },
            ty: None,
            span: expr_span,
        }))
    }

    /// Resolve a defaultdict factory argument (Python AST name) to an integer tag.
    fn resolve_defaultdict_factory(&self, arg: &py::Expr, span: Span) -> Result<i64> {
        if let py::Expr::Name(name) = arg {
            match name.id.as_str() {
                "int" => Ok(0),
                "float" => Ok(1),
                "str" => Ok(2),
                "bool" => Ok(3),
                "list" => Ok(4),
                "dict" => Ok(5),
                "set" => Ok(6),
                other => Err(CompilerError::parse_error(
                    format!(
                        "defaultdict factory must be int, float, str, bool, list, dict, or set, got '{}'",
                        other
                    ),
                    span,
                )),
            }
        } else {
            Err(CompilerError::parse_error(
                "defaultdict factory must be a type name (int, str, list, etc.)",
                span,
            ))
        }
    }
}
