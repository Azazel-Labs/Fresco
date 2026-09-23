//! Display choices for the example engine's authored fullscreen inspectors.
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct BufferView {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub description: &'static str,
    #[serde(skip_serializing_if = "<[&str]>::is_empty")]
    pub tiles: &'static [&'static str],
    #[serde(skip)]
    pub mode: u32,
}

pub const VIEWS: &[BufferView] = &[
    BufferView {
        id: "shaded",
        label: "Shaded",
        group: "Output",
        description: "Final shaded image",
        tiles: &[],
        mode: 0,
    },
    BufferView {
        id: "shadow",
        label: "Sun shadow depth",
        group: "Lighting",
        description: "Shadow map depth: black near, white far",
        tiles: &[],
        mode: 1,
    },
    BufferView {
        id: "tiles",
        label: "Tile light count",
        group: "Lighting",
        description: "Point-light candidates per 16×16 tile: blue 0, red/yellow 64 (logarithmic scale)",
        tiles: &[],
        mode: 2,
    },
    BufferView {
        id: "depth",
        label: "Depth",
        group: "Geometry",
        description: "Device depth raised to power 32 for contrast: black near, white far",
        tiles: &[],
        mode: 3,
    },
    BufferView {
        id: "albedo",
        label: "Albedo",
        group: "GBuffer",
        description: "Linear albedo decoded from the sRGB attachment",
        tiles: &[],
        mode: 4,
    },
    BufferView {
        id: "normals",
        label: "World normals",
        group: "GBuffer",
        description: "Octahedral normals decoded and mapped from −1…1 to RGB",
        tiles: &[],
        mode: 5,
    },
    BufferView {
        id: "roughness",
        label: "Roughness",
        group: "GBuffer",
        description: "Black smooth, white rough",
        tiles: &[],
        mode: 6,
    },
    BufferView {
        id: "metallic",
        label: "Metallic",
        group: "GBuffer",
        description: "Black dielectric, white metallic",
        tiles: &[],
        mode: 7,
    },
    BufferView {
        id: "emissive",
        label: "Emissive / custom response",
        group: "GBuffer",
        description: "HDR color displayed using c / (1 + c)",
        tiles: &[],
        mode: 8,
    },
    BufferView {
        id: "occlusion",
        label: "Occlusion",
        group: "GBuffer",
        description: "Black occluded, white unoccluded",
        tiles: &[],
        mode: 9,
    },
    BufferView {
        id: "material-id",
        label: "Material IDs",
        group: "GBuffer",
        description: "Integer material IDs shown as stable colors; zero is background",
        tiles: &[],
        mode: 10,
    },
    BufferView {
        id: "all",
        label: "View all buffers",
        group: "Output",
        description: "Live overview of shaded output, lighting, depth, and GBuffer channels",
        tiles: &[
            "Shaded",
            "Sun shadow depth",
            "Tile light count",
            "Depth",
            "Albedo",
            "World normals",
            "Roughness",
            "Metallic",
            "Emissive / custom response",
            "Occlusion",
            "Material IDs",
        ],
        mode: 11,
    },
];

#[cfg(feature = "runtime")]
pub(crate) fn available(passes: impl Iterator<Item = impl AsRef<str>>) -> Vec<BufferView> {
    let mut common = false;
    let mut deferred = false;
    for pass in passes {
        common |= pass.as_ref() == "preview_buffer_view";
        deferred |= pass.as_ref() == "deferred_buffer_view";
    }
    VIEWS
        .iter()
        .copied()
        .filter(|view| deferred || (common && view.mode <= 3))
        .collect()
}
