use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco, run_fresco_with_args, unique_temp_path};

#[test]
fn arbitrary_grouped_brace_blocks_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  space stage = centered(aspect: preserve)

  {
    compose {
      fill(#0a1220)
      {
        in space stage {
          {
            circle(at: (0.5, 0.5), radius: 0.12) |> fill(#ffffff)
          }
        }
      }
    }
  }
}
"#;

    let path = unique_temp_path("grouped_brace_block_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected grouped brace blocks to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains("sdf: 1 evaluation(s)"),
        "expected grouped brace blocks to preserve inner layer evaluation\nexplain:\n{explain}"
    );
}

#[test]
fn grouped_brace_block_bindings_do_not_leak() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  {
    let inner = circle(at: (0.5, 0.5), radius: 0.12)
    inner |> fill(#ffffff)
  }

  inner |> fill(#ff00ff)
}
"#;

    let path = unique_temp_path("grouped_brace_scope_no_leak");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected grouped brace scope leak to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unknown name `inner`"),
        "expected unknown-name diagnostic for leaked binding\nstderr:\n{stderr}"
    );
}

#[test]
fn viewport_units_vw_vh_vmax_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let c = circle(at: center, radius: 4vmax)
  compose {
    fill(#0b1220)
    c |> glow(reach: 2vw, strength: 0.5, color: #7dd3fc) |> blend(add)
    box(at: (0.5, 0.15), size: (30vw, 8vh)) |> fill(#ffffff22)
    c |> fill(#93c5fd)
  }
}
"#;

    let path = unique_temp_path("viewport_units_vw_vh_vmax_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected vw/vh/vmax units to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn in_space_empty_block_is_allowed() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    fill(#0b1220)
    in space centered(aspect: preserve) {
    }
  }
}
"#;

    let path = unique_temp_path("in_space_empty_block_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected empty in-space block to compile as no-op\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn in_space_statement_blend_modes_are_preserved() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    fill(#0b1220)
    in space centered(aspect: preserve) {
      circle(at: (0.44, 0.5), radius: 0.11) |> fill(#ff5ca8)
      circle(at: (0.56, 0.5), radius: 0.11) |> fill(#7dd3fc) |> blend(add)
    }
  }
}
"#;

    let path = unique_temp_path("in_space_stmt_blend_modes");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected in-space statement-level blend to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert!(
        wgsl.contains("compose_add"),
        "expected additive compose path in lowered WGSL\nwgsl:\n{wgsl}"
    );
}

#[test]
fn scatter_add_blend_does_not_force_opaque_alpha() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let stars = scatter 1 within screen seed 7 strategy compact {
    circle(at: instance.pos, radius: 1px) |> fill(#ffffff)
  }

  compose {
    fill(#05070f)
    stars |> blend(add)
  }
}
"#;

    let path = unique_temp_path("scatter_add_non_opaque_alpha");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scatter add blend test to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert!(
        !wgsl.contains("clamp((0f + 1f), 0f, 1f)"),
        "scatter add blend should not lower to constant opaque alpha\nwgsl:\n{wgsl}"
    );
}

#[test]
fn nested_in_space_block_preserves_multiple_layers() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  space stage = centered(aspect: preserve)
  let spin = wave(period: 4s, shape: saw, range: 0deg .. 360deg)

  compose {
    fill(#0a1220)
    in space stage {
      in space rotate(angle: spin, around: (0.5, 0.5)) {
        capsule(from: (0.50, 0.00), to: (0.82, 0.00), radius: 0.045) |> fill(#9ccfff)
      }

      circle(at: (0.5, 0.5), radius: 0.03) |> fill(#ffd28f)
    }
  }
}
"#;

    let path = unique_temp_path("nested_in_space_layers");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected nested in-space block to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains("sdf: 2 evaluation(s)"),
        "expected both nested in-space layers to be retained (2 SDF evaluations)\nexplain:\n{explain}"
    );
}

#[test]
fn aspect_space_transform_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  space framed = aspect(ratio: 16/9)

  compose {
    fill(#0b1220)

    in space framed {
      compose {
        box(at: (0.5, 0.5), size: (0.7, 0.45)) |> fill(#ffffff)
      }
    }
  }
}
"#;

    let path = unique_temp_path("aspect_space_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected aspect space transform to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn aspect_space_transform_rejects_non_positive_ratio() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space aspect(ratio: 0) {
      box(at: center, size: (0.7, 0.45)) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("aspect_space_bad_ratio");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected non-positive aspect ratio to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("`aspect` ratio must be > 0"),
        "expected ratio validation diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn repeat_y_space_transform_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space repeat_y(every: 0.1) {
      box(at: (0.5, 0.5), size: (0.6, 0.02)) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("repeat_y_space_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected repeat_y space transform to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn repeat_space_named_cell_binding_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    fill(#101827)
    in space repeat(every: (0.12, 0.12), cell: tile) {
      let r = 0.018 + 0.01 * hash(tile)
      circle(at: tile.center, radius: r)
        |> fill(mix(#5eead4, #f59e0b, hash(tile, salt: 1)))
    }
  }
}
"#;

    let path = unique_temp_path("repeat_space_named_cell_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected named repeat-cell binding to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        !stderr.contains("deprecated"),
        "did not expect deprecation warning for `cell:`\nstderr:\n{stderr}"
    );
}

#[test]
fn repeat_space_id_cell_binding_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    fill(#101827)
    in space repeat(every: (0.12, 0.12), id: cell) {
      let r = 0.018 + 0.01 * hash(cell.id)
      circle(at: cell.center, radius: r)
        |> fill(mix(#5eead4, #f59e0b, hash(cell.id, salt: 1)))
    }
  }
}
"#;

    let path = unique_temp_path("repeat_space_id_cell_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected `id:` repeat-cell binding to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("Warning")
            && stderr.contains("`repeat(..., id: ...)` is deprecated; use `cell:`"),
        "expected deprecation warning for `id:` repeat-cell binding\nstderr:\n{stderr}"
    );
}

#[test]
fn repeat_space_as_cell_binding_compiles_with_warning() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    fill(#101827)
    in space repeat(every: (0.12, 0.12), as: tile) {
      let r = 0.018 + 0.01 * hash(tile.id)
      circle(at: tile.center, radius: r)
        |> fill(mix(#5eead4, #f59e0b, hash(tile.id, salt: 1)))
    }
  }
}
"#;

    let path = unique_temp_path("repeat_space_as_cell_binding_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected `as:` repeat-cell binding to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("Warning")
            && stderr.contains("`repeat(..., as: ...)` is deprecated; use `cell:`"),
        "expected deprecation warning for `as:` repeat-cell binding\nstderr:\n{stderr}"
    );
}

#[test]
fn repeat_space_rejects_duplicate_binding_aliases() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    fill(#101827)
    in space repeat(every: (0.12, 0.12), cell: cell, id: tile) {
      circle(at: cell.center, radius: 0.02)
        |> fill(#5eead4)
    }
  }
}
"#;

    let path = unique_temp_path("repeat_space_duplicate_binding_aliases");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected duplicate repeat binding aliases to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("accepts at most one of `cell: ...`, `id: ...`, or `as: ...`"),
        "expected duplicate repeat binding alias diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn repeat_space_named_cell_binding_nests_cleanly() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    fill(#0f172a)
    in space repeat(every: (0.25, 0.25), cell: tile) {
      let tint = hash(tile)
      in space repeat(every: (0.5, 0.5), cell: micro) {
        let dx = (micro.uv.x - 0.5) * 0.08
        let dy = (micro.uv.y - 0.5) * 0.08
        circle(at: tile.center + (dx, dy), radius: 0.012 + 0.004 * hash(micro, salt: tile.id.x))
          |> fill(mix(#22d3ee, #f97316, tint))
      }
    }
  }
}
"#;

    let path = unique_temp_path("repeat_space_named_cell_nested_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected nested named repeat-cell bindings to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn repeat_space_hash_cell_matches_hash_cell_id() {
    let direct = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    fill(#101827)
    in space repeat(every: (0.12, 0.12), cell: tile) {
      let r = 0.018 + 0.01 * hash(tile)
      circle(at: tile.center, radius: r)
        |> fill(mix(#5eead4, #f59e0b, hash(tile, salt: 1)))
    }
  }
}
"#;
    let via_id = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    fill(#101827)
    in space repeat(every: (0.12, 0.12), cell: tile) {
      let r = 0.018 + 0.01 * hash(tile.id)
      circle(at: tile.center, radius: r)
        |> fill(mix(#5eead4, #f59e0b, hash(tile.id, salt: 1)))
    }
  }
}
"#;

    let direct_path = unique_temp_path("repeat_space_hash_cell_direct");
    fs::write(&direct_path, direct).expect("failed to write direct hash test source");
    let direct_output = run_fresco(&direct_path);
    let _ = fs::remove_file(&direct_path);

    let via_id_path = unique_temp_path("repeat_space_hash_cell_id");
    fs::write(&via_id_path, via_id).expect("failed to write cell-id hash test source");
    let via_id_output = run_fresco(&via_id_path);
    let _ = fs::remove_file(&via_id_path);

    assert!(
        direct_output.status.success(),
        "expected `hash(cell)` program to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&direct_output.stdout),
        normalize(&direct_output.stderr)
    );
    assert!(
        via_id_output.status.success(),
        "expected `hash(cell.id)` program to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&via_id_output.stdout),
        normalize(&via_id_output.stderr)
    );

    assert_eq!(
        normalize(&direct_output.stdout),
        normalize(&via_id_output.stdout),
        "`hash(cell)` should lower identically to `hash(cell.id)`"
    );
}

#[test]
fn repeat_y_space_transform_rejects_non_positive_period() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space repeat_y(every: 0) {
      box(at: center, size: (0.6, 0.02)) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("repeat_y_space_bad_period");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected non-positive repeat_y period to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("`repeat_y` period must be > 0"),
        "expected repeat_y period validation diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn repeat_radial_space_transform_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space repeat_radial(count: 9, around: center, from: -90deg, to: 90deg) {
      capsule(from: (0.5, 0.5), to: (0.5, 0.1), radius: 0.004) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("repeat_radial_space_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected repeat_radial space transform to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn repeat_radial_space_transform_rejects_invalid_count() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space repeat_radial(count: 1, around: center, from: -90deg, to: 90deg) {
      circle(at: center, radius: 0.1) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("repeat_radial_space_bad_count");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected invalid repeat_radial count to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("`repeat_radial` count must be >= 2"),
        "expected repeat_radial count validation diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn repeat_radial_space_transform_accepts_explicit_angle_list() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let center = (0.5, 0.5)
  compose {
    in space repeat_radial(around: center, angles: [-90deg, -45deg, 0deg, 45deg, 90deg]) {
      capsule(from: (0.5, 0.5), to: (0.5, 0.1), radius: 0.004) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("repeat_radial_space_angles_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected repeat_radial explicit angle list to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn repeat_radial_space_transform_accepts_non_uniform_angle_list() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let center = (0.5, 0.5)
  compose {
    in space repeat_radial(around: center, angles: [-90deg, -20deg, 0deg, 90deg]) {
      circle(at: center, radius: 0.1) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("repeat_radial_space_angles_non_uniform");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected non-uniform repeat_radial angle list to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn repeat_radial_space_transform_rejects_non_ascending_angle_list() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let center = (0.5, 0.5)
  compose {
    in space repeat_radial(around: center, angles: [90deg, 10deg, -20deg, -90deg]) {
      circle(at: center, radius: 0.1) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("repeat_radial_space_angles_non_ascending");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected non-ascending repeat_radial angle list to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains(
            "`repeat_radial(angles: [...])` currently requires strictly increasing angles"
        ),
        "expected explicit non-ascending angle-list diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn perspective_space_transform_compiles_flat_v1() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space perspective(fov: 45deg, near: 0.01, far: 100.0, origin: center) {
      box(at: center, size: (0.5, 0.3)) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("perspective_space_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected perspective space transform to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn perspective_space_transform_rejects_invalid_planes() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space perspective(fov: 45deg, near: 1.0, far: 0.5, origin: center) {
      box(at: center, size: (0.5, 0.3)) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("perspective_space_bad_planes");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected invalid perspective planes to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("`perspective` far plane must be greater than near"),
        "expected perspective plane validation diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn perspective_space_supports_translate3_and_rotate_y() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space perspective(fov: 45deg, near: 0.01, far: 100.0, origin: center).translate3(x: 0.0, y: 0.0, z: 0.2).rotate_y(angle: 20deg, around: center) {
      box(at: center, size: (0.5, 0.3)) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("perspective_space_rotate_y_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected perspective translate3 + rotate_y to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn in_space_transform_chain_allows_newline_before_dot() {
    let input = r##"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space perspective(fov: 55deg, near: 0.02, far: 12.0, origin: center)
      .rotate_x(angle: -8deg, around: center)
      .rotate_y(angle: 18deg, around: center)
      .translate3(x: 0.0, y: 0.0, z: 0.03) {
      box(at: center, size: (0.30, 0.20)) |> fill(#ffffff)
    }
  }
}
"##;

    let path = unique_temp_path("aspect_in_space_dot_newline_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected newline-before-dot space transform chain to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn perspective_space_supports_rotate_x_and_rotate_z() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space perspective(fov: 60deg, near: 0.02, far: 50.0, origin: center).rotate_x(angle: 15deg, around: center).rotate_z(angle: -5deg, around: center) {
      box(at: center, size: (0.5, 0.3)) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("perspective_space_rotate_xz_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected perspective rotate_x + rotate_z to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn orientation_space_transform_compiles_for_y_down() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  space screen = orientation(y: down)

  compose {
    in space screen {
      compose {
        circle(at: (0.5, 0.8), radius: 0.12) |> fill(#ffd27a)
      }
    }
  }
}
"#;

    let path = unique_temp_path("orientation_space_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected orientation space transform to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn orientation_space_transform_rejects_unknown_axis_direction() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space orientation(y: sideways) {
      circle(at: center, radius: 0.1) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("orientation_space_bad_axis");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected invalid orientation axis to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("`orientation(y: ...)` expects `up` or `down`"),
        "expected orientation validation diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn orientation_space_sets_default_orientation_with_explicit_wrapper() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space orientation(y: down) {
      circle(at: (0.5, 0.8), radius: 0.12) |> fill(#ffd27a)
    }
  }
}
"#;

    let path = unique_temp_path("orientation_space_wrapper_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected explicit orientation space to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn canvas_space_is_rejected() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  canvas_space = orientation(y: up)
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("canvas_space_removed_reject");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected canvas_space usage to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("`canvas_space` has been removed"),
        "expected removed canvas_space diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn orientation_accepts_enum_qualified_variant() {
    let input = r#"enum VerticalAxis {
  up,
  down,
}

canvas t(uv: coord, time: signal) -> color {
  compose {
    in space orientation(y: VerticalAxis.down) {
      circle(at: center, radius: 0.1) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("enum_orientation_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected enum-qualified orientation to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn orientation_rejects_ambiguous_bare_enum_variant() {
    let input = r#"enum VerticalAxis {
  down,
}

enum Flow {
  down,
}

canvas t(uv: coord, time: signal) -> color {
  compose {
    in space orientation(y: down) {
      circle(at: center, radius: 0.1) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("enum_orientation_ambiguous");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected ambiguous bare enum variant to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("ambiguous enum variant `down`"),
        "expected ambiguous enum variant diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn anchor_literal_compiles_in_coord_like_builtin_param() {
    let input = r#"canvas t(pos: coord, t: signal) -> color {
  compose {
    box(at: center, size: (0.7, 0.45)) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("anchor_literal_coord_like_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected anchor literal in coord_like builtin param to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn qualified_anchor_literal_compiles_in_coord_like_builtin_param() {
    let input = r#"canvas t(pos: coord, t: signal) -> color {
  compose {
    circle(at: Anchor.top_center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("qualified_anchor_literal_coord_like_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected qualified anchor literal in coord_like builtin param to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}
