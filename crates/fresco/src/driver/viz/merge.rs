#![allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]

use naga::Handle;

pub(super) fn user_module_with_entry(
    variant_wgsl: &str,
    fn_name: &str,
) -> Result<(naga::Module, Handle<naga::Function>), String> {
    let module = naga::front::wgsl::parse_str(variant_wgsl)
        .map_err(|err| format!("failed to parse variant WGSL into naga module: {err}"))?;
    let user_fn = module
        .functions
        .iter()
        .find_map(|(handle, function)| {
            if function.name.as_deref() == Some(fn_name) {
                Some(handle)
            } else {
                None
            }
        })
        .ok_or_else(|| format!("entry function `{fn_name}` not found in parsed module"))?;
    Ok((module, user_fn))
}
