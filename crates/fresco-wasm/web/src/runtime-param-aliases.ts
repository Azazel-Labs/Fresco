const WGSL_RESERVED = new Set([
  "alias", "break", "case", "const", "continue", "continuing", "default",
  "diagnostic", "discard", "else", "enable", "false", "fn", "for", "if",
  "let", "loop", "override", "requires", "return", "struct", "switch", "true",
  "var", "while", "bitcast", "array", "atomic", "bool", "f32", "f16", "i32",
  "mat2x2", "mat2x3", "mat2x4", "mat3x2", "mat3x3", "mat3x4", "mat4x2", "mat4x3",
  "mat4x4", "sampler", "sampler_comparison", "texture_1d", "texture_2d",
  "texture_2d_array", "texture_3d", "texture_cube", "texture_cube_array",
  "texture_multisampled_2d", "texture_storage_1d", "texture_storage_2d",
  "texture_storage_2d_array", "texture_storage_3d", "texture_depth_2d",
  "texture_depth_2d_array", "texture_depth_cube", "texture_depth_cube_array",
  "texture_depth_multisampled_2d", "texture_external", "u32", "vec2", "vec3", "vec4"
]);

type RuntimeParamDef = {
  name?: unknown;
  reflectedGroup?: Array<{ name?: unknown }>;
};

export function sanitizeWgslIdentifier(name: unknown, fallback = "param"): string {
  const raw = String(name || "").trim();
  let value = raw.replace(/[^A-Za-z0-9_]/g, "_");
  value = value.replace(/^_+/, "");
  if (!value) {
    value = fallback;
  }
  if (!/^[A-Za-z_]/.test(value)) {
    value = `_${value}`;
  }
  if (WGSL_RESERVED.has(value)) {
    value = `${value}_param`;
  }
  return value;
}

export function buildReadableRuntimeParamAliases(defs: RuntimeParamDef[], reserved: unknown[] = []): string[][] {
  const used = new Set(reserved.map((name) => String(name)));
  return defs.map((def, index) => {
    const reflectedGroup = Array.isArray(def?.reflectedGroup) ? def.reflectedGroup : [];
    const fallbackBase = `param${index + 1}`;
    return reflectedGroup.map((entry, groupIndex) => {
      const singleEntryGroup = reflectedGroup.length <= 1;
      const preferredName = singleEntryGroup
        ? (def?.name || entry?.name || fallbackBase)
        : (entry?.name || `${String(def?.name || fallbackBase)}_${groupIndex + 1}`);
      const base = sanitizeWgslIdentifier(preferredName, fallbackBase);
      let alias = base;
      let suffix = 2;
      while (used.has(alias)) {
        alias = `${base}_${suffix}`;
        suffix += 1;
      }
      used.add(alias);
      return alias;
    });
  });
}
