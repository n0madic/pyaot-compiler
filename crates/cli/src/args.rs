//! CLI argument parsing

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "pyaot")]
#[command(about = "Python AOT Compiler", long_about = None)]
pub struct Args {
    /// Input Python source file
    #[arg(value_name = "FILE")]
    pub input: PathBuf,

    /// Output executable file
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Target triple (e.g., x86_64-unknown-linux-gnu)
    #[arg(long, value_name = "TRIPLE")]
    pub target: Option<String>,

    /// Emit HIR (JSON)
    #[arg(long)]
    pub emit_hir: bool,

    /// Emit MIR (JSON)
    #[arg(long)]
    pub emit_mir: bool,

    /// Path to runtime library
    #[arg(long, default_value = "target/release/libpyaot_runtime.a")]
    pub runtime_lib: PathBuf,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Run the compiled executable immediately after compilation
    #[arg(long)]
    pub run: bool,

    /// Additional directories to search for imported modules
    #[arg(long = "module-path", value_name = "DIR")]
    pub module_path: Vec<PathBuf>,

    /// Enable function inlining optimization
    #[arg(long)]
    pub inline: bool,

    /// Maximum instruction count for inlining (default: 50)
    #[arg(long, default_value = "50")]
    pub inline_threshold: usize,

    /// Enable dead code elimination optimization
    #[arg(long)]
    pub dce: bool,

    /// Enable constant folding and propagation optimization
    #[arg(long)]
    pub constfold: bool,

    /// Enable devirtualization (replace virtual calls with direct calls)
    #[arg(long)]
    pub devirtualize: bool,

    /// Enable property flattening (inline trivial @property getters)
    #[arg(long)]
    pub flatten_properties: bool,

    /// Enable all optimizations (inline + constfold + dce + devirtualize + flatten-properties)
    #[arg(short = 'O', long)]
    pub optimize: bool,

    /// Include debug information in the generated executable
    #[arg(long)]
    pub debug: bool,
}
