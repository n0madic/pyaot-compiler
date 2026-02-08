use super::TypeChecker;
use pyaot_diagnostics::CompilerError;
use pyaot_hir::{
    BinOp, Builtin, Expr, ExprKind, Function, KeywordArg, MethodKind, Module, Param, ParamKind,
    Stmt, StmtKind, UnOp,
};
use pyaot_types::Type;
use pyaot_utils::{FuncId, Span, StringInterner, VarId};
use std::collections::HashSet;

fn dummy_span() -> Span {
    Span { start: 0, end: 0 }
}

fn create_test_module(interner: &mut StringInterner) -> Module {
    let module_name = interner.intern("test_module");
    Module::new(module_name)
}

#[test]
fn test_binop_type_inference() {
    let interner = StringInterner::new();
    let checker = TypeChecker::new(&interner);

    assert_eq!(
        checker.infer_binop_type(BinOp::Add, &Type::Int, &Type::Int),
        Type::Int
    );
    assert_eq!(
        checker.infer_binop_type(BinOp::Add, &Type::Int, &Type::Float),
        Type::Float
    );
    assert_eq!(
        checker.infer_binop_type(BinOp::Add, &Type::Str, &Type::Str),
        Type::Str
    );
    assert_eq!(
        checker.infer_binop_type(BinOp::Div, &Type::Int, &Type::Int),
        Type::Float
    );
}

#[test]
fn test_unop_type_inference() {
    let interner = StringInterner::new();
    let checker = TypeChecker::new(&interner);

    assert_eq!(checker.infer_unop_type(UnOp::Neg, &Type::Int), Type::Int);
    assert_eq!(
        checker.infer_unop_type(UnOp::Neg, &Type::Float),
        Type::Float
    );
    assert_eq!(checker.infer_unop_type(UnOp::Not, &Type::Bool), Type::Bool);
}

#[test]
fn test_function_call_wrong_arg_count() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);

    // Create function: def foo(x: int, y: int) -> int: return x
    let func_id = FuncId::new(0);
    let func_name = interner.intern("foo");
    let param_x_name = interner.intern("x");
    let param_y_name = interner.intern("y");

    let x_var = VarId::new(0);
    let y_var = VarId::new(1);

    let x_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Var(x_var),
        ty: Some(Type::Int),
        span: dummy_span(),
    });

    let return_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Return(Some(x_expr)),
        span: dummy_span(),
    });

    let func = Function {
        id: func_id,
        name: func_name,
        params: vec![
            Param {
                name: param_x_name,
                var: x_var,
                ty: Some(Type::Int),
                default: None,
                kind: ParamKind::Regular,
                span: dummy_span(),
            },
            Param {
                name: param_y_name,
                var: y_var,
                ty: Some(Type::Int),
                default: None,
                kind: ParamKind::Regular,
                span: dummy_span(),
            },
        ],
        return_type: Some(Type::Int),
        body: vec![return_stmt],
        span: dummy_span(),
        cell_vars: HashSet::new(),
        nonlocal_vars: HashSet::new(),
        is_generator: false,
        method_kind: MethodKind::default(),
        is_abstract: false,
    };

    module.functions.push(func_id);
    module.func_defs.insert(func_id, func);

    // Call foo with only 1 argument
    let func_ref = module.exprs.alloc(Expr {
        kind: ExprKind::FuncRef(func_id),
        ty: None,
        span: dummy_span(),
    });

    let arg1 = module.exprs.alloc(Expr {
        kind: ExprKind::Int(42),
        ty: Some(Type::Int),
        span: dummy_span(),
    });

    let call_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Call {
            func: func_ref,
            args: vec![pyaot_hir::CallArg::Regular(arg1)],
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

    let mut checker = TypeChecker::new(&interner);
    let result = checker.check_module(&module);

    assert!(result.is_err());
    if let Err(CompilerError::TypeError { message, .. }) = result {
        assert!(message.contains("missing"));
    } else {
        panic!("Expected TypeError for missing argument");
    }
}

