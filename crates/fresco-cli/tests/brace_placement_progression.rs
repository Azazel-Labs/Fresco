use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco, unique_temp_path};

fn assert_compiles_with_style(suffix: &str, input: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected brace-placement style sample to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn compose_allows_newline_before_open_brace() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose
  {
    circle(at: center, radius: 0.2) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_style("brace_style_compose", input);
}

#[test]
fn compose_if_allows_newline_before_open_brace() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    if time > 0.0
    {
      circle(at: center, radius: 0.2) |> fill(#ffffff)
    }
  }
}
"#;

    assert_compiles_with_style("brace_style_if", input);
}

#[test]
fn compose_for_allows_newline_before_open_brace() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    for i in 0 .. 3
    {
      circle(at: (0.35 + i * 0.1, 0.5), radius: 0.04) |> fill(#7dd3fc)
    }
  }
}
"#;

    assert_compiles_with_style("brace_style_for", input);
}

#[test]
fn compose_in_space_allows_newline_before_open_brace() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    in space centered(aspect: preserve)
    {
      circle(at: center, radius: 0.2) |> fill(#ffffff)
    }
  }
}
"#;

    assert_compiles_with_style("brace_style_in_space", input);
}

#[test]
fn compose_if_accepts_bool_literals_true_false() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    if true {
      circle(at: center, radius: 0.2) |> fill(#ffffff)
    }
    if false {
      circle(at: center, radius: 0.2) |> fill(#ff0000)
    }
  }
}
"#;

    assert_compiles_with_style("brace_style_bool_literals", input);
}

#[test]
fn compose_if_allows_else_on_its_own_line() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    if true
    {
      circle(at: center, radius: 0.2) |> fill(#ffffff)
    }
    else
    {
      circle(at: center, radius: 0.2) |> fill(#000000)
    }
  }
}
"#;

    assert_compiles_with_style("brace_style_else_newline", input);
}
