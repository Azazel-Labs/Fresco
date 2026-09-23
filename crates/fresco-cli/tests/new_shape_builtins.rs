use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco_with_args, unique_temp_path};

#[test]
fn new_shape_builtins_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let c = center
    let p = plus(at: c, size: (0.5, 0.08), thickness: 0.08)
    let x = x_cross(at: c, size: (0.42, 0.42), thickness: 0.05)
    let r = ring(at: c, radius: 0.28, width: 0.04)
    let ln = line(from: (0.15, 0.2), to: (0.85, 0.8), thickness: 0.02)
    let pl = polyline(points: [(0.1, 0.7), (0.25, 0.55), (0.4, 0.62), (0.55, 0.45)], thickness: 0.02, closed: false)
    let sec = sector(at: c, radius: 0.24, from: -1.2, to: 0.8)
    let ar = arc(at: c, radius: 0.36, width: 0.03, from: -2.0, to: -0.2)
    let sup = superellipse(at: (0.24, 0.28), radii: (0.11, 0.08), power: 4.5)
    let rp = rounded_polygon(points: [(0.62, 0.18), (0.82, 0.22), (0.76, 0.36), (0.58, 0.32)], round: 0.02)
    let cr = crescent(at: (0.2, 0.8), radius: 0.11, offset: (0.04, 0.0))
    let ls = lens(at: (0.82, 0.78), radius: 0.08, separation: 0.07)
    let h = heart(at: (0.5, 0.2), size: 0.05)
    let g = gear(at: (0.15, 0.5), inner: 0.03, outer: 0.06, teeth: 10)
    let sb = starburst(at: (0.85, 0.5), inner: 0.02, outer: 0.07, rays: 14)
    let rh = rhombus(at: (0.32, 0.84), size: (0.13, 0.09))
    let pg = parallelogram(at: (0.68, 0.84), size: (0.14, 0.08), skew: 0.03)
    let dr = droplet(at: (0.5, 0.84), size: 0.05)

    compose {
        p |> fill(#e11d48)
        x |> fill(#fb923c)
        r |> fill(#eab308)
        ln |> fill(#22c55e)
        pl |> fill(#14b8a6)
        sec |> fill(#0ea5e9)
        ar |> fill(#6366f1)
        sup |> fill(#8b5cf6)
        rp |> fill(#d946ef)
        cr |> fill(#f43f5e)
        ls |> fill(#f59e0b)
        h |> fill(#ef4444)
        g |> fill(#f97316)
        sb |> fill(#84cc16)
        rh |> fill(#06b6d4)
        pg |> fill(#3b82f6)
        dr |> fill(#a855f7)
    }
}
"#;

    let path = unique_temp_path("new_shape_builtins_compile");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected new shape builtins sample to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}
