//! GPU geometry resources shared by mesh hosts. Shader and render-state policy
//! stay with the renderer that installs the authored mesh pass.
use std::collections::BTreeMap;

use fresco_artifact::ManifestVertexFactory;

use super::{
    RuntimeError,
    vertices::{VertexLimits, VertexValues, pack_vertices},
};

#[derive(Clone)]
pub struct MeshGeometry {
    vertex_buffer: wgpu::Buffer,
    index_buffer: Option<wgpu::Buffer>,
    attributes: Vec<wgpu::VertexAttribute>,
    stride: u64,
    vertex_count: u32,
    index_count: u32,
    draw_range: std::ops::Range<u32>,
    base_vertex: i32,
    prepared: bool,
    source_vectors: BTreeMap<String, std::sync::Arc<[[f32; 3]]>>,
    source_indices: Option<std::sync::Arc<[u32]>>,
}

fn gpu_format(name: &str) -> Option<wgpu::VertexFormat> {
    use wgpu::VertexFormat as F;
    Some(match name {
        "float32" => F::Float32,
        "float32x2" => F::Float32x2,
        "float32x3" => F::Float32x3,
        "float32x4" => F::Float32x4,
        "sint32" => F::Sint32,
        "sint32x2" => F::Sint32x2,
        "sint32x3" => F::Sint32x3,
        "sint32x4" => F::Sint32x4,
        "uint32" => F::Uint32,
        "uint32x2" => F::Uint32x2,
        "uint32x3" => F::Uint32x3,
        "uint32x4" => F::Uint32x4,
        _ => return None,
    })
}

impl MeshGeometry {
    pub(crate) fn supports(&self, factory: &ManifestVertexFactory) -> bool {
        let mut locations = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        factory.array_stride.map(u64::from) == Some(self.stride)
            && factory.attributes.len() == self.attributes.len()
            && factory.attributes.iter().all(|expected| {
                locations.insert(expected.shader_location)
                    && names.insert(&expected.name)
                    && self.attributes.iter().any(|actual| {
                        actual.shader_location == expected.shader_location
                            && Some(actual.offset) == expected.offset.map(u64::from)
                            && Some(actual.format)
                                == expected.gpu_format.as_deref().and_then(gpu_format)
                    })
            })
    }

