type ExampleEntry = {
  id: string;
  label: string;
  source: string;
  previewMode?: "mesh" | "particles";
};

type ExampleAssetOption = {
  relPath: string;
  url: string;
};

const EXAMPLE_MODULES = import.meta.glob("./examples/**/*.fr", {
  eager: true,
  query: "?raw",
  import: "default"
}) as Record<string, string>;

const EXAMPLE_ASSET_MODULES = import.meta.glob("./examples/**/*.{png,jpg,jpeg,webp,avif}", {
  eager: true,
  import: "default"
}) as Record<string, string>;

// Presentation settings travel beside the example rather than being inferred
// from directory names or shader source text.
const PREVIEW_SETTINGS = import.meta.glob("./examples/**/*.preview.json", {
  eager: true,
  import: "default",
}) as Record<string, { renderMode: "mesh" | "particles" }>;

for (const [path, settings] of Object.entries(PREVIEW_SETTINGS)) {
  if (settings.renderMode !== "mesh" && settings.renderMode !== "particles") {
    throw new Error(`${path}: renderMode must be mesh or particles`);
  }
}

export const EXAMPLES: ExampleEntry[] = Object.entries(EXAMPLE_MODULES)
  .map(([path, source]) => {
    const id = path.replace(/^\.\/examples\//, "").replace(/\.fr$/, "");
    return {
      id,
      label: id,
      source: String(source ?? ""),
      previewMode: PREVIEW_SETTINGS[path.replace(/\.fr$/, ".preview.json")]?.renderMode
    };
  })
  .sort((a, b) => a.label.localeCompare(b.label, undefined, { numeric: true, sensitivity: "base" }));

export const EXAMPLES_BY_ID = new Map(EXAMPLES.map((example) => [example.id, example]));

// Detect multi-file example directories: directories that contain a "main" entry
// plus at least one sibling .fr file. The key is the main entry's id
// (e.g. "30) libraries/iq_distance/main"); the value is a Map<filename, source>
// where filenames include the .fr extension.
const MULTI_FILE_EXAMPLES = (() => {
  const byDir = new Map<string, Map<string, string>>();
  for (const ex of EXAMPLES) {
    const slash = ex.id.lastIndexOf("/");
    if (slash < 0) continue;
    const dir = ex.id.slice(0, slash);
    const base = ex.id.slice(slash + 1);
    if (!byDir.has(dir)) byDir.set(dir, new Map());
    byDir.get(dir)?.set(base, ex.source);
  }
  const result = new Map<string, Map<string, string>>();
  for (const [dir, files] of byDir) {
    if (files.size > 1 && files.has("main")) {
      const bundle = new Map<string, string>();
      for (const [base, src] of files) {
        bundle.set(`${base}.fr`, src);
      }
      result.set(`${dir}/main`, bundle);
    }
  }
  return result;
})();

// For the example dropdown, hide sibling files from multi-file directories.
export const FILTERED_EXAMPLES = EXAMPLES.filter((ex) => {
  const slash = ex.id.lastIndexOf("/");
  if (slash < 0) return true;
  const dir = ex.id.slice(0, slash);
  const base = ex.id.slice(slash + 1);
  return base === "main" || !MULTI_FILE_EXAMPLES.has(`${dir}/main`);
});

export const EXAMPLE_ASSET_OPTIONS: ExampleAssetOption[] = Object.entries(EXAMPLE_ASSET_MODULES)
  .map(([path, url]) => ({
    relPath: path.replace(/^\.\/examples\//, "").replace(/\\/g, "/"),
    url: String(url || "")
  }))
  .filter((entry): entry is ExampleAssetOption => Boolean(entry.relPath && entry.url))
  .sort((a, b) => a.relPath.localeCompare(b.relPath, undefined, { numeric: true, sensitivity: "base" }));

export function bundleForExampleId(exampleId: string): Map<string, string> | null {
  return MULTI_FILE_EXAMPLES.get(exampleId) ?? null;
}
