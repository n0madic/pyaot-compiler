//! Builtin function type inference

use pyaot_hir::{Builtin, ExprId, Module};
use pyaot_types::Type;

use crate::context::TypeChecker;

impl<'a> TypeChecker<'a> {
    /// Infer type of builtin function call
    pub(crate) fn infer_builtin_type(
        &mut self,
        builtin: Builtin,
        args: &[ExprId],
        kwargs: &[pyaot_hir::KeywordArg],
        module: &Module,
    ) -> Type {
        match builtin {
            Builtin::Print => Type::None,
            Builtin::Len => Type::Int,
            Builtin::Str => Type::Str,
            Builtin::Int => Type::Int,
            Builtin::Float => Type::Float,
            Builtin::Bool => Type::Bool,
            Builtin::Bytes => Type::Bytes,
            Builtin::Abs => Type::Any, // Returns same type as argument
            Builtin::Pow => Type::Float,
            Builtin::Min => Type::Any,   // Returns same type as arguments
            Builtin::Max => Type::Any,   // Returns same type as arguments
            Builtin::Round => Type::Any, // Int if 1 arg, Float if 2 args
            Builtin::Sum => Type::Any,   // Type depends on list elements
            Builtin::All => Type::Bool,
            Builtin::Any => Type::Bool,
            Builtin::Chr => Type::Str,
            Builtin::Ord => Type::Int,
            Builtin::Isinstance => Type::Bool,
            Builtin::Issubclass => Type::Bool,
            Builtin::Hash => Type::Int,
            Builtin::Id => Type::Int,
            Builtin::Range => {
                // range() returns an iterator of integers
                Type::Iterator(Box::new(Type::Int))
            }
            Builtin::BuiltinException(_) => {
                // Exception builtins return exception objects
                Type::Any
            }
            Builtin::Iter => {
                // iter() returns an iterator over the container's elements
                if args.is_empty() {
                    return Type::Iterator(Box::new(Type::Any));
                }
                let arg_type = self.infer_expr_type(args[0], module);
                let elem_type = self.extract_iterable_element_type(&arg_type);
                Type::Iterator(Box::new(elem_type))
            }
            Builtin::Next => {
                // next() returns the next element from an iterator
                Type::Any
            }
            Builtin::Reversed => {
                // reversed() returns a reverse iterator over the container's elements
                if args.is_empty() {
                    return Type::Iterator(Box::new(Type::Any));
                }
                let arg_type = self.infer_expr_type(args[0], module);
                let elem_type = self.extract_iterable_element_type(&arg_type);
                Type::Iterator(Box::new(elem_type))
            }
            Builtin::Sorted => {
                // sorted() returns a sorted list from any iterable
                if args.is_empty() {
                    return Type::List(Box::new(Type::Any));
                }
                let arg_type = self.infer_expr_type(args[0], module);
                let elem_type = self.extract_iterable_element_type(&arg_type);
                Type::List(Box::new(elem_type))
            }
            Builtin::Set => {
                // set() or set(iterable)
                if args.is_empty() {
                    return Type::Set(Box::new(Type::Any));
                }
                let arg_type = self.infer_expr_type(args[0], module);
                let elem_type = self.extract_iterable_element_type(&arg_type);
                Type::Set(Box::new(elem_type))
            }
            Builtin::Open => {
                // open() returns a file object
                Type::File
            }
            Builtin::Enumerate => {
                // enumerate() returns an iterator of (index, element) tuples
                if args.is_empty() {
                    return Type::Iterator(Box::new(Type::Tuple(vec![Type::Int, Type::Any])));
                }
                let arg_type = self.infer_expr_type(args[0], module);
                let elem_type = self.extract_iterable_element_type(&arg_type);
                Type::Iterator(Box::new(Type::Tuple(vec![Type::Int, elem_type])))
            }
            // Phase 1: Quick Wins
            Builtin::Divmod => {
                // divmod(a, b) returns (quotient, remainder)
                Type::Tuple(vec![Type::Int, Type::Int])
            }
            Builtin::Input => {
                // input() reads from stdin and returns a string
                Type::Str
            }
            Builtin::Bin
            | Builtin::Hex
            | Builtin::Oct
            | Builtin::FmtBin
            | Builtin::FmtHex
            | Builtin::FmtHexUpper
            | Builtin::FmtOct => {
                // bin/hex/oct and format-specific variants convert int to string
                Type::Str
            }
            Builtin::Repr => {
                // repr() returns string representation
                Type::Str
            }
            Builtin::Ascii => {
                // ascii() returns string representation with non-ASCII escaped
                Type::Str
            }
            Builtin::Format => {
                // format(value, format_spec='') returns formatted string
                Type::Str
            }
            Builtin::Reduce => {
                // reduce(func, iterable, initial?) returns accumulated value
                Type::Any
            }
            // Phase 5: Introspection
            Builtin::Type => {
                // type() returns a string (simplified implementation)
                Type::Str
            }
            Builtin::Callable => {
                // callable() returns a bool
                Type::Bool
            }
            Builtin::Hasattr => {
                // hasattr() returns a bool
                Type::Bool
            }
            Builtin::Getattr => {
                // getattr() returns Any (attribute value)
                Type::Any
            }
            Builtin::Setattr => {
                // setattr() returns None
                Type::None
            }
            // Phase 4: Iterators
            Builtin::Zip => {
                // zip() returns an iterator of tuples
                if args.is_empty() {
                    return Type::Iterator(Box::new(Type::Tuple(vec![])));
                }
                // Infer tuple element types from each argument
                let elem_types: Vec<Type> = args
                    .iter()
                    .map(|arg| {
                        let arg_type = self.infer_expr_type(*arg, module);
                        self.extract_iterable_element_type(&arg_type)
                    })
                    .collect();
                Type::Iterator(Box::new(Type::Tuple(elem_types)))
            }
            Builtin::Map => {
                // map() returns an iterator over transformed elements
                Type::Iterator(Box::new(Type::Any))
            }
            Builtin::Filter => {
                // filter(func, iterable) returns an iterator over filtered elements
                // Element type comes from the second argument (the iterable)
                if args.len() < 2 {
                    return Type::Iterator(Box::new(Type::Any));
                }
                let iterable_type = self.infer_expr_type(args[1], module);
                let elem_type = self.extract_iterable_element_type(&iterable_type);
                Type::Iterator(Box::new(elem_type))
            }
            // Collection constructors
            Builtin::List => {
                // list() or list(iterable)
                if args.is_empty() {
                    return Type::List(Box::new(Type::Any));
                }
                let arg_type = self.infer_expr_type(args[0], module);
                let elem_type = self.extract_iterable_element_type(&arg_type);
                Type::List(Box::new(elem_type))
            }
            Builtin::Tuple => {
                // tuple() or tuple(iterable)
                if args.is_empty() {
                    return Type::Tuple(vec![]);
                }
                let arg_type = self.infer_expr_type(args[0], module);
                let elem_type = self.extract_iterable_element_type(&arg_type);
                // For dynamic tuple from iterable, use vec![elem_type] as placeholder
                Type::Tuple(vec![elem_type])
            }
            Builtin::Dict => {
                // dict() or dict(iterable) or dict(**kwargs)
                if args.is_empty() && kwargs.is_empty() {
                    return Type::Dict(Box::new(Type::Any), Box::new(Type::Any));
                }
                // dict(a=1, b=2) -> dict[str, int]
                if !kwargs.is_empty() {
                    let value_types: Vec<Type> = kwargs
                        .iter()
                        .map(|kw| self.infer_expr_type(kw.value, module))
                        .collect();
                    let value_type = Type::normalize_union(value_types);
                    return Type::Dict(Box::new(Type::Str), Box::new(value_type));
                }
                // dict(iterable) - complex to infer pair types, use Any
                Type::Dict(Box::new(Type::Any), Box::new(Type::Any))
            }
            Builtin::Chain => {
                // itertools.chain returns an iterator
                Type::Iterator(Box::new(Type::Any))
            }
            Builtin::ISlice => {
                // itertools.islice returns an iterator
                Type::Iterator(Box::new(Type::Any))
            }
        }
    }
}