    /// Prepare an independently owned candidate. Keep installed geometry until
    /// this future succeeds; no writes target the installed buffers.
    pub async fn prepare(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        factory: &ManifestVertexFactory,
        vertex_count: u32,
        streams: &BTreeMap<String, VertexValues<'_>>,
        indices: Option<&[u32]>,
    ) -> Result<Self, RuntimeError> {
        let limits = device.limits();
        let invalid = |reason: &str| RuntimeError::VertexLayout {
            factory: factory.name.clone(),
            reason: reason.into(),
        };
        let vertices = pack_vertices(
            factory,
            vertex_count,
            streams,
            VertexLimits {
                max_attributes: limits.max_vertex_attributes,
                max_stride: limits.max_vertex_buffer_array_stride,
                max_buffer_bytes: limits.max_buffer_size,
            },
        )?;
        let mut index_bytes = Vec::new();
        let index_count = if let Some(indices) = indices {
            let count = u32::try_from(indices.len())
                .map_err(|_| invalid("index count exceeds draw limits"))?;
            let bytes = indices
                .len()
                .checked_mul(4)
                .filter(|bytes| {
                    u64::try_from(*bytes).is_ok_and(|size| size <= limits.max_buffer_size)
                })
                .ok_or_else(|| invalid("index buffer exceeds device or host limits"))?;
            if indices.iter().any(|index| *index >= vertex_count) {
                return Err(invalid("index refers outside the supplied vertices"));
            }
            index_bytes
                .try_reserve_exact(bytes)
                .map_err(|_| invalid("cannot allocate index data"))?;
            for index in indices {
                index_bytes.extend_from_slice(&index.to_le_bytes());
            }
            count
        } else {
            0
        };
        let attributes = factory
            .attributes
            .iter()
            .map(|attribute| {
                Ok(wgpu::VertexAttribute {
                    format: attribute
                        .gpu_format
                        .as_deref()
                        .and_then(gpu_format)
                        .ok_or_else(|| invalid("unsupported vertex GPU format"))?,
                    offset: u64::from(
                        attribute
                            .offset
                            .ok_or_else(|| invalid("missing vertex offset"))?,
                    ),
                    shader_location: attribute.shader_location,
                })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let upload = |label, bytes: &[u8], usage| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: (bytes.len() as u64).max(4),
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            if !bytes.is_empty() {
                queue.write_buffer(&buffer, 0, bytes);
            }
            buffer
        };
        let vertex_buffer = upload(
            "mesh vertices",
            &vertices,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
        );
        let index_buffer = indices.map(|_| {
            upload(
                "mesh indices",
                &index_bytes,
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::STORAGE,
            )
        });
        // Pop every nested scope before yielding, so concurrent preparations do
        // not leave scopes stacked across an await in browser WebGPU.
        let allocation_result = allocation.pop();
        let validation_result = validation.pop();
        let allocation_error = allocation_result.await;
        let validation_error = validation_result.await;
        if let Some(error) = allocation_error.or(validation_error) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(Self {
            base_vertex: 0,
            prepared: false,
            source_vectors: factory
                .attributes
                .iter()
                .filter(|a| a.gpu_format.as_deref() == Some("float32x3"))
                .map(|a| {
                    let VertexValues::F32(values) = streams[&a.name] else {
                        unreachable!("validated stream type")
                    };
                    (
                        a.name.clone(),
                        std::sync::Arc::from(values.as_chunks::<3>().0),
                    )
                })
                .collect(),
            source_indices: indices.map(std::sync::Arc::from),
            vertex_buffer,
            index_buffer,
            attributes,
            stride: u64::from(factory.array_stride.expect("packing validates stride")),
            vertex_count,
            index_count,
            draw_range: 0..if indices.is_some() {
                index_count
            } else {
                vertex_count
            },
        })
    }

    /// Select an absolute index or vertex range while sharing the uploaded buffers.
    /// Each scene object retains this range through every `for self` operation.
    pub fn with_draw_range(&self, range: std::ops::Range<u32>) -> Result<Self, RuntimeError> {
        let count = if self.index_buffer.is_some() {
            self.index_count
        } else {
            self.vertex_count
        };
        if range.start > range.end || range.end > count {
            return Err(RuntimeError::PassPlan(
                "draw range is outside the uploaded geometry".into(),
            ));
        }
        Ok(Self {
            draw_range: range,
            ..self.clone()
        })
    }

    /// Compute a range-local AABB from an explicitly selected source vector stream.
    /// The engine owns the choice of position stream and subsequent deformation.
    pub fn source_bounds(
        &self,
        stream: &str,
    ) -> Result<Option<super::bounds::Bounds>, RuntimeError> {
        let vectors = self.source_vectors.get(stream).ok_or_else(|| {
            RuntimeError::PassPlan(format!("bounds position stream `{stream}` is unavailable"))
        })?;
        super::bounds::Bounds::from_points(self.draw_range.clone().map(|i| {
            let index = self
                .source_indices
                .as_ref()
                .map_or(i, |indices| indices[i as usize]);
            vectors[index as usize]
        }))
    }

    pub(crate) fn prepare_stream(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        metadata: &fresco_artifact::ManifestMeshPreparation,
    ) -> Result<(Self, BTreeMap<String, super::resources::Resource>), RuntimeError> {
        use super::resources::Resource;
        let invalid = |message: &str| RuntimeError::PassPlan(message.into());
        let count = self.draw_range.end - self.draw_range.start;
        if metadata.workgroup_size != 64
            || metadata.vertex_stride == 0
            || !metadata.vertex_stride.is_multiple_of(4)
        {
            return Err(invalid("invalid preparation layout or workgroup size"));
        }
        let limits = device.limits();
        let compute_limits = super::compute_plan::ComputeLimits::from(&limits);
        let allocation = super::compute_plan::BufferAllocation::new(
            count,
            u64::from(metadata.vertex_stride),
            compute_limits,
        )?;
        let index_allocation =
            super::compute_plan::BufferAllocation::new(count, 4, compute_limits)?;
        super::compute_plan::DispatchPlan::new([count, 1, 1], [64, 1, 1], compute_limits)?;
        if self.vertex_buffer.size() > limits.max_storage_buffer_binding_size
            || self
                .index_buffer
                .as_ref()
                .is_some_and(|b| b.size() > limits.max_storage_buffer_binding_size)
        {
            return Err(invalid("raw geometry exceeds storage binding limits"));
        }
        let index_bytes = usize::try_from(u64::from(count) * 4)
            .map_err(|_| invalid("prepared indices exceed host address space"))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(index_bytes)
            .map_err(|_| invalid("cannot allocate prepared index data"))?;
        for index in 0..count {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("prepared vertices"),
            size: allocation.bytes(),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("prepared indices"),
            size: index_allocation.bytes(),
            usage: wgpu::BufferUsages::INDEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if !bytes.is_empty() {
            queue.write_buffer(&indices, 0, &bytes);
        }
        let counts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("geometry counts"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let values = [
            count,
            count,
            self.draw_range.start,
            u32::from(self.index_buffer.is_some()),
        ];
        queue.write_buffer(
            &counts,
            0,
            &values
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let storage = |buffer: &wgpu::Buffer| Resource::Storage {
            buffer: buffer.clone(),
            min_size: buffer.size(),
        };
        let bounds = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("prepared geometry bounds"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let resources = BTreeMap::from([
            ("vertices".into(), storage(&vertices)),
            ("output".into(), storage(&vertices)),
            ("indices".into(), storage(&indices)),
            ("raw_vertices".into(), storage(&self.vertex_buffer)),
            (
                "raw_indices".into(),
                storage(self.index_buffer.as_ref().unwrap_or(&indices)),
            ),
            ("counts".into(), Resource::Uniform(counts)),
            ("bounds".into(), Resource::Uniform(bounds)),
        ]);
        Ok((
            Self {
                base_vertex: 0,
                prepared: true,
                source_vectors: BTreeMap::new(),
                source_indices: None,
                vertex_buffer: vertices,
                index_buffer: Some(indices),
                attributes: Vec::new(),
                stride: u64::from(metadata.vertex_stride),
                vertex_count: count,
                index_count: count,
                draw_range: 0..count,
            },
            resources,
        ))
    }

    pub(crate) fn preparation_groups(&self) -> u32 {
        self.vertex_count.div_ceil(64)
    }

    pub(crate) fn logical_counts(&self) -> (u32, u32) {
        (self.vertex_count, self.index_count)
    }

    pub fn vertex_layout(&self) -> wgpu::VertexBufferLayout<'_> {
        wgpu::VertexBufferLayout {
            array_stride: self.stride,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &self.attributes,
        }
    }

    /// Replace a prepared range's vertex storage while retaining its local indices.
    /// The caller must schedule the buffer's compute producer before this draw.
    pub fn with_generated_vertices(
        &self,
        vertices: wgpu::Buffer,
        stride: u64,
        count: u32,
        offset: u32,
    ) -> Result<Self, RuntimeError> {
        if !self.prepared || self.index_buffer.is_none() || !self.attributes.is_empty() {
            return Err(RuntimeError::PassPlan(
                "generated vertices require prepared indexed geometry".into(),
            ));
        }
        if !vertices.usage().contains(wgpu::BufferUsages::STORAGE) {
            return Err(RuntimeError::PassPlan(
                "generated vertices require storage buffer usage".into(),
            ));
        }
        let base_vertex =
            generated_range(self.vertex_count, count, stride, vertices.size(), offset)?;
        Ok(Self {
            vertex_buffer: vertices,
            stride,
            base_vertex,
            ..self.clone()
        })
    }

    /// Issue one object's draw after the caller binds a compatible pipeline and
    /// resources. Empty geometry issues no draw and needs no special host path.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.vertex_count == 0 {
            return;
        }
        // Prepared/generated vertices are read through explicit storage bindings
        // by vertex-index shaders; only authored vertex interfaces use fetch slots.
        if !self.attributes.is_empty() {
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        }
        if let Some(indices) = &self.index_buffer {
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(self.draw_range.clone(), self.base_vertex, 0..1);
        } else {
            pass.draw(self.draw_range.clone(), 0..1);
        }
    }
}

fn generated_range(
    source_count: u32,
    output_count: u32,
    stride: u64,
    bytes: u64,
    offset: u32,
) -> Result<i32, RuntimeError> {
    let invalid = |message: &str| RuntimeError::PassPlan(message.into());
    let required = u64::from(output_count)
        .checked_mul(stride)
        .filter(|_| stride > 0)
        .ok_or_else(|| invalid("generated vertex byte extent overflow or zero stride"))?;
    if required > bytes {
        return Err(invalid(
            "generated vertex allocation is smaller than its declared extent",
        ));
    }
    let end = offset
        .checked_add(source_count)
        .ok_or_else(|| invalid("generated vertex range overflow"))?;
    if end > output_count {
        return Err(invalid(
            "generated vertex range exceeds the output capacity",
        ));
    }
    i32::try_from(offset)
        .map_err(|_| invalid("generated base vertex exceeds the signed draw limit"))
}

#[cfg(test)]
mod generated_tests {
    use super::generated_range;

    #[test]
    fn generated_ranges_preserve_capacity_and_checked_draw_offsets() {
        assert_eq!(generated_range(30, 90, 48, 4320, 60).unwrap(), 60);
        assert_eq!(generated_range(0, 0, 48, 48, 0).unwrap(), 0);
        assert_eq!(
            generated_range(1, i32::MAX as u32 + 1, 4, u64::MAX, i32::MAX as u32).unwrap(),
            i32::MAX
        );
        for (source, output, stride, bytes, offset) in [
            (30, 90, 48, 4320, 61),
            (30, 90, 48, 4319, 60),
            (1, u32::MAX, 4, u64::MAX, u32::MAX),
            (1, u32::MAX, u64::MAX, u64::MAX, 0),
            (1, 1, 0, 4, 0),
            (0, u32::MAX, 4, u64::MAX, i32::MAX as u32 + 1),
        ] {
            assert!(generated_range(source, output, stride, bytes, offset).is_err());
        }
    }
}
