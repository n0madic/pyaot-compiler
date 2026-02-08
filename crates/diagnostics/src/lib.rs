//! Diagnostic reporting for the Python AOT compiler

#![forbid(unsafe_code)]
#![allow(unused_assignments)]

use miette::{Diagnostic, SourceSpan};
use pyaot_utils::Span;
use thiserror::Error;

/// Wrapper for Span that can be converted to SourceSpan
#[derive(Debug, Clone, Copy)]
pub struct DiagnosticSpan(pub Span);

impl From<Span> for DiagnosticSpan {
    fn from(span: Span) -> Self {
        DiagnosticSpan(span)
    }
}

impl From<DiagnosticSpan> for SourceSpan {
    fn from(span: DiagnosticSpan) -> Self {
        SourceSpan::new(
            miette::SourceOffset::from(span.0.start as usize),
            (span.0.end - span.0.start) as usize,
        )
    }
}

#[derive(Debug, Error, Diagnostic)]
pub enum CompilerError {
    #[error("Parse error: {message}")]
    ParseError {
        message: String,
        #[label("here")]
        span: DiagnosticSpan,
    },

    #[error("Type error: {message}")]
    TypeError {
        message: String,
        #[label("here")]
        span: DiagnosticSpan,
    },

    #[error("Name error: undefined name '{name}'")]
    NameError {
        name: String,
        #[label("undefined name")]
        span: DiagnosticSpan,
    },

    #[error("Semantic error: {message}")]
    SemanticError {
        message: String,
        #[label("here")]
        span: DiagnosticSpan,
    },

    #[error("Too many positional arguments: expected {expected}, got {got}")]
    TooManyPositionalArguments {
        expected: usize,
        got: usize,
        #[label("too many arguments")]
        span: DiagnosticSpan,
    },

    #[error("Duplicate keyword argument: '{name}'")]
    DuplicateKeywordArgument {
        name: String,
        #[label("duplicate argument")]
        span: DiagnosticSpan,
    },

    #[error("Unexpected keyword argument: '{name}'")]
    UnexpectedKeywordArgument {
        name: String,
        #[label("unexpected keyword argument")]
        span: DiagnosticSpan,
    },

    #[error("Missing required argument: '{name}'")]
    MissingRequiredArgument {
        name: String,
        #[label("missing argument")]
        span: DiagnosticSpan,
    },

    #[error("Codegen error: {message}")]
    CodegenError { message: String },

    #[error("Link error: {message}")]
    LinkError { message: String },

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, CompilerError>;

// =============================================================================
// Compiler Warnings
// =============================================================================

/// Compiler warnings (non-fatal diagnostics)
#[derive(Debug, Clone)]
pub enum CompilerWarning {
    /// Unreachable code detected (e.g., isinstance check that's always True/False)
    DeadCode { span: Span, message: String },
}

impl CompilerWarning {
    /// Create a dead code warning
    pub fn dead_code(message: impl Into<String>, span: Span) -> Self {
        Self::DeadCode {
            span,
            message: message.into(),
        }
    }

    /// Format the warning for display using miette
    pub fn format(&self, source_name: &str, source: &str) -> String {
        match self {
            CompilerWarning::DeadCode { span, message } => {
                use miette::{GraphicalReportHandler, NamedSource};

                let diagnostic_span = DiagnosticSpan(*span);
                let source_span: SourceSpan = diagnostic_span.into();

                let report = miette::Report::new(
                    miette::MietteDiagnostic::new(message.clone())
                        .with_severity(miette::Severity::Warning)
                        .with_labels(vec![miette::LabeledSpan::at(
                            source_span,
                            "unreachable code",
                        )]),
                )
                .with_source_code(NamedSource::new(source_name, source.to_string()));

                let mut output = String::new();
                GraphicalReportHandler::new()
                    .render_report(&mut output, report.as_ref())
                    .expect("failed to render warning");
                output
            }
        }
    }
}

/// Collection of warnings emitted during compilation
#[derive(Debug, Default, Clone)]
pub struct CompilerWarnings {
    warnings: Vec<CompilerWarning>,
}

impl CompilerWarnings {
    /// Create a new empty warnings collection
    pub fn new() -> Self {
        Self {
            warnings: Vec::new(),
        }
    }

    /// Add a warning to the collection
    pub fn add(&mut self, warning: CompilerWarning) {
        self.warnings.push(warning);
    }

    /// Check if there are any warnings
    pub fn is_empty(&self) -> bool {
        self.warnings.is_empty()
    }

    /// Get the number of warnings
    pub fn len(&self) -> usize {
        self.warnings.len()
    }

    /// Emit all warnings to stderr
    pub fn emit_all(&self, source_name: &str, source: &str) {
        for warning in &self.warnings {
            eprintln!("{}", warning.format(source_name, source));
        }
    }

    /// Merge warnings from another collection
    pub fn merge(&mut self, other: CompilerWarnings) {
        self.warnings.extend(other.warnings);
    }

    /// Get an iterator over the warnings
    pub fn iter(&self) -> impl Iterator<Item = &CompilerWarning> {
        self.warnings.iter()
    }
}

impl CompilerError {
    pub fn parse_error(message: impl Into<String>, span: Span) -> Self {
        Self::ParseError {
            message: message.into(),
            span: span.into(),
        }
    }

    pub fn type_error(message: impl Into<String>, span: Span) -> Self {
        Self::TypeError {
            message: message.into(),
            span: span.into(),
        }
    }

    pub fn name_error(name: impl Into<String>, span: Span) -> Self {
        Self::NameError {
            name: name.into(),
            span: span.into(),
        }
    }

    pub fn semantic_error(message: impl Into<String>, span: Span) -> Self {
        Self::SemanticError {
            message: message.into(),
            span: span.into(),
        }
    }

    pub fn codegen_error(message: impl Into<String>) -> Self {
        Self::CodegenError {
            message: message.into(),
        }
    }

    pub fn link_error(message: impl Into<String>) -> Self {
        Self::LinkError {
            message: message.into(),
        }
    }

    pub fn too_many_positional_arguments(expected: usize, got: usize, span: Span) -> Self {
        Self::TooManyPositionalArguments {
            expected,
            got,
            span: span.into(),
        }
    }

    pub fn duplicate_keyword_argument(name: impl Into<String>, span: Span) -> Self {
        Self::DuplicateKeywordArgument {
            name: name.into(),
            span: span.into(),
        }
    }

    pub fn unexpected_keyword_argument(name: impl Into<String>, span: Span) -> Self {
        Self::UnexpectedKeywordArgument {
            name: name.into(),
            span: span.into(),
        }
    }

    pub fn missing_required_argument(name: impl Into<String>, span: Span) -> Self {
        Self::MissingRequiredArgument {
            name: name.into(),
            span: span.into(),
        }
    }
}
