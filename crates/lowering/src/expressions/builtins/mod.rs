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
            hir::Builtin::FmtBin => self.lower_fmt_int(
                args,
                hir_module,
                mir_func,
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_FMT_BIN),
            ),
            hir::Builtin::FmtHex => self.lower_fmt_int(
                args,
                hir_module,
                mir_func,
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_FMT_HEX),
            ),
            hir::Builtin::FmtHexUpper => self.lower_fmt_int(
                args,
                hir_module,
                mir_func,
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_FMT_HEX_UPPER),
            ),
            hir::Builtin::FmtOct => self.lower_fmt_int(
                args,
                hir_module,
                mir_func,
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_FMT_OCT),
            ),
            hir::Builtin::FmtIntGrouped => self.lower_fmt_int_grouped(args, hir_module, mir_func),
            hir::Builtin::FmtFloatGrouped => {
                self.lower_fmt_float_grouped(args, hir_module, mir_func)
            }
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
