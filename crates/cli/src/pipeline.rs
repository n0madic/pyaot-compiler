//! Single-module compilation pipeline

use crate::types::ParsedModule;
use miette::{NamedSource, Report, Result};
use pyaot_utils::StringInterner;
use std::path::{Path, PathBuf};

/// Compile a single module through semantic analysis, type checking, and MIR lowering
pub fn compile_single_module(
    mut parsed: ParsedModule,
    emit_hir: bool,
    verbose: bool,
) -> Result<(pyaot_mir::Module, StringInterner)> {
    if emit_hir {
        println!("HIR: {:#?}", parsed.hir);
    }

    // Create source context for error reporting
    let source_name = parsed.path.display().to_string();
    let source_code = parsed.source.clone();

    // Semantic analysis
    if verbose {
        println!("Performing semantic analysis...");
    }
    let mut sem_analyzer = pyaot_semantics::SemanticAnalyzer::new(&parsed.interner);
    sem_analyzer.analyze(&parsed.hir).map_err(|e| {
        Report::new(e).with_source_code(NamedSource::new(&source_name, source_code.clone()))
    })?;

    // Lower to MIR (includes type inference + codegen in one pass)
    if verbose {
        println!("Lowering to MIR...");
    }
    let func_count = parsed.hir.functions.len();
    let class_count = parsed.hir.class_defs.len();
    let lowering =
        pyaot_lowering::Lowering::new_with_capacity(&mut parsed.interner, func_count, class_count);
    let (mir_module, warnings) = lowering.lower_module(parsed.hir).map_err(|e| {
        Report::new(e).with_source_code(NamedSource::new(&source_name, source_code.clone()))
    })?;

    // Emit all warnings (type errors are warnings, not fatal)
    if !warnings.is_empty() {
        warnings.emit_all(&source_name, &source_code);
    }

    Ok((mir_module, parsed.interner))
}

/// Determine the output path for the compiled executable
pub fn determine_output_path(input: &Path, output: Option<PathBuf>) -> PathBuf {
    output.unwrap_or_else(|| {
        let mut path = input.to_path_buf();
        path.set_extension("");
        path
    })
}