#[test]
fn test_function_call_wrong_arg_type() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);

    // Create function: def add(x: int, y: int) -> int: return x
    let func_id = FuncId::new(0);
    let func_name = interner.intern("add");
    let param_x_name = interner.intern("x");
    let param_y_name = interner.intern("y");

    let x_var = VarId::new(0);
    let y_var = VarId::new(1);

    let x_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Var(x_var),
        ty: Some(Type::Int),
        span: dummy_span(),
    });

    let return_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Return(Some(x_expr)),
        span: dummy_span(),
    });

    let func = Function {
        id: func_id,
        name: func_name,
        params: vec![
            Param {
                name: param_x_name,
                var: x_var,
                ty: Some(Type::Int),
                default: None,
                kind: ParamKind::Regular,
                span: dummy_span(),
            },
            Param {
                name: param_y_name,
                var: y_var,
                ty: Some(Type::Int),
                default: None,
                kind: ParamKind::Regular,
                span: dummy_span(),
            },
        ],
        return_type: Some(Type::Int),
        body: vec![return_stmt],
        span: dummy_span(),
        cell_vars: HashSet::new(),
        nonlocal_vars: HashSet::new(),
        is_generator: false,
        method_kind: MethodKind::default(),
        is_abstract: false,
    };

    module.functions.push(func_id);
    module.func_defs.insert(func_id, func);

    // Call add(1, "hello") - wrong second argument type
    let func_ref = module.exprs.alloc(Expr {
        kind: ExprKind::FuncRef(func_id),
        ty: None,
        span: dummy_span(),
    });

    let arg1 = module.exprs.alloc(Expr {
        kind: ExprKind::Int(1),
        ty: Some(Type::Int),
        span: dummy_span(),
    });

    let arg2 = module.exprs.alloc(Expr {
        kind: ExprKind::Str(interner.intern("hello")),
        ty: Some(Type::Str),
        span: dummy_span(),
    });

    let call_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Call {
            func: func_ref,
            args: vec![
                pyaot_hir::CallArg::Regular(arg1),
                pyaot_hir::CallArg::Regular(arg2),
            ],
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

    let mut checker = TypeChecker::new(&interner);
    let result = checker.check_module(&module);

    assert!(result.is_err());
    if let Err(CompilerError::TypeError { message, .. }) = result {
        assert!(message.contains("int"));
        assert!(message.contains("str"));
    } else {
        panic!("Expected TypeError for wrong argument type");
    }
}

#[test]
fn test_return_type_mismatch() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);

    // Create function: def get_number() -> int: return "hello"
    let func_id = FuncId::new(0);
    let func_name = interner.intern("get_number");

    let str_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Str(interner.intern("hello")),
        ty: Some(Type::Str),
        span: dummy_span(),
    });

    let return_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Return(Some(str_expr)),
        span: dummy_span(),
    });

    let func = Function {
        id: func_id,
        name: func_name,
        params: vec![],
        return_type: Some(Type::Int),
        body: vec![return_stmt],
        span: dummy_span(),
        cell_vars: HashSet::new(),
        nonlocal_vars: HashSet::new(),
        is_generator: false,
        method_kind: MethodKind::default(),
        is_abstract: false,
    };

    module.functions.push(func_id);
    module.func_defs.insert(func_id, func);

    let mut checker = TypeChecker::new(&interner);
    let result = checker.check_module(&module);

    assert!(result.is_err());
    if let Err(CompilerError::TypeError { message, .. }) = result {
        assert!(message.contains("return type"));
        assert!(message.contains("str"));
        assert!(message.contains("int"));
    } else {
        panic!("Expected TypeError for return type mismatch");
    }
}

#[test]
fn test_assignment_type_mismatch() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);

    // Create: x: int = "hello"
    let str_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Str(interner.intern("hello")),
        ty: Some(Type::Str),
        span: dummy_span(),
    });

    let var_id = VarId::new(0);
    let assign_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Assign {
            target: var_id,
            value: str_expr,
            type_hint: Some(Type::Int),
        },
        span: dummy_span(),
    });

    module.module_init_stmts.push(assign_stmt);

    let mut checker = TypeChecker::new(&interner);
    let result = checker.check_module(&module);

    assert!(result.is_err());
    if let Err(CompilerError::TypeError { message, .. }) = result {
        assert!(message.contains("assign"));
        assert!(message.contains("str"));
        assert!(message.contains("int"));
    } else {
        panic!("Expected TypeError for assignment type mismatch");
    }
}

