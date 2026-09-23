//! Backend-independent resource type facts shared by compiler and artifact hosts.
//!
//! Membership here describes a language/IR type, not a promise that a particular
//! GPU or backend supports every use. Hosts must check their device capabilities.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageScalar {
    F32,
    I32,
    U32,
    U64,
}

/// Immutable standard-library sampler values; no backend-specific enum values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SamplerPreset {
    NearestRepeat,
    NearestClamp,
    LinearRepeat,
    LinearClamp,
}

impl SamplerPreset {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "nearest_repeat" => Some(Self::NearestRepeat),
            "nearest_clamp" => Some(Self::NearestClamp),
            "linear_repeat" => Some(Self::LinearRepeat),
            "linear_clamp" => Some(Self::LinearClamp),
            _ => None,
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::NearestRepeat => "nearest_repeat",
            Self::NearestClamp => "nearest_clamp",
            Self::LinearRepeat => "linear_repeat",
            Self::LinearClamp => "linear_clamp",
        }
    }
    pub const fn filtering(self) -> bool {
        matches!(self, Self::LinearRepeat | Self::LinearClamp)
    }
    pub const fn repeats(self) -> bool {
        matches!(self, Self::NearestRepeat | Self::LinearRepeat)
    }
}

impl ImageScalar {
    /// Numeric class of a fragment output. Extra components may be discarded by
    /// the attachment; backend validation checks the concrete pipeline layout.
    pub fn accepts_shader_output(self, ty: &str) -> bool {
        match self {
            Self::F32 => matches!(ty, "f32" | "vec2" | "vec3" | "vec4" | "color"),
            Self::I32 => matches!(ty, "i32" | "ivec2" | "ivec3" | "ivec4"),
            Self::U32 => matches!(ty, "u32" | "uvec2" | "uvec3" | "uvec4"),
            Self::U64 => matches!(ty, "u64" | "vec2<u64>" | "vec3<u64>" | "vec4<u64>"),
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::U64 => "u64",
        }
    }
    pub const fn vector(self) -> &'static str {
        match self {
            Self::F32 => "vec4",
            Self::I32 => "ivec4",
            Self::U32 => "uvec4",
            Self::U64 => "vec4<u64>",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageClass {
    Color,
    Depth,
    Stencil,
    DepthStencil,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageUse {
    Sampled,
    Storage,
    ColorAttachment,
    DepthStencilAttachment,
}

/// An authored image may require a concrete format or a floating color format
/// supplied by the host. The latter is not a concrete allocation/storage format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormatRequirement {
    Concrete(ImageFormat),
    FloatColor,
}

impl ImageFormatRequirement {
    pub fn parse(name: &str) -> Option<Self> {
        if name == "float_color" {
            Some(Self::FloatColor)
        } else {
            ImageFormat::parse(name).map(Self::Concrete)
        }
    }

    pub fn accepts_shader_output(self, ty: &str) -> bool {
        match self {
            Self::Concrete(format) => format.info().accepts_shader_output(ty),
            Self::FloatColor => ImageScalar::F32.accepts_shader_output(ty),
        }
    }

    pub fn sampled_shader_type(self) -> Option<String> {
        match self {
            Self::Concrete(format) => format.info().sampled_shader_type(),
            Self::FloatColor => Some("texture_2d<f32>".into()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ImageFormatInfo {
    pub name: &'static str,
    pub scalar: ImageScalar,
    pub channels: u8,
    /// None for depth formats whose physical representation is backend-defined.
    pub texel_bytes: Option<u8>,
    pub class: ImageClass,
    pub storage: bool,
}

impl ImageFormatInfo {
    pub fn sampled_shader_type(self) -> Option<String> {
        match self.class {
            ImageClass::Color => Some(format!("texture_2d<{}>", self.scalar.name())),
            ImageClass::Depth | ImageClass::DepthStencil => Some("texture_depth_2d".into()),
            ImageClass::Stencil => None,
        }
    }
    pub fn accepts_shader_output(self, ty: &str) -> bool {
        self.supports(ImageUse::ColorAttachment) && self.scalar.accepts_shader_output(ty)
    }
    pub const fn supports(self, usage: ImageUse) -> bool {
        match usage {
            ImageUse::Storage => self.storage,
            ImageUse::Sampled => !matches!(self.class, ImageClass::Stencil),
            ImageUse::ColorAttachment => matches!(self.class, ImageClass::Color),
            ImageUse::DepthStencilAttachment => !matches!(self.class, ImageClass::Color),
        }
    }
}

macro_rules! formats {
    (storage { $( $variant:ident: ($name:literal, $scalar:ident, $channels:literal, $bytes:literal) ),* $(,)? }
     other { $( $other:ident: ($other_name:literal, $other_scalar:ident, $other_channels:literal, $other_bytes:expr, $class:ident) ),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum ImageFormat { $( $variant, )* $( $other, )* }
        impl ImageFormat {
            pub const ALL: &'static [Self] = &[$( Self::$variant, )* $( Self::$other, )*];
            pub fn parse(name: &str) -> Option<Self> {
                match name { $( $name => Some(Self::$variant), )* $( $other_name => Some(Self::$other), )* _ => None }
            }
            pub const fn info(self) -> ImageFormatInfo {
                match self {
                    $( Self::$variant => ImageFormatInfo { name: $name, scalar: ImageScalar::$scalar, channels: $channels, texel_bytes: Some($bytes), class: ImageClass::Color, storage: true }, )*
                    $( Self::$other => ImageFormatInfo { name: $other_name, scalar: ImageScalar::$other_scalar, channels: $other_channels, texel_bytes: $other_bytes, class: ImageClass::$class, storage: false }, )*
                }
            }
            #[cfg(feature = "naga")]
            pub const fn naga_storage(self) -> Option<naga::StorageFormat> {
                match self { $( Self::$variant => Some(naga::StorageFormat::$variant), )* $( Self::$other => None, )* }
            }
        }
        // Deliberately exhaustive: a new Naga storage format requires registry
        // metadata instead of silently falling through a backend-specific list.
        #[cfg(feature = "naga")]
        impl From<naga::StorageFormat> for ImageFormat {
            fn from(format: naga::StorageFormat) -> Self {
                match format { $( naga::StorageFormat::$variant => Self::$variant, )* }
            }
        }
    };
}

formats! {
    storage {
        R8Unorm: ("r8unorm", F32, 1, 1),
        R8Snorm: ("r8snorm", F32, 1, 1),
        R8Uint: ("r8uint", U32, 1, 1),
        R8Sint: ("r8sint", I32, 1, 1),
        R16Uint: ("r16uint", U32, 1, 2),
        R16Sint: ("r16sint", I32, 1, 2),
        R16Float: ("r16float", F32, 1, 2),
        Rg8Unorm: ("rg8unorm", F32, 2, 2),
        Rg8Snorm: ("rg8snorm", F32, 2, 2),
        Rg8Uint: ("rg8uint", U32, 2, 2),
        Rg8Sint: ("rg8sint", I32, 2, 2),
        R32Uint: ("r32uint", U32, 1, 4),
        R32Sint: ("r32sint", I32, 1, 4),
        R32Float: ("r32float", F32, 1, 4),
        Rg16Uint: ("rg16uint", U32, 2, 4),
        Rg16Sint: ("rg16sint", I32, 2, 4),
        Rg16Float: ("rg16float", F32, 2, 4),
        Rgba8Unorm: ("rgba8unorm", F32, 4, 4),
        Rgba8Snorm: ("rgba8snorm", F32, 4, 4),
        Rgba8Uint: ("rgba8uint", U32, 4, 4),
        Rgba8Sint: ("rgba8sint", I32, 4, 4),
        Bgra8Unorm: ("bgra8unorm", F32, 4, 4),
        Rgb10a2Uint: ("rgb10a2uint", U32, 4, 4),
        Rgb10a2Unorm: ("rgb10a2unorm", F32, 4, 4),
        Rg11b10Ufloat: ("rg11b10ufloat", F32, 3, 4),
        R64Uint: ("r64uint", U64, 1, 8),
        Rg32Uint: ("rg32uint", U32, 2, 8),
        Rg32Sint: ("rg32sint", I32, 2, 8),
        Rg32Float: ("rg32float", F32, 2, 8),
        Rgba16Uint: ("rgba16uint", U32, 4, 8),
        Rgba16Sint: ("rgba16sint", I32, 4, 8),
        Rgba16Float: ("rgba16float", F32, 4, 8),
        Rgba32Uint: ("rgba32uint", U32, 4, 16),
        Rgba32Sint: ("rgba32sint", I32, 4, 16),
        Rgba32Float: ("rgba32float", F32, 4, 16),
        R16Unorm: ("r16unorm", F32, 1, 2),
        R16Snorm: ("r16snorm", F32, 1, 2),
        Rg16Unorm: ("rg16unorm", F32, 2, 4),
        Rg16Snorm: ("rg16snorm", F32, 2, 4),
        Rgba16Unorm: ("rgba16unorm", F32, 4, 8),
        Rgba16Snorm: ("rgba16snorm", F32, 4, 8),
    }
    other {
        Rgba8UnormSrgb: ("rgba8unorm_srgb", F32, 4, Some(4), Color),
        Bgra8UnormSrgb: ("bgra8unorm_srgb", F32, 4, Some(4), Color),
        Depth16Unorm: ("depth16unorm", F32, 1, Some(2), Depth),
        Depth24Plus: ("depth24plus", F32, 1, None, Depth),
        Depth24PlusStencil8: ("depth24plus_stencil8", F32, 2, None, DepthStencil),
        Depth32Float: ("depth32float", F32, 1, Some(4), Depth),
        Depth32FloatStencil8: ("depth32float_stencil8", F32, 2, None, DepthStencil),
        Stencil8: ("stencil8", U32, 1, Some(1), Stencil),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_bindings_use_scalar_class_instead_of_format_name_special_cases() {
        let host_color = ImageFormatRequirement::parse("float_color").unwrap();
        assert!(host_color.accepts_shader_output("color"));
        assert!(!host_color.accepts_shader_output("uvec4"));
        assert_eq!(
            host_color.sampled_shader_type().as_deref(),
            Some("texture_2d<f32>")
        );
        assert!(ImageFormat::parse("float_color").is_none());
        assert!(ImageFormatRequirement::parse("unknown_color").is_none());
        assert_eq!(
            ImageFormat::Rg32Uint
                .info()
                .sampled_shader_type()
                .as_deref(),
            Some("texture_2d<u32>")
        );
        assert!(ImageFormat::Rg32Uint.info().accepts_shader_output("uvec2"));
        assert!(!ImageFormat::Rg32Uint.info().accepts_shader_output("vec2"));
        assert!(ImageFormat::Rg32Float.info().accepts_shader_output("vec2"));
        assert!(
            !ImageFormat::Depth32Float
                .info()
                .accepts_shader_output("f32")
        );
        assert!(ImageFormat::Stencil8.info().sampled_shader_type().is_none());
    }
    #[test]
    fn format_registry_has_unique_names_and_consistent_uses() {
        let mut names = std::collections::BTreeSet::new();
        for format in ImageFormat::ALL {
            let info = format.info();
            assert!(names.insert(info.name));
            assert_eq!(ImageFormat::parse(info.name), Some(*format));
            assert_ne!(
                info.supports(ImageUse::ColorAttachment),
                info.supports(ImageUse::DepthStencilAttachment)
            );
            assert!(info.texel_bytes.is_none_or(|size| size > 0));
            if info.storage {
                assert_eq!(info.class, ImageClass::Color);
            }
        }
        assert!(ImageFormat::parse("not_a_format").is_none());
    }
    #[cfg(feature = "naga")]
    #[test]
    fn storage_metadata_agrees_with_naga_ir() {
        for format in ImageFormat::ALL {
            let Some(storage) = format.naga_storage() else {
                continue;
            };
            assert_eq!(ImageFormat::from(storage), *format);
            let scalar = naga::Scalar::from(storage);
            let expected = match format.info().scalar {
                ImageScalar::F32 => (naga::ScalarKind::Float, 4),
                ImageScalar::I32 => (naga::ScalarKind::Sint, 4),
                ImageScalar::U32 => (naga::ScalarKind::Uint, 4),
                ImageScalar::U64 => (naga::ScalarKind::Uint, 8),
            };
            assert_eq!((scalar.kind, scalar.width), expected);
        }
    }
}
