//! Preserve the browser response's materialized collections while the shared
//! file format omits unused resources. This is a JS transport policy, not a
//! second manifest schema or an engine resource fallback.
use js_sys::{Array, Reflect};
use serde::Serialize;
use wasm_bindgen::JsValue;

pub(super) trait ManifestResponse: Serialize {}

fn materialize(object: &JsValue, arrays: &[&str], optional: &[&str]) -> Result<(), JsValue> {
    for name in arrays {
        let key = JsValue::from_str(name);
        if Reflect::get(object, &key)?.is_undefined() {
            Reflect::set(object, &key, &Array::new())?;
        }
    }
    for name in optional {
        let key = JsValue::from_str(name);
        if !Reflect::has(object, &key)? {
            Reflect::set(object, &key, &JsValue::UNDEFINED)?;
        }
    }
    Ok(())
}

pub(super) fn serialize_success(value: &impl ManifestResponse) -> Result<JsValue, JsValue> {
    // Match the artifact JSON and generated Record types, including technique maps.
    let result = value
        .serialize(&serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true))
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let manifest = Reflect::get(&result, &JsValue::from_str("manifest"))?;
    materialize(
        &manifest,
        &[
            "surfaces",
            "pipelines",
            "vertex_factories",
            "config_axes",
            "renderers",
            "tables",
            "gpu_programs",
            "techniques",
        ],
        &["build_profile"],
    )?;
    let canvases = Reflect::get(&manifest, &JsValue::from_str("canvases"))?;
    for canvas in Array::from(&canvases) {
        materialize(
            &canvas,
            &[
                "global_uniforms",
                "storage_params",
                "textures",
                "path_buffers",
            ],
            &["sampler", "engine_pass"],
        )?;
    }
    let surfaces = Reflect::get(&manifest, &JsValue::from_str("surfaces"))?;
    for surface in Array::from(&surfaces) {
        materialize(
            &surface,
            &[
                "global_uniforms",
                "params",
                "textures",
                "evaluation_variants",
                "custom_channels",
                "mesh_passes",
            ],
            &[
                "sampler",
                "material_properties",
                "surface_shader_entry",
                "schema_evaluator",
                "evaluation_shader_entry",
                "evaluation_shader_source",
                "evaluation_contract",
            ],
        )?;
    }
    Ok(result)
}
