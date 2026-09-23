import { afterEach, expect, it, vi } from "vitest";
import { createImplementationSelection } from "../app/implementation-selection";
class Element {
  children = []; events = {}; hidden = false; value = "";
  get childElementCount() { return this.children.length; }
  replaceChildren(...children) { this.children = children; }
  prepend(...children) { this.children.unshift(...children); }
  append(...children) { this.children.push(...children); }
  add(child) { this.append(child); if (child.selected) this.value = child.value; }
  setAttribute() {}
  setCustomValidity(message) { this.validity = message; }
  reportValidity() { return !this.validity; }
  addEventListener(event, callback) { const previous = this.events[event]; this.events[event] = () => { previous?.(); callback(); }; }
  querySelector() { return this.children.find(c => c.tag === "button"); }
}
afterEach(() => vi.unstubAllGlobals());
it("overrides symbolic selections without changing source and resets per document", () => {
  vi.stubGlobal("document", { createElement(tag) { const element = new Element(); element.tag = tag; return element; } });
  vi.stubGlobal("Option", class { constructor(text, value, unused, selected) { Object.assign(this, {text, value, selected}); } });
  const host = new Element();
  const staticHost = new Element();
  const recompile = vi.fn();
  const controller = createImplementationSelection(host, staticHost, recompile);
  const files = new Map([["main.fr", "unchanged source"], ["fresco.config.json", JSON.stringify({ renderer: "deferred", property_overrides: { other: { count: 2 } } })]]);
  controller.bundle(files, "document-a");
  controller.render([{name: "material", settings: {implementations: [{name: "style", symbol: "StandardGGX", available: ["StandardGGX"], editable: true}]}}]);
  expect(host.hidden).toBe(true);
  expect(host.childElementCount).toBe(0);
  controller.render([{name: "material", settings: {implementations: [{name: "style", symbol: "Default", id: 7, available: ["Default", "External"], editable: true}]}}]);
  expect(host.hidden).toBe(false);
  expect(host.children[0].textContent).toBe("Style & properties");
  expect(host.children[2].children[0].textContent).toBe("Style");
  expect(staticHost.hidden).toBe(true);
  const select = host.children[2].children[1];
  select.value = "External"; select.events.change();
  const bundle = controller.bundle(files, "document-a");
  expect(JSON.parse(bundle.get("fresco.config.json"))).toEqual({renderer: "deferred", property_overrides: { other: { count: 2 }, material: {style: "External"} }});
  expect(bundle.get("main.fr")).toBe(files.get("main.fr"));
  expect(JSON.parse(files.get("fresco.config.json")).property_overrides.material).toBeUndefined();
  expect(recompile).toHaveBeenCalledTimes(1);
  expect(host.querySelector("button")).toBeUndefined();
  // The dropdown remains usable after a failed compile: choose the previous style.
  select.value = "Default"; select.events.change();
  expect(JSON.parse(controller.bundle(files, "document-a").get("fresco.config.json")).property_overrides.material.style).toBe("Default");
  select.value = "External"; select.events.change();
  controller.bundle(files, "document-b");
  expect(host.hidden).toBe(true);
  expect(host.childElementCount).toBe(0);
  // No controls are shown when only one style remains.
  controller.render([{name: "material", settings: {implementations: [{name: "style", symbol: "External", available: ["External"], editable: true}]}}]);
  expect(host.childElementCount).toBe(0);
  expect(host.hidden).toBe(true);
  expect(controller.bundle(files, "document-b").get("fresco.config.json")).toBe(files.get("fresco.config.json"));
  controller.render([{ name: "canvas" }]);
  expect(host.hidden).toBe(true);
});

it("static settings rebuild symbolic selections even when only one style exists", () => {
  vi.stubGlobal("document", { createElement(tag) { const element = new Element(); element.tag = tag; return element; } });
  const host = new Element(), staticHost = new Element(), recompile = vi.fn();
  const controller = createImplementationSelection(host, staticHost, recompile);
  const files = new Map([["main.fr", "source"]]);
  controller.bundle(files, "document");
  const selection = { name: "style", symbol: "Fur", available: ["Fur"], editable: true,
    parameters: [{name: "seed", type: "u32", default: 4294967295}],
    static_parameters: [{name: "shells", type: "u32", default: 12, min: 1, max: 32}, {name: "enabled", type: "bool", default: true}] };
  controller.render([{name: "material", settings: { implementations: [selection] }}]);
  expect(host.hidden).toBe(true);
  expect(staticHost.hidden).toBe(false);
  expect(staticHost.children[0].textContent).toBe("Static options");
  const shells = staticHost.children[2].children[1], enabled = staticHost.children[3].children[1];
  expect(shells.type).toBe("number");
  expect(enabled.type).toBe("checkbox");
  shells.value = "24"; shells.events.change();
  enabled.checked = false; enabled.events.change();
  expect(recompile).toHaveBeenCalledTimes(2);
  expect(JSON.parse(controller.bundle(files, "document").get("fresco.config.json")).property_overrides.material.style)
    .toEqual({symbol: "Fur", settings: {shells: 24, enabled: false, seed: 4294967295}});
  shells.value = "bad"; shells.events.change();
  expect(recompile).toHaveBeenCalledTimes(2);
  expect(controller.bundle(files, "new document").has("fresco.config.json")).toBe(false);
  expect(staticHost.hidden).toBe(true);
  expect(staticHost.childElementCount).toBe(0);
  controller.render([{name: "material", settings: { implementations: [selection] }}]);
  controller.render([{name: "material"}]);
  expect(staticHost.hidden).toBe(true);
});


