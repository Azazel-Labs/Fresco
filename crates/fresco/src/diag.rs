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
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Diag {
            severity: Severity::Error,
            span,
            message: message.into(),
            code: None,
            label: None,
            related_labels: Vec::new(),
            help: None,
            file: None,
        }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Diag {
            severity: Severity::Warning,
            span,
            message: message.into(),
            code: None,
            label: None,
            related_labels: Vec::new(),
            help: None,
            file: None,
        }
    }

    pub fn info(span: Span, message: impl Into<String>) -> Self {
        Diag {
            severity: Severity::Info,
            span,
            message: message.into(),
            code: None,
            label: None,
            related_labels: Vec::new(),
            help: None,
            file: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_tag(self, tag: impl Into<String>) -> Self {
        self.with_code(tag)
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
