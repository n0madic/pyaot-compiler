//! Method calls and attribute type inference

use pyaot_hir::{ExprId, Module};
use pyaot_stdlib_defs::lookup_object_field;
use pyaot_types::{typespec_to_type, Type};
use pyaot_utils::InternedString;

use crate::context::TypeChecker;

impl<'a> TypeChecker<'a> {
    /// Infer type of method call
    pub(crate) fn infer_method_call_type(
        &mut self,
        obj_expr: ExprId,
        method: InternedString,
        _args: &[ExprId],
        module: &Module,
    ) -> Type {
        let obj_type = self.infer_expr_type(obj_expr, module);
        let method_name = self.interner.resolve(method);

        match &obj_type {
            Type::Str => match method_name {
                "upper" | "lower" | "strip" | "replace" | "lstrip" | "rstrip" => Type::Str,
                "title" | "capitalize" | "swapcase" => Type::Str,
                "center" | "ljust" | "rjust" | "zfill" => Type::Str,
                "join" => Type::Str,
                "startswith" | "endswith" => Type::Bool,
                "isdigit" | "isalpha" | "isalnum" | "isspace" | "isupper" | "islower" => Type::Bool,
                "find" | "index" | "count" => Type::Int,
                "format" => Type::Str,
                "split" => Type::List(Box::new(Type::Str)),
                _ => Type::Any,
            },
            Type::List(elem_type) => match method_name {
                "append" | "insert" | "remove" | "clear" | "reverse" | "sort" => Type::None,
                "pop" => (**elem_type).clone(),
                "copy" => Type::List(elem_type.clone()),
                "index" | "count" => Type::Int,
                _ => Type::Any,
            },
            Type::Dict(key_type, value_type) => match method_name {
                "get" | "pop" => (**value_type).clone(),
                "keys" => Type::List(key_type.clone()),
                "values" => Type::List(value_type.clone()),
                "items" => Type::List(Box::new(Type::Tuple(vec![
                    (**key_type).clone(),
                    (**value_type).clone(),
                ]))),
                "clear" | "update" => Type::None,
                "copy" => Type::Dict(key_type.clone(), value_type.clone()),
                _ => Type::Any,
            },
            Type::Tuple(_) => match method_name {
                "index" | "count" => Type::Int,
                _ => Type::Any,
            },
            Type::Class { class_id, .. } => {
                if let Some(class_info) = self.class_info.get(class_id) {
                    if let Some(method_id) = class_info.methods.get(&method) {
                        if let Some(func) = module.func_defs.get(method_id) {
                            return func.return_type.clone().unwrap_or(Type::None);
                        }
                    }
                }
                Type::Any
            }
            _ => Type::Any,
        }
    }

    /// Extract element type from an iterable type for container constructor inference
    pub(crate) fn extract_iterable_element_type(&mut self, iterable_type: &Type) -> Type {
        match iterable_type {
            Type::List(elem) => (**elem).clone(),
            Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
            Type::Tuple(_) => Type::Any,
            Type::Set(elem) => (**elem).clone(),
            Type::Dict(key, _) => (**key).clone(),
            Type::Str => Type::Str,
            Type::Bytes => Type::Int,
            Type::Iterator(elem) => (**elem).clone(),
            _ => Type::Any,
        }
    }

    /// Infer type of attribute access
    pub(crate) fn infer_attribute_type(&self, obj_type: &Type, attr: InternedString) -> Type {
        let attr_name = self.interner.resolve(attr);

        // Handle RuntimeObject attributes (StructTime, CompletedProcess, Match, etc.)
        // using Single Source of Truth from stdlib-defs
        if let Type::RuntimeObject(type_tag) = obj_type {
            if let Some(field_def) = lookup_object_field(*type_tag, attr_name) {
                return typespec_to_type(&field_def.field_type);
            }
            return Type::Any;
        }

        // Handle File attributes
        if matches!(obj_type, Type::File) {
            match attr_name {
                "closed" => return Type::Bool,
                "name" => return Type::Str,
                _ => return Type::Any,
            }
        }

        if let Type::Class { class_id, .. } = obj_type {
            if let Some(class_info) = self.class_info.get(class_id) {
                // Check for property first
                if let Some(prop_type) = class_info.properties.get(&attr) {
                    return prop_type.clone();
                }
                // Then check for regular field
                if let Some(field_type) = class_info.fields.get(&attr) {
                    return field_type.clone();
                }
            }
        }
        Type::Any
    }
}
