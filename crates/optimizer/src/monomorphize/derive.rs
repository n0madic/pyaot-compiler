//! Substitution derivation for monomorphization.
//!
//! `derive_subst` walks a pair of (template parameter type, concrete call-arg type)
//! lists in lockstep, building a map from TypeVar names to concrete types.

use std::collections::HashMap;

use pyaot_types::Type;
use pyaot_utils::InternedString;

/// Derive a substitution map from template parameter types and concrete argument types.
///
/// For each `(Var(name), concrete)` pair, inserts `name → concrete` into the map.
/// Recurses into `Generic{base,args}` pairs element-wise when bases match.
/// Returns `None` when derivation fails (mismatch, Var on the call-arg side, etc.).
pub fn derive_subst(
    template_params: &[Type],
    call_arg_types: &[Type],
) -> Option<HashMap<InternedString, Type>> {
    if template_params.len() != call_arg_types.len() {
        return None;
    }
    let mut subst = HashMap::new();
    for (param, arg) in template_params.iter().zip(call_arg_types.iter()) {
        unify_into_subst(param, arg, &mut subst)?;
    }
    Some(subst)
}

/// Recursively unify one `(template_type, concrete_type)` pair into `subst`.
///
/// Returns `None` on conflict (same Var bound to two different concrete types)
/// or if the structure is incompatible (e.g., Generic base mismatch).
fn unify_into_subst(
    template_ty: &Type,
    concrete_ty: &Type,
    subst: &mut HashMap<InternedString, Type>,
) -> Option<()> {
    match template_ty {
        Type::Var(name) => {
            if let Some(existing) = subst.get(name) {
                if existing != concrete_ty {
                    return None;
                }
            } else {
                subst.insert(*name, concrete_ty.clone());
            }
            Some(())
        }
        Type::Generic { base, args } => {
            if let Type::Generic {
                base: base2,
                args: args2,
            } = concrete_ty
            {
                if base != base2 || args.len() != args2.len() {
                    return None;
                }
                for (a, b) in args.iter().zip(args2.iter()) {
                    unify_into_subst(a, b, subst)?;
                }
                Some(())
            } else {
                None
            }
        }
        Type::Union(ts) => {
            if let Type::Union(ts2) = concrete_ty {
                if ts.len() != ts2.len() {
                    return None;
                }
                for (a, b) in ts.iter().zip(ts2.iter()) {
                    unify_into_subst(a, b, subst)?;
                }
                Some(())
            } else {
                None
            }
        }
        Type::Iterator(inner) => {
            if let Type::Iterator(inner2) = concrete_ty {
                unify_into_subst(inner, inner2, subst)
            } else {
                None
            }
        }
        Type::DefaultDict(k, v) => {
            if let Type::DefaultDict(k2, v2) = concrete_ty {
                unify_into_subst(k, k2, subst)?;
                unify_into_subst(v, v2, subst)
            } else {
                None
            }
        }
        // For non-parameterized types: they must match exactly (no Vars to bind).
        _ => {
            if template_ty == concrete_ty {
                Some(())
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_types::Type;
    use pyaot_utils::StringInterner;

    struct Ctx {
        interner: StringInterner,
    }

    impl Ctx {
        fn new() -> Self {
            Self {
                interner: StringInterner::default(),
            }
        }

        fn intern(&mut self, s: &str) -> InternedString {
            self.interner.intern(s)
        }

        fn var(&mut self, s: &str) -> Type {
            Type::Var(self.intern(s))
        }
    }

    #[test]
    fn test_derive_subst_simple() {
        let mut ctx = Ctx::new();
        let t = ctx.intern("T");
        let params = vec![Type::Var(t)];
        let args = vec![Type::Int];
        let subst = derive_subst(&params, &args).unwrap();
        assert_eq!(subst[&t], Type::Int);
    }

    #[test]
    fn test_derive_subst_multiple_vars() {
        let mut ctx = Ctx::new();
        let t = ctx.intern("T");
        let u = ctx.intern("U");
        let params = vec![Type::Var(t), Type::Var(u)];
        let args = vec![Type::Int, Type::Str];
        let subst = derive_subst(&params, &args).unwrap();
        assert_eq!(subst[&t], Type::Int);
        assert_eq!(subst[&u], Type::Str);
    }

    #[test]
    fn test_derive_subst_repeated_var_consistent() {
        let mut ctx = Ctx::new();
        let t = ctx.intern("T");
        let params = vec![Type::Var(t), Type::Var(t)];
        let args = vec![Type::Int, Type::Int];
        let subst = derive_subst(&params, &args).unwrap();
        assert_eq!(subst[&t], Type::Int);
    }

    #[test]
    fn test_derive_subst_repeated_var_conflict() {
        let mut ctx = Ctx::new();
        let t = ctx.intern("T");
        let params = vec![Type::Var(t), Type::Var(t)];
        let args = vec![Type::Int, Type::Str];
        assert!(derive_subst(&params, &args).is_none());
    }

    #[test]
    fn test_derive_subst_nested_generic() {
        let mut ctx = Ctx::new();
        let t = ctx.intern("T");
        let params = vec![Type::list_of(Type::Var(t))];
        let args = vec![Type::list_of(Type::Str)];
        let subst = derive_subst(&params, &args).unwrap();
        assert_eq!(subst[&t], Type::Str);
    }

    #[test]
    fn test_derive_subst_length_mismatch() {
        let mut ctx = Ctx::new();
        let params = vec![ctx.var("T")];
        let args = vec![Type::Int, Type::Str];
        assert!(derive_subst(&params, &args).is_none());
    }

    #[test]
    fn test_derive_subst_concrete_match() {
        let params = vec![Type::Int];
        let args = vec![Type::Int];
        let subst = derive_subst(&params, &args).unwrap();
        assert!(subst.is_empty());
    }

    #[test]
    fn test_derive_subst_concrete_mismatch() {
        let params = vec![Type::Int];
        let args = vec![Type::Float];
        assert!(derive_subst(&params, &args).is_none());
    }

    #[test]
    fn test_derive_subst_generic_base_mismatch() {
        let mut ctx = Ctx::new();
        let t = ctx.intern("T");
        let list_t = Type::list_of(Type::Var(t));
        let set_int = Type::set_of(Type::Int);
        assert!(derive_subst(&[list_t], &[set_int]).is_none());
    }
}
