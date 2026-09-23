type Parameter = { name: string; type: string; default: unknown; min?: number | null; max?: number | null };
type Availability = { symbol: string; supported: boolean; reasons: string[]; static_parameters?: Parameter[] };
type Selection = { availability?: Availability[]; name: string; symbol: string; available: string[]; editable: boolean; parameters?: Parameter[]; static_parameters?: Parameter[] };
type Override = string | { symbol: string; settings: Record<string, unknown> };
type Surface = { name: string; settings?: { implementations?: Selection[] } | null };

/** Preview choices persist symbols, never artifact-local dispatch IDs. */
export function createImplementationSelection(host: HTMLElement, staticHost: HTMLElement, recompile: () => void) {
  let documentId: string | undefined;
  const overrides: Record<string, Record<string, Override>> = Object.create(null);
  function reset() { for (const key of Object.keys(overrides)) delete overrides[key]; }
  return {
    bundle(files: Map<string, string>, nextDocument: string) {
      if (documentId !== nextDocument) { reset(); documentId = nextDocument; host.replaceChildren(); host.hidden = true; staticHost.replaceChildren(); staticHost.hidden = true; }
      const out = new Map(files);
      if (!Object.keys(overrides).length) return out;
      const configuration = JSON.parse(out.get("fresco.config.json") || "{}");
      const properties = Object.assign(Object.create(null), configuration.property_overrides);
      for (const [entry, values] of Object.entries(overrides)) properties[entry] = { ...properties[entry], ...values };
      out.set("fresco.config.json", JSON.stringify({ ...configuration, property_overrides: properties }));
      return out;
    },
    render(surfaces: Surface[]) {
      host.replaceChildren();
      staticHost.replaceChildren();
      for (const surface of surfaces) for (const selection of surface.settings?.implementations || []) {
        if (!selection.editable) continue;
        const requested = overrides[surface.name]?.[selection.name];
        const requestedSymbol = typeof requested === "string" ? requested : requested?.symbol ?? selection.symbol;
        // A successful artifact describes installed work, not necessarily the user's
        // pending request. Keep that request selectable even after a failed rebuild.
        const symbols = [...new Set([...selection.available, selection.symbol, requestedSymbol])];
        if (symbols.length > 1) {
          const label = document.createElement("label");
          label.className = "param-row static-option-row";
          const title = selection.name.charAt(0).toUpperCase() + selection.name.slice(1);
          const name = document.createElement("span");
          name.textContent = surfaces.length > 1 ? `${surface.name}: ${title}` : title;
          label.append(name);
          const select = document.createElement("select");
          select.setAttribute("aria-label", `${surface.name} ${selection.name}`);
          for (const symbol of symbols) {
            const availability = selection.availability?.find(candidate => candidate.symbol === symbol);
            const unsupported = availability?.supported === false || !selection.available.includes(symbol);
            const title = unsupported ? `${symbol} (unavailable)` : symbol;
            const option = new Option(title, symbol, false, symbol === requestedSymbol);
            option.title = availability?.reasons.join("\n") ?? "";
            select.add(option);
          }
          select.addEventListener("change", () => {
            (overrides[surface.name] ||= Object.create(null))[selection.name] = select.value;
            this.render(surfaces);
            recompile();
          });
          label.append(select); host.append(label);
          const reasons = document.createElement("p");
          reasons.className = "static-options-hint";
          reasons.setAttribute("role", "status");
          const showReasons = () => {
            reasons.textContent = selection.availability?.find(candidate => candidate.symbol === select.value)?.reasons.join(" ") ?? "";
            reasons.hidden = !reasons.textContent;
          };
          showReasons();
          select.addEventListener("change", showReasons);
          host.append(reasons);
        }
        // Candidate defaults let users disable unsupported optional work before a
        // successful compile. Never reuse settings from a different installed style.
        const selected = requestedSymbol === selection.symbol;
        const candidate = selection.availability?.find(candidate => candidate.symbol === requestedSymbol);
        const staticParameters = (selected ? selection.static_parameters : candidate?.static_parameters) ?? [];
        const runtimeParameters = selected ? selection.parameters ?? [] : [];
        for (const parameter of staticParameters) {
          const label = document.createElement("label");
          label.className = "param-row static-option-row";
          const name = document.createElement("span");
          name.textContent = `${surfaces.length > 1 ? `${surface.name}: ` : ""}${selection.name}.${parameter.name}`;
          label.append(name);
          const input = document.createElement("input");
          input.setAttribute("aria-label", `${surface.name} ${selection.name}.${parameter.name}`);
          input.type = parameter.type === "bool" ? "checkbox" : ["f32", "u32", "i32"].includes(parameter.type) ? "number" : "text";
          const value = typeof requested === "object" && Object.hasOwn(requested.settings, parameter.name)
            ? requested.settings[parameter.name] : parameter.default;
          if (input.type === "checkbox") input.checked = Boolean(value);
          else input.value = typeof value === "number" ? String(value) : JSON.stringify(value);
          if (parameter.min != null) input.min = String(parameter.min);
          if (parameter.max != null) input.max = String(parameter.max);
          input.step = parameter.type === "f32" ? "any" : "1";
          input.addEventListener("change", () => {
            let value: unknown;
            try {
              value = input.type === "checkbox" ? input.checked : JSON.parse(input.value);
            } catch { input.setCustomValidity("Enter a valid setting value"); return; }
            input.setCustomValidity("");
            if (!input.reportValidity()) return;
            const previous = overrides[surface.name]?.[selection.name];
            const settings = typeof previous === "object" && previous.symbol === requestedSymbol
              ? { ...previous.settings }
              : Object.fromEntries([...runtimeParameters, ...staticParameters].map(p => [p.name, p.default]));
            settings[parameter.name] = value;
            (overrides[surface.name] ||= Object.create(null))[selection.name] = { symbol: requestedSymbol, settings };
            recompile();
          });
          label.append(input); staticHost.append(label);
        }
      }
      host.hidden = !host.childElementCount;
      staticHost.hidden = !staticHost.childElementCount;
      for (const [group, title] of [[host, "Style & properties"], [staticHost, "Static options"]] as const) {
        if (group.hidden) continue;
        const legend = document.createElement("legend");
        legend.textContent = title;
        const hint = document.createElement("p");
        hint.className = "static-options-hint";
        hint.textContent = "Changes rebuild the shader.";
        group.prepend(legend, hint);
      }
    },
  };
}
