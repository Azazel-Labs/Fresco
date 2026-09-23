//! Shared binding resources and texture uploads for canvas and material draws.
use super::textures::TextureInputs;
use fresco_artifact::{ManifestSampler, ManifestTexture};
use std::collections::BTreeMap;

#[derive(Clone)]
pub(crate) enum Resource {
    Uniform(wgpu::Buffer),
    Storage { buffer: wgpu::Buffer, min_size: u64 },
    Texture(wgpu::TextureView),
    Sampler(wgpu::Sampler, wgpu::SamplerBindingType),
}

impl Resource {
    pub(crate) fn binding_type(&self) -> wgpu::BindingType {
        match self {
            Self::Uniform(buffer) => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(buffer.size()),
            },
            Self::Storage { min_size, .. } => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(*min_size),
            },
            Self::Texture(_) => wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            Self::Sampler(_, kind) => wgpu::BindingType::Sampler(*kind),
        }
    }
    pub(crate) fn binding(&self) -> wgpu::BindingResource<'_> {
        match self {
            Self::Uniform(buffer) | Self::Storage { buffer, .. } => buffer.as_entire_binding(),
            Self::Texture(view) => wgpu::BindingResource::TextureView(view),
            Self::Sampler(sampler, _) => wgpu::BindingResource::Sampler(sampler),
        }
    }
}

pub(crate) fn sampler_preset(
    device: &wgpu::Device,
    preset: fresco_artifact::types::SamplerPreset,
) -> Resource {
    let address = if preset.repeats() {
        wgpu::AddressMode::Repeat
    } else {
        wgpu::AddressMode::ClampToEdge
    };
    let filter = if preset.filtering() {
        wgpu::FilterMode::Linear
    } else {
        wgpu::FilterMode::Nearest
    };
    let mipmap = if preset.filtering() {
        wgpu::MipmapFilterMode::Linear
    } else {
        wgpu::MipmapFilterMode::Nearest
    };
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(preset.name()),
        address_mode_u: address,
        address_mode_v: address,
        address_mode_w: address,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: mipmap,
        ..Default::default()
    });
    Resource::Sampler(
        sampler,
        if preset.filtering() {
            wgpu::SamplerBindingType::Filtering
        } else {
            wgpu::SamplerBindingType::NonFiltering
        },
    )
}

/// Call after CPU binding/image validation, inside the caller's GPU error scopes.
pub(crate) fn append_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    definitions: &[ManifestTexture],
    sampler: Option<&ManifestSampler>,
    textures: &TextureInputs,
    resources: &mut BTreeMap<u32, Vec<(u32, Resource)>>,
) {
    for def in definitions {
        let image = &textures[&def.name];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&def.name),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &image.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width * 4),
                rows_per_image: Some(image.height),
            },
            texture.size(),
        );
        resources.entry(def.group).or_default().push((
            def.binding,
            Resource::Texture(texture.create_view(&Default::default())),
        ));
    }
    if let Some(def) = sampler {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("example repeat/linear sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        resources.entry(def.group).or_default().push((
            def.binding,
            Resource::Sampler(sampler, wgpu::SamplerBindingType::Filtering),
        ));
    }
}