#[test]
fn test_empty_container_to_typed_variable() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);

    // Create: nums: list[int] = []
    let empty_list = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: Some(Type::List(Box::new(Type::Any))),
        span: dummy_span(),
    });

    let var_id = VarId::new(0);
    let assign_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::Assign {
            target: var_id,
            value: empty_list,
            type_hint: Some(Type::List(Box::new(Type::Int))),
        },
        span: dummy_span(),
    });

    module.module_init_stmts.push(assign_stmt);

    let mut checker = TypeChecker::new(&interner);
    let result = checker.check_module(&module);

    // Should succeed - empty list[Any] can be assigned to list[int]
    assert!(result.is_ok());
}

#[test]
fn test_tuple_unpacking_length_mismatch() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);

    // Create: a, b, c = (1, 2) - wrong number of elements
    let one = module.exprs.alloc(Expr {
        kind: ExprKind::Int(1),
        ty: Some(Type::Int),
        span: dummy_span(),
    });

    let two = module.exprs.alloc(Expr {
        kind: ExprKind::Int(2),
        ty: Some(Type::Int),
        span: dummy_span(),
    });

    let tuple_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Tuple(vec![one, two]),
        ty: Some(Type::Tuple(vec![Type::Int, Type::Int])),
        span: dummy_span(),
    });

    let unpack_stmt = module.stmts.alloc(Stmt {
        kind: StmtKind::UnpackAssign {
            before_star: vec![VarId::new(0), VarId::new(1), VarId::new(2)],
            starred: None,
            after_star: vec![],
            value: tuple_expr,
        },
        span: dummy_span(),
    });

    module.module_init_stmts.push(unpack_stmt);

    let mut checker = TypeChecker::new(&interner);
    let result = checker.check_module(&module);

    assert!(result.is_err());
    if let Err(CompilerError::TypeError { message, .. }) = result {
        assert!(message.contains("unpack"));
    } else {
        panic!("Expected TypeError for unpacking length mismatch");
    }
}

#[test]
fn test_extract_iterable_element_type() {
    let interner = StringInterner::new();
    let mut checker = TypeChecker::new(&interner);

    // List[int] -> int
    assert_eq!(
        checker.extract_iterable_element_type(&Type::List(Box::new(Type::Int))),
        Type::Int
    );

    // Tuple[str, str, str] -> str (first element)
    assert_eq!(
        checker.extract_iterable_element_type(&Type::Tuple(vec![Type::Str, Type::Str, Type::Str])),
        Type::Str
    );

    // Empty tuple -> Any
    assert_eq!(
        checker.extract_iterable_element_type(&Type::Tuple(vec![])),
        Type::Any
    );

    // Set[float] -> float
    assert_eq!(
        checker.extract_iterable_element_type(&Type::Set(Box::new(Type::Float))),
        Type::Float
    );

    // Dict[str, int] -> str (key type)
    assert_eq!(
        checker
            .extract_iterable_element_type(&Type::Dict(Box::new(Type::Str), Box::new(Type::Int))),
        Type::Str
    );

    // str -> str
    assert_eq!(checker.extract_iterable_element_type(&Type::Str), Type::Str);

    // bytes -> int
    assert_eq!(
        checker.extract_iterable_element_type(&Type::Bytes),
        Type::Int
    );

    // Iterator[bool] -> bool
    assert_eq!(
        checker.extract_iterable_element_type(&Type::Iterator(Box::new(Type::Bool))),
        Type::Bool
    );

    // Unknown type -> Any
    assert_eq!(checker.extract_iterable_element_type(&Type::Int), Type::Any);
}

