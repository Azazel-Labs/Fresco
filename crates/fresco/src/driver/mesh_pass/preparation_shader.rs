//! Materialize the engine preparation function into a range-local stream.
use super::*;
use crate::ast::PreparedDraw;
use fresco_artifact::{ManifestMeshPreparation, ManifestVertexAttribute};

fn shape(ty: &str) -> Result<(&str, u32), String> {
    let ty = ty.split_whitespace().next().ok_or("empty vertex type")?;
    match ty {
        "f32" | "u32" | "i32" => Ok((ty, 1)),
        "vec2" => Ok(("f32", 2)),
        "vec3" => Ok(("f32", 3)),
        "vec4" | "color" => Ok(("f32", 4)),
        _ => Err(format!(
            "geometry preparation does not support vertex field type `{ty}`"
        )),
    }
}

pub(super) fn emit(
    draw: &PreparedDraw,
    members: &[&VertexInterfaceMemberDecl],
    input_ty: &str,
    function: &str,
    context: &ExprContext<'_>,
    structs: &[StructDecl],
) -> Result<(String, ManifestMeshPreparation), String> {
    let record = find_struct(
        structs,
        &super::super::prepared_geometry::element(&draw.resource),
    )
    .ok_or("missing prepared record")?;
    let mut offset = 0_u32;
    let mut alignment = 4;
    let mut attributes = Vec::new();
    for (location, field) in record.fields.iter().enumerate() {
        let (scalar, width) = shape(&field.ty_name)?;
        if !field.attrs.is_empty() {
            return Err("prepared vertex fields must be plain values".into());
        }
        let align = if width == 3 { 16 } else { width * 4 };
        alignment = alignment.max(align);
        offset = offset
            .checked_next_multiple_of(align)
            .ok_or("prepared vertex layout overflow")?;
        let format = match scalar {
            "f32" => "float32",
            "i32" => "sint32",
            "u32" => "uint32",
            _ => unreachable!("validated scalar"),
        };
        attributes.push(ManifestVertexAttribute {
            name: field.name.clone(),
            ty: field.ty_name.clone(),
            shader_location: u32::try_from(location).map_err(|_| "too many prepared fields")?,
            offset: Some(offset),
            gpu_format: Some(if width == 1 {
                format.into()
            } else {
                format!("{format}x{width}")
            }),
            required: true,
            defaulted: false,
        });
        offset = offset
            .checked_add(width * 4)
            .ok_or("prepared vertex layout overflow")?;
    }
    let stride = offset
        .checked_next_multiple_of(alignment)
        .ok_or("prepared vertex layout overflow")?;
    let raw = &context.binding_vars["__prepare_raw_vertices"];
    let indices = &context.binding_vars["__prepare_raw_indices"];
    let counts = &context.binding_vars["__prepare_counts"];
    let output = &context.binding_vars["__prepare_output"];
    let mut words = 0_u32;
    let mut arguments = Vec::new();
    for member in members {
        let (scalar, width) = shape(&member.ty_name)?;
        let components: Vec<_> = (0..width)
            .map(|i| {
                let value = format!("{raw}[base + {}u]", words + i);
                if scalar == "u32" {
                    value
                } else {
                    format!("bitcast<{scalar}>({value})")
                }
            })
            .collect();
        arguments.push(if width == 1 {
            components[0].clone()
        } else {
            format!("{}({})", wgsl_type(&member.ty_name)?, components.join(","))
        });
        words = words
            .checked_add(width)
            .ok_or("raw vertex layout overflow")?;
    }
    let entry = format!("{function}_compute");
    let code = format!(
        "@compute @workgroup_size(64) fn {entry}(@builtin(global_invocation_id) id: vec3<u32>) {{
        let local = id.x;
        if local >= {counts}.vertex_count {{ return; }}
        var source = {counts}.first + local;
        if {counts}.indexed != 0u {{ source = {indices}[source]; }}
        let base = source * {words}u;
        {output}[local] = {function}({input_ty}({}));
    }}\n",
        arguments.join(",")
    );
    Ok((
        code,
        ManifestMeshPreparation {
            node: draw.producer_node.clone(),
            resource_type: draw.resource.name.clone(),
            entry,
            vertex_stride: stride,
            attributes,
            workgroup_size: 64,
        },
    ))
}
