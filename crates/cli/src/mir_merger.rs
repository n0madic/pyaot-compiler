//! MIR merging and ID remapping

use crate::types::ParsedModule;
use miette::{NamedSource, Report, Result};
use pyaot_hir as hir;
use pyaot_lowering::CrossModuleClassInfo;
use pyaot_types::{BuiltinExceptionKind, Type, TypeTagKind};
use pyaot_utils::{ClassId, FuncId, InternedString, StringInterner, VarId};
use std::collections::HashMap;

/// Interner-free mirror of `Type`.
///
/// The cross-module export pipeline resolves every `InternedString` through
/// the owning module's interner (first pass), stores the result as a raw
/// `String`, and re-interns through the caller's interner on demand (second
/// pass). `Class` carries an already-offset class id so that callers can
/// look it up directly in `cross_module_class_info`.
#[derive(Debug, Clone)]
enum RawType {
    Int,
    Float,
    Bool,
    Str,
    Bytes,
    None,
    Any,
    HeapAny,
    File(bool),
    Never,
    List(Box<RawType>),
    Dict(Box<RawType>, Box<RawType>),
    DefaultDict(Box<RawType>, Box<RawType>),
    Set(Box<RawType>),
    Tuple(Vec<RawType>),
    TupleVar(Box<RawType>),
    Union(Vec<RawType>),
    Function {
        params: Vec<RawType>,
        ret: Box<RawType>,
    },
    Var(String),
    Class {
        class_id: ClassId,
        name: String,
    },
    Iterator(Box<RawType>),
    BuiltinException(BuiltinExceptionKind),
    RuntimeObject(TypeTagKind),
}

/// Serialize a `Type` from a source module's interner into an interner-free
/// `RawType`. Applies the source module's class-id offset to `Type::Class`
/// entries so the caller's `cross_module_class_info` lookup (which is keyed
/// by remapped id) finds the right class info.
fn type_to_raw(ty: &Type, source_interner: &StringInterner, class_id_offset: u32) -> RawType {
    match ty {
        Type::Int => RawType::Int,
        Type::Float => RawType::Float,
        Type::Bool => RawType::Bool,
        Type::Str => RawType::Str,
        Type::Bytes => RawType::Bytes,
        Type::None => RawType::None,
        Type::Any => RawType::Any,
        Type::HeapAny => RawType::HeapAny,
        Type::File(binary) => RawType::File(*binary),
        Type::Never => RawType::Never,
        Type::List(t) => RawType::List(Box::new(type_to_raw(t, source_interner, class_id_offset))),
        Type::Dict(k, v) => RawType::Dict(
            Box::new(type_to_raw(k, source_interner, class_id_offset)),
            Box::new(type_to_raw(v, source_interner, class_id_offset)),
        ),
        Type::DefaultDict(k, v) => RawType::DefaultDict(
            Box::new(type_to_raw(k, source_interner, class_id_offset)),
            Box::new(type_to_raw(v, source_interner, class_id_offset)),
        ),
        Type::Set(t) => RawType::Set(Box::new(type_to_raw(t, source_interner, class_id_offset))),
        Type::Tuple(ts) => RawType::Tuple(
            ts.iter()
                .map(|t| type_to_raw(t, source_interner, class_id_offset))
                .collect(),
        ),
        Type::TupleVar(t) => {
            RawType::TupleVar(Box::new(type_to_raw(t, source_interner, class_id_offset)))
        }
        Type::Union(ts) => RawType::Union(
            ts.iter()
                .map(|t| type_to_raw(t, source_interner, class_id_offset))
                .collect(),
        ),
        Type::Function { params, ret } => RawType::Function {
            params: params
                .iter()
                .map(|t| type_to_raw(t, source_interner, class_id_offset))
                .collect(),
            ret: Box::new(type_to_raw(ret, source_interner, class_id_offset)),
        },
        Type::Var(name) => RawType::Var(source_interner.resolve(*name).to_string()),
        Type::Class { class_id, name } => {
            // Placeholder class ids occupy the top of u32 space (u32::MAX - N).
            // They are only resolved in the second pass; if we encounter one
            // here (first-pass variable-type scanning), emit Any so the
            // cross-module export is treated as opaque rather than panicking
            // on u32 overflow.
            if let Some(remapped) = class_id.0.checked_add(class_id_offset) {
                RawType::Class {
                    class_id: ClassId(remapped),
                    name: source_interner.resolve(*name).to_string(),
                }
            } else {
                RawType::Any
            }
        }
        Type::Iterator(t) => {
            RawType::Iterator(Box::new(type_to_raw(t, source_interner, class_id_offset)))
        }
        Type::BuiltinException(k) => RawType::BuiltinException(*k),
        Type::RuntimeObject(k) => RawType::RuntimeObject(*k),
        // NotImplementedT is a transient dispatch sentinel — it is consumed
        // at binary-op call sites within the same module and should never
        // appear in a cross-module signature. Serialize as `Any` so that a
        // stale appearance does not crash the merger.
        Type::NotImplementedT => RawType::Any,
    }
}

