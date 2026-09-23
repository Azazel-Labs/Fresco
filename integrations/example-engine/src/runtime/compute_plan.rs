//! Checked host arithmetic for logical compute work and owned output allocations.
//!
//! Logical extents are shader-visible; physical allocations may contain a sentinel
//! element for empty work. Callers must guard shader accesses by the logical extent,
//! not by the physical buffer length or rounded workgroup count.
use super::RuntimeError;

#[derive(Clone, Copy, Debug)]
pub struct ComputeLimits {
    pub max_workgroup_size: [u32; 3],
    pub max_workgroup_invocations: u32,
    pub max_workgroups: u32,
    pub max_buffer_bytes: u64,
    pub max_storage_binding_bytes: u64,
    pub max_texture_dimension_2d: u32,
}

#[cfg(feature = "runtime")]
impl From<&wgpu::Limits> for ComputeLimits {
    fn from(limits: &wgpu::Limits) -> Self {
        Self {
            max_workgroup_size: [
                limits.max_compute_workgroup_size_x,
                limits.max_compute_workgroup_size_y,
                limits.max_compute_workgroup_size_z,
            ],
            max_workgroup_invocations: limits.max_compute_invocations_per_workgroup,
            max_workgroups: limits.max_compute_workgroups_per_dimension,
            max_buffer_bytes: limits.max_buffer_size,
            max_storage_binding_bytes: limits.max_storage_buffer_binding_size,
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
        }
    }
}

fn invalid(message: &str) -> RuntimeError {
    RuntimeError::PassPlan(format!("compute output: {message}"))
}

