type ParamDefLike = {
  name: string;
  type: unknown;
  default?: unknown;
};

type RendererLike = {
  paramValues?: Map<string, unknown>;
  normalizeParamValue(def: ParamDefLike, value: unknown): unknown;
  setParamValue(def: ParamDefLike, value: unknown): void;
};

export function createShaderParamViewModel(renderer: RendererLike) {
  const readRaw = (def: ParamDefLike) => renderer.paramValues?.get(def.name);

  const normalize = (def: ParamDefLike, value: unknown) => renderer.normalizeParamValue(def, value);

  const seedFromDefault = (def: ParamDefLike) => {
    const seeded = normalize(def, undefined);
    if (renderer.paramValues) {
      renderer.paramValues.set(def.name, Array.isArray(seeded) ? seeded.slice() : seeded);
    }
    return seeded;
  };

  const getNormalized = (def: ParamDefLike) => normalize(def, readRaw(def));

  const getArrayValue = (def: ParamDefLike, expectedSlots: number): any[] => {
    const normalized = getNormalized(def);
    if (Array.isArray(normalized) && (expectedSlots <= 0 || normalized.length >= expectedSlots)) {
      return normalized;
    }

    const seeded = seedFromDefault(def);
    if (Array.isArray(seeded) && (expectedSlots <= 0 || seeded.length >= expectedSlots)) {
      return seeded.slice();
    }

    if (expectedSlots > 0) {
      return Array.from({ length: expectedSlots }, () => 0);
    }
    return [];
  };

  const getColorValue = (def: ParamDefLike): [number, number, number, number] => {
    const normalized = getNormalized(def);
    if (Array.isArray(normalized) && normalized.length >= 4) {
      return [
        Number(normalized[0]) || 0,
        Number(normalized[1]) || 0,
        Number(normalized[2]) || 0,
        Number(normalized[3]) || 1,
      ];
    }

    const seeded = seedFromDefault(def);
    if (Array.isArray(seeded) && seeded.length >= 4) {
      return [
        Number(seeded[0]) || 0,
        Number(seeded[1]) || 0,
        Number(seeded[2]) || 0,
        Number(seeded[3]) || 1,
      ];
    }

    return [0, 0, 0, 1];
  };

  return {
    getNormalized,
    getArrayValue,
    getColorValue,
    setValue(def: ParamDefLike, value: unknown) {
      renderer.setParamValue(def, value);
    },
    seedFromDefault,
  };
}
