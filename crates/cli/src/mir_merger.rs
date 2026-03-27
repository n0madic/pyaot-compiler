//! MIR merging and ID remapping

use crate::types::{CrossModuleClassInfo, ParsedModule};
use miette::{NamedSource, Report, Result};
use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::{ClassId, FuncId, InternedString, StringInterner, VarId};
use std::collections::HashMap;

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

        // Module exports: (module_name, var_name) -> (remapped VarId, Type)
        let mut module_var_exports: HashMap<(String, String), (VarId, Type)> = HashMap::new();
        // Module function exports: (module_name, func_name) -> return Type
        let mut module_func_exports: HashMap<(String, String), Type> = HashMap::new();
        // Module class exports: (module_name, class_name) -> (remapped ClassId, class_name as String)
        let mut module_class_exports: HashMap<(String, String), (ClassId, String)> = HashMap::new();
        // Cross-module class field/method info: ClassId -> (field_offsets, field_types, method_return_types)
        let mut cross_module_class_info: HashMap<ClassId, CrossModuleClassInfo> = HashMap::new();

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

                module_var_exports.insert(
                    (module_name.clone(), var_name_str),
                    (remapped_var_id, var_type),
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
                module_func_exports.insert((module_name.clone(), func_name), return_type);
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
                let mut field_types: HashMap<String, pyaot_types::Type> = HashMap::new();
                let mut method_return_types: HashMap<String, pyaot_types::Type> = HashMap::new();
                for (i, field) in class_def.fields.iter().enumerate() {
                    let field_name = parsed.interner.resolve(field.name).to_string();
                    field_offsets.insert(field_name.clone(), i);
                    field_types.insert(field_name, field.ty.clone());
                }
                // Extract method return types
                for method_func_id in &class_def.methods {
                    if let Some(func_def) = parsed.hir.func_defs.get(method_func_id) {
                        let method_name = parsed.interner.resolve(func_def.name).to_string();
                        if let Some(ref ret_ty) = func_def.return_type {
                            method_return_types.insert(method_name, ret_ty.clone());
                        }
                    }
                }
                cross_module_class_info.insert(
                    remapped_class_id,
                    CrossModuleClassInfo {
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

            // Lower to MIR with module exports (type inference runs inside lower_module)
            let func_count = parsed.hir.functions.len();
            let class_count = parsed.hir.class_defs.len();
            let mut lowering = pyaot_lowering::Lowering::new_with_capacity(
                &mut parsed.interner,
                func_count,
                class_count,
            );
            lowering.set_module_var_exports(module_var_exports.clone());
            lowering.set_module_func_exports(module_func_exports.clone());
            lowering.set_module_class_exports(module_class_exports.clone());
            lowering.set_cross_module_class_info(cross_module_class_info.clone());
            lowering.set_var_id_offset(this_var_offset);
            lowering.set_class_id_offset(this_class_offset);

            let (module_mir, warnings) = lowering.lower_module(&parsed.hir).map_err(|e| {
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

            // Merge MIR functions into the unified module with new FuncIds
            for (_old_func_id, mut func) in module_mir.functions {
                // Mangle function name for non-main modules
                // Replace dots with underscores for valid symbol names
                let safe_module_name = module_name.replace('.', "_");
                let original_name = func.name.clone();
                if !is_main && original_name != "__pyaot_module_init__" {
                    func.name = format!("__module_{}_{}", safe_module_name, original_name);
                } else if !is_main && original_name == "__pyaot_module_init__" {
                    func.name = format!("__module_{}_init__", safe_module_name);
                }

                // Assign new unique FuncId to avoid collisions
                let new_func_id = FuncId::from(next_func_id);
                next_func_id += 1;
                func.id = new_func_id;

                // Track module init for calling order
                if !is_main && original_name == "__pyaot_module_init__" {
                    merged_mir
                        .module_init_order
                        .push((module_name.clone(), new_func_id));
                }

                // Intern function name
                merged_interner.intern(&func.name);

                // Remap InternedStrings in locals
                for (_local_id, local) in func.locals.iter_mut() {
                    if let Some(name) = local.name {
                        local.name = Some(remap_str(name));
                    }
                }

                // Remap InternedStrings in instructions
                for (_block_id, block) in func.blocks.iter_mut() {
                    for inst in block.instructions.iter_mut() {
                        Self::remap_instruction_strings(&mut inst.kind, &remap_str);
                    }
                    // Remap strings in terminator
                    Self::remap_terminator_strings(&mut block.terminator, &remap_str);
                }

                merged_mir.functions.insert(new_func_id, func);
            }

            // Merge vtables (note: vtables reference old FuncIds, may need fixing for complex cases)
            for vtable in module_mir.vtables {
                merged_mir.vtables.push(vtable);
            }
        }

        Ok((merged_mir, merged_interner))
    }

    /// Find the type of a variable from its assignment in module_init_stmts
    fn find_var_type_in_stmts(
        var_id: VarId,
        stmt_ids: &[hir::StmtId],
        hir_module: &hir::Module,
    ) -> Type {
        for stmt_id in stmt_ids {
            let stmt = &hir_module.stmts[*stmt_id];
            if let hir::StmtKind::Assign {
                target,
                type_hint,
                value,
            } = &stmt.kind
            {
                if *target == var_id {
                    // Prefer explicit type hint
                    if let Some(ty) = type_hint {
                        return ty.clone();
                    }
                    // Fall back to expression type
                    let value_expr = &hir_module.exprs[*value];
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