#[test]
fn test_builtin_list_type_inference() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);

    // Create expressions before borrowing interner for checker
    let int_list_expr = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: Some(Type::List(Box::new(Type::Int))),
        span: dummy_span(),
    });
    let abc_str = interner.intern("abc");
    let str_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Str(abc_str),
        ty: Some(Type::Str),
        span: dummy_span(),
    });

    // Now create checker after all mutations
    let mut checker = TypeChecker::new(&interner);

    // list() with no args -> list[Any]
    let result = checker.infer_builtin_type(Builtin::List, &[], &[], &module);
    assert_eq!(result, Type::List(Box::new(Type::Any)));

    // list([1, 2, 3]) -> list[int]
    let result = checker.infer_builtin_type(Builtin::List, &[int_list_expr], &[], &module);
    assert_eq!(result, Type::List(Box::new(Type::Int)));

    // list("abc") -> list[str]
    let result = checker.infer_builtin_type(Builtin::List, &[str_expr], &[], &module);
    assert_eq!(result, Type::List(Box::new(Type::Str)));
}

#[test]
fn test_builtin_tuple_type_inference() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);
    let mut checker = TypeChecker::new(&interner);

    // tuple() with no args -> tuple[]
    let result = checker.infer_builtin_type(Builtin::Tuple, &[], &[], &module);
    assert_eq!(result, Type::Tuple(vec![]));

    // tuple([1, 2, 3]) -> tuple[int]
    let int_list_expr = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: Some(Type::List(Box::new(Type::Int))),
        span: dummy_span(),
    });
    let result = checker.infer_builtin_type(Builtin::Tuple, &[int_list_expr], &[], &module);
    assert_eq!(result, Type::Tuple(vec![Type::Int]));
}

#[test]
fn test_builtin_set_type_inference() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);

    // Create expressions before borrowing interner for checker
    let int_list_expr = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: Some(Type::List(Box::new(Type::Int))),
        span: dummy_span(),
    });
    let abc_str = interner.intern("abc");
    let str_expr = module.exprs.alloc(Expr {
        kind: ExprKind::Str(abc_str),
        ty: Some(Type::Str),
        span: dummy_span(),
    });

    // Now create checker after all mutations
    let mut checker = TypeChecker::new(&interner);

    // set() with no args -> set[Any]
    let result = checker.infer_builtin_type(Builtin::Set, &[], &[], &module);
    assert_eq!(result, Type::Set(Box::new(Type::Any)));

    // set([1, 2, 3]) -> set[int]
    let result = checker.infer_builtin_type(Builtin::Set, &[int_list_expr], &[], &module);
    assert_eq!(result, Type::Set(Box::new(Type::Int)));

    // set("abc") -> set[str]
    let result = checker.infer_builtin_type(Builtin::Set, &[str_expr], &[], &module);
    assert_eq!(result, Type::Set(Box::new(Type::Str)));
}

#[test]
fn test_builtin_dict_type_inference() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);

    // Intern names and create expressions before borrowing interner for checker
    let a_name = interner.intern("a");
    let b_name = interner.intern("b");
    let int_expr1 = module.exprs.alloc(Expr {
        kind: ExprKind::Int(1),
        ty: Some(Type::Int),
        span: dummy_span(),
    });
    let int_expr2 = module.exprs.alloc(Expr {
        kind: ExprKind::Int(2),
        ty: Some(Type::Int),
        span: dummy_span(),
    });
    let kwargs = vec![
        KeywordArg {
            name: a_name,
            value: int_expr1,
            span: dummy_span(),
        },
        KeywordArg {
            name: b_name,
            value: int_expr2,
            span: dummy_span(),
        },
    ];

    // Now create checker after all mutations
    let mut checker = TypeChecker::new(&interner);

    // dict() with no args -> dict[Any, Any]
    let result = checker.infer_builtin_type(Builtin::Dict, &[], &[], &module);
    assert_eq!(result, Type::Dict(Box::new(Type::Any), Box::new(Type::Any)));

    // dict(a=1, b=2) -> dict[str, int]
    let result = checker.infer_builtin_type(Builtin::Dict, &[], &kwargs, &module);
    assert_eq!(result, Type::Dict(Box::new(Type::Str), Box::new(Type::Int)));
}

