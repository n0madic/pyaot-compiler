use super::*;
use indexmap::IndexSet;
use pyaot_hir::{BindingTarget, ClassDef, ExceptHandler, Expr, Module, Stmt};
use pyaot_utils::{ClassId, Span, StringInterner, VarId};

fn dummy_span() -> Span {
    Span { start: 0, end: 0 }
}

fn create_test_module() -> (Module, StringInterner) {
    let mut interner = StringInterner::new();
    let module_name = interner.intern("test_module");
    let module = Module::new(module_name);
    (module, interner)
}

#[test]
fn test_analyzer_creation() {
    let interner = StringInterner::new();
    let analyzer = SemanticAnalyzer::new(&interner);
    assert_eq!(analyzer.loop_depth, 0);
    assert_eq!(analyzer.except_depth, 0);
}

#[test]
fn test_break_outside_loop_fails() {
    let (mut module, interner) = create_test_module();

    // Add a break statement at module level
    let break_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Break,
        span: dummy_span(),
    });
    module.module_init_stmts.push(break_stmt);

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
    module.module_init_stmts.push(continue_stmt);

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
    module.module_init_stmts.push(raise_stmt);

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

    let while_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::While {
            cond: true_expr,
            body: vec![break_stmt],
            else_block: vec![],
        },
        span: dummy_span(),
    });
    module.module_init_stmts.push(while_stmt);

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
    let for_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::ForBind {
            target: BindingTarget::Var(var_id),
            iter: empty_list,
            body: vec![continue_stmt],
            else_block: vec![],
        },
        span: dummy_span(),
    });
    module.module_init_stmts.push(for_stmt);

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

    let try_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Try {
            body: vec![pass_stmt],
            handlers: vec![ExceptHandler {
                ty: None,
                name: None,
                body: vec![raise_stmt],
            }],
            else_block: vec![],
            finally_block: vec![],
        },
        span: dummy_span(),
    });
    module.module_init_stmts.push(try_stmt);

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

    let inner_while = module.stmts.alloc(Stmt {
        kind: StmtKind::While {
            cond: true_expr2,
            body: vec![break_stmt],
            else_block: vec![],
        },
        span: dummy_span(),
    });

    let outer_while = module.stmts.alloc(Stmt {
        kind: StmtKind::While {
            cond: true_expr1,
            body: vec![inner_while],
            else_block: vec![],
        },
        span: dummy_span(),
    });
    module.module_init_stmts.push(outer_while);

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

    let try_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Try {
            body: vec![pass_stmt],
            handlers: vec![],
            else_block: vec![],
            finally_block: vec![raise_stmt],
        },
        span: dummy_span(),
    });
    module.module_init_stmts.push(try_stmt);

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
    module.module_init_stmts.push(raise_stmt);

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
    module.module_init_stmts.push(call_stmt);

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
    module.module_init_stmts.push(call_stmt);

    let mut analyzer = SemanticAnalyzer::new(&interner);
    let result = analyzer.analyze(&module);

    assert!(result.is_ok());
}
