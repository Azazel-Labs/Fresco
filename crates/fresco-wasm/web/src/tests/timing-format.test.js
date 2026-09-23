import { describe, expect, it } from "vitest";

import { formatHostTimingBreakdown } from "../app/timing-format";

describe("host timing formatting", () => {
  it("includes startup time-to-first-compile telemetry", () => {
    const formatted = formatHostTimingBreakdown({
      startup_time_to_first_compile_ms: 321.7,
      total_wall_ms: 0,
      active_wall_ms: 0,
      worker_round_trip_ms: 0,
      worker_compile_ms: 0,
      worker_overhead_ms: 0,
      queue_wait_ms: 0,
      debounce_delay_ms: 0,
      debounce_timer_lag_ms: 0,
      browser_consume_wall_ms: 0,
      main_apply_ms: 0,
      set_source_diagnostics_ms: 0,
      set_wgsl_editor_ms: 0,
      render_explain_panel_ms: 0,
      render_diagnostics_panel_ms: 0,
      preview_shader_build_ms: 0,
      preview_shader_build_captured: false,
      preview_apply_mode: "staged",
      preview_async_build_pending: false,
      preview_async_build_running_ms: 0,
      preview_last_async_build_total_ms: 0,
      preview_last_async_build_age_ms: 0,
      preview_last_async_build_error: "",
      preview_pipeline_cache_hit: false,
      preview_setup_texture_bindings_ms: 0,
      preview_build_shader_code_ms: 0,
      preview_create_shader_module_ms: 0,
      preview_create_pipeline_layout_ms: 0,
      preview_create_pipeline_async_ms: 0,
      preview_first_draw_warmup_ms: 0,
      preview_first_draw_ok: false,
      render_params_panel_ms: 0,
      payload_wgsl_chars: 0,
      payload_manifest_chars: 0,
      payload_explain_chars: 0,
    });

    expect(formatted).toContain("startup time-to-first-compile: 321.7ms");
  });
});