it("keeps a pending symbol visible when an installed manifest refreshes and permits recovery", () => {
  vi.stubGlobal("document", { createElement(tag) { const element = new Element(); element.tag = tag; return element; } });
  vi.stubGlobal("Option", class { constructor(text, value, unused, selected) { Object.assign(this, {text, value, selected}); } });
  const host = new Element(), staticHost = new Element(), recompile = vi.fn();
  const controller = createImplementationSelection(host, staticHost, recompile);
  const files = new Map([["main.fr", "source"]]);
  controller.bundle(files, "document");
  const surface = available => [{name: "material", settings: {implementations: [{
    name: "style", symbol: "Base", available, editable: true,
    static_parameters: [{name: "count", type: "u32", default: 4}],
  }]}}];
  controller.render(surface(["Base", "External"]));
  const original = host.children[2].children[1];
  original.value = "External"; original.events.change();
  controller.render(surface(["Base"]));
  const pending = host.children[2].children[1];
  expect(pending.value).toBe("External");
  expect(pending.children.find(o => o.value === "External").text).toBe("External (unavailable)");
  expect(staticHost.hidden).toBe(true);
  expect(JSON.parse(controller.bundle(files, "document").get("fresco.config.json")).property_overrides.material.style).toBe("External");
  pending.value = "Base"; pending.events.change();
  controller.render(surface(["Base"]));
  const setting = staticHost.children[2].children[1];
  setting.value = "9"; setting.events.change();
  controller.render(surface(["Base"]));
  expect(staticHost.children[2].children[1].value).toBe("9");
  expect(recompile).toHaveBeenCalledTimes(3);
});


it("shows compiler compatibility reasons without disabling unsupported choices", () => {
  vi.stubGlobal("document", { createElement(tag) { const element = new Element(); element.tag = tag; return element; } });
  vi.stubGlobal("Option", class { constructor(text, value, unused, selected) { Object.assign(this, {text, value, selected}); } });
  const host = new Element(), staticHost = new Element(), recompile = vi.fn();
  const controller = createImplementationSelection(host, staticHost, recompile);
  controller.bundle(new Map(), "document");
  controller.render([{name: "material", settings: {implementations: [{name: "style", symbol: "Base", available: ["Base", "External"], editable: true,
    availability: [{symbol: "Base", supported: true, reasons: []}, {symbol: "External", supported: false, reasons: ["Renderer does not provide PreparedGeometry"]}],
  }]}}]);
  const select = host.children[2].children[1];
  expect(select.children[1].text).toBe("External (unavailable)");
  expect(select.children[1].disabled).not.toBe(true);
  expect(select.children[1].title).toContain("PreparedGeometry");
  select.value = "External"; select.events.change();
  expect(host.children[3].textContent).toContain("PreparedGeometry");
  expect(host.children[3].hidden).toBe(false);
  expect(recompile).toHaveBeenCalledTimes(1);
  select.value = "Base"; select.events.change();
  expect(host.children[3].hidden).toBe(true);
});


it("allows static specialization to recover a currently unsupported alternative", () => {
  vi.stubGlobal("document", { createElement(tag) { const element = new Element(); element.tag = tag; return element; } });
  vi.stubGlobal("Option", class { constructor(text, value, unused, selected) { Object.assign(this, {text, value, selected}); } });
  const host = new Element(), staticHost = new Element(), recompile = vi.fn();
  const controller = createImplementationSelection(host, staticHost, recompile);
  const files = new Map([["main.fr", "source"]]);
  controller.bundle(files, "document");
  controller.render([{name: "material", settings: {implementations: [{name: "style", symbol: "Base", available: ["Base", "External"], editable: true,
    parameters: [{name: "unrelated", type: "f32", default: 3}],
    availability: [{symbol: "External", supported: false, reasons: ["missing geometry"], static_parameters: [{name: "outline", type: "bool", default: true}]}],
  }]}}]);
  const select = host.children[2].children[1];
  select.value = "External"; select.events.change();
  const outline = staticHost.children[2].children[1];
  expect(outline.checked).toBe(true);
  outline.checked = false; outline.events.change();
  expect(JSON.parse(controller.bundle(files, "document").get("fresco.config.json")).property_overrides.material.style)
    .toEqual({symbol: "External", settings: {outline: false}});
  expect(recompile).toHaveBeenCalledTimes(2);
});
