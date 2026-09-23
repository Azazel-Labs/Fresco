//! Initialize compiler-owned path geometry without a compiler or source filesystem.
use std::collections::BTreeSet;

use fresco_artifact::{ManifestCanvas, PATH_SEGMENT_LAYOUT, PATH_SEGMENT_STRIDE};

use super::RuntimeError;
use super::storage::{StorageLimits, validate_canvas_storage_count};

struct Buffer {
    name: String,
    group: u32,
    binding: u32,
    bytes: Vec<u8>,
}

pub struct PathBuffers {
    buffers: Vec<Buffer>,
}

pub struct PathUpload<'a> {
    pub name: &'a str,
    pub group: u32,
    pub binding: u32,
    pub bytes: &'a [u8],
}

fn invalid(name: &str, reason: impl Into<String>) -> RuntimeError {
    RuntimeError::PathBuffer {
        name: name.into(),
        reason: reason.into(),
    }
}

impl PathBuffers {
    pub fn new(canvas: &ManifestCanvas, limits: StorageLimits) -> Result<Self, RuntimeError> {
        validate_canvas_storage_count(canvas, limits)?;
        let mut bindings: BTreeSet<_> = canvas
            .engine_pass
            .iter()
            .map(|pass| (pass.instance_uniform_group, pass.instance_uniform_binding))
            .chain(
                canvas
                    .global_uniforms
                    .iter()
                    .map(|def| (def.group, def.binding)),
            )
            .chain(canvas.textures.iter().map(|def| (def.group, def.binding)))
            .chain(canvas.sampler.iter().map(|def| (def.group, def.binding)))
            .chain(
                canvas
                    .storage_params
                    .iter()
                    .map(|def| (def.group, def.binding)),
            )
            .collect();
        let mut names = BTreeSet::new();
        let mut buffers = Vec::new();
        for def in &canvas.path_buffers {
            if !names.insert(&def.name) || !bindings.insert((def.group, def.binding)) {
                return Err(invalid(
                    &def.name,
                    "duplicate path name or colliding resource binding",
                ));
            }
            if def.group >= limits.max_bind_groups || def.binding >= limits.max_bindings_per_group {
                return Err(invalid(&def.name, "binding exceeds device limits"));
            }
            let data = def.data.as_ref().ok_or_else(|| invalid(&def.name,
                "artifact has no path geometry; recompile it with a compiler that emits path data"))?;
            if data.layout != PATH_SEGMENT_LAYOUT || data.stride != PATH_SEGMENT_STRIDE {
                return Err(invalid(
                    &def.name,
                    "unsupported path record layout or stride",
                ));
            }
            if def.segments == 0
                || def.segments != data.rows.len()
                || u32::try_from(def.segments).is_err()
            {
                return Err(invalid(
                    &def.name,
                    "segment count must match nonempty geometry and fit u32",
                ));
            }
            let size = def
                .segments
                .checked_mul(PATH_SEGMENT_STRIDE as usize)
                .filter(|size| {
                    u64::try_from(*size).is_ok_and(|size| size <= limits.max_buffer_bytes)
                })
                .ok_or_else(|| invalid(&def.name, "buffer size exceeds device limit"))?;
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(size).map_err(|error| {
                invalid(&def.name, format!("cannot allocate path buffer: {error}"))
            })?;
            bytes.resize(size, 0);
            for (row, output) in data
                .rows
                .iter()
                .zip(bytes.as_chunks_mut::<{ PATH_SEGMENT_STRIDE as usize }>().0)
            {
                let values = [
                    row.p0[0], row.p0[1], row.p1[0], row.p1[1], row.p2[0], row.p2[1], row.p3[0],
                    row.p3[1], row.s0, row.len,
                ];
                if values.iter().any(|value| !value.is_finite())
                    || !row.mid_u.is_finite()
                    || row.s0 < 0.0
                    || row.len < 0.0
                    || !(0.0..=1.0).contains(&row.mid_u)
                    || row.kind > 1
                {
                    return Err(invalid(
                        &def.name,
                        "invalid segment coordinates, lengths, kind, or midpoint",
                    ));
                }
                for (value, bytes) in values.iter().zip(output[..40].as_chunks_mut::<4>().0) {
                    bytes.copy_from_slice(&value.to_le_bytes());
                }
                output[40..44].copy_from_slice(&row.kind.to_le_bytes());
                output[44..48].copy_from_slice(&row.mid_u.to_le_bytes());
            }
            buffers.push(Buffer {
                name: def.name.clone(),
                group: def.group,
                binding: def.binding,
                bytes,
            });
        }
        Ok(Self { buffers })
    }

    pub fn uploads(&self) -> impl Iterator<Item = PathUpload<'_>> {
        self.buffers.iter().map(|buffer| PathUpload {
            name: &buffer.name,
            group: buffer.group,
            binding: buffer.binding,
            bytes: &buffer.bytes,
        })
    }
}
