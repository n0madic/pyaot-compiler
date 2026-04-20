use super::*;
use indexmap::IndexSet;
use pyaot_hir::cfg_builder::{CfgBuilder, CfgExceptHandler, CfgStmt};
use pyaot_hir::{BindingTarget, ClassDef, Expr, Function, HirTerminator, MethodKind, Module, Stmt};
use pyaot_utils::{ClassId, FuncId, Span, StringInterner, VarId};

fn dummy_span() -> Span {
    Span { start: 0, end: 0 }
}

fn create_test_module() -> (Module, StringInterner) {
    let mut interner = StringInterner::new();
    let module_name = interner.intern("test_module");
    let module = Module::new(module_name);
    (module, interner)
}

fn install_module_init(module: &mut Module, body: Vec<CfgStmt>) {
    let mut cfg = CfgBuilder::new();
    let entry_block = cfg.new_block();
    cfg.enter(entry_block);
    cfg.lower_cfg_stmts(&body, module);
    cfg.terminate_if_open(HirTerminator::Return(None));
    let (blocks, entry_block, try_scopes) = cfg.finish(entry_block);

    let func_id = FuncId::new(0);
    module.module_init_func = Some(func_id);
    module.functions.push(func_id);
    module.func_defs.insert(
        func_id,
        Function {
            id: func_id,
            name: module.name,
            params: Vec::new(),
            return_type: None,
            span: dummy_span(),
            cell_vars: std::collections::HashSet::new(),
            nonlocal_vars: std::collections::HashSet::new(),
            is_generator: false,
            method_kind: MethodKind::default(),
            is_abstract: false,
            blocks,
            entry_block,
            try_scopes,
        },
    );
}

#[test]
fn test_analyzer_creation() {
    let interner = StringInterner::new();
    let _analyzer = SemanticAnalyzer::new(&interner);
    // Analyzer no longer carries depth counters (§1.17b-e, 2026-04-19);
    // per-stmt loop/handler depth comes from `HirBlock` annotations now.
}

#[test]
fn test_break_outside_loop_fails() {
    let (mut module, interner) = create_test_module();

    // Add a break statement at module level
    let break_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Break,
        span: dummy_span(),
    });
    install_module_init(&mut module, vec![CfgStmt::stmt(break_stmt)]);

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_err());
    if let Err(CompilerError::SemanticError { message, .. }) = result {
        assert!(message.contains("break"));
    } else {
        panic!("Expected SemanticError");
    }
}

#[test]
fn test_continue_outside_loop_fails() {
    let (mut module, interner) = create_test_module();

    // Add a continue statement at module level
    let continue_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Continue,
        span: dummy_span(),
    });
    install_module_init(&mut module, vec![CfgStmt::stmt(continue_stmt)]);

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_err());
    if let Err(CompilerError::SemanticError { message, .. }) = result {
        assert!(message.contains("continue"));
    } else {
        panic!("Expected SemanticError");
    }
}

#[test]
fn test_bare_raise_outside_except_fails() {
    let (mut module, interner) = create_test_module();

    // Add a bare raise statement at module level
    let raise_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Raise {
            exc: None,
            cause: None,
        },
        span: dummy_span(),
    });
    install_module_init(&mut module, vec![CfgStmt::stmt(raise_stmt)]);

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_err());
    if let Err(CompilerError::SemanticError { message, .. }) = result {
        assert!(message.contains("raise"));
    } else {
        panic!("Expected SemanticError");
    }
}

#[test]
fn test_break_inside_while_loop_succeeds() {
    let (mut module, interner) = create_test_module();

    // Create: while True: break
    let true_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Bool(true),
        ty: None,
        span: dummy_span(),
    });

    let break_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Break,
        span: dummy_span(),
    });

    install_module_init(
        &mut module,
        vec![CfgStmt::While {
            cond: true_expr,
            body: vec![CfgStmt::stmt(break_stmt)],
            else_body: vec![],
            span: dummy_span(),
        }],
    );

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_ok());
}

#[test]
fn test_continue_inside_for_loop_succeeds() {
    let (mut module, interner) = create_test_module();

    // Create: for x in []: continue
    let empty_list = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: None,
        span: dummy_span(),
    });

    let continue_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Continue,
        span: dummy_span(),
    });

    let var_id = VarId::new(0);
    install_module_init(
        &mut module,
        vec![CfgStmt::For {
            target: BindingTarget::Var(var_id),
            iter: empty_list,
            body: vec![CfgStmt::stmt(continue_stmt)],
            else_body: vec![],
            span: dummy_span(),
        }],
    );

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_ok());
}

#[test]
fn test_bare_raise_inside_except_succeeds() {
    let (mut module, interner) = create_test_module();

    // Create: try: pass except: raise
    let pass_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Pass,
        span: dummy_span(),
    });

    let raise_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Raise {
            exc: None,
            cause: None,
        },
        span: dummy_span(),
    });

    install_module_init(
        &mut module,
        vec![CfgStmt::Try {
            body: vec![CfgStmt::stmt(pass_stmt)],
            handlers: vec![CfgExceptHandler {
                ty: None,
                name: None,
                body: vec![CfgStmt::stmt(raise_stmt)],
            }],
            else_body: vec![],
            finally_body: vec![],
            span: dummy_span(),
        }],
    );

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_ok());
}

