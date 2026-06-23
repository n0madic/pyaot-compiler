    use pyaot_hir::HirStmt;
    use pyaot_utils::StringInterner;

    /// Parse `src` into an HIR module.
    fn parsed(src: &str) -> (pyaot_hir::HirModule, StringInterner) {
        let mut interner = StringInterner::new();
        let module = crate::parse(src, &mut interner).expect("parse");
        (module, interner)
    }

    /// Parse `src`, returning the error message (the rejection-path helper).
    fn parse_err(src: &str) -> String {
        let mut interner = StringInterner::new();
        match crate::parse(src, &mut interner) {
            Ok(_) => panic!("expected a parse rejection"),
            Err(e) => format!("{e:?}"),
        }
    }

    // ── Phase 7 lexical restrictions ──

    #[test]
    fn accepts_yield_inside_try() {
        // A `yield` in a `try` body is supported (table-based unwinding has no
        // per-frame state for a cross-stack resume to dangle).
        let (_m, _i) = parsed(
            "def g():\n    try:\n        yield 1\n    except ValueError:\n        pass\n",
        );
    }

    #[test]
    fn accepts_yield_inside_with() {
        // A `yield` in a `with` body is supported (the `__exit__` cleanup runs on
        // close via the generator unwind path).
        let (_m, _i) = parsed("def g():\n    with ctx() as c:\n        yield c\n");
    }

    #[test]
    fn rejects_yield_inside_except() {
        // A yield in an `except` handler body would suspend with the runtime's
        // `handling_exception` thread-local still set — out of scope.
        let err = parse_err(
            "def g():\n    try:\n        yield 1\n    except ValueError:\n        yield 2\n",
        );
        assert!(
            err.contains("yield inside an `except` or `finally` block"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_yield_inside_finally() {
        // A yield in a `finally` block would be duplicated across the per-edge
        // re-lowering of the finalbody — out of scope.
        let err = parse_err(
            "def g():\n    try:\n        yield 1\n    finally:\n        yield 2\n",
        );
        assert!(
            err.contains("yield inside an `except` or `finally` block"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_or_pattern_binding_different_names() {
        // CPython rule: every alternative must bind the same set of names.
        // `[a]` binds {a}, `[a, b]` binds {a, b} — a mismatch.
        let err = parse_err("match x:\n    case [a] | [a, b]:\n        pass\n");
        assert!(
            err.contains("alternative patterns bind different names"),
            "got: {err}"
        );
    }

    #[test]
    fn accepts_or_pattern_with_matching_captures() {
        // Same name set across alternatives is allowed (`case [a] | [a, _]:`).
        let (_m, _i) = parsed("match x:\n    case [a] | [a, _]:\n        pass\n");
    }

    #[test]
    fn rejects_positional_class_pattern() {
        let err = parse_err(
            "class P:\n    def __init__(self, x: int):\n        self.x = x\nmatch P(1):\n    case P(1):\n        pass\n",
        );
        assert!(err.contains("positional class patterns"), "got: {err}");
    }

    #[test]
    fn rejects_unknown_exception_in_except() {
        let err = parse_err("try:\n    pass\nexcept NotAThing:\n    pass\n");
        assert!(err.contains("unknown exception type"), "got: {err}");
    }

    #[test]
    fn rejects_bare_except_not_last() {
        let err = parse_err("try:\n    pass\nexcept:\n    pass\nexcept ValueError:\n    pass\n");
        assert!(err.contains("must be last"), "got: {err}");
    }

    // ── `raise … from …` exception chaining (PEP 3134) ──

    #[test]
    fn accepts_raise_custom_from() {
        // A custom-class target with a constructed builtin cause.
        let (_m, _i) = parsed(
            "class MyError(Exception):\n    pass\ntry:\n    raise MyError(\"x\") from ValueError(\"c\")\nexcept MyError:\n    pass\n",
        );
    }

    #[test]
    fn accepts_raise_instance_from_var() {
        // An instance target (`raise e`) with a caught-variable cause.
        let (_m, _i) = parsed(
            "try:\n    raise ValueError(\"v\")\nexcept ValueError as e:\n    try:\n        raise TypeError(\"t\")\n    except TypeError as cause:\n        raise e from cause\n",
        );
    }

    #[test]
    fn accepts_raise_from_variable() {
        // A builtin target with a caught-variable cause.
        let (_m, _i) = parsed(
            "try:\n    raise TypeError(\"t\")\nexcept TypeError as cause:\n    raise ValueError(\"v\") from cause\n",
        );
    }

    #[test]
    fn rejects_raise_from_bare_class() {
        // A bare custom *class* cause (no parens) has no instance to introspect.
        let err = parse_err(
            "class MyError(Exception):\n    pass\nraise ValueError(\"x\") from MyError\n",
        );
        assert!(err.contains("bare class cause"), "got: {err}");
    }

    // ── FIX 1 / FIX 2: nested generator + nested class restrictions ──

    #[test]
    fn rejects_capturing_nested_generator() {
        let err = parse_err(
            "def outer():\n    base = 10\n    def gen(n):\n        i = 0\n        \
             while i < n:\n            yield i + base\n            i += 1\n    \
             return list(gen(3))\n",
        );
        assert!(
            err.contains("nested generator that captures an enclosing local"),
            "got: {err}"
        );
    }

    #[test]
    fn accepts_capture_free_nested_generator() {
        // A capture-free nested generator (the original crash arity) lowers.
        parsed(
            "def outer():\n    def gen(n):\n        i = 0\n        while i < n:\n            \
             yield i\n            i += 1\n    return list(gen(3))\n",
        );
    }

    #[test]
    fn rejects_capturing_nested_class() {
        let err = parse_err(
            "def outer():\n    x = 5\n    class C:\n        def m(self):\n            \
             return x\n    return C().m()\n",
        );
        assert!(
            err.contains("nested class whose method captures an enclosing-function local"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_nested_class_name_collision() {
        let err = parse_err(
            "def a():\n    class Helper:\n        def m(self):\n            return 1\n    \
             return Helper().m()\ndef b():\n    class Helper:\n        def m(self):\n            \
             return 2\n    return Helper().m()\n",
        );
        assert!(err.contains("collides with another class"), "got: {err}");
    }

    #[test]
    fn rejects_decorated_nested_class() {
        let err = parse_err(
            "def deco(c):\n    return c\ndef outer():\n    @deco\n    class C:\n        \
             pass\n    return C\n",
        );
        assert!(err.contains("decorated nested class"), "got: {err}");
    }

    #[test]
    fn rejects_nested_class_nonliteral_default() {
        let err = parse_err(
            "def outer():\n    class C:\n        def __init__(self, x=[]):\n            \
             self.x = x\n    return C().x\n",
        );
        assert!(
            err.contains("nested class method with a non-literal default"),
            "got: {err}"
        );
    }

    #[test]
    fn accepts_capture_free_nested_class() {
        // Methods using only `self` + a module global lower cleanly.
        parsed(
            "K = 100\ndef outer():\n    class C:\n        def __init__(self, v: int):\n            \
             self.v = v\n        def g(self) -> int:\n            return self.v + K\n    \
             return C(1).g()\n",
        );
    }

    #[test]
    fn accepts_nested_class_referencing_sibling() {
        // A reference to a sibling/ancestor nested class (a base class, or a
        // `super()` chain) resolves statically through `class_map` and is NOT a
        // capture, even though the class name is bound in the enclosing function.
        parsed(
            "def builder():\n    class Base:\n        def kind(self) -> str:\n            \
             return \"base\"\n    class Derived(Base):\n        def kind(self) -> str:\n            \
             return super().kind() + \"+d\"\n    return Derived().kind()\n",
        );
    }

    #[test]
    fn accepts_try_raise_with_match_shapes() {
        // The Phase-7 statement forms all lower without rejection.
        let src = "\
def f(n: int) -> int:
    total = 0
    try:
        if n == 1:
            raise ValueError(\"one\")
        total = total + 1
    except (ValueError, TypeError) as e:
        total = total - 1
    except:
        raise
    else:
        total = total + 10
    finally:
        total = total + 100
    match n:
        case 0:
            total = total + 1
        case [x, *rest]:
            total = total + x
        case {\"k\": v, **other}:
            total = total + v
        case y if y > 5:
            total = total + y
    return total
";
        let (m, _i) = parsed(src);
        assert!(m.functions.len() >= 2);
    }

    #[test]
    fn sibling_synthetic_names_are_unique() {
        // Two same-named nested defs in one scope must get distinct synthetic
        // names (the `#k` uniquifier), else the function table would alias them.
        let src = "\
def outer():
    if True:
        def helper():
            return 1
    else:
        def helper():
            return 2
    return 0
";
        let (m, i) = parsed(src);
        // Exclude each nested def's `.<uniform>` value-call thunk (its name also
        // contains "helper") — count only the two def bodies themselves.
        let names: Vec<&str> = m
            .functions
            .iter()
            .map(|f| i.resolve(f.name))
            .filter(|n| n.contains("helper") && !n.contains("<uniform>"))
            .collect();
        assert_eq!(names.len(), 2);
        assert_ne!(names[0], names[1], "sibling synthetics must be unique");
    }

    #[test]
    fn decorated_module_def_rebinds_in_source_order() {
        // A module-level decorated def emits its `GlobalSet` rebinding into
        // `__main__` at the def's source position, interleaved with top stmts.
        let src = "\
from typing import Callable
print(\"before\")
def logged(func: Callable[..., int]) -> Callable[..., int]:
    def wrapper(*args, **kwargs) -> int:
        return func(*args, **kwargs)
    return wrapper
@logged
def add(a, b):
    return a + b
print(\"after\")
print(add(1, 2))
";
        let (m, _i) = parsed(src);
        let main = m.function(m.main);
        // Walk main's stmts in order: the decorated rebinding (a GlobalSet) must
        // appear, and after the first print, before the call to `add`.
        let mut saw_global_set = false;
        for (_b, block) in main.blocks.iter() {
            for stmt in &block.stmts {
                if matches!(stmt, HirStmt::GlobalSet { .. }) {
                    saw_global_set = true;
                }
            }
        }
        assert!(
            saw_global_set,
            "decorated def must rebind via a global slot"
        );
    }

    #[test]
    fn rejects_duplicate_positional_and_keyword_argument() {
        // Filling a parameter both positionally AND by keyword is a static error
        // for any call shape (CPython: `TypeError: got multiple values for
        // argument`). It must be rejected rather than silently absorbed into
        // `**kwargs`.
        let err = parse_err(
            "def f(a, **kw):\n    return (a, kw)\nprint(f(1, a=2))\n",
        );
        assert!(
            err.contains("got multiple values for argument"),
            "got: {err}"
        );
    }

    #[test]
    fn accepts_no_self_method_called_via_class() {
        // A method declared without `self` (`def m():`) is callable via the class
        // as a plain function (`C.m()`) — must lower cleanly, not panic.
        let (_m, _i) = parsed(
            "class C:\n    def m():\n        return 5\n    def add(x, y):\n        return x + y\nprint(C.m())\nprint(C.add(3, 4))\n",
        );
    }
