import { parseDynamicArrayParamType, scalarSlotWidthForType } from "./param-codegen";

// The UI edits flat components; the runtime's storage JSON contains elements.
export function engineParameterToEditor(type: string, value: unknown): any {
  const array = parseDynamicArrayParamType(type);
  return array && scalarSlotWidthForType(array.elementType) > 1 && Array.isArray(value)
    ? value.flat() : value;
}

export function editorParameterToEngine(type: string, value: unknown): unknown {
  const array = parseDynamicArrayParamType(type);
  const width = array ? scalarSlotWidthForType(array.elementType) : 0;
  if (width <= 1 || !Array.isArray(value)) return value;
  if (value.length % width !== 0) throw new Error(`Incomplete ${array.elementType} storage element`);
  return Array.from({ length: value.length / width }, (_, i) => value.slice(i * width, (i + 1) * width));
}
