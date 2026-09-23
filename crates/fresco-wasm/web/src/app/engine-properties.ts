export type EngineProperty = {
  editable?: boolean;
  block?: string | null; block_present?: boolean;
  entry: string; name: string; ty: string; value: number; choices: [string, number][];
  permutation: boolean; value_span: { start: number; end: number } | null; insert_at: number;
};

/** Compiler ranges are UTF-8 byte offsets; preserve the rest of the authored file. */
export function editEngineProperty(source: string, property: EngineProperty, expression: string): string {
  if (property.editable === false) throw new Error("This property is configured by the engine");
  const bytes = new TextEncoder().encode(source);
  const start = property.value_span?.start ?? property.insert_at;
  const end = property.value_span?.end ?? property.insert_at;
  if (start < 0 || end < start || end > bytes.length) throw new Error("Property source range is stale; recompile first");
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const prefix = decoder.decode(bytes.slice(0, start));
  const suffix = decoder.decode(bytes.slice(end));
  const assignment = `${property.name}: ${expression}`;
  const replacement = property.value_span ? expression
    : property.block && !property.block_present ? `\n    ${property.block} {\n        ${assignment}\n    }\n`
    : property.block ? `\n        ${assignment}\n    ` : `${assignment}\n    `;
  return prefix + replacement + suffix;
}

export function renderEngineProperties(host: HTMLElement | null, properties: EngineProperty[], commit: (property: EngineProperty, expression: string) => void) {
  properties = properties.filter(property => property.editable !== false);
  if (!host || !properties.length) return;
  const panel = document.createElement("section"); panel.className = "engine-properties";
  const title = document.createElement("h3"); title.textContent = "Engine properties"; panel.append(title);
  const note = document.createElement("p"); note.textContent = "Changes update the effect source and rebuild the preview."; panel.append(note);
  for (const property of properties) {
    const label = document.createElement("label"); label.className = "manifest-inspector-picker";
    const text = document.createElement("span"); text.textContent = `${property.entry} ? ${property.name}` + (property.permutation ? " (permutation)" : ""); label.append(text);
    const input = property.choices.length ? document.createElement("select") : document.createElement("input");
    input.setAttribute("data-engine-property", property.name);
    if (input instanceof HTMLSelectElement) {
      for (const [expression, value] of property.choices) {
        const option = document.createElement("option"); option.value = expression;
        option.textContent = expression.split(".").at(-1) || expression; option.selected = value === property.value; input.append(option);
      }
    } else {
      input.type = "number"; input.step = property.ty === "u32" || property.ty === "i32" ? "1" : "any";
      input.value = String(property.value);
    }
    input.addEventListener("change", () => {
      if (input instanceof HTMLInputElement && (!input.value.trim() || !Number.isFinite(Number(input.value)) || !input.checkValidity())) return;
      try {
        commit(property, input.value);
        panel.querySelectorAll("input,select").forEach(element => (element as HTMLInputElement).disabled = true);
      } catch (error) { note.textContent = String(error); }
    });
    label.append(input); panel.append(label);
  }
  host.prepend(panel);
}
