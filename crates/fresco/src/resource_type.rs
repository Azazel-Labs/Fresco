//! Backend-independent authored resource types and access validation.
use fresco_artifact::types::{ImageFormat, ImageUse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResourceType {
    Buffer(String),
    Image(ImageFormat),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShaderResourceType {
    Data(ResourceType),
    Sampler,
}

/// None means this is not a shader resource spelling. A recognized but invalid
/// resource remains an error rather than falling through to a record type.
pub(crate) fn shader_resource_type(ty: &str) -> Option<Result<ShaderResourceType, String>> {
    let ty: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
    if ty == "sampler" {
        Some(Ok(ShaderResourceType::Sampler))
    } else if ty.starts_with("texture2d<") || ty.starts_with("buffer<") {
        Some(resource_type(&ty, "read").map(ShaderResourceType::Data))
    } else {
        None
    }
}

/// Inputs and returned handles are immutable; only the operation's owned output
/// has write access. Keep access checking separate from physical WGSL storage
/// buffer access, where a write-only declaration lowers to read_write.
pub(crate) fn resource_type(ty: &str, access: &str) -> Result<ResourceType, String> {
    let ty: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
    let (image, inner) = if let Some(inner) = ty.strip_prefix("buffer<") {
        (false, inner)
    } else if let Some(inner) = ty.strip_prefix("texture2d<") {
        (true, inner)
    } else {
        return Err("resource types require a buffer or texture2d type".into());
    };
    let inner = inner
        .strip_suffix('>')
        .ok_or("unterminated resource type")?;
    let mut depth = 0usize;
    let mut separator = None;
    for (index, character) in inner.char_indices() {
        match character {
            '<' => depth += 1,
            '>' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or("unbalanced resource element type")?;
            }
            ',' if depth == 0 && separator.replace(index).is_some() => {
                return Err("resource type accepts an element and one access qualifier".into());
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("unbalanced resource element type".into());
    }
    let (element, actual) = separator.map_or((inner, "read"), |index| {
        (&inner[..index], &inner[index + 1..])
    });
    if actual != access {
        return Err(format!(
            "resource requires `{access}` access, found `{actual}`"
        ));
    }
    if image {
        let format = ImageFormat::parse(element)
            .ok_or_else(|| format!("unknown image format `{element}`"))?;
        let usage = if access == "write" {
            ImageUse::Storage
        } else {
            ImageUse::Sampled
        };
        if !format.info().supports(usage) {
            return Err(format!(
                "image format `{element}` does not support {usage:?} use"
            ));
        }
        Ok(ResourceType::Image(format))
    } else if element.is_empty() {
        Err("buffer requires an element type".into())
    } else {
        Ok(ResourceType::Buffer(element.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_buffer_elements_do_not_become_access_qualifiers() {
        for spelling in ["buffer<array<vec4, 2>>", "buffer<array<vec4, 2>, read>"] {
            assert_eq!(
                resource_type(spelling, "read").unwrap(),
                ResourceType::Buffer("array<vec4,2>".into())
            );
        }
        for spelling in [
            "buffer<array<vec4,2>",
            "buffer<vec4>>",
            "buffer<vec4,read,write>",
        ] {
            assert!(resource_type(spelling, "read").is_err(), "{spelling}");
        }
        assert!(resource_type("buffer<array<vec4,2>,write>", "read").is_err());
    }
}