/// Reconstruct a `Type` for a caller by re-interning class names through the
/// caller's interner. Class ids are already remapped (see `type_to_raw`).
fn raw_to_type(raw: &RawType, caller_interner: &mut StringInterner) -> Type {
    match raw {
        RawType::Int => Type::Int,
        RawType::Float => Type::Float,
        RawType::Bool => Type::Bool,
        RawType::Str => Type::Str,
        RawType::Bytes => Type::Bytes,
        RawType::None => Type::None,
        RawType::Any => Type::Any,
        RawType::HeapAny => Type::HeapAny,
        RawType::File(binary) => Type::File(*binary),
        RawType::Never => Type::Never,
        RawType::List(t) => Type::List(Box::new(raw_to_type(t, caller_interner))),
        RawType::Dict(k, v) => Type::Dict(
            Box::new(raw_to_type(k, caller_interner)),
            Box::new(raw_to_type(v, caller_interner)),
        ),
        RawType::DefaultDict(k, v) => Type::DefaultDict(
            Box::new(raw_to_type(k, caller_interner)),
            Box::new(raw_to_type(v, caller_interner)),
        ),
        RawType::Set(t) => Type::Set(Box::new(raw_to_type(t, caller_interner))),
        RawType::Tuple(ts) => {
            Type::Tuple(ts.iter().map(|t| raw_to_type(t, caller_interner)).collect())
        }
        RawType::TupleVar(t) => Type::TupleVar(Box::new(raw_to_type(t, caller_interner))),
        RawType::Union(ts) => {
            Type::Union(ts.iter().map(|t| raw_to_type(t, caller_interner)).collect())
        }
        RawType::Function { params, ret } => Type::Function {
            params: params
                .iter()
                .map(|t| raw_to_type(t, caller_interner))
                .collect(),
            ret: Box::new(raw_to_type(ret, caller_interner)),
        },
        RawType::Var(name) => Type::Var(caller_interner.intern(name)),
        RawType::Class { class_id, name } => Type::Class {
            class_id: *class_id,
            name: caller_interner.intern(name),
        },
        RawType::Iterator(t) => Type::Iterator(Box::new(raw_to_type(t, caller_interner))),
        RawType::BuiltinException(k) => Type::BuiltinException(*k),
        RawType::RuntimeObject(k) => Type::RuntimeObject(*k),
    }
}

/// String-based intermediate for cross-module class info.
/// Built during the first pass (each module has its own interner),
/// then converted to InternedString-based `CrossModuleClassInfo` per module.
struct RawCrossModuleClassInfo {
    field_offsets: HashMap<String, usize>,
    field_types: HashMap<String, RawType>,
    method_return_types: HashMap<String, RawType>,
    total_field_count: usize,
}

pub struct MirMerger;

