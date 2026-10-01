//! Diagnostics: a tiny internal representation rendered through `ariadne`
//! for pretty, span-labelled compiler errors.

use crate::ast::Span;
use ariadne::{Color, Label, Report, ReportKind, Source};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diag {
    pub severity: Severity,
    pub span: Span,
    pub message: String,
    pub code: Option<String>,
    pub label: Option<String>,
    pub related_labels: Vec<(Span, String)>,
    pub help: Option<String>,
    pub file: Option<String>,
}

impl Diag {
    /// Render a parser error as a diagnostic, naming the innermost parse context.
    pub(crate) fn from_parse_error(e: crate::parser::PError<'_>) -> Self {
        if let chumsky::error::RichReason::Custom(msg) = e.reason() {
            let mut d = Self::error(e.span().clone(), msg.to_string());
            if let Some((label, _)) = e.contexts().last() {
                d = d.with_label(format!("while parsing {label}"));
            }
            return d;
        }

        let found = e
            .found()
            .map(ToString::to_string)
            .unwrap_or_else(|| "end of input".to_string());
        let mut expected: Vec<String> = e.expected().map(ToString::to_string).collect();
        expected.sort();
        expected.dedup();

        let mut d = Self::error(e.span().clone(), format!("unexpected {found}"));
        if let Some((label, _)) = e.contexts().last() {
            d = d.with_label(format!("while parsing {label}"));
        } else {
            d = d.with_label("unexpected token");
        }
        if !expected.is_empty() {
            d = d.with_help(format!("expected {}", expected.join(", ")));
        }
        d
    }
}

impl Diag {
    fn new(severity: Severity, span: Span, message: impl Into<String>) -> Self {
        Diag {
            severity,
            span,
            message: message.into(),
            code: None,
            label: None,
            related_labels: Vec::new(),
            help: None,
            file: None,
        }
    }

    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, span, message)
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, span, message)
    }

    pub fn info(span: Span, message: impl Into<String>) -> Self {
        Self::new(Severity::Info, span, message)
    }

    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_related_label(mut self, span: Span, label: impl Into<String>) -> Self {
        self.related_labels.push((span, label.into()));
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// Attach a source file when one is known; `None` leaves the diagnostic
    /// unchanged.
    pub fn with_file_opt(self, file: Option<impl Into<String>>) -> Self {
        match file {
            Some(file) => self.with_file(file),
            None => self,
        }
    }
}

/// Returns `true` when any diagnostic in `diags` is an error.
#[must_use]
pub fn has_errors(diags: &[Diag]) -> bool {
    diags.iter().any(Diag::is_error)
}

/// Render all diagnostics for one source file to stderr.
pub fn emit_all(filename: &str, src: &str, diags: &[Diag]) {
    for d in diags {
        let (kind, color) = match d.severity {
            Severity::Error => (ReportKind::Error, Color::Red),
            Severity::Warning => (ReportKind::Warning, Color::Yellow),
            Severity::Info => (ReportKind::Advice, Color::Blue),
        };

        let title = if let Some(code) = &d.code {
            format!("[{code}] {}", d.message)
        } else {
            d.message.clone()
        };

        let mut report = Report::build(kind, (filename, d.span.clone()))
            .with_message(title)
            .with_label(
                Label::new((filename, d.span.clone()))
                    .with_message(d.label.clone().unwrap_or_else(|| "here".to_string()))
                    .with_color(color),
            );
        for (span, label) in &d.related_labels {
            report = report.with_label(
                Label::new((filename, span.clone()))
                    .with_message(label.clone())
                    .with_color(color),
            );
        }
        if let Some(h) = &d.help {
            report = report.with_help(h);
        }
        // eprint is infallible enough for a CLI; ignore IO errors.
        let _ = report.finish().eprint((filename, Source::from(src)));
    }
}
