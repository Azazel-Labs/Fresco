#![allow(
    dead_code,
    reason = "Shared integration-test support: each test crate includes this module but uses only a subset of its helpers."
)]

//! naga-based WGSL validation shared by integration tests.

pub use naga::valid::Capabilities;
use naga::valid::{ValidationFlags, Validator};

/// Parses `wgsl` and validates it with all validation flags under `caps`.
pub fn validate_wgsl(wgsl: &str, caps: Capabilities) -> Result<naga::Module, String> {
    let module = naga::front::wgsl::parse_str(wgsl)
        .map_err(|error| format!("WGSL parse failed:\n{error}"))?;
    Validator::new(ValidationFlags::all(), caps)
        .validate(&module)
        .map_err(|error| format!("WGSL validation failed:\n{error:#?}"))?;
    Ok(module)
}

/// Like [`validate_wgsl`], but panics with the error and the full WGSL source.
pub fn assert_valid_wgsl(wgsl: &str, caps: Capabilities) -> naga::Module {
    validate_wgsl(wgsl, caps).unwrap_or_else(|error| panic!("{error}\nwgsl:\n{wgsl}"))
}