impl MirMerger {
    /// Lower each module separately and merge at MIR level
    pub fn compile_modules(
        mut parsed_modules: HashMap<String, ParsedModule>,
        sorted_modules: &[String],
        main_module_name: &str,
        verbose: bool,
    ) -> Result<(pyaot_mir::Module, StringInterner)> {
        use pyaot_mir as mir;

        let mut merged_mir = mir::Module::new();
        let mut merged_interner = StringInterner::new();

        // Track the next available FuncId to avoid collisions when merging
        let mut next_func_id: u32 = 0;

        // First pass: compute VarId and ClassId offsets for each module
        // and build module exports
        let mut var_id_offset: u32 = 0;
        let mut class_id_offset: u32 = 0;
        let mut module_var_offsets: HashMap<String, u32> = HashMap::new();
        let mut module_class_offsets: HashMap<String, u32> = HashMap::new();

        // Module exports: (module_name, var_name) -> (remapped VarId, RawType).
        // `RawType` is interner-free — it is resolved into the caller's
        // interner in the second pass. Storing raw lets cross-module class
        // types round-trip across modules with remapped class ids.
        let mut module_var_exports: HashMap<(String, String), (VarId, RawType)> = HashMap::new();
        // Module function exports: (module_name, func_name) -> raw return type.
        let mut module_func_exports: HashMap<(String, String), RawType> = HashMap::new();
        // Module function parameters: ordered param list (names + simple
        // defaults) for cross-module kwargs + default-arg filling.
        let mut module_func_params: HashMap<(String, String), Vec<pyaot_lowering::ExportedParam>> =
            HashMap::new();
        // Module class exports: (module_name, class_name) -> (remapped ClassId, class_name as String)
        let mut module_class_exports: HashMap<(String, String), (ClassId, String)> = HashMap::new();
        // Cross-module class field/method info (string-keyed intermediate)
        let mut raw_cross_module_class_info: HashMap<ClassId, RawCrossModuleClassInfo> =
            HashMap::new();

        for module_name in sorted_modules {
            let parsed = parsed_modules
                .get(module_name)
                .expect("module must exist in parsed_modules for sorted module name");

            // Record the offset for this module
            module_var_offsets.insert(module_name.clone(), var_id_offset);
            module_class_offsets.insert(module_name.clone(), class_id_offset);

            // Count VarIds: find the maximum VarId used in module_var_map and globals
            let max_var_from_module_vars = parsed
                .hir
                .module_var_map
                .values()
                .map(|v| v.0)
                .max()
                .unwrap_or(0);
            let max_var_from_globals = parsed.hir.globals.iter().map(|v| v.0).max().unwrap_or(0);
            let max_var_id = max_var_from_module_vars.max(max_var_from_globals);
            let module_var_count =
                if parsed.hir.module_var_map.is_empty() && parsed.hir.globals.is_empty() {
                    0
                } else {
                    max_var_id + 1
                };

            // Build variable exports for this module
            for (var_name, var_id) in &parsed.hir.module_var_map {
                let var_name_str = parsed.interner.resolve(*var_name).to_string();
                // Remap VarId with offset
                let remapped_var_id = VarId::new(var_id.0 + var_id_offset);

                // Determine variable type by scanning module_init_stmts for assignments
                let var_type = Self::find_var_type_in_stmts(
                    *var_id,
                    &parsed.hir.module_init_stmts,
                    &parsed.hir,
                );
                let raw_var_type = type_to_raw(&var_type, &parsed.interner, class_id_offset);

                module_var_exports.insert(
                    (module_name.clone(), var_name_str),
                    (remapped_var_id, raw_var_type),
                );
            }

            // Build function exports for this module (for cross-module function calls)
            for func_def in parsed.hir.func_defs.values() {
                let func_name = parsed.interner.resolve(func_def.name).to_string();
                // Skip internal functions like __pyaot_module_init__
                if func_name.starts_with("__") {
                    continue;
                }
                // Get return type, defaulting to None
                let return_type = func_def.return_type.clone().unwrap_or(Type::None);
                let raw_return_type = type_to_raw(&return_type, &parsed.interner, class_id_offset);
                module_func_exports
                    .insert((module_name.clone(), func_name.clone()), raw_return_type);

                // Extract parameter names + simple-constant defaults so a
                // cross-module caller can map keyword arguments to slots
                // and omit args with defaults.
                let params = func_def
                    .params
                    .iter()
                    .map(|p| pyaot_lowering::ExportedParam {
                        name: parsed.interner.resolve(p.name).to_string(),
                        default: p.default.and_then(|eid| {
                            Self::simple_default_from_expr(
                                &parsed.hir.exprs[eid].kind,
                                &parsed.interner,
                            )
                        }),
                    })
                    .collect::<Vec<_>>();
                module_func_params.insert((module_name.clone(), func_name), params);
            }

            // Build class exports for this module
            for (class_id, class_def) in &parsed.hir.class_defs {
                let class_name_str = parsed.interner.resolve(class_def.name).to_string();
                // Remap ClassId with offset
                let remapped_class_id = ClassId(class_id.0 + class_id_offset);

                module_class_exports.insert(
                    (module_name.clone(), class_name_str.clone()),
                    (remapped_class_id, class_name_str),
                );

                // Build field and method info for cross-module access
                let mut field_offsets: HashMap<String, usize> = HashMap::new();
                let mut field_types: HashMap<String, RawType> = HashMap::new();
                let mut method_return_types: HashMap<String, RawType> = HashMap::new();
                for (i, field) in class_def.fields.iter().enumerate() {
                    let field_name = parsed.interner.resolve(field.name).to_string();
                    field_offsets.insert(field_name.clone(), i);
                    field_types.insert(
                        field_name,
                        type_to_raw(&field.ty, &parsed.interner, class_id_offset),
                    );
                }
                // Extract method return types. HIR stores method func names
                // as `ClassName$method`; strip the prefix so lookups by bare
                // method name (as produced by `r.ok()`-style callsites) hit.
                for method_func_id in &class_def.methods {
                    if let Some(func_def) = parsed.hir.func_defs.get(method_func_id) {
                        let raw_name = parsed.interner.resolve(func_def.name).to_string();
                        let method_name = raw_name
                            .rsplit_once('$')
                            .map(|(_cls, m)| m.to_string())
                            .unwrap_or(raw_name);
                        if let Some(ref ret_ty) = func_def.return_type {
                            method_return_types.insert(
                                method_name,
                                type_to_raw(ret_ty, &parsed.interner, class_id_offset),
                            );
                        }
                    }
                }
                raw_cross_module_class_info.insert(
                    remapped_class_id,
                    RawCrossModuleClassInfo {
                        field_offsets,
                        field_types,
                        method_return_types,
                        total_field_count: class_def.fields.len(),
                    },
                );
            }

            // Update offsets for next module
            var_id_offset += module_var_count;
            class_id_offset += parsed.hir.class_defs.len() as u32;
        }

        if verbose && !module_var_exports.is_empty() {
            println!(
                "Module variable exports: {:?}",
                module_var_exports.keys().collect::<Vec<_>>()
            );
        }
        if verbose && !module_class_exports.is_empty() {
            println!(
                "Module class exports: {:?}",
                module_class_exports.keys().collect::<Vec<_>>()
            );
        }

        // Second pass: process each module and merge
        for module_name in sorted_modules {
            let mut parsed = parsed_modules
                .remove(module_name)
                .expect("module must exist in parsed_modules for sorted module name");
            let is_main = module_name == main_module_name;
            let this_var_offset = *module_var_offsets.get(module_name).unwrap_or(&0);
            let this_class_offset = *module_class_offsets.get(module_name).unwrap_or(&0);

            // Resolve cross-module user-class type annotations. The frontend
            // allocated a unique placeholder `ClassId` per imported class
            // (see `AstToHir::alloc_external_class_ref`) and stored the
            // `(module, class_name)` pair in `hir.external_class_refs`. We
            // walk the HIR's types and rewrite every `Type::Class` with a
            // placeholder id to the real remapped id.
            if !parsed.hir.external_class_refs.is_empty() {
                let mut placeholder_remap: HashMap<ClassId, (ClassId, String)> = HashMap::new();
                for (placeholder_id, (src_mod, src_name)) in &parsed.hir.external_class_refs {
                    let key = (src_mod.clone(), src_name.clone());
                    if let Some((remapped_id, class_name)) = module_class_exports.get(&key) {
                        placeholder_remap
                            .insert(*placeholder_id, (*remapped_id, class_name.clone()));
                    }
                }
                if !placeholder_remap.is_empty() {
                    resolve_external_class_refs(&mut parsed, &placeholder_remap);
                }
            }

            if verbose {
                println!(
                    "Processing module: {} (var_offset={}, class_offset={})",
                    module_name, this_var_offset, this_class_offset
                );
            }

            // Create source context for error reporting
            let source_name = parsed.path.display().to_string();
            let source_code = parsed.source.clone();

            // Semantic analysis
            let mut sem_analyzer = pyaot_semantics::SemanticAnalyzer::new(&parsed.interner);
            sem_analyzer.analyze(&parsed.hir).map_err(|e| {
                Report::new(e).with_source_code(NamedSource::new(&source_name, source_code.clone()))
            })?;

            // Re-intern cross-module class info keys into this module's interner
            // (must be done before Lowering borrows the interner)
            let interned_class_info: HashMap<ClassId, CrossModuleClassInfo> =
                raw_cross_module_class_info
                    .iter()
                    .map(|(&class_id, raw)| {
                        let field_offsets = raw
                            .field_offsets
                            .iter()
                            .map(|(name, &offset)| (parsed.interner.intern(name), offset))
                            .collect();
                        let field_types = raw
                            .field_types
                            .iter()
                            .map(|(name, raw_ty)| {
                                (
                                    parsed.interner.intern(name),
                                    raw_to_type(raw_ty, &mut parsed.interner),
                                )
                            })
                            .collect();
                        let method_return_types = raw
                            .method_return_types
                            .iter()
                            .map(|(name, raw_ty)| {
                                (
                                    parsed.interner.intern(name),
                                    raw_to_type(raw_ty, &mut parsed.interner),
                                )
                            })
                            .collect();
                        (
                            class_id,
                            CrossModuleClassInfo {
                                field_offsets,
                                field_types,
                                method_return_types,
                                total_field_count: raw.total_field_count,
                            },
                        )
                    })
                    .collect();

            // Re-intern module var/func exports into this module's interner
            // so `Type::Class` references resolve to the caller's interner.
            let var_exports_for_caller: HashMap<(String, String), (VarId, Type)> =
                module_var_exports
                    .iter()
                    .map(|(key, (var_id, raw_ty))| {
                        (
                            key.clone(),
                            (*var_id, raw_to_type(raw_ty, &mut parsed.interner)),
                        )
                    })
                    .collect();
            let func_exports_for_caller: HashMap<(String, String), Type> = module_func_exports
                .iter()
                .map(|(key, raw_ty)| (key.clone(), raw_to_type(raw_ty, &mut parsed.interner)))
                .collect();

            // Lower to MIR with module exports (type inference runs inside lower_module)
            let func_count = parsed.hir.functions.len();
            let class_count = parsed.hir.class_defs.len();
            let mut lowering = pyaot_lowering::Lowering::new_with_capacity(
                &mut parsed.interner,
                func_count,
                class_count,
            );
            lowering.set_module_var_exports(var_exports_for_caller);
            lowering.set_module_func_exports(func_exports_for_caller);
            lowering.set_module_func_params(module_func_params.clone());
            lowering.set_module_class_exports(module_class_exports.clone());
            lowering.set_cross_module_class_info(interned_class_info);
            lowering.set_var_id_offset(this_var_offset);
            lowering.set_class_id_offset(this_class_offset);

            let (module_mir, warnings) = lowering.lower_module(parsed.hir).map_err(|e| {
                Report::new(e).with_source_code(NamedSource::new(&source_name, source_code.clone()))
            })?;

            // Emit any warnings collected during lowering for this module
            if !warnings.is_empty() {
                warnings.emit_all(&source_name, &source_code);
            }

            // Build mapping from old InternedStrings to new ones in merged_interner
            let mut string_remap: HashMap<InternedString, InternedString> = HashMap::new();
            for (old_id, s) in parsed.interner.iter() {
                let new_id = merged_interner.intern(s);
                string_remap.insert(old_id, new_id);
            }

            // Helper to remap an InternedString
            let remap_str =
                |old: InternedString| -> InternedString { *string_remap.get(&old).unwrap_or(&old) };

            // First sub-pass: assign fresh FuncIds and build a per-module
            // old→new remap. We need the full remap table before rewriting
            // instructions since a `CallDirect` may target a function
            // defined later in the same module.
            let mut func_id_remap: HashMap<FuncId, FuncId> = HashMap::new();
            let mut module_functions: Vec<(FuncId, pyaot_mir::Function, String)> = Vec::new();
            for (old_func_id, mut func) in module_mir.functions {
                let safe_module_name = module_name.replace('.', "_");
                let original_name = func.name.clone();
                if !is_main && original_name != "__pyaot_module_init__" {
                    func.name = format!("__module_{}_{}", safe_module_name, original_name);
                } else if !is_main && original_name == "__pyaot_module_init__" {
                    func.name = format!("__module_{}_init__", safe_module_name);
                }

                let new_func_id = FuncId::from(next_func_id);
                next_func_id += 1;
                func.id = new_func_id;
                func_id_remap.insert(old_func_id, new_func_id);

                if !is_main && original_name == "__pyaot_module_init__" {
                    merged_mir
                        .module_init_order
                        .push((module_name.clone(), new_func_id));
                }

                module_functions.push((new_func_id, func, original_name));
            }

            let remap_func = |old: FuncId| -> FuncId { *func_id_remap.get(&old).unwrap_or(&old) };

            // Second sub-pass: remap FuncIds and InternedStrings inside each
            // function body, then insert into the merged module.
            for (new_func_id, mut func, _original_name) in module_functions {
                merged_interner.intern(&func.name);

                for (_local_id, local) in func.locals.iter_mut() {
                    if let Some(name) = local.name {
                        local.name = Some(remap_str(name));
                    }
                }

                for (_block_id, block) in func.blocks.iter_mut() {
                    for inst in block.instructions.iter_mut() {
                        Self::remap_instruction_strings(&mut inst.kind, &remap_str);
                        Self::remap_instruction_func_ids(&mut inst.kind, &remap_func);
                    }
                    Self::remap_terminator_strings(&mut block.terminator, &remap_str);
                }

                merged_mir.functions.insert(new_func_id, func);
            }

            // Merge vtables, remapping their FuncId references to the new ids.
            for mut vtable in module_mir.vtables {
                for entry in vtable.entries.iter_mut() {
                    entry.method_func_id = remap_func(entry.method_func_id);
                }
                merged_mir.vtables.push(vtable);
            }
        }

        Ok((merged_mir, merged_interner))
    }