/// Extent arithmetic uses the same u32 domain as shader indices. Widening a
/// product only on the host would permit a shader-side index overflow.
pub fn checked_extent_product(extents: &[u32]) -> Result<u32, RuntimeError> {
    extents.iter().try_fold(1u32, |product, extent| {
        product
            .checked_mul(*extent)
            .ok_or_else(|| invalid("logical extent product exceeds u32"))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DispatchPlan {
    threads: [u32; 3],
    groups: [u32; 3],
}

impl DispatchPlan {
    pub fn new(
        threads: [u32; 3],
        workgroup: [u32; 3],
        limits: ComputeLimits,
    ) -> Result<Self, RuntimeError> {
        if workgroup
            .iter()
            .zip(limits.max_workgroup_size)
            .any(|(size, limit)| *size == 0 || *size > limit)
            || checked_extent_product(&workgroup)? > limits.max_workgroup_invocations
        {
            return Err(invalid("workgroup size exceeds device limits"));
        }
        if threads.contains(&0) {
            return Ok(Self {
                threads,
                groups: [0; 3],
            });
        }
        let mut groups = [0; 3];
        for axis in 0..3 {
            groups[axis] = threads[axis].div_ceil(workgroup[axis]);
            if groups[axis] > limits.max_workgroups {
                return Err(invalid("dispatch exceeds device workgroup count limits"));
            }
            // Every launched global invocation ID, including the padded tail,
            // must remain representable. The maximum ID is groups * size - 1.
            let launched = u64::from(groups[axis]) * u64::from(workgroup[axis]);
            if launched > u64::from(u32::MAX) + 1 {
                return Err(invalid("rounded dispatch overflows shader invocation IDs"));
            }
        }
        Ok(Self { threads, groups })
    }

    pub fn threads(self) -> [u32; 3] {
        self.threads
    }

    pub fn groups(self) -> [u32; 3] {
        self.groups
    }

    pub fn is_empty(self) -> bool {
        self.groups == [0; 3]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferAllocation {
    elements: u32,
    bytes: u64,
    stride: u64,
}

impl BufferAllocation {
    /// `stride` must be the validated shader layout's array stride, including
    /// alignment padding, rather than a sum of authored field sizes.
    pub fn new(elements: u32, stride: u64, limits: ComputeLimits) -> Result<Self, RuntimeError> {
        if stride == 0 || !stride.is_multiple_of(4) {
            return Err(invalid(
                "storage element stride must be a positive multiple of four",
            ));
        }
        let bytes = u64::from(elements.max(1))
            .checked_mul(stride)
            .ok_or_else(|| invalid("buffer byte size overflow"))?;
        if bytes > limits.max_buffer_bytes || bytes > limits.max_storage_binding_bytes {
            return Err(invalid(
                "buffer exceeds device allocation or storage binding limits",
            ));
        }
        Ok(Self {
            elements,
            bytes,
            stride,
        })
    }

    pub fn elements(self) -> u32 {
        self.elements
    }

    pub fn bytes(self) -> u64 {
        self.bytes
    }

    pub fn stride(self) -> u64 {
        self.stride
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageAllocation {
    logical: [u32; 2],
    physical: [u32; 2],
}

impl ImageAllocation {
    /// A zero-area logical image uses a one-texel sentinel. No invocation may
    /// sample or store that sentinel as authored output.
    pub fn new(extent: [u32; 2], limits: ComputeLimits) -> Result<Self, RuntimeError> {
        if extent.iter().any(|n| *n > limits.max_texture_dimension_2d)
            || limits.max_texture_dimension_2d == 0
        {
            return Err(invalid("image extent exceeds device limits"));
        }
        checked_extent_product(&extent)?;
        Ok(Self {
            logical: extent,
            physical: if extent.contains(&0) { [1, 1] } else { extent },
        })
    }

    pub fn logical(self) -> [u32; 2] {
        self.logical
    }

    pub fn physical(self) -> [u32; 2] {
        self.physical
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnedAllocation {
    Buffer(BufferAllocation),
    Image {
        format: fresco_artifact::types::ImageFormat,
        allocation: ImageAllocation,
        bytes: u64,
    },
}

impl OwnedAllocation {
    pub fn bytes(self) -> u64 {
        match self {
            Self::Buffer(buffer) => buffer.bytes(),
            Self::Image { bytes, .. } => bytes,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComputeInvocationPlan {
    pub dispatch: DispatchPlan,
    pub output: OwnedAllocation,
}

impl ComputeInvocationPlan {
    /// Resolve the authored plan without allocating GPU resources. The caller
    /// supplies an explicit remaining transient-byte budget for the frame/view.
    pub fn new(
        program: &fresco_artifact::ManifestGpuProgram,
        limits: ComputeLimits,
        byte_budget: u64,
        read: &impl Fn(&str, Option<&str>) -> Result<fresco_artifact::ComputeScalar, RuntimeError>,
    ) -> Result<Self, RuntimeError> {
        use fresco_artifact::types::{ImageFormat, ImageUse};
        use fresco_artifact::{ComputeScalar, ManifestComputeOutputLayout};
        let invocation = program
            .compute_invocation
            .as_ref()
            .ok_or_else(|| invalid("missing invocation plan"))?;
        if invocation.bindings != program.compute_bindings {
            return Err(invalid(
                "invocation bindings disagree with shader provenance",
            ));
        }
        let [entry] = program.entries.as_slice() else {
            return Err(invalid("owned compute requires one entry"));
        };
        program
            .compute_shader_reads(&entry.entry)
            .map_err(|m| RuntimeError::PassPlan(format!("compute output: {m}")))?;
        let evaluate = |value| super::compute_expression::evaluate(value, read);
        for requirement in &invocation.requirements {
            match evaluate(requirement)? {
                ComputeScalar::Bool(true) => {}
                ComputeScalar::Bool(false) => return Err(invalid("operation precondition failed")),
                _ => return Err(invalid("operation precondition must be bool")),
            }
        }
        let extent = |value| match evaluate(value)? {
            ComputeScalar::U32(value) => Ok(value),
            _ => Err(invalid("logical extent must be u32")),
        };
        let threads: Vec<_> = invocation
            .threads
            .iter()
            .map(extent)
            .collect::<Result<_, _>>()?;
        let threads: [u32; 3] = threads
            .try_into()
            .map_err(|_| invalid("dispatch requires three extents"))?;
        let workgroup = program
            .workgroup_size
            .ok_or_else(|| invalid("missing reflected workgroup size"))?;
        let dispatch = DispatchPlan::new(threads, workgroup, limits)?;
        let extents: Vec<_> = invocation
            .output
            .extents
            .iter()
            .map(extent)
            .collect::<Result<_, _>>()?;
        let binding = program
            .bindings
            .iter()
            .find(|binding| binding.name == invocation.output.binding)
            .ok_or_else(|| invalid("owned output binding is missing"))?;
        if binding.access != "write"
            || !matches!(
                program.compute_bindings.get(&binding.name),
                Some(fresco_artifact::ManifestComputeBindingSource::Output)
            )
        {
            return Err(invalid("owned output requires a write-only output binding"));
        }
        let output = match &invocation.output.layout {
            ManifestComputeOutputLayout::Buffer { element, stride } => {
                if binding.kind != "storage"
                    || binding.element_stride != Some(*stride)
                    || binding.ty != *element
                {
                    return Err(invalid(
                        "owned buffer layout disagrees with shader reflection",
                    ));
                }
                let [count] = extents.as_slice() else {
                    return Err(invalid("owned buffer requires one extent"));
                };
                OwnedAllocation::Buffer(BufferAllocation::new(*count, u64::from(*stride), limits)?)
            }
            ManifestComputeOutputLayout::Image { format } => {
                let format = ImageFormat::parse(format)
                    .ok_or_else(|| invalid("unknown owned image format"))?;
                let info = format.info();
                let signature: String = binding.ty.chars().filter(|c| !c.is_whitespace()).collect();
                if !info.supports(ImageUse::Storage)
                    || binding.kind != "storage_texture"
                    || signature != format!("texture_storage_2d<{},write>", info.name)
                {
                    return Err(invalid(
                        "owned image layout disagrees with shader reflection",
                    ));
                }
                let [width, height] = extents.as_slice() else {
                    return Err(invalid("owned image requires two extents"));
                };
                let allocation = ImageAllocation::new([*width, *height], limits)?;
                let texel_bytes = info
                    .texel_bytes
                    .ok_or_else(|| invalid("owned image format has no known texel size"))?;
                let bytes = allocation
                    .physical()
                    .iter()
                    .try_fold(u64::from(texel_bytes), |bytes, extent| {
                        bytes.checked_mul(u64::from(*extent))
                    })
                    .ok_or_else(|| invalid("image byte size overflow"))?;
                OwnedAllocation::Image {
                    format,
                    allocation,
                    bytes,
                }
            }
        };
        if output.bytes() > byte_budget {
            return Err(invalid(
                "owned output exceeds the remaining transient-byte budget",
            ));
        }
        Ok(Self { dispatch, output })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> ComputeLimits {
        ComputeLimits {
            max_workgroup_size: [256, 256, 64],
            max_workgroup_invocations: 256,
            max_workgroups: 65535,
            max_buffer_bytes: 1 << 30,
            max_storage_binding_bytes: 1 << 27,
            max_texture_dimension_2d: 8192,
        }
    }

    #[test]
    fn dispatch_keeps_logical_extents_separate_from_padding() {
        let plan = DispatchPlan::new([65, 9, 1], [8, 8, 1], limits()).unwrap();
        assert_eq!(plan.threads(), [65, 9, 1]);
        assert_eq!(plan.groups(), [9, 2, 1]);
        assert!(!plan.is_empty());
        for threads in [[0, 9, 1], [9, 0, 1], [9, 9, 0]] {
            let empty = DispatchPlan::new(threads, [8, 8, 1], limits()).unwrap();
            assert!(empty.is_empty());
            assert_eq!(empty.threads(), threads);
        }
    }

    #[test]
    fn dispatch_rejects_invalid_pipeline_and_launch_sizes() {
        for workgroup in [[0, 1, 1], [257, 1, 1], [1, 1, 65], [32, 32, 1]] {
            // Empty work must not hide an invalid pipeline.
            assert!(DispatchPlan::new([0; 3], workgroup, limits()).is_err());
        }
        assert!(DispatchPlan::new([65536, 1, 1], [1; 3], limits()).is_err());
        let large = ComputeLimits {
            max_workgroups: u32::MAX,
            ..limits()
        };
        assert!(DispatchPlan::new([u32::MAX, 1, 1], [7, 1, 1], large).is_err());
        assert!(DispatchPlan::new([u32::MAX, 1, 1], [8, 1, 1], large).is_ok());
    }

    #[test]
    fn buffer_allocation_checks_stride_empty_storage_and_both_limits() {
        let empty = BufferAllocation::new(0, 48, limits()).unwrap();
        assert_eq!(empty.elements(), 0);
        assert_eq!(empty.bytes(), 48);
        assert_eq!(BufferAllocation::new(3, 48, limits()).unwrap().bytes(), 144);
        for stride in [0, 3, 6, u64::MAX - 3] {
            assert!(BufferAllocation::new(2, stride, limits()).is_err());
        }
        assert!(BufferAllocation::new((1 << 25) + 1, 4, limits()).is_err());
        assert!(
            BufferAllocation::new(
                2,
                4,
                ComputeLimits {
                    max_buffer_bytes: 4,
                    ..limits()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn image_and_extent_arithmetic_preserve_empty_work_and_reject_overflow() {
        assert_eq!(checked_extent_product(&[100, 32]).unwrap(), 3200);
        assert!(checked_extent_product(&[u32::MAX, 2]).is_err());
        let empty = ImageAllocation::new([0, 128], limits()).unwrap();
        assert_eq!(empty.logical(), [0, 128]);
        assert_eq!(empty.physical(), [1, 1]);
        assert!(ImageAllocation::new([8193, 0], limits()).is_err());
        assert_eq!(
            ImageAllocation::new([64, 32], limits()).unwrap().physical(),
            [64, 32]
        );
    }
}
