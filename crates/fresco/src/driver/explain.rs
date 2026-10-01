use crate::driver::EmitTarget;
use crate::driver::pass_plan::{self, KernelStrategy};
use crate::hir::Hir;
use crate::lower::Stats;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

/// WGSL module-level size metrics derived by analyzing the emitted WGSL text.
struct WgslEmitStats {
    fn_count: usize,
    largest_fn_lines: usize,
}

/// Analyze emitted WGSL text to count functions and measure the largest function.
///
/// Naga-emitted WGSL has every function definition starting with `fn ` at
/// column 0, with K&R brace style.  No string literals or macro-generated
/// braces exist in the output, so a simple depth counter is reliable.
fn analyze_wgsl_emit_stats(wgsl: &str) -> WgslEmitStats {
    let mut fn_count = 0usize;
    let mut largest_fn_lines = 0usize;
    let mut current_fn_lines = 0usize;
    let mut brace_depth: i64 = 0;
    let mut in_fn = false;

    for line in wgsl.lines() {
        if line.starts_with("fn ") {
            // Flush the previous function's line count before starting the new one.
            if in_fn && current_fn_lines > largest_fn_lines {
                largest_fn_lines = current_fn_lines;
            }
            fn_count += 1;
            in_fn = true;
            current_fn_lines = 1;
            brace_depth = 0;
            for ch in line.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => brace_depth -= 1,
                    _ => {}
                }
            }
        } else if in_fn {
            current_fn_lines += 1;
            let mut ended = false;
            for ch in line.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => {
                        brace_depth -= 1;
                        if brace_depth == 0 {
                            ended = true;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if ended {
                if current_fn_lines > largest_fn_lines {
                    largest_fn_lines = current_fn_lines;
                }
                in_fn = false;
            }
        }
    }
    // Handle a function that reaches end-of-file without a closing brace (malformed, but safe).
    if in_fn && current_fn_lines > largest_fn_lines {
        largest_fn_lines = current_fn_lines;
    }

    WgslEmitStats {
        fn_count,
        largest_fn_lines,
    }
}

fn parse_feature_labels(hir: &Hir) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    for note in &hir.notes {
        if let Some(rest) = note.strip_prefix("explain: feature_label ")
            && let Some((feature, label)) = rest.split_once('=')
        {
            labels.insert(feature.trim().to_string(), label.trim().to_string());
        }
    }
    labels
}

fn parse_gradient_guardrail_counts(hir: &Hir) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for note in &hir.notes {
        let Some(rest) = note.strip_prefix("gradient guardrail:") else {
            continue;
        };
        let Some((reasons, _)) = rest
            .split_once('[')
            .and_then(|(_, bracketed)| bracketed.split_once(']'))
        else {
            continue;
        };
        for reason in reasons.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            *counts.entry(reason.to_string()).or_insert(0) += 1;
        }
    }
    counts
}

/// Emitted text to analyze for size metrics.
/// Function-level analysis (fn count, largest fn) is only performed for WGSL output.
pub(crate) struct EmitForStats<'a> {
    pub text: &'a str,
    pub target: EmitTarget,
}

/// Write one `+count` row per feature, most instructions first, ties by name.
fn write_feature_rows(
    out: &mut String,
    indent: &str,
    counts: &BTreeMap<String, usize>,
    feature_labels: &BTreeMap<String, String>,
) -> fmt::Result {
    let mut rows = counts
        .iter()
        .map(|(feature, count)| (feature.as_str(), *count))
        .collect::<Vec<_>>();
    rows.sort_by_key(|&(feature, count)| (Reverse(count), feature));
    for (feature, count) in rows {
        if let Some(label) = feature_labels.get(feature) {
            writeln!(out, "{indent}{label} [{feature}]: +{count}")?;
        } else {
            writeln!(out, "{indent}{feature}: +{count}")?;
        }
    }
    Ok(())
}

pub fn render(hirs: &[Hir], stats: &[Stats], emit_for_stats: Option<EmitForStats<'_>>) -> String {
    let mut out = String::new();
    write_explain(&mut out, hirs, stats, emit_for_stats.as_ref())
        .expect("writing to a String cannot fail");
    out
}