    /// Materialise a simple-constant default argument expression into a
    /// cross-module-safe `SimpleDefault`. Returns `None` for anything that
    /// can't be encoded as one of the four primitive constants or `None`.
    /// Callers treat `None` here as "no default" — callers of the exported
    /// function must then provide the arg explicitly.
    fn simple_default_from_expr(
        kind: &hir::ExprKind,
        interner: &pyaot_utils::StringInterner,
    ) -> Option<pyaot_lowering::SimpleDefault> {
        match kind {
            hir::ExprKind::None => Some(pyaot_lowering::SimpleDefault::None),
            hir::ExprKind::Int(v) => Some(pyaot_lowering::SimpleDefault::Int(*v)),
            hir::ExprKind::Float(v) => Some(pyaot_lowering::SimpleDefault::Float(*v)),
            hir::ExprKind::Bool(v) => Some(pyaot_lowering::SimpleDefault::Bool(*v)),
            hir::ExprKind::Str(s) => Some(pyaot_lowering::SimpleDefault::Str(
                interner.resolve(*s).to_string(),
            )),
            _ => None,
        }
    }

    /// Find the type of a variable from its assignment in module_init_stmts
    fn find_var_type_in_stmts(
        var_id: VarId,
        stmt_ids: &[hir::StmtId],
        hir_module: &hir::Module,
    ) -> Type {
        for stmt_id in stmt_ids {
            let stmt = &hir_module.stmts[*stmt_id];
            let var_assign = match &stmt.kind {
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    type_hint,
                    value,
                } => Some((*target_var, type_hint.as_ref(), *value)),
                _ => None,
            };
            if let Some((target, type_hint, value)) = var_assign {
                if target == var_id {
                    // Prefer explicit type hint
                    if let Some(ty) = type_hint {
                        return ty.clone();
                    }
                    // Fall back to expression type
                    let value_expr = &hir_module.exprs[value];
                    if let Some(ty) = &value_expr.ty {
                        return ty.clone();
                    }
                    // Infer from literal expression kind
                    return match &value_expr.kind {
                        hir::ExprKind::Int(_) => Type::Int,
                        hir::ExprKind::Float(_) => Type::Float,
                        hir::ExprKind::Bool(_) => Type::Bool,
                        hir::ExprKind::Str(_) => Type::Str,
                        hir::ExprKind::None => Type::None,
                        hir::ExprKind::List(_) => Type::List(Box::new(Type::Any)),
                        hir::ExprKind::Dict(_) => {
                            Type::Dict(Box::new(Type::Any), Box::new(Type::Any))
                        }
                        hir::ExprKind::Tuple(_) => Type::Tuple(vec![]),
                        hir::ExprKind::Set(_) => Type::Set(Box::new(Type::Any)),
                        _ => Type::Any,
                    };
                }
            }
        }
        // Default to Any if not found
        Type::Any
    }

    /// Remap `FuncId` references embedded in an instruction. Only
    /// `CallDirect` carries a `FuncId` directly — other call kinds route
    /// through a symbol name (`CallNamed`), an operand (`Call`), or a
    /// vtable slot (`CallVirtual{,Named}`), none of which change when
    /// module-local ids are merged into a global namespace.
    fn remap_instruction_func_ids<F>(kind: &mut pyaot_mir::InstructionKind, remap: &F)
    where
        F: Fn(FuncId) -> FuncId,
    {
        use pyaot_mir::InstructionKind;
        if let InstructionKind::CallDirect { func, .. } = kind {
            *func = remap(*func);
        }
    }

    /// Remap InternedStrings in an instruction
    fn remap_instruction_strings<F>(kind: &mut pyaot_mir::InstructionKind, remap: &F)
    where
        F: Fn(pyaot_utils::InternedString) -> pyaot_utils::InternedString,
    {
        use pyaot_mir::InstructionKind;

        match kind {
            InstructionKind::Const { value, .. } => {
                Self::remap_constant_strings(value, remap);
            }
            InstructionKind::BinOp { left, right, .. } => {
                Self::remap_operand_strings(left, remap);
                Self::remap_operand_strings(right, remap);
            }
            InstructionKind::UnOp { operand, .. } => {
                Self::remap_operand_strings(operand, remap);
            }
            InstructionKind::Copy { src, .. } => {
                Self::remap_operand_strings(src, remap);
            }
            InstructionKind::Call { func, args, .. } => {
                Self::remap_operand_strings(func, remap);
                for arg in args.iter_mut() {
                    Self::remap_operand_strings(arg, remap);
                }
            }
            InstructionKind::CallDirect { args, .. } => {
                for arg in args.iter_mut() {
                    Self::remap_operand_strings(arg, remap);
                }
            }
            InstructionKind::CallNamed { args, .. } => {
                for arg in args.iter_mut() {
                    Self::remap_operand_strings(arg, remap);
                }
            }
            InstructionKind::CallVirtual { obj, args, .. }
            | InstructionKind::CallVirtualNamed { obj, args, .. } => {
                Self::remap_operand_strings(obj, remap);
                for arg in args.iter_mut() {
                    Self::remap_operand_strings(arg, remap);
                }
            }
            InstructionKind::RuntimeCall { args, .. } => {
                for arg in args.iter_mut() {
                    Self::remap_operand_strings(arg, remap);
                }
            }
            InstructionKind::FloatToInt { src, .. }
            | InstructionKind::BoolToInt { src, .. }
            | InstructionKind::IntToFloat { src, .. }
            | InstructionKind::FloatBits { src, .. }
            | InstructionKind::IntBitsToFloat { src, .. }
            | InstructionKind::FloatAbs { src, .. } => {
                Self::remap_operand_strings(src, remap);
            }
            // Other instructions don't contain InternedStrings
            _ => {}
        }
    }

    /// Remap InternedStrings in a terminator
    fn remap_terminator_strings<F>(term: &mut pyaot_mir::Terminator, remap: &F)
    where
        F: Fn(pyaot_utils::InternedString) -> pyaot_utils::InternedString,
    {
        use pyaot_mir::Terminator;

        match term {
            Terminator::Return(Some(op)) => {
                Self::remap_operand_strings(op, remap);
            }
            Terminator::Branch { cond, .. } => {
                Self::remap_operand_strings(cond, remap);
            }
            Terminator::Raise { message, cause, .. } => {
                if let Some(msg) = message {
                    Self::remap_operand_strings(msg, remap);
                }
                if let Some(c) = cause {
                    if let Some(msg) = &mut c.message {
                        Self::remap_operand_strings(msg, remap);
                    }
                }
            }
            _ => {}
        }
    }

    /// Remap InternedStrings in an operand
    fn remap_operand_strings<F>(op: &mut pyaot_mir::Operand, remap: &F)
    where
        F: Fn(pyaot_utils::InternedString) -> pyaot_utils::InternedString,
    {
        if let pyaot_mir::Operand::Constant(c) = op {
            Self::remap_constant_strings(c, remap);
        }
    }

    /// Remap InternedStrings in a constant
    fn remap_constant_strings<F>(c: &mut pyaot_mir::Constant, remap: &F)
    where
        F: Fn(pyaot_utils::InternedString) -> pyaot_utils::InternedString,
    {
        if let pyaot_mir::Constant::Str(s) = c {
            *s = remap(*s);
        }
    }
}

