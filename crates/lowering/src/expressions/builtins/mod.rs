//! Built-in function expression lowering: print, len, str, int, float, bool, abs, pow, min, max, etc.
//!
//! This module handles lowering of all built-in function calls from HIR to MIR.
//! It is organized into submodules by functionality:
//! - `print`: print() function
//! - `conversions`: str(), int(), float(), bool(), bytes(), chr(), ord()
//! - `math`: abs(), pow(), round(), min(), max(), sum()
//! - `predicates`: all(), any()
//! - `introspection`: isinstance(), hash(), id()
//! - `iteration`: iter(), next(), reversed(), sorted()
//! - `collections`: len(), set()

mod collections;
mod conversions;
mod file;
mod introspection;
mod iteration;
mod math;
mod predicates;
mod print;
mod reductions;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Dev-time guard against selection-builtin handler/seed drift.
    ///
    /// `min` / `max` / `sorted` *select* (or collect) elements from their
    /// iterable, so the result local's logical type must stay *comparable*
    /// with the seed authority's view of the whole-call type
    /// (`extract_iterable_element_type` for `min`/`max`; `list[elem]` for
    /// `sorted`). A handler that re-derives a primitive result type
    /// incomparable with the seed — e.g. an `Int` result local over a `Str`
    /// iterable, the exact historical min/max-over-heap bug — trips this
    /// assertion.
    ///
    /// `sum` is intentionally **exempt**: its `Tagged`/`Int` accumulator
    /// legitimately differs from the element seed (see
    /// `classify_reduction_elem`).
    ///
    /// Debug-only: a pure consistency check between two already-computed
    /// types, no MIR effect. Precision differences (Any/Never/Var/Union and
    /// the numeric tower) never fire — only genuinely disjoint types do.
    #[cfg(debug_assertions)]
    pub(crate) fn debug_assert_selection_builtin_seed(
        &self,
        builtin: &hir::Builtin,
        call_expr: &hir::Expr,
        result: &mir::Operand,
        hir_module: &hir::Module,
        mir_func: &mir::Function,
    ) {
        use crate::type_planning::infer::SeedMode;
        use pyaot_types::Type;

        if !matches!(
            builtin,
            hir::Builtin::Min | hir::Builtin::Max | hir::Builtin::Sorted
        ) {
            return;
        }

        // `imprecise` types are top/bottom/placeholder/widened — a difference
        // there is a precision gap, never drift.
        fn imprecise(t: &Type) -> bool {
            matches!(t, Type::Any | Type::Never | Type::Var(_)) || t.is_union()
        }
        // Comparable up to subtyping; for same-kind list results unwrap one
        // level (is_subtype_of does not uniformly model container covariance,
        // and the bug class is about the *element* primitive type anyway).
        fn drift(result_ty: &Type, seed_ty: &Type) -> bool {
            if imprecise(result_ty) || imprecise(seed_ty) {
                return false;
            }
            if let (Some(a), Some(b)) = (result_ty.list_elem(), seed_ty.list_elem()) {
                return drift(a, b);
            }
            !result_ty.is_subtype_of(seed_ty) && !seed_ty.is_subtype_of(result_ty)
        }

        let result_ty = self.operand_type(result, mir_func);
        // Seed authority for the whole call (Lowering shell, the same view
        // the handlers seed their result locals from).
        let seed_ty = self.arm_dispatch(call_expr, hir_module, SeedMode::Lowering);
        debug_assert!(
            !drift(&result_ty, &seed_ty),
            "selection-builtin seed drift: {:?} result local typed {:?}, but the seed authority \
             says {:?} (incomparable) at {:?}",
            builtin,
            result_ty,
            seed_ty,
            call_expr.span,
        );
    }

    /// Lower a built-in function call expression.
    pub(crate) fn lower_builtin_call(
        &mut self,
        builtin: &hir::Builtin,
        args: &[hir::ExprId],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        match builtin {
            hir::Builtin::Print => self.lower_print(args, kwargs, hir_module, mir_func),
            hir::Builtin::Range => {
                // Range is handled specially by for-loop CFG lowering.
                // If it appears as a standalone expression, just return None.
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            hir::Builtin::Len => self.lower_len(args, hir_module, mir_func),
            hir::Builtin::Str => self.lower_str(args, hir_module, mir_func),
            hir::Builtin::Int => self.lower_int(args, hir_module, mir_func),
            hir::Builtin::Float => self.lower_float(args, hir_module, mir_func),
            hir::Builtin::Bool => self.lower_bool(args, hir_module, mir_func),
            hir::Builtin::Bytes => self.lower_bytes(args, hir_module, mir_func),
            hir::Builtin::Abs => self.lower_abs(args, hir_module, mir_func),
            hir::Builtin::BuiltinException(_) => {
                // Exception builtins - used in raise statements
                // When used as expression, we just return None since exceptions
                // are handled specially in raise statement lowering
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            hir::Builtin::Pow => self.lower_pow(args, hir_module, mir_func),
            hir::Builtin::Min => {
                self.lower_minmax_builtin(args, kwargs, hir_module, mir_func, true)
            }
            hir::Builtin::Max => {
                self.lower_minmax_builtin(args, kwargs, hir_module, mir_func, false)
            }
            hir::Builtin::Round => self.lower_round(args, hir_module, mir_func),
            hir::Builtin::Chr => self.lower_chr(args, hir_module, mir_func),
            hir::Builtin::Ord => self.lower_ord(args, hir_module, mir_func),
            hir::Builtin::Sum => self.lower_sum(args, hir_module, mir_func),
            hir::Builtin::All => self.lower_all(args, hir_module, mir_func),
            hir::Builtin::Any => self.lower_any(args, hir_module, mir_func),
            hir::Builtin::Isinstance => self.lower_isinstance(args, hir_module, mir_func),
            hir::Builtin::Issubclass => self.lower_issubclass(args, hir_module, mir_func),
            hir::Builtin::Hash => self.lower_hash(args, hir_module, mir_func),
            hir::Builtin::Id => self.lower_id(args, hir_module, mir_func),
            hir::Builtin::Iter => self.lower_iter(args, hir_module, mir_func),
            hir::Builtin::Next => self.lower_next(args, hir_module, mir_func),
            hir::Builtin::Reversed => self.lower_reversed(args, hir_module, mir_func),
            hir::Builtin::Sorted => self.lower_sorted(args, kwargs, hir_module, mir_func),
            hir::Builtin::Set => self.lower_set_builtin(args, hir_module, mir_func),
            hir::Builtin::Open => self.lower_open(args, kwargs, hir_module, mir_func),
            hir::Builtin::Enumerate => self.lower_enumerate(args, kwargs, hir_module, mir_func),
            // Phase 1: Quick Wins
            hir::Builtin::Divmod => self.lower_divmod(args, hir_module, mir_func),
            hir::Builtin::Input => self.lower_input(args, hir_module, mir_func),
            hir::Builtin::Bin => self.lower_bin(args, hir_module, mir_func),
            hir::Builtin::Hex => self.lower_hex(args, hir_module, mir_func),
            hir::Builtin::Oct => self.lower_oct(args, hir_module, mir_func),
            hir::Builtin::Repr => self.lower_repr(args, hir_module, mir_func),
            hir::Builtin::Ascii => self.lower_ascii(args, hir_module, mir_func),
            // Phase 5: Introspection
            hir::Builtin::Type => self.lower_type(args, hir_module, mir_func),
            hir::Builtin::Callable => self.lower_callable(args, hir_module, mir_func),
            hir::Builtin::Hasattr => self.lower_hasattr(args, hir_module, mir_func),
            hir::Builtin::Getattr => self.lower_getattr(args, hir_module, mir_func),
            hir::Builtin::Setattr => self.lower_setattr(args, hir_module, mir_func),
            // Phase 4: Iterators
            hir::Builtin::Zip => self.lower_zip(args, hir_module, mir_func),
            hir::Builtin::Map => self.lower_map(args, hir_module, mir_func),
            hir::Builtin::Filter => self.lower_filter(args, hir_module, mir_func),
            // Collection constructors
            hir::Builtin::List => self.lower_list_builtin(args, hir_module, mir_func),
            hir::Builtin::Tuple => self.lower_tuple_builtin(args, hir_module, mir_func),
            hir::Builtin::Dict => self.lower_dict_builtin(args, kwargs, hir_module, mir_func),
            hir::Builtin::DefaultDict => self.lower_defaultdict(args, hir_module, mir_func),
            hir::Builtin::Counter => self.lower_counter(args, hir_module, mir_func),
            hir::Builtin::Deque => self.lower_deque(args, hir_module, mir_func),
            hir::Builtin::Format => self.lower_format(args, hir_module, mir_func),
            hir::Builtin::ObjectNew => self.lower_object_new(args, hir_module, mir_func),
            hir::Builtin::Reduce => self.lower_reduce(args, hir_module, mir_func),
            // itertools
            hir::Builtin::Chain => self.lower_chain(args, hir_module, mir_func),
            hir::Builtin::ISlice => self.lower_islice(args, hir_module, mir_func),
        }
    }
}
