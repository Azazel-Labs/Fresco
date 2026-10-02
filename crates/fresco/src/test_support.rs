//! Crate-owned engine contracts for compiler regressions.
use std::collections::HashMap;

pub(crate) const POLICY: &str = include_str!("../tests/fixtures/engines/policy/engine/engine.fr");

/// The engine-registered `canvas` entry contract shared by raster regressions.
pub(crate) const CANVAS: &str = include_str!("../tests/fixtures/engines/canvas.fr");

/// The shared test engine: explicit rendering policy plus the `canvas` registration.
pub(crate) fn engine() -> String {
    format!("{POLICY}\n{CANVAS}")
}

pub(crate) fn files(source: &str, filename: &str) -> HashMap<String, String> {
    HashMap::from([
        (filename.into(), source.into()),
        ("engine/engine.fr".into(), engine()),
    ])
}

pub(crate) fn compile_source(
    source: &str,
    filename: &str,
    target: &str,
    explain: bool,
) -> Result<crate::driver::CompileOutput, Vec<crate::driver::DiagnosticRecord>> {
    crate::driver::compile_virtual_with_context(
        &files(source, filename),
        filename,
        target,
        explain,
        &crate::driver::CompileContext::default(),
    )
}

pub(crate) const PRELUDE: &str = include_str!("../tests/fixtures/engines/prelude.fr");
