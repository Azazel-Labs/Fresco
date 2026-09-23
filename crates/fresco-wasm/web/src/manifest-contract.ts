type JsonSchemaNode = {
  $ref?: string;
  type?: string | string[];
  required?: string[];
  properties?: Record<string, JsonSchemaNode>;
  items?: JsonSchemaNode;
};

type JsonSchemaRoot = JsonSchemaNode & Record<string, unknown>;

function isObjectLike(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object";
}

function resolveRef(rootSchema: JsonSchemaRoot, ref: string): JsonSchemaNode | null {
  if (typeof ref !== "string" || !ref.startsWith("#/")) {
    return null;
  }
  const parts = ref.slice(2).split("/");
  let cursor: unknown = rootSchema;
  for (const part of parts) {
    if (!isObjectLike(cursor) || !(part in cursor)) {
      return null;
    }
    cursor = cursor[part];
  }
  if (!isObjectLike(cursor)) {
    return null;
  }
  return cursor as JsonSchemaNode;
}

function valueMatchesType(value: unknown, expectedType: string): boolean {
  if (expectedType === "array") return Array.isArray(value);
  if (expectedType === "object") return value !== null && typeof value === "object" && !Array.isArray(value);
  if (expectedType === "integer") return Number.isInteger(value);
  if (expectedType === "null") return value === null;
  return typeof value === expectedType;
}

function checkNode(node: unknown, schema: JsonSchemaNode, rootSchema: JsonSchemaRoot, path: string, errors: string[]): void {
  const target = schema.$ref ? resolveRef(rootSchema, schema.$ref) : schema;
  if (!target || typeof target !== "object") {
    errors.push(`${path}: unresolved schema reference`);
    return;
  }

  if (target.type) {
    const types = Array.isArray(target.type) ? target.type : [target.type];
    if (!types.some((t) => valueMatchesType(node, t))) {
      errors.push(`${path}: type mismatch, expected ${types.join("|")}`);
      return;
    }
  }

  if (target.required && node && typeof node === "object" && !Array.isArray(node)) {
    for (const key of target.required) {
      if (!(key in node)) {
        errors.push(`${path}: missing required key "${key}"`);
      }
    }
  }

  if (target.properties && node && typeof node === "object" && !Array.isArray(node)) {
    for (const [key, childSchema] of Object.entries(target.properties)) {
      if (key in node) {
        checkNode((node as Record<string, unknown>)[key], childSchema, rootSchema, `${path}.${key}`, errors);
      }
    }
  }

  if (target.items && Array.isArray(node)) {
    for (let i = 0; i < node.length; i += 1) {
      checkNode(node[i], target.items, rootSchema, `${path}[${i}]`, errors);
    }
  }
}

export function validateManifestContract(manifest: unknown, contractSchema: JsonSchemaRoot) {
  const errors: string[] = [];
  checkNode(manifest, contractSchema, contractSchema, "$", errors);
  return { ok: errors.length === 0, errors };
}

// Preserve legacy WASM Map values as well as current JSON-compatible objects
// when exporting a manifest or handing it to the engine.
export function stringifyManifest(manifest: unknown, space?: number): string {
  return JSON.stringify(manifest, (_, value) => value instanceof Map ? Object.fromEntries(value) : value, space);
}