/// Rewrite every `Type::Class` with a placeholder id in `parsed.hir` to its
/// real remapped class id. The `remap` table is keyed by the placeholder id
/// the frontend emitted (from `AstToHir::alloc_external_class_ref`).
///
/// This walks every Type embedded in the HIR — function parameter/return
/// types, expression types, statement type hints, and class-def field /
/// class-attribute types — so cross-module user-class annotations work in
/// the same positions CPython allows them. Non-placeholder `Type::Class`
/// entries are left untouched.
fn resolve_external_class_refs(
    parsed: &mut crate::types::ParsedModule,
    remap: &HashMap<ClassId, (ClassId, String)>,
) {
    let remap_ty = |ty: &mut Type, interner: &mut StringInterner| {
        rewrite_class_type(ty, remap, interner);
    };

    // Function parameters and return types
    for func_def in parsed.hir.func_defs.values_mut() {
        if let Some(ret) = func_def.return_type.as_mut() {
            remap_ty(ret, &mut parsed.interner);
        }
        for param in func_def.params.iter_mut() {
            if let Some(ty) = param.ty.as_mut() {
                remap_ty(ty, &mut parsed.interner);
            }
        }
    }

    // Class field and class-attribute annotations
    for class_def in parsed.hir.class_defs.values_mut() {
        for field in class_def.fields.iter_mut() {
            remap_ty(&mut field.ty, &mut parsed.interner);
        }
        for attr in class_def.class_attrs.iter_mut() {
            remap_ty(&mut attr.ty, &mut parsed.interner);
        }
    }

    // Expression types (populated by the frontend for annotated values)
    for expr in parsed.hir.exprs.iter_mut() {
        if let Some(ty) = expr.1.ty.as_mut() {
            remap_ty(ty, &mut parsed.interner);
        }
    }

    // Statement type hints (annotated assignments)
    for stmt in parsed.hir.stmts.iter_mut() {
        if let pyaot_hir::StmtKind::Bind { type_hint, .. } = &mut stmt.1.kind {
            if let Some(ty) = type_hint.as_mut() {
                remap_ty(ty, &mut parsed.interner);
            }
        }
    }
}

