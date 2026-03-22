//! Type inference for HIR expressions
//!
//! This module provides type inference for HIR expressions with memoization.
//! The `get_type_of_expr_id` method is the cached entry point that should be used
//! when an ExprId is available. For cases where only an expression reference is
//! available, `get_expr_type` can be used directly (it still benefits from caching
//! on recursive calls).

use pyaot_hir as hir;
use pyaot_stdlib_defs::{lookup_object_field, lookup_object_type, ALL_OBJECT_TYPES};
use pyaot_types::{typespec_to_type, Type};

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Get the type of an expression by its ID (cached).
    ///
    /// This is the preferred entry point when an ExprId is available,
    /// as it leverages memoization to avoid redundant type computations.
    pub(crate) fn get_type_of_expr_id(
        &self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
    ) -> Type {
        // Check cache first
        if let Some(cached) = self.get_cached_expr_type(&expr_id) {
            return cached;
        }

        // Compute type and cache it
        let expr = &hir_module.exprs[expr_id];
        let result = self.compute_expr_type(expr, hir_module);

        self.cache_expr_type(expr_id, result.clone());
        result
    }

    /// Get the effective type of an expression, considering tracked var_types.
    ///
    /// This method computes the type without caching at the top level.
    /// Use `get_type_of_expr_id` when an ExprId is available for better performance.
    pub(crate) fn get_expr_type(&self, expr: &hir::Expr, hir_module: &hir::Module) -> Type {
        self.compute_expr_type(expr, hir_module)
    }

    /// Internal type computation (the actual inference logic).
    fn compute_expr_type(&self, expr: &hir::Expr, hir_module: &hir::Module) -> Type {
        match &expr.kind {
            hir::ExprKind::Var(var_id) => self
                .get_var_type(var_id)
                .cloned()
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Any),
            hir::ExprKind::BinOp { op, left, right } => {
                let left_ty = self.get_type_of_expr_id(*left, hir_module);
                let right_ty = self.get_type_of_expr_id(*right, hir_module);

                // Class types with arithmetic dunders return the class type
                if matches!(left_ty, Type::Class { .. }) {
                    return left_ty;
                }

                // Set operations (|, &, -, ^) return Set type
                if let Type::Set(elem_ty) = &left_ty {
                    match op {
                        hir::BinOp::BitOr
                        | hir::BinOp::BitAnd
                        | hir::BinOp::Sub
                        | hir::BinOp::BitXor => return Type::Set(elem_ty.clone()),
                        _ => {}
                    }
                }

                // List concatenation (+) returns List type
                if let Type::List(elem_ty) = &left_ty {
                    if matches!(op, hir::BinOp::Add) {
                        return Type::List(elem_ty.clone());
                    }
                }

                // Dict merge (|) returns Dict type
                if let Type::Dict(key_ty, val_ty) = &left_ty {
                    if matches!(op, hir::BinOp::BitOr) {
                        return Type::Dict(key_ty.clone(), val_ty.clone());
                    }
                }

                // Python 3: true division (/) always returns float
                if matches!(op, hir::BinOp::Div) {
                    return Type::Float;
                }

                // String operations return strings
                if matches!(left_ty, Type::Str) {
                    match op {
                        hir::BinOp::Add | hir::BinOp::Mul => Type::Str,
                        _ => expr.ty.clone().unwrap_or(Type::Any),
                    }
                } else if matches!(left_ty, Type::Float) || matches!(right_ty, Type::Float) {
                    // Float operations return Float
                    Type::Float
                } else if matches!(left_ty, Type::Int) && matches!(right_ty, Type::Int) {
                    // Integer operations (except Div which is handled above)
                    Type::Int
                } else {
                    expr.ty.clone().unwrap_or(Type::Any)
                }
            }
            hir::ExprKind::MethodCall { obj, method, .. } => {
                let raw_obj_ty = self.get_type_of_expr_id(*obj, hir_module);
                let method_name = self.resolve(*method);
                // Unwrap Optional[T] (Union[T, None]) → T so method dispatch works
                let obj_ty = match &raw_obj_ty {
                    Type::Union(variants)
                        if variants.len() == 2 && variants.contains(&Type::None) =>
                    {
                        variants
                            .iter()
                            .find(|t| **t != Type::None)
                            .cloned()
                            .unwrap_or(raw_obj_ty)
                    }
                    _ => raw_obj_ty,
                };
                match &obj_ty {
                    Type::Str => match method_name {
                        // String transformation methods
                        "upper" | "lower" | "strip" | "lstrip" | "rstrip" | "replace" | "title"
                        | "capitalize" | "swapcase" | "center" | "ljust" | "rjust" | "zfill"
                        | "join" | "removeprefix" | "removesuffix" | "expandtabs" => Type::Str,
                        // Methods returning list
                        "split" | "splitlines" => Type::List(Box::new(Type::Str)),
                        // Methods returning tuple
                        "partition" | "rpartition" => {
                            Type::Tuple(vec![Type::Str, Type::Str, Type::Str])
                        }
                        // Boolean predicates
                        "startswith" | "endswith" | "isdigit" | "isalpha" | "isalnum"
                        | "isspace" | "isupper" | "islower" => Type::Bool,
                        // Integer methods
                        "find" | "count" => Type::Int,
                        _ => expr.ty.clone().unwrap_or(Type::Any),
                    },
                    Type::List(elem_ty) => match method_name {
                        "pop" => (**elem_ty).clone(),
                        "copy" => Type::List(elem_ty.clone()),
                        "index" | "count" => Type::Int,
                        "append" | "insert" | "remove" | "clear" | "reverse" => Type::None,
                        _ => expr.ty.clone().unwrap_or(Type::Any),
                    },
                    Type::Dict(key_ty, value_ty) => match method_name {
                        "get" | "pop" | "setdefault" => (**value_ty).clone(),
                        "copy" => Type::Dict(key_ty.clone(), value_ty.clone()),
                        "keys" => Type::List(key_ty.clone()),
                        "values" => Type::List(value_ty.clone()),
                        "items" => {
                            let tuple_ty =
                                Type::Tuple(vec![(**key_ty).clone(), (**value_ty).clone()]);
                            Type::List(Box::new(tuple_ty))
                        }
                        "popitem" => Type::Tuple(vec![(**key_ty).clone(), (**value_ty).clone()]),
                        "clear" | "update" => Type::None,
                        _ => expr.ty.clone().unwrap_or(Type::Any),
                    },
                    Type::Set(elem_ty) => match method_name {
                        "copy"
                        | "union"
                        | "intersection"
                        | "difference"
                        | "symmetric_difference" => Type::Set(elem_ty.clone()),
                        "add" | "remove" | "discard" | "clear" => Type::None,
                        "issubset" | "issuperset" | "isdisjoint" => Type::Bool,
                        _ => expr.ty.clone().unwrap_or(Type::Any),
                    },
                    Type::File => match method_name {
                        "read" | "readline" => Type::Str, // Text mode returns str
                        "readlines" => Type::List(Box::new(Type::Str)),
                        "write" => Type::Int,
                        "close" | "flush" => Type::None,
                        _ => expr.ty.clone().unwrap_or(Type::Any),
                    },
                    Type::Class { ref class_id, .. } => {
                        // Get method return type from class info
                        if let Some(class_info) = self.get_class_info(class_id) {
                            // Check all method categories: instance, class, and static methods
                            let method_maps = [
                                &class_info.method_funcs,
                                &class_info.class_methods,
                                &class_info.static_methods,
                            ];
                            for methods in method_maps {
                                if let Some(&method_func_id) = methods.get(method) {
                                    if let Some(func_def) =
                                        hir_module.func_defs.get(&method_func_id)
                                    {
                                        return func_def.return_type.clone().unwrap_or(Type::None);
                                    }
                                }
                            }
                        }
                        expr.ty.clone().unwrap_or(Type::Any)
                    }
                    // Handle RuntimeObject methods (Match.group(), etc.)
                    // using Single Source of Truth from stdlib-defs
                    Type::RuntimeObject(type_tag) => {
                        if let Some(obj_def) = lookup_object_type(*type_tag) {
                            if let Some(method_def) = obj_def.get_method(method_name) {
                                return typespec_to_type(&method_def.return_type);
                            }
                        }
                        expr.ty.clone().unwrap_or(Type::Any)
                    }
                    // For Type::Any, check object type methods as fallback
                    Type::Any => {
                        // Look up the method in all object types (Single Source of Truth)
                        for obj_def in ALL_OBJECT_TYPES {
                            if let Some(method_def) = obj_def.get_method(method_name) {
                                return typespec_to_type(&method_def.return_type);
                            }
                        }
                        // Fallback to expression annotation or Any
                        expr.ty.clone().unwrap_or(Type::Any)
                    }
                    _ => expr.ty.clone().unwrap_or(Type::Any),
                }
            }
            hir::ExprKind::Slice { obj, .. } => {
                // Slicing preserves the type
                self.get_type_of_expr_id(*obj, hir_module)
            }
            hir::ExprKind::Index { obj, index } => {
                let obj_ty = self.get_type_of_expr_id(*obj, hir_module);
                // Indexing: string returns string (single char), bytes returns int
                match obj_ty {
                    Type::Str => Type::Str,
                    Type::Bytes => Type::Int, // bytes indexing returns an int (0-255)
                    Type::List(elem_ty) => *elem_ty,
                    Type::Tuple(elems) => {
                        // For heterogeneous tuples, try to extract the index value at compile-time
                        // to return the precise element type
                        let index_expr = &hir_module.exprs[*index];
                        if let hir::ExprKind::Int(idx) = &index_expr.kind {
                            // Handle negative indices
                            let len = elems.len() as i64;
                            let actual_idx = if *idx < 0 { len + idx } else { *idx };
                            if actual_idx >= 0 && (actual_idx as usize) < elems.len() {
                                return elems[actual_idx as usize].clone();
                            }
                        }
                        // Fallback: if we can't determine the index at compile-time,
                        // return a union of all element types for heterogeneous tuples,
                        // or the single element type for homogeneous tuples
                        if elems.is_empty() {
                            Type::Any
                        } else if elems.iter().all(|t| t == &elems[0]) {
                            // Homogeneous tuple - all elements have the same type
                            elems[0].clone()
                        } else {
                            // Heterogeneous tuple - return union of all types
                            Type::normalize_union(elems.clone())
                        }
                    }
                    Type::Dict(_, value_ty) => *value_ty,
                    Type::Class { class_id, .. } => {
                        // Class with __getitem__ - return type from the dunder method
                        self.get_class_info(&class_id)
                            .and_then(|info| info.getitem_func)
                            .and_then(|func_id| self.get_func_return_type(&func_id).cloned())
                            .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
                    }
                    _ => expr.ty.clone().unwrap_or(Type::Any),
                }
            }
            hir::ExprKind::List(elements) => {
                // Infer list element type from all elements
                if elements.is_empty() {
                    // For empty lists, prefer the annotated type if available
                    expr.ty.clone().unwrap_or(Type::List(Box::new(Type::Any)))
                } else {
                    let first_ty = self.get_type_of_expr_id(elements[0], hir_module);

                    // Check if all elements have the same type
                    let mut all_same = true;
                    let mut unique_types = vec![first_ty.clone()];
                    for elem_id in &elements[1..] {
                        let elem_ty = self.get_type_of_expr_id(*elem_id, hir_module);
                        if elem_ty != first_ty {
                            all_same = false;
                            if !unique_types.contains(&elem_ty) {
                                unique_types.push(elem_ty);
                            }
                        }
                    }

                    if all_same {
                        Type::List(Box::new(first_ty))
                    } else {
                        Type::List(Box::new(Type::Union(unique_types)))
                    }
                }
            }
            hir::ExprKind::Tuple(elements) => {
                // Infer tuple element types from all elements
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.get_type_of_expr_id(*e, hir_module))
                    .collect();
                Type::Tuple(elem_types)
            }
            hir::ExprKind::Dict(pairs) => {
                // Infer dict key/value types from all pairs
                if pairs.is_empty() {
                    Type::Dict(Box::new(Type::Any), Box::new(Type::Any))
                } else {
                    let (first_key_id, first_val_id) = &pairs[0];
                    let first_key_ty = self.get_type_of_expr_id(*first_key_id, hir_module);
                    let first_val_ty = self.get_type_of_expr_id(*first_val_id, hir_module);

                    let mut key_all_same = true;
                    let mut val_all_same = true;
                    let mut unique_key_types = vec![first_key_ty.clone()];
                    let mut unique_val_types = vec![first_val_ty.clone()];

                    for (key_id, val_id) in &pairs[1..] {
                        let k_ty = self.get_type_of_expr_id(*key_id, hir_module);
                        let v_ty = self.get_type_of_expr_id(*val_id, hir_module);
                        if k_ty != first_key_ty {
                            key_all_same = false;
                            if !unique_key_types.contains(&k_ty) {
                                unique_key_types.push(k_ty);
                            }
                        }
                        if v_ty != first_val_ty {
                            val_all_same = false;
                            if !unique_val_types.contains(&v_ty) {
                                unique_val_types.push(v_ty);
                            }
                        }
                    }

                    let key_ty = if key_all_same {
                        first_key_ty
                    } else {
                        Type::Union(unique_key_types)
                    };
                    let val_ty = if val_all_same {
                        first_val_ty
                    } else {
                        Type::Union(unique_val_types)
                    };
                    Type::Dict(Box::new(key_ty), Box::new(val_ty))
                }
            }
            hir::ExprKind::Set(elements) => {
                // Infer set element type from all elements
                if elements.is_empty() {
                    Type::Set(Box::new(Type::Any))
                } else {
                    let first_ty = self.get_type_of_expr_id(elements[0], hir_module);
                    let mut all_same = true;
                    let mut unique_types = vec![first_ty.clone()];
                    for elem_id in &elements[1..] {
                        let elem_ty = self.get_type_of_expr_id(*elem_id, hir_module);
                        if elem_ty != first_ty {
                            all_same = false;
                            if !unique_types.contains(&elem_ty) {
                                unique_types.push(elem_ty);
                            }
                        }
                    }
                    if all_same {
                        Type::Set(Box::new(first_ty))
                    } else {
                        Type::Set(Box::new(Type::Union(unique_types)))
                    }
                }
            }
            hir::ExprKind::UnOp { op, operand } => match op {
                hir::UnOp::Not => Type::Bool,
                hir::UnOp::Neg => self.get_type_of_expr_id(*operand, hir_module),
                hir::UnOp::Invert => Type::Int, // Bitwise NOT always returns Int
            },
            hir::ExprKind::Call { func, .. } => {
                // Get function return type for direct calls
                let func_expr = &hir_module.exprs[*func];
                if let hir::ExprKind::FuncRef(func_id) = &func_expr.kind {
                    // Check inferred return types first (for generators), then HIR definition
                    if let Some(return_type) = self.get_func_return_type(func_id) {
                        return return_type.clone();
                    }
                    if let Some(func_def) = hir_module.func_defs.get(func_id) {
                        return func_def.return_type.clone().unwrap_or(Type::None);
                    }
                }
                // Check for variable that holds a function reference
                if let hir::ExprKind::Var(var_id) = &func_expr.kind {
                    // Check if this variable holds a class instance with __call__
                    if let Some(Type::Class { class_id, .. }) =
                        self.get_var_type(var_id).cloned().as_ref()
                    {
                        if let Some(call_func_id) = self
                            .get_class_info(class_id)
                            .and_then(|info| info.call_func)
                        {
                            if let Some(return_type) = self.get_func_return_type(&call_func_id) {
                                return return_type.clone();
                            }
                            if let Some(func_def) = hir_module.func_defs.get(&call_func_id) {
                                return func_def.return_type.clone().unwrap_or(Type::None);
                            }
                        }
                    }
                    // Check if this is a function pointer parameter (inside wrapper function)
                    // In this case, use the wrapper function's return type
                    if self.is_func_ptr_param(var_id) {
                        if let Some(return_type) = self.get_current_func_return_type() {
                            return return_type.clone();
                        }
                        return Type::Any;
                    }
                    // Check if this is a wrapped function (decorator that returns wrapper closure)
                    if let Some((wrapper_func_id, _)) = self.get_var_wrapper(var_id) {
                        // Return the wrapper function's return type
                        if let Some(return_type) = self.get_func_return_type(&wrapper_func_id) {
                            return return_type.clone();
                        }
                        if let Some(func_def) = hir_module.func_defs.get(&wrapper_func_id) {
                            return func_def.return_type.clone().unwrap_or(Type::None);
                        }
                    }
                    if let Some(func_id) = self.get_var_func(var_id) {
                        if let Some(return_type) = self.get_func_return_type(&func_id) {
                            return return_type.clone();
                        }
                        if let Some(func_def) = hir_module.func_defs.get(&func_id) {
                            return func_def.return_type.clone().unwrap_or(Type::None);
                        }
                    }
                }
                // Check for class instantiation
                if let hir::ExprKind::ClassRef(class_id) = &func_expr.kind {
                    if let Some(class_def) = hir_module.class_defs.get(class_id) {
                        return Type::Class {
                            class_id: *class_id,
                            name: class_def.name,
                        };
                    }
                }
                // Check for cross-module class instantiation
                if let hir::ExprKind::ModuleAttr { module, attr } = &func_expr.kind {
                    let attr_name = self.resolve(*attr).to_string();
                    let key = (module.clone(), attr_name);
                    if let Some((class_id, _)) = self.get_module_class_export(&key) {
                        // Return proper Type::Class with the remapped ClassId
                        // attr is already the InternedString for the class name
                        return Type::Class {
                            class_id: *class_id,
                            name: *attr,
                        };
                    }
                }
                // Check for cross-module function calls via ImportedRef
                if let hir::ExprKind::ImportedRef { module, name } = &func_expr.kind {
                    let key = (module.clone(), name.clone());
                    if let Some(return_type) = self.get_module_func_export(&key) {
                        return return_type.clone();
                    }
                }
                expr.ty.clone().unwrap_or(Type::Any)
            }
            hir::ExprKind::Attribute { obj, attr } => {
                // Get field type from class definition
                let obj_ty = self.get_type_of_expr_id(*obj, hir_module);

                // Handle RuntimeObject attributes (StructTime, CompletedProcess, Match, etc.)
                // using Single Source of Truth from stdlib-defs
                if let Type::RuntimeObject(type_tag) = &obj_ty {
                    let attr_name = self.resolve(*attr);
                    if let Some(field_def) = lookup_object_field(*type_tag, attr_name) {
                        return typespec_to_type(&field_def.field_type);
                    }
                    return Type::Any;
                }

                // Handle File attributes
                if matches!(obj_ty, Type::File) {
                    let attr_name = self.resolve(*attr);
                    return match attr_name {
                        "closed" => Type::Bool,
                        "name" => Type::Str,
                        _ => Type::Any,
                    };
                }

                if let Type::Class { class_id, .. } = &obj_ty {
                    if let Some(class_info) = self.get_class_info(class_id) {
                        if let Some(field_ty) = class_info.field_types.get(attr) {
                            return field_ty.clone();
                        }
                    }
                }
                expr.ty.clone().unwrap_or(Type::Any)
            }
            hir::ExprKind::ClassRef(class_id) => {
                // ClassRef itself doesn't have a value type (it's used in calls)
                // Return Any since this shouldn't be used as a value directly
                if let Some(class_def) = hir_module.class_defs.get(class_id) {
                    Type::Class {
                        class_id: *class_id,
                        name: class_def.name,
                    }
                } else {
                    Type::Any
                }
            }
            hir::ExprKind::ClassAttrRef { class_id, attr } => {
                // Look up class attribute type from class info
                if let Some(class_info) = self.get_class_info(class_id) {
                    if let Some(attr_type) = class_info.class_attr_types.get(attr) {
                        return attr_type.clone();
                    }
                }
                // Fall back to Any if not found
                Type::Any
            }
            hir::ExprKind::BuiltinCall { builtin, args, .. } => {
                // Infer return types for builtin functions
                match builtin {
                    hir::Builtin::Sum => {
                        // sum(iterable, start=0) -> int | float
                        // Returns float if list element type is float or start is float
                        if args.is_empty() {
                            return Type::Int;
                        }
                        let iterable_type = self.get_type_of_expr_id(args[0], hir_module);
                        let element_type = match &iterable_type {
                            Type::List(elem_ty) => (**elem_ty).clone(),
                            _ => Type::Int,
                        };
                        let start_type = if args.len() > 1 {
                            self.get_type_of_expr_id(args[1], hir_module)
                        } else {
                            Type::Int
                        };
                        if element_type == Type::Float || start_type == Type::Float {
                            Type::Float
                        } else {
                            Type::Int
                        }
                    }
                    hir::Builtin::Len => Type::Int,
                    hir::Builtin::Abs => {
                        // abs() returns the same type as input
                        if !args.is_empty() {
                            self.get_type_of_expr_id(args[0], hir_module)
                        } else {
                            Type::Int
                        }
                    }
                    hir::Builtin::Min | hir::Builtin::Max => {
                        // min/max return float if any arg is float, or element type if single list argument
                        if args.is_empty() {
                            return Type::Int;
                        }

                        // Check for single list argument case
                        if args.len() == 1 {
                            let arg_type = self.get_type_of_expr_id(args[0], hir_module);
                            if let Type::List(elem_type) = arg_type {
                                return elem_type.as_ref().clone();
                            }
                        }

                        // Multiple arguments - check if any is float
                        let mut has_float = false;
                        for arg_id in args {
                            if self.get_type_of_expr_id(*arg_id, hir_module) == Type::Float {
                                has_float = true;
                                break;
                            }
                        }
                        if has_float {
                            Type::Float
                        } else {
                            Type::Int
                        }
                    }
                    hir::Builtin::Pow => Type::Float,
                    hir::Builtin::Round => {
                        // round(x) -> int, round(x, n) -> float
                        if args.len() > 1 {
                            Type::Float
                        } else {
                            Type::Int
                        }
                    }
                    hir::Builtin::Int => Type::Int,
                    hir::Builtin::Float => Type::Float,
                    hir::Builtin::Bool => Type::Bool,
                    hir::Builtin::Str => Type::Str,
                    hir::Builtin::Bytes => Type::Bytes,
                    hir::Builtin::Chr => Type::Str,
                    hir::Builtin::Ord => Type::Int,
                    hir::Builtin::All | hir::Builtin::Any => Type::Bool,
                    hir::Builtin::Print => Type::None,
                    hir::Builtin::Hash | hir::Builtin::Id => Type::Int,
                    hir::Builtin::Isinstance | hir::Builtin::Issubclass => Type::Bool,
                    hir::Builtin::Iter => {
                        // iter(x) returns Iterator[element_type]
                        if args.is_empty() {
                            return Type::Iterator(Box::new(Type::Any));
                        }
                        let arg_type = self.get_type_of_expr_id(args[0], hir_module);
                        // Class with __iter__ returns the class type itself
                        if let Type::Class { class_id, .. } = &arg_type {
                            if self
                                .get_class_info(class_id)
                                .and_then(|info| info.iter_func)
                                .is_some()
                            {
                                return arg_type;
                            }
                        }
                        let elem_type = match &arg_type {
                            Type::List(elem) => (**elem).clone(),
                            Type::Tuple(elems) if !elems.is_empty() => {
                                Type::normalize_union(elems.clone())
                            }
                            Type::Tuple(_) => Type::Any,
                            Type::Dict(key, _) => (**key).clone(),
                            Type::Set(elem) => (**elem).clone(),
                            Type::Str => Type::Str,
                            Type::Bytes => Type::Int, // bytes iteration yields integers
                            _ => Type::Any,
                        };
                        Type::Iterator(Box::new(elem_type))
                    }
                    hir::Builtin::Set => {
                        // set() or set(iterable)
                        if args.is_empty() {
                            return Type::Set(Box::new(Type::Any));
                        }
                        let arg_type = self.get_type_of_expr_id(args[0], hir_module);
                        let elem_type = match &arg_type {
                            Type::List(elem) => (**elem).clone(),
                            Type::Tuple(elems) if !elems.is_empty() => {
                                Type::normalize_union(elems.clone())
                            }
                            Type::Tuple(_) => Type::Any,
                            Type::Set(elem) => (**elem).clone(),
                            Type::Dict(key, _) => (**key).clone(),
                            Type::Str => Type::Str,
                            _ => Type::Any,
                        };
                        Type::Set(Box::new(elem_type))
                    }
                    hir::Builtin::Next => {
                        // next(iter) returns element_type from Iterator[element_type]
                        if args.is_empty() {
                            return Type::Any;
                        }
                        let arg_type = self.get_type_of_expr_id(args[0], hir_module);
                        match &arg_type {
                            Type::Iterator(elem) => (**elem).clone(),
                            // Class with __next__ returns the __next__ return type
                            Type::Class { class_id, .. } => self
                                .get_class_info(class_id)
                                .and_then(|info| info.next_func)
                                .and_then(|func_id| self.get_func_return_type(&func_id).cloned())
                                .unwrap_or(Type::Any),
                            _ => Type::Any,
                        }
                    }
                    hir::Builtin::Reversed => {
                        // reversed(x) returns Iterator[element_type]
                        if args.is_empty() {
                            return Type::Iterator(Box::new(Type::Any));
                        }
                        let arg_type = self.get_type_of_expr_id(args[0], hir_module);
                        let elem_type = match &arg_type {
                            Type::List(elem) => (**elem).clone(),
                            Type::Tuple(elems) if !elems.is_empty() => {
                                Type::normalize_union(elems.clone())
                            }
                            Type::Tuple(_) => Type::Any,
                            Type::Dict(key, _) => (**key).clone(),
                            Type::Str => Type::Str,
                            _ => Type::Any,
                        };
                        Type::Iterator(Box::new(elem_type))
                    }
                    hir::Builtin::Open => Type::File,
                    hir::Builtin::Enumerate => {
                        // enumerate(iterable, start=0) -> Iterator[Tuple[Int, elem_type]]
                        if args.is_empty() {
                            return Type::Iterator(Box::new(Type::Any));
                        }
                        let arg_type = self.get_type_of_expr_id(args[0], hir_module);
                        let elem_type = match &arg_type {
                            Type::List(elem) => (**elem).clone(),
                            Type::Tuple(elems) if !elems.is_empty() => {
                                Type::normalize_union(elems.clone())
                            }
                            Type::Str => Type::Str,
                            Type::Dict(key, _) => (**key).clone(),
                            Type::Set(elem) => (**elem).clone(),
                            Type::Bytes => Type::Int,
                            _ => Type::Any,
                        };
                        Type::Iterator(Box::new(Type::Tuple(vec![Type::Int, elem_type])))
                    }
                    hir::Builtin::Zip => {
                        // zip(iter1, iter2, ...) -> Iterator[Tuple[elem1, elem2, ...]]
                        if args.is_empty() {
                            return Type::Iterator(Box::new(Type::Tuple(vec![])));
                        }
                        let mut elem_types = Vec::new();
                        for arg_id in args {
                            let arg_expr = &hir_module.exprs[*arg_id];
                            // Special case: range() returns Int elements
                            if let hir::ExprKind::BuiltinCall {
                                builtin: hir::Builtin::Range,
                                ..
                            } = &arg_expr.kind
                            {
                                elem_types.push(Type::Int);
                                continue;
                            }
                            let arg_type = self.get_type_of_expr_id(*arg_id, hir_module);
                            let elem_type = match &arg_type {
                                Type::List(elem) => (**elem).clone(),
                                Type::Tuple(elems) if !elems.is_empty() => {
                                    Type::normalize_union(elems.clone())
                                }
                                Type::Str => Type::Str,
                                Type::Dict(key, _) => (**key).clone(),
                                Type::Set(elem) => (**elem).clone(),
                                Type::Bytes => Type::Int,
                                Type::Iterator(elem) => (**elem).clone(),
                                _ => Type::Any,
                            };
                            elem_types.push(elem_type);
                        }
                        Type::Iterator(Box::new(Type::Tuple(elem_types)))
                    }
                    hir::Builtin::Map => {
                        // map(func, iterable) returns an iterator
                        // Try to infer element type from the function's return type
                        let elem_type = if args.len() >= 2 {
                            let func_expr = &hir_module.exprs[args[0]];
                            // Extract func_id from FuncRef or Closure
                            let func_id = match &func_expr.kind {
                                hir::ExprKind::FuncRef(id) => Some(*id),
                                hir::ExprKind::Closure { func, .. } => Some(*func),
                                _ => None,
                            };
                            if let Some(func_id) = func_id {
                                if let Some(return_type) = self.get_func_return_type(&func_id) {
                                    return_type.clone()
                                } else if let Some(func_def) = hir_module.func_defs.get(&func_id) {
                                    func_def.return_type.clone().unwrap_or(Type::Any)
                                } else {
                                    Type::Any
                                }
                            } else {
                                Type::Any
                            }
                        } else {
                            Type::Any
                        };
                        Type::Iterator(Box::new(elem_type))
                    }
                    hir::Builtin::Filter => {
                        // filter(func, iterable) returns an iterator with same element type as input
                        if args.len() >= 2 {
                            let iterable_type = self.get_type_of_expr_id(args[1], hir_module);
                            let elem_type = match &iterable_type {
                                Type::List(elem) => (**elem).clone(),
                                Type::Tuple(elems) if !elems.is_empty() => {
                                    Type::normalize_union(elems.clone())
                                }
                                Type::Str => Type::Str,
                                Type::Dict(key, _) => (**key).clone(),
                                Type::Set(elem) => (**elem).clone(),
                                Type::Iterator(elem) => (**elem).clone(),
                                _ => Type::Any,
                            };
                            Type::Iterator(Box::new(elem_type))
                        } else {
                            Type::Iterator(Box::new(Type::Any))
                        }
                    }
                    hir::Builtin::List => {
                        // list() or list(iterable)
                        if args.is_empty() {
                            return Type::List(Box::new(Type::Any));
                        }
                        let arg_type = self.get_type_of_expr_id(args[0], hir_module);
                        let elem_type = match &arg_type {
                            Type::List(elem) => (**elem).clone(),
                            Type::Tuple(elems) if !elems.is_empty() => {
                                Type::normalize_union(elems.clone())
                            }
                            Type::Tuple(_) => Type::Any,
                            Type::Set(elem) => (**elem).clone(),
                            Type::Dict(key, _) => (**key).clone(),
                            Type::Str => Type::Str,
                            Type::Iterator(elem) => (**elem).clone(),
                            _ => Type::Any,
                        };
                        Type::List(Box::new(elem_type))
                    }
                    hir::Builtin::Tuple => {
                        // tuple() or tuple(iterable)
                        if args.is_empty() {
                            return Type::Tuple(vec![]);
                        }
                        let arg_type = self.get_type_of_expr_id(args[0], hir_module);
                        let elem_type = match &arg_type {
                            Type::List(elem) => (**elem).clone(),
                            Type::Tuple(elems) if !elems.is_empty() => {
                                Type::normalize_union(elems.clone())
                            }
                            Type::Tuple(_) => Type::Any,
                            Type::Set(elem) => (**elem).clone(),
                            Type::Dict(key, _) => (**key).clone(),
                            Type::Str => Type::Str,
                            Type::Iterator(elem) => (**elem).clone(),
                            _ => Type::Any,
                        };
                        // For dynamic tuple from iterable, use vec![elem_type] as placeholder
                        Type::Tuple(vec![elem_type])
                    }
                    hir::Builtin::Dict => {
                        // dict() or dict(iterable) or dict(**kwargs)
                        Type::Dict(Box::new(Type::Any), Box::new(Type::Any))
                    }
                    hir::Builtin::Sorted => {
                        // sorted(iterable, key=None, reverse=False) -> List[elem_type]
                        if args.is_empty() {
                            return Type::List(Box::new(Type::Any));
                        }
                        let arg_type = self.get_type_of_expr_id(args[0], hir_module);
                        let elem_type = match &arg_type {
                            Type::List(elem) => (**elem).clone(),
                            Type::Tuple(elems) if !elems.is_empty() => {
                                Type::normalize_union(elems.clone())
                            }
                            Type::Tuple(_) => Type::Any,
                            Type::Set(elem) => (**elem).clone(),
                            Type::Dict(key, _) => (**key).clone(),
                            Type::Str => Type::Str,
                            Type::Iterator(elem) => (**elem).clone(),
                            _ => Type::Any,
                        };
                        Type::List(Box::new(elem_type))
                    }
                    hir::Builtin::Format
                    | hir::Builtin::Repr
                    | hir::Builtin::Ascii
                    | hir::Builtin::Bin
                    | hir::Builtin::Hex
                    | hir::Builtin::Oct
                    | hir::Builtin::Input
                    | hir::Builtin::Type => Type::Str,
                    hir::Builtin::Divmod => {
                        // divmod(a, b) -> (int, int) for ints, (float, float) for floats
                        let result_ty = if !args.is_empty() {
                            let a_ty = self.get_type_of_expr_id(args[0], hir_module);
                            let b_ty = if args.len() > 1 {
                                self.get_type_of_expr_id(args[1], hir_module)
                            } else {
                                Type::Int
                            };
                            if matches!(a_ty, Type::Float) || matches!(b_ty, Type::Float) {
                                Type::Float
                            } else {
                                Type::Int
                            }
                        } else {
                            Type::Int
                        };
                        Type::Tuple(vec![result_ty.clone(), result_ty])
                    }
                    hir::Builtin::Chain => {
                        // itertools.chain(*iterables) -> Iterator[Any]
                        Type::Iterator(Box::new(Type::Any))
                    }
                    hir::Builtin::ISlice => {
                        // itertools.islice(iterable, ...) -> Iterator[elem_type]
                        if !args.is_empty() {
                            let iterable_type = self.get_type_of_expr_id(args[0], hir_module);
                            let elem_type = match &iterable_type {
                                Type::List(elem) => (**elem).clone(),
                                Type::Tuple(elems) if !elems.is_empty() => {
                                    Type::normalize_union(elems.clone())
                                }
                                Type::Set(elem) => (**elem).clone(),
                                Type::Dict(key, _) => (**key).clone(),
                                Type::Str => Type::Str,
                                Type::Iterator(elem) => (**elem).clone(),
                                _ => Type::Any,
                            };
                            Type::Iterator(Box::new(elem_type))
                        } else {
                            Type::Iterator(Box::new(Type::Any))
                        }
                    }
                    hir::Builtin::Reduce => {
                        // Infer from iterable's element type (second argument)
                        // For reduce(func, iterable), the result has the same type as elements
                        if args.len() >= 2 {
                            let iterable_type = self.get_type_of_expr_id(args[1], hir_module);
                            match &iterable_type {
                                Type::List(elem) => (**elem).clone(),
                                Type::Tuple(elems) if !elems.is_empty() => {
                                    Type::normalize_union(elems.clone())
                                }
                                Type::Set(elem) => (**elem).clone(),
                                Type::Iterator(elem) => (**elem).clone(),
                                Type::Str => Type::Str,
                                _ => Type::Any,
                            }
                        } else {
                            Type::Any
                        }
                    }
                    _ => expr.ty.clone().unwrap_or(Type::Any),
                }
            }
            hir::ExprKind::Closure { func, .. } => {
                // Closure type is the return type of the underlying function
                if let Some(func_def) = hir_module.func_defs.get(func) {
                    func_def.return_type.clone().unwrap_or(Type::Any)
                } else {
                    Type::Any
                }
            }
            hir::ExprKind::LogicalOp { left, right, .. } => {
                // Logical and/or return one of the operands
                // For bool operands, result is bool
                // For mixed types, return the common type or Any
                let left_ty = self.get_type_of_expr_id(*left, hir_module);
                let right_ty = self.get_type_of_expr_id(*right, hir_module);

                if left_ty == right_ty {
                    left_ty
                } else {
                    // For mixed types, return Any
                    Type::Any
                }
            }
            hir::ExprKind::Compare { .. } => {
                // Comparison always returns bool
                Type::Bool
            }
            hir::ExprKind::IfExpr {
                then_val, else_val, ..
            } => {
                // Ternary: if condition then then_val else else_val
                // Type is the common type of then_val and else_val
                let then_ty = self.get_type_of_expr_id(*then_val, hir_module);
                let else_ty = self.get_type_of_expr_id(*else_val, hir_module);

                if then_ty == else_ty {
                    then_ty
                } else {
                    // For mixed types, return Any
                    Type::Any
                }
            }
            hir::ExprKind::StdlibCall { func, .. } => {
                // Use return type from function definition (Single Source of Truth)
                typespec_to_type(&func.return_type)
            }
            hir::ExprKind::StdlibAttr(attr_def) => {
                // Use type from definition (Single Source of Truth)
                typespec_to_type(&attr_def.ty)
            }
            hir::ExprKind::StdlibConst(const_def) => {
                // Use type from definition (Single Source of Truth)
                typespec_to_type(&const_def.ty)
            }
            hir::ExprKind::ModuleAttr { module, attr } => {
                // Look up type from module_var_exports
                let attr_name = self.resolve(*attr).to_string();
                let key = (module.clone(), attr_name);
                if let Some((_var_id, var_type)) = self.get_module_var_export(&key) {
                    return var_type.clone();
                }
                // Not found - return Any
                Type::Any
            }
            hir::ExprKind::ImportedRef { module, name } => {
                // Look up type from module_var_exports
                let key = (module.clone(), name.clone());
                if let Some((_var_id, var_type)) = self.get_module_var_export(&key) {
                    return var_type.clone();
                }
                // Not found - return Any
                Type::Any
            }
            _ => expr.ty.clone().unwrap_or(Type::Any),
        }
    }
}
