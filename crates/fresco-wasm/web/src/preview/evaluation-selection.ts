export type EvaluationVariantBinding = {
  axis?: string;
  value?: string | number | null;
};

export type EvaluationVariant = {
  entry?: string;
  bindings?: EvaluationVariantBinding[];
};

export type EvaluationContract = {
  inputs?: string[];
  runtime?: string[];
};

export type EvaluationManifestSurface = {
  evaluation_shader_entry?: string;
  evaluation_contract?: EvaluationContract;
  evaluation_variants?: EvaluationVariant[];
  settings?: { evaluation_axes?: Record<string, string> } | null;
};

// The inspector follows the same explicit axes as compilation. Axis names and
// numeric magnitudes carry no renderer meaning here.
export function chooseEvaluationVariantEntry(surface: EvaluationManifestSurface | null | undefined): string | null {
  const axes = Object.entries(surface?.settings?.evaluation_axes ?? {});
  if (axes.length === 0) {
    const base = surface?.evaluation_shader_entry?.trim();
    return base && surface?.evaluation_variants?.some(variant => variant.entry === base) ? base : null;
  }
  const matches = (surface?.evaluation_variants ?? []).filter(variant =>
    axes.every(([axis, value]) => variant.bindings?.some(binding =>
      binding.axis === axis && String(binding.value) === value)));
  return matches.length === 1 ? matches[0].entry?.trim() || null : null;
}
