use crate::{enum_decl, type_decl};

type_decl!("shape", Shape, "2D signed-distance shape value");
type_decl!("path", Path, "path value with arc-length metadata channels");
type_decl!("layer", Layer, "composable layer/effect value");
type_decl!("scalar", Scalar, "scalar expression value");
type_decl!("f32", Scalar, "32-bit floating-point scalar value");
type_decl!("i32", I32, "signed 32-bit integer scalar value");
type_decl!("u32", U32, "unsigned 32-bit integer scalar value");
type_decl!(
    "f64",
    F64,
    "64-bit floating-point scalar value (lowered to runtime scalar)"
);
type_decl!(
    "half",
    Half,
    "16-bit floating-point scalar value (lowered to runtime scalar)"
);
type_decl!("vec2", Vec2, "2D vector value");
type_decl!("vec3", Vec3, "3D vector value");
type_decl!("vec4", Vec4, "4D vector value");
type_decl!("mat2", Mat2, "2x2 matrix value");
type_decl!("mat3", Mat3, "3x3 matrix value");
type_decl!("mat4", Mat4, "4x4 matrix value");
type_decl!("color", Color, "RGBA color value");
type_decl!("color_field", ColorField, "dynamic color expression");
type_decl!("coverage", Coverage, "coverage signal value");
type_decl!("mask", Mask, "mask signal value");
type_decl!("gradient", Gradient, "gradient construction value");
type_decl!("bool", Bool, "boolean scalar value");
type_decl!("coord", Coord, "canvas coordinate alias for vec2");
type_decl!("coord_like", CoordLike, "coordinate-like vec2 value");
type_decl!("signal", Signal, "time signal scalar value");
type_decl!("delta", Delta, "delta-time signal scalar value");
type_decl!("resolution", Resolution, "canvas resolution alias for vec2");
type_decl!("angle", Angle, "angle scalar value");
type_decl!("length", Length, "length scalar value");

enum_decl!(
    "FillRule",
    "polygon and path fill rule",
    [
        ("non_zero", "non-zero winding fill rule"),
        ("even_odd", "even-odd fill rule"),
    ]
);

enum_decl!(
    "Axis",
    "coordinate axis for line families and grids",
    [("x", "horizontal axis"), ("y", "vertical axis"),]
);

enum_decl!(
    "PlaneMode",
    "polygon 3D projection mode",
    [
        ("auto", "auto-resolve plane behavior"),
        ("explicit", "explicit plane normal and origin"),
    ]
);

enum_decl!(
    "Anchor",
    "canonical shape anchor points",
    [
        ("center", "center anchor"),
        ("top_center", "top-center anchor"),
        ("bottom_center", "bottom-center anchor"),
        ("left_center", "left-center anchor"),
        ("right_center", "right-center anchor"),
        ("top_left", "top-left anchor"),
        ("top_right", "top-right anchor"),
        ("bottom_left", "bottom-left anchor"),
        ("bottom_right", "bottom-right anchor"),
    ]
);

enum_decl!(
    "WaveShape",
    "waveform shape for signal synthesis",
    [
        ("sine", "sine wave"),
        ("saw", "sawtooth wave"),
        ("triangle", "triangle wave"),
        ("square", "square wave"),
    ]
);

enum_decl!(
    "Easing",
    "signal easing function",
    [
        ("linear", "linear easing"),
        ("out_quad", "quadratic ease-out"),
        ("out_back", "back-overshoot ease-out"),
    ]
);

enum_decl!(
    "EaseTransition",
    "signal easing transition family",
    [
        ("linear", "linear transition"),
        ("sine", "sinusoidal transition"),
        ("quad", "quadratic transition"),
        ("cubic", "cubic transition"),
        ("quart", "quartic transition"),
        ("quint", "quintic transition"),
        ("expo", "exponential transition"),
        ("circ", "circular transition"),
        ("back", "overshoot back transition"),
        ("elastic", "elastic overshoot transition"),
        ("bounce", "bounce transition"),
    ]
);

enum_decl!(
    "EaseMode",
    "signal easing direction mode",
    [
        ("in", "ease-in mode"),
        ("out", "ease-out mode"),
        ("in_out", "ease-in then ease-out mode"),
        ("out_in", "ease-out then ease-in mode"),
    ]
);

enum_decl!(
    "CellSampling",
    "screen-pixel sampling for cell ownership and content",
    [
        ("center", "one sample at the pixel center; no supersampling"),
        ("grid2x2", "four samples on a 2 by 2 grid"),
        ("grid3x3", "nine samples on a 3 by 3 grid"),
        ("grid4x4", "sixteen samples on a 4 by 4 grid"),
    ],
    contextual
);

enum_decl!(
    "CellLayout",
    "cell ownership and site arrangement",
    [
        ("square", "regular square cells with centered sites"),
        (
            "brick",
            "square cells staggered by half a column on alternate rows"
        ),
        (
            "hex",
            "hexagonal ownership around a staggered triangular lattice"
        ),
        ("jittered", "square ownership with seeded displaced sites"),
        (
            "voronoi",
            "nearest-site ownership around seeded displaced sites"
        ),
    ]
);

enum_decl!(
    "CenteredMode",
    "aspect mode for centered transform",
    [
        ("preserve", "preserve authored framing"),
        ("fit", "fit content inside frame"),
        ("fill", "fill frame while cropping overflow"),
    ]
);

enum_decl!(
    "YAxis",
    "vertical axis orientation",
    [
        ("up", "positive y points upward"),
        ("down", "positive y points downward"),
    ]
);

enum_decl!(
    "PolarDir",
    "rotation direction in polar mapping",
    [
        ("clockwise", "angle increases clockwise"),
        ("counterclockwise", "angle increases counterclockwise"),
    ]
);

enum_decl!(
    "BlendMode",
    "compose block blend mode",
    [
        ("over", "alpha compositing over"),
        ("add", "additive blend"),
        ("screen", "screen blend"),
        ("multiply", "multiply blend"),
    ]
);

enum_decl!(
    "GradientAxis",
    "gradient axis shorthand",
    [
        ("x", "horizontal x axis"),
        ("y", "vertical y axis"),
        ("horizontal", "horizontal axis alias"),
        ("vertical", "vertical axis alias"),
    ]
);

enum_decl!(
    "BandProfile",
    "distance band falloff",
    [
        ("solid", "solid line with a smooth edge"),
        ("soft", "exponential glow, half-bright at half width")
    ],
    contextual
);
enum_decl!(
    "ContourDirection",
    "contour traversal direction",
    [
        ("clockwise", "clockwise in the normalized cell plane"),
        (
            "counterclockwise",
            "counterclockwise in the normalized cell plane"
        )
    ],
    contextual
);

type_decl!(
    "contour",
    Contour,
    "Closed cell boundary with distance, arc-length, and point queries; a compile-time geometry value, not a drawing layer."
);
enum_decl!(
    "ContourMotion",
    "how a chase moves around a contour",
    [
        ("perimeter", "constant distance along the border"),
        ("angular", "constant rotation around the cell site")
    ],
    contextual
);
