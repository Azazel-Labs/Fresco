//! Crate-owned engine contracts for compiler regressions.
use std::collections::HashMap;

pub(crate) const POLICY: &str = include_str!("../tests/fixtures/engines/policy/engine/engine.fr");

pub(crate) fn files(source: &str, filename: &str) -> HashMap<String, String> {
    HashMap::from([
        (filename.into(), source.into()),
        ("engine/engine.fr".into(), POLICY.into()),
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