/// Rewrite `Type::Class` placeholder ids inside a `Type` tree, re-interning
/// the class name through `interner` so the result is valid in the current
/// caller's interner. Non-class structural types (List, Dict, Union, ...)
/// are descended into recursively.
fn rewrite_class_type(
    ty: &mut Type,
    remap: &HashMap<ClassId, (ClassId, String)>,
    interner: &mut StringInterner,
) {
    match ty {
        Type::Class { class_id, name } => {
            if let Some((real_id, class_name)) = remap.get(class_id) {
                *class_id = *real_id;
                *name = interner.intern(class_name);
            }
        }
        Type::List(inner) | Type::Set(inner) | Type::Iterator(inner) => {
            rewrite_class_type(inner, remap, interner);
        }
        Type::Dict(k, v) | Type::DefaultDict(k, v) => {
            rewrite_class_type(k, remap, interner);
            rewrite_class_type(v, remap, interner);
        }
        Type::Tuple(elems) | Type::Union(elems) => {
            for t in elems.iter_mut() {
                rewrite_class_type(t, remap, interner);
            }
        }
        Type::Function { params, ret } => {
            for t in params.iter_mut() {
                rewrite_class_type(t, remap, interner);
            }
            rewrite_class_type(ret, remap, interner);
        }
        _ => {}
    }
}