#[test]
fn test_builtin_iter_type_inference() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);
    let mut checker = TypeChecker::new(&interner);

    // iter() with no args -> Iterator[Any]
    let result = checker.infer_builtin_type(Builtin::Iter, &[], &[], &module);
    assert_eq!(result, Type::Iterator(Box::new(Type::Any)));

    // iter([1, 2, 3]) -> Iterator[int]
    let int_list_expr = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: Some(Type::List(Box::new(Type::Int))),
        span: dummy_span(),
    });
    let result = checker.infer_builtin_type(Builtin::Iter, &[int_list_expr], &[], &module);
    assert_eq!(result, Type::Iterator(Box::new(Type::Int)));
}

#[test]
fn test_builtin_reversed_type_inference() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);
    let mut checker = TypeChecker::new(&interner);

    // reversed([1, 2, 3]) -> Iterator[int]
    let int_list_expr = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: Some(Type::List(Box::new(Type::Int))),
        span: dummy_span(),
    });
    let result = checker.infer_builtin_type(Builtin::Reversed, &[int_list_expr], &[], &module);
    assert_eq!(result, Type::Iterator(Box::new(Type::Int)));
}

#[test]
fn test_builtin_sorted_type_inference() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);
    let mut checker = TypeChecker::new(&interner);

    // sorted([3, 1, 2]) -> list[int]
    let int_list_expr = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: Some(Type::List(Box::new(Type::Int))),
        span: dummy_span(),
    });
    let result = checker.infer_builtin_type(Builtin::Sorted, &[int_list_expr], &[], &module);
    assert_eq!(result, Type::List(Box::new(Type::Int)));
}

#[test]
fn test_builtin_enumerate_type_inference() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);
    let mut checker = TypeChecker::new(&interner);

    // enumerate(["a", "b"]) -> Iterator[Tuple[int, str]]
    let str_list_expr = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: Some(Type::List(Box::new(Type::Str))),
        span: dummy_span(),
    });
    let result = checker.infer_builtin_type(Builtin::Enumerate, &[str_list_expr], &[], &module);
    assert_eq!(
        result,
        Type::Iterator(Box::new(Type::Tuple(vec![Type::Int, Type::Str])))
    );
}

#[test]
fn test_builtin_zip_type_inference() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);
    let mut checker = TypeChecker::new(&interner);

    // zip() with no args -> Iterator[Tuple[]]
    let result = checker.infer_builtin_type(Builtin::Zip, &[], &[], &module);
    assert_eq!(result, Type::Iterator(Box::new(Type::Tuple(vec![]))));

    // zip([1,2], ["a","b"]) -> Iterator[Tuple[int, str]]
    let int_list_expr = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: Some(Type::List(Box::new(Type::Int))),
        span: dummy_span(),
    });
    let str_list_expr = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: Some(Type::List(Box::new(Type::Str))),
        span: dummy_span(),
    });
    let result =
        checker.infer_builtin_type(Builtin::Zip, &[int_list_expr, str_list_expr], &[], &module);
    assert_eq!(
        result,
        Type::Iterator(Box::new(Type::Tuple(vec![Type::Int, Type::Str])))
    );
}

#[test]
fn test_builtin_filter_type_inference() {
    let mut interner = StringInterner::new();
    let mut module = create_test_module(&mut interner);
    let mut checker = TypeChecker::new(&interner);

    // filter(func, [1, 2, 3]) -> Iterator[int]
    let func_expr = module.exprs.alloc(Expr {
        kind: ExprKind::None,
        ty: Some(Type::None),
        span: dummy_span(),
    });
    let int_list_expr = module.exprs.alloc(Expr {
        kind: ExprKind::List(vec![]),
        ty: Some(Type::List(Box::new(Type::Int))),
        span: dummy_span(),
    });
    let result =
        checker.infer_builtin_type(Builtin::Filter, &[func_expr, int_list_expr], &[], &module);
    assert_eq!(result, Type::Iterator(Box::new(Type::Int)));
}

#[test]
fn test_builtin_range_type_inference() {
    let mut interner = StringInterner::new();
    let module = create_test_module(&mut interner);
    let mut checker = TypeChecker::new(&interner);

    // range() -> Iterator[int]
    let result = checker.infer_builtin_type(Builtin::Range, &[], &[], &module);
    assert_eq!(result, Type::Iterator(Box::new(Type::Int)));
}