fn write_explain(
    out: &mut String,
    hirs: &[Hir],
    stats: &[Stats],
    emit_for_stats: Option<&EmitForStats<'_>>,
) -> fmt::Result {
    let emit_analysis = emit_for_stats.map(|e| {
        let wgsl_stats = (e.target == EmitTarget::Wgsl).then(|| analyze_wgsl_emit_stats(e.text));
        (e.text.len(), e.target.as_str(), wgsl_stats)
    });
    for (hir, st) in hirs.iter().zip(stats) {
        let feature_labels = parse_feature_labels(hir);
        let gradient_guardrail_counts = parse_gradient_guardrail_counts(hir);
        let plan = pass_plan::build_for_hir(hir);
        writeln!(
            out,
            "== fresco --explain: {} `{}` ==",
            hir.entry_kind, hir.name
        )?;

        out.push_str("pass plan (phase-1 locality partitioner DAG):\n");
        for p in &plan.passes {
            let strategy_note = match p.kernel_strategy {
                KernelStrategy::Fused => String::new(),
                KernelStrategy::InlineTaps {
                    radius_px,
                    half_taps,
                } => format!(
                    "  [inline-taps: r={radius_px:.1}px, {}×{} grid — §11 rung 1]",
                    2 * half_taps + 1,
                    2 * half_taps + 1
                ),
                KernelStrategy::SeparableHV { radius_px } => {
                    format!("  [separable-hv: r={radius_px:.1}px — §11 rung 2]")
                }
                KernelStrategy::DownsampleChain { radius_px, levels } => format!(
                    "  [downsample-chain: r={radius_px:.1}px, {levels} levels — §11 rung 3]"
                ),
                KernelStrategy::GlobalReduction => "  [global-reduction — §11]".to_string(),
            };
            let target_note = match p.output_target {
                None => "  → swapchain".to_string(),
                Some(tid) => format!("  → rt{tid}"),
            };
            writeln!(
                out,
                "pass {}  stage={}  locality={}  layers {}..={} ({} layer node(s)){}{}",
                p.id,
                p.stage,
                p.locality.as_str(),
                p.start_layer,
                p.end_layer,
                p.count,
                strategy_note,
                target_note,
            )?;
        }
        if !plan.targets.is_empty() {
            out.push_str("intermediate targets:\n");
            for t in &plan.targets {
                writeln!(
                    out,
                    "  rt{}  format={}  scale={}x  lifetime={}",
                    t.id,
                    t.format,
                    t.scale,
                    t.lifetime.label(),
                )?;
            }
        }
        if plan.edges.is_empty() {
            out.push_str("pass dag edges: none\n");
        } else {
            out.push_str("pass dag edges:\n");
            for (from, to, reason) in &plan.edges {
                writeln!(out, "  p{from} -> p{to}  reason={reason}")?;
            }
        }
        let (point_count, local_count, global_count) = pass_plan::locality_counts(hir);
        writeln!(
            out,
            "locality marks: point={point_count}, local={local_count}, global={global_count} (phase-1 persisted)"
        )?;
        if local_count > 0 || global_count > 0 {
            writeln!(
                out,
                "pass-plan estimate: {} pass(es) prior to full partitioner",
                pass_plan::estimated_passes(point_count, local_count, global_count)
            )?;
        }

        // Pattern Filtering section
        let pattern_notes: Vec<_> = hir
            .notes
            .iter()
            .filter(|note| note.starts_with("pattern:"))
            .collect();

        if !pattern_notes.is_empty() {
            out.push_str("pattern filtering:\n");

            // Show footprint source
            let has_prefiltered_pattern = pattern_notes
                .iter()
                .any(|note| note.contains("prefiltered analytic"));
            if has_prefiltered_pattern {
                out.push_str(
                    "  footprint source: symbolic Jacobian at pattern site (includes active in-space transforms)\n",
                );
            } else {
                out.push_str("  footprint source: none (patterns use point-sampling or filtering disabled)\n");
            }

            // Show each pattern's filtering status
            for note in &pattern_notes {
                writeln!(out, "  • {}", note.trim_start_matches("pattern: "))?;
                if note.contains("prefiltered analytic") {
                    out.push_str("    method: analytic box-filter approximation (sep-box AABB)\n");
                    out.push_str(
                        "    accuracy: ε ≤ 0.02 (validated against supersampled reference)\n",
                    );
                } else if note.contains("unfiltered point-sample") {
                    out.push_str(
                        "    method: point-sampling (aliasing may occur under minification)\n",
                    );
                } else if note.contains("unfiltered - filtering requested") {
                    out.push_str("    method: point-sampling fallback (filtering requested but no footprint)\n");
                }
            }
        }

        if !gradient_guardrail_counts.is_empty() {
            let total_fallbacks: usize = gradient_guardrail_counts.values().sum();
            out.push_str("gradient guardrails:\n");
            writeln!(out, "  fallback hits: {total_fallbacks}")?;
            out.push_str("  reason frequencies:\n");
            for (reason, count) in &gradient_guardrail_counts {
                writeln!(out, "    {reason}: {count}")?;
            }
        }

        // Show all non-pattern notes
        for note in &hir.notes {
            if !note.starts_with("pattern:")
                && !note.starts_with("explain: feature_label ")
                && !note.starts_with("gradient guardrail:")
            {
                writeln!(out, "  {note}")?;
            }
        }
        if !hir.path_profiles.is_empty() {
            let const_tables = hir
                .path_profiles
                .iter()
                .filter(|p| p.storage == crate::hir::PathStorageDecision::ConstModuleEmbedded)
                .count();
            let buffer_tables = hir
                .path_profiles
                .iter()
                .filter(|p| {
                    p.storage == crate::hir::PathStorageDecision::BufferExceedsConstThreshold
                })
                .count();
            writeln!(
                out,
                "  path backend: const_tables={const_tables}, buffer_tables={buffer_tables}"
            )?;
        }
        if !hir.specialization_notes.is_empty() {
            out.push_str("specializations (§17.4):\n");
            for note in &hir.specialization_notes {
                writeln!(out, "  {note}")?;
            }
        }
        if st.outline_thin_fade_count > 0 {
            writeln!(
                out,
                "  stroke aa: thin-fade enabled for {} outline stroke(s) when effective width drops below one display pixel",
                st.outline_thin_fade_count
            )?;
        }
        writeln!(
            out,
            "  sdf: {} evaluation(s), {} reuse(s) via shape CSE (§10.3)",
            st.sdf_evals, st.sdf_cache_hits
        )?;
        writeln!(
            out,
            "  scatter lowering: procedural_path={}, compact_path={}, branch_tree_path={}, occupied_bins={}, emitted_instances={}, procedural_samples={}, procedural_cap={}",
            st.scatter_procedural_paths,
            st.scatter_compact_paths,
            st.scatter_branch_tree_paths,
            st.scatter_occupied_bins,
            st.scatter_emitted_instances,
            st.scatter_procedural_samples,
            st.scatter_procedural_cap
        )?;
        writeln!(
            out,
            "  scatter strategy reasons: compact_due_to_bins={}, compact_due_to_instances={}",
            st.scatter_compact_reason_bins, st.scatter_compact_reason_instances
        )?;

        writeln!(
            out,
            "  instructions: total emitted={}",
            st.emitted_instructions
        )?;
        if !st.feature_instruction_counts.is_empty() {
            out.push_str("  instruction attribution by feature:\n");
            write_feature_rows(out, "    ", &st.feature_instruction_counts, &feature_labels)?;
        }
        if !st.feature_instruction_counts_by_locality.is_empty() {
            out.push_str("  instruction attribution by locality:\n");
            for locality in ["point", "local", "global"] {
                let Some(by_feature) = st.feature_instruction_counts_by_locality.get(locality)
                else {
                    continue;
                };
                writeln!(out, "    {locality}:")?;
                write_feature_rows(out, "      ", by_feature, &feature_labels)?;
            }
        }
        if st.scatter_layers.len() > 1 {
            out.push_str("  scatter per-layer metrics:\n");
            for layer in &st.scatter_layers {
                writeln!(
                    out,
                    "    l{}: strategy={}, occupied_bins={}, emitted_instances={}, procedural_samples={}, procedural_cap={}",
                    layer.layer_id,
                    layer.strategy,
                    layer.occupied_bins,
                    layer.emitted_instances,
                    layer.procedural_samples,
                    layer.procedural_cap
                )?;
            }
        }
        writeln!(
            out,
            "  dag: {} shape node(s), {} layer node(s)",
            hir.shapes.len(),
            hir.layers.len()
        )?;
        if let Some((total_bytes, fmt, ref wgsl_stats)) = emit_analysis {
            let kb = total_bytes as f64 / 1024.0;
            let size_str = if kb >= 1.0 {
                format!("{kb:.0}KB")
            } else {
                format!("{total_bytes}B")
            };
            if let Some(ws) = wgsl_stats {
                writeln!(
                    out,
                    "  emitted: {} {}, {} fns, largest fn {} lines, iter loops: {}, unrolled: {}",
                    size_str,
                    fmt,
                    ws.fn_count,
                    ws.largest_fn_lines,
                    st.iter_loops,
                    st.unrolled_iters
                )?;
            } else {
                writeln!(out, "  emitted: {size_str} {fmt}")?;
            }
        }
        out.push('\n');
    }
    Ok(())
}

pub fn emit(hirs: &[Hir], stats: &[Stats], emit_for_stats: Option<EmitForStats<'_>>) {
    eprint!("{}", render(hirs, stats, emit_for_stats));
}
