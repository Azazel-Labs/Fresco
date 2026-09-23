export function formatMs(value: unknown): string {
  const n = Number(value);
  if (!Number.isFinite(n)) {
    return "n/a";
  }
  if (n >= 1000) {
    return `${(n / 1000).toFixed(2)}s`;
  }
  return `${n.toFixed(1)}ms`;
}

export function formatTimingBreakdown(timings: any): string {
  if (!timings || typeof timings !== "object") {
    return "";
  }

  const p = timings.pipeline || {};
  const lines = [
    "Compile timings:",
    `  total: ${formatMs(timings.total_ms)}`,
    `  pipeline total: ${formatMs(p.total_ms)}`,
    `    lex: ${formatMs(p.lex_ms)}`,
    `    parse: ${formatMs(p.parse_ms)}`,
    `    resolve imports: ${formatMs(p.resolve_imports_ms)}`,
    `    check + rewrite: ${formatMs(p.check_rewrite_ms)}`,
    `    lower + validate: ${formatMs(p.lower_validate_ms)}`,
    `    map diagnostics: ${formatMs(p.map_diagnostics_ms)}`,
    `  emit wgsl: ${formatMs(timings.emit_wgsl_ms)}`,
    `  emit manifest: ${formatMs(timings.emit_manifest_ms)}`,
    `  render explain: ${formatMs(timings.explain_ms)}`
  ];

  return lines.join("\n");
}

export function formatHostTimingBreakdown(hostTimings: any): string {
  if (!hostTimings || typeof hostTimings !== "object") {
    return "";
  }

  const lines = [
    "Host timings (UI thread):",
    `  startup time-to-first-compile: ${formatMs(hostTimings.startup_time_to_first_compile_ms)}`,
    `  total wall (queued->done): ${formatMs(hostTimings.total_wall_ms)}`,
    `  active wall (worker start->done): ${formatMs(hostTimings.active_wall_ms)}`,
    `  worker round-trip: ${formatMs(hostTimings.worker_round_trip_ms)}`,
    `  worker compile wall: ${formatMs(hostTimings.worker_compile_ms)}`,
    `  worker overhead (round-trip - compile): ${formatMs(hostTimings.worker_overhead_ms)}`,
    `  queue wait before worker: ${formatMs(hostTimings.queue_wait_ms)}`,
    `  debounce target delay: ${formatMs(hostTimings.debounce_delay_ms)}`,
    `  debounce timer lag: ${formatMs(hostTimings.debounce_timer_lag_ms)}`,
    `  browser consume wall (worker result->done): ${formatMs(hostTimings.browser_consume_wall_ms)}`,
    `  main apply wall: ${formatMs(hostTimings.main_apply_ms)}`,
    `  set source diagnostics: ${formatMs(hostTimings.set_source_diagnostics_ms)}`,
    `  set WGSL editor text: ${formatMs(hostTimings.set_wgsl_editor_ms)}`,
    `  render explain panel: ${formatMs(hostTimings.render_explain_panel_ms)}`,
    `  render diagnostics panel: ${formatMs(hostTimings.render_diagnostics_panel_ms)}`,
    `  preview shader build: ${formatMs(hostTimings.preview_shader_build_ms)}`,
    `    captured in this compile: ${hostTimings.preview_shader_build_captured ? "yes" : "no (async staged)"}`,
    `    apply mode: ${hostTimings.preview_apply_mode}`,
    `    async pending now: ${hostTimings.preview_async_build_pending ? "yes" : "no"}`,
    `    async running now: ${formatMs(hostTimings.preview_async_build_running_ms)}`,
    `    last async build total: ${formatMs(hostTimings.preview_last_async_build_total_ms)}`,
    `    last async build age: ${formatMs(hostTimings.preview_last_async_build_age_ms)}`,
    `    last async error: ${hostTimings.preview_last_async_build_error || "none"}`,
    `    pipeline cache hit: ${hostTimings.preview_pipeline_cache_hit ? "yes" : "no"}`,
    `    setup texture bindings: ${formatMs(hostTimings.preview_setup_texture_bindings_ms)}`,
    `    build fs args + shader code: ${formatMs(hostTimings.preview_build_shader_code_ms)}`,
    `    create shader module: ${formatMs(hostTimings.preview_create_shader_module_ms)}`,
    `    create pipeline layout: ${formatMs(hostTimings.preview_create_pipeline_layout_ms)}`,
    `    create render pipeline async: ${formatMs(hostTimings.preview_create_pipeline_async_ms)}`,
    `    first draw warmup: ${formatMs(hostTimings.preview_first_draw_warmup_ms)}`,
    `    first draw succeeded: ${hostTimings.preview_first_draw_ok ? "yes" : "no"}`,
    `  render params panel: ${formatMs(hostTimings.render_params_panel_ms)}`,
    `  payload WGSL chars: ${hostTimings.payload_wgsl_chars}`,
    `  payload manifest chars: ${hostTimings.payload_manifest_chars}`,
    `  payload explain chars: ${hostTimings.payload_explain_chars}`
  ];

  return lines.join("\n");
}