#[test]
fn test_nested_loops_succeed() {
    let (mut module, interner) = create_test_module();

    // Create: while True: while True: break
    let true_expr1 = module.exprs.alloc(Expr {
        kind: ExprKind::Bool(true),
        ty: None,
        span: dummy_span(),
    });

    let true_expr2 = module.exprs.alloc(Expr {
        kind: ExprKind::Bool(true),
        ty: None,
        span: dummy_span(),
    });

    let break_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Break,
        span: dummy_span(),
    });

    install_module_init(
        &mut module,
        vec![CfgStmt::While {
            cond: true_expr1,
            body: vec![CfgStmt::While {
                cond: true_expr2,
                body: vec![CfgStmt::stmt(break_stmt)],
                else_body: vec![],
                span: dummy_span(),
            }],
            else_body: vec![],
            span: dummy_span(),
        }],
    );

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_ok());
}

#[test]
fn test_bare_raise_in_finally_succeeds() {
    let (mut module, interner) = create_test_module();

    // CPython allows bare raise in finally — it re-raises the active exception
    let pass_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Pass,
        span: dummy_span(),
    });

    let raise_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Raise {
            exc: None,
            cause: None,
        },
        span: dummy_span(),
    });

    install_module_init(
        &mut module,
        vec![CfgStmt::Try {
            body: vec![CfgStmt::stmt(pass_stmt)],
            handlers: vec![],
            else_body: vec![],
            finally_body: vec![CfgStmt::stmt(raise_stmt)],
            span: dummy_span(),
        }],
    );

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_ok());
}

#[test]
fn test_raise_with_expression_outside_except_succeeds() {
    let (mut module, interner) = create_test_module();

    // Create: raise Exception("test") - should succeed anywhere
    let exc_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Int(42), // Simplified, doesn't matter for semantic analysis
        ty: None,
        span: dummy_span(),
    });

    let raise_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Raise {
            exc: Some(exc_expr),
            cause: None,
        },
        span: dummy_span(),
    });
    install_module_init(&mut module, vec![CfgStmt::stmt(raise_stmt)]);

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_ok());
}

#[test]
fn test_abstract_class_instantiation_fails() {
    let (mut module, mut interner) = create_test_module();

    // Create an abstract class with one unimplemented method
    let class_id = ClassId::new(0);
    let method_name = interner.intern("do_something");
    let class_name = interner.intern("MyAbstractClass");

    let mut abstract_methods = IndexSet::new();
    abstract_methods.insert(method_name);

    let class_def = ClassDef {
        id: class_id,
        name: class_name,
        base_class: None,
        fields: vec![],
        class_attrs: vec![],
        methods: vec![],
        init_method: None,
        properties: vec![],
        abstract_methods,
        span: dummy_span(),
        is_exception_class: false,
        base_exception_type: None,
        is_protocol: false,
    };
    module.class_defs.insert(class_id, class_def);

    // Create: MyAbstractClass() — should fail
    let class_ref = module.exprs.alloc(Expr {
        kind: ExprKind::ClassRef(class_id),
        ty: None,
        span: dummy_span(),
    });

    let call_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Call {
            func: class_ref,
            args: vec![],
            kwargs: vec![],
            kwargs_unpack: None,
        },
        ty: None,
        span: dummy_span(),
    });

    let call_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Expr(call_expr),
        span: dummy_span(),
    });
    install_module_init(&mut module, vec![CfgStmt::stmt(call_stmt)]);

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_err());
    if let Err(CompilerError::SemanticError { message, .. }) = result {
        assert!(message.contains("Cannot instantiate abstract class"));
        assert!(message.contains("MyAbstractClass"));
        assert!(message.contains("do_something"));
    } else {
        panic!("Expected SemanticError");
    }
}

#[test]
fn test_concrete_class_instantiation_succeeds() {
    let (mut module, mut interner) = create_test_module();

    // Create a concrete class (no abstract methods)
    let class_id = ClassId::new(0);
    let class_name = interner.intern("MyClass");

    let class_def = ClassDef {
        id: class_id,
        name: class_name,
        base_class: None,
        fields: vec![],
        class_attrs: vec![],
        methods: vec![],
        init_method: None,
        properties: vec![],
        abstract_methods: IndexSet::new(),
        span: dummy_span(),
        is_exception_class: false,
        base_exception_type: None,
        is_protocol: false,
    };
    module.class_defs.insert(class_id, class_def);

    // Create: MyClass() — should succeed
    let class_ref = module.exprs.alloc(Expr {
        kind: ExprKind::ClassRef(class_id),
        ty: None,
        span: dummy_span(),
    });

    let call_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Call {
            func: class_ref,
            args: vec![],
            kwargs: vec![],
            kwargs_unpack: None,
        },
        ty: None,
        span: dummy_span(),
    });

    let call_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Expr(call_expr),
        span: dummy_span(),
    });
    install_module_init(&mut module, vec![CfgStmt::stmt(call_stmt)]);

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_ok());
}
