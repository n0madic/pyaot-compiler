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

// =============================================================================
// Argument Error Kind
// =============================================================================

/// Structured data for argument-related errors.
/// Consolidates TooManyPositionalArguments, DuplicateKeywordArgument,
/// UnexpectedKeywordArgument, and MissingRequiredArgument into one variant.
#[derive(Debug, Clone)]
pub enum ArgumentErrorKind {
    TooManyPositional { expected: usize, got: usize },
    DuplicateKeyword { name: String },
    UnexpectedKeyword { name: String },
    MissingRequired { name: String },
}

impl std::fmt::Display for ArgumentErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyPositional { expected, got } => {
                write!(
                    f,
                    "Too many positional arguments: expected {expected}, got {got}"
                )
            }
            Self::DuplicateKeyword { name } => {
                write!(f, "Duplicate keyword argument: '{name}'")
            }
            Self::UnexpectedKeyword { name } => {
                write!(f, "Unexpected keyword argument: '{name}'")
            }
            Self::MissingRequired { name } => {
                write!(f, "Missing required argument: '{name}'")
            }
        }
    }
}

impl ArgumentErrorKind {
    /// Miette label text for the span annotation
    pub fn label(&self) -> String {
        match self {
            Self::TooManyPositional { expected, .. } => {
                format!("expected {expected} argument(s)")
            }
            Self::DuplicateKeyword { name } => format!("'{name}' already provided"),
            Self::UnexpectedKeyword { .. } => "not a valid parameter".to_string(),
            Self::MissingRequired { name } => format!("missing argument '{name}'"),
        }
    }

    /// Miette help text (None if no help message applies)
    pub fn help(&self) -> Option<String> {
        match self {
            Self::TooManyPositional { .. } => Some(
                "check the function signature for the expected number of parameters".to_string(),
            ),
            Self::DuplicateKeyword { .. } => None,
            Self::UnexpectedKeyword { name } => Some(format!(
                "remove the '{name}' argument or check the function signature"
            )),
            Self::MissingRequired { name } => Some(format!("add the missing '{name}' argument")),
        }
    }
}

// =============================================================================
// Compiler Errors
// =============================================================================

#[derive(Debug, Clone, Error, Diagnostic)]
pub enum CompilerError {
    #[error("Parse error: {message}")]
    ParseError {
        message: String,
        #[label("parse error")]
        span: DiagnosticSpan,
    },

    #[error("Type error: {message}")]
    TypeError {
        message: String,
        #[label("type mismatch")]
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
        #[label("{message}")]
        span: DiagnosticSpan,
    },

    #[error("{kind}")]
    ArgumentError {
        kind: ArgumentErrorKind,
        #[label("{label}")]
        span: DiagnosticSpan,
        /// Precomputed label text from `kind.label()` for miette interpolation
        label: String,
        /// Precomputed help text from `kind.help()` for miette
        #[help]
        help: Option<String>,
    },

    #[error("Codegen error: {message}")]
    CodegenError {
        message: String,
        #[label("codegen error")]
        span: Option<SourceSpan>,
    },

    #[error("Link error: {message}")]
    LinkError { message: String },

    #[error("IO error: {0}")]
    IoError(String),
}

impl From<std::io::Error> for CompilerError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e.to_string())
    }
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
    /// Type mismatch detected during type checking
    TypeError { span: Span, message: String },
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
        let (span, message, label) = match self {
            CompilerWarning::DeadCode { span, message } => (span, message, "unreachable code"),
            CompilerWarning::TypeError { span, message } => (span, message, "type mismatch"),
        };

        use miette::{GraphicalReportHandler, NamedSource};

        let diagnostic_span = DiagnosticSpan(*span);
        let source_span: SourceSpan = diagnostic_span.into();

        let severity = match self {
            CompilerWarning::DeadCode { .. } => miette::Severity::Warning,
            CompilerWarning::TypeError { .. } => miette::Severity::Error,
        };

        let report = miette::Report::new(
            miette::MietteDiagnostic::new(message.clone())
                .with_severity(severity)
                .with_labels(vec![miette::LabeledSpan::at(source_span, label)]),
        )
        .with_source_code(NamedSource::new(source_name, source.to_string()));

        let mut output = String::new();
        GraphicalReportHandler::new()
            .render_report(&mut output, report.as_ref())
            .expect("failed to render warning");
        output
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

    /// Check if any warnings are type errors
    pub fn has_type_errors(&self) -> bool {
        self.warnings
            .iter()
            .any(|w| matches!(w, CompilerWarning::TypeError { .. }))
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

// =============================================================================
// Builder API
// =============================================================================

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

    /// Create a codegen error without source location
    pub fn codegen_error(message: impl Into<String>) -> Self {
        Self::CodegenError {
            message: message.into(),
            span: None,
        }
    }

    /// Create a codegen error with source location
    pub fn codegen_error_at(message: impl Into<String>, span: Span) -> Self {
        Self::CodegenError {
            message: message.into(),
            span: Some(DiagnosticSpan(span).into()),
        }
    }

    pub fn link_error(message: impl Into<String>) -> Self {
        Self::LinkError {
            message: message.into(),
        }
    }

    pub fn too_many_positional_arguments(expected: usize, got: usize, span: Span) -> Self {
        let kind = ArgumentErrorKind::TooManyPositional { expected, got };
        let label = kind.label();
        let help = kind.help();
        Self::ArgumentError {
            kind,
            span: span.into(),
            label,
            help,
        }
    }

    pub fn duplicate_keyword_argument(name: impl Into<String>, span: Span) -> Self {
        let kind = ArgumentErrorKind::DuplicateKeyword { name: name.into() };
        let label = kind.label();
        let help = kind.help();
        Self::ArgumentError {
            kind,
            span: span.into(),
            label,
            help,
        }
    }

    pub fn unexpected_keyword_argument(name: impl Into<String>, span: Span) -> Self {
        let kind = ArgumentErrorKind::UnexpectedKeyword { name: name.into() };
        let label = kind.label();
        let help = kind.help();
        Self::ArgumentError {
            kind,
            span: span.into(),
            label,
            help,
        }
    }

    pub fn missing_required_argument(name: impl Into<String>, span: Span) -> Self {
        let kind = ArgumentErrorKind::MissingRequired { name: name.into() };
        let label = kind.label();
        let help = kind.help();
        Self::ArgumentError {
            kind,
            span: span.into(),
            label,
            help,
        }
    }

    /// Format the error for display using miette's graphical renderer.
    pub fn format(&self, source_name: &str, source: &str) -> String {
        let has_span = !matches!(
            self,
            Self::CodegenError { span: None, .. } | Self::LinkError { .. } | Self::IoError(_)
        );

        if has_span {
            use miette::{GraphicalReportHandler, NamedSource};

            let report = miette::Report::new(self.clone())
                .with_source_code(NamedSource::new(source_name, source.to_string()));

            let mut output = String::new();
            GraphicalReportHandler::new()
                .render_report(&mut output, report.as_ref())
                .expect("failed to render error");
            output
        } else {
            format!("  x {}\n", self)
        }
    }
}
