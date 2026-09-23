import {
  isColorLikeParamType,
  isIntegerParamType,
  normalizeTypeName
} from "./preview/helpers";
import { renderColorParamEditor } from "./app/param-editors/color-param-editor";
import { renderMatrixParamEditor } from "./app/param-editors/matrix-param-editor";
import {
  renderArrayItemParamEditor,
  renderBooleanParamEditor,
  renderNumberParamEditor,
  renderVectorParamEditor
} from "./app/param-editors/scalar-param-editors";
import { renderDynamicArrayParamEditor } from "./app/param-editors/dynamic-array-param-editor";
import { parseDynamicArrayParamType } from "./preview/param-codegen";
import { createShaderParamViewModel } from "./app/shader-param-view-model";

const ARRAY_PAGE_SIZE = 10;
const COMPACT_PANEL_ROW_MAX = 4;
const SIMPLE_ARRAY_MIN_COLUMNS = 2;
const SIMPLE_ARRAY_COLUMN_MIN_WIDTH_PX = 170;
const SIMPLE_ARRAY_MAX_COLUMNS = 4;

function parseArrayParamType(type: unknown) {
  const raw = normalizeTypeName(type);
  const match = /^array<\s*(.+?)\s*,\s*(\d+)(?:u)?\s*>$/i.exec(raw);
  if (!match) {
    return null;
  }
  return {
    elementType: canonicalizeParamType(match[1]),
    length: Number.parseInt(match[2], 10) || 0
  };
}

function canonicalizeParamType(type: unknown) {
  const raw = normalizeTypeName(type);
  if (!raw) {
    return "";
  }

  if (raw === "color") {
    return "color";
  }

  const vecMatch = /^vec([234])(?:<f32>|f)?$/i.exec(raw);
  if (vecMatch) {
    return `vec${vecMatch[1]}<f32>`;
  }

  const matMatch = /^mat([234])(?:x([234]))?(?:<f32>|f)?$/i.exec(raw);
  if (matMatch) {
    const cols = matMatch[1];
    const rows = matMatch[2] || matMatch[1];
    return `mat${cols}x${rows}<f32>`;
  }

  return raw;
}

function scalarSlotWidthForType(type: unknown) {
  const t = canonicalizeParamType(type);
  if (!t) {
    return 0;
  }
  if (t === "f32" || t === "i32" || t === "u32" || t === "bool") {
    return 1;
  }
  if (t === "color") {
    return 4;
  }
  const vecMatch = /^vec([234])<f32>$/.exec(t);
  if (vecMatch) {
    return Number.parseInt(vecMatch[1], 10) || 0;
  }
  const matMatch = /^mat([234])x([234])<f32>$/.exec(t);
  if (matMatch) {
    const cols = Number.parseInt(matMatch[1], 10) || 0;
    const rows = Number.parseInt(matMatch[2], 10) || 0;
    return cols * rows;
  }
  return 0;
}

function matrixDimsForType(type: unknown) {
  const t = canonicalizeParamType(type);
  const match = /^mat([234])x([234])<f32>$/.exec(t);
  if (!match) {
    return null;
  }
  const cols = Number.parseInt(match[1], 10) || 0;
  const rows = Number.parseInt(match[2], 10) || 0;
  if (cols <= 0 || rows <= 0) {
    return null;
  }
  return { cols, rows, width: cols * rows };
}

function totalSlotWidthForType(type: unknown): number {
  const arrayInfo = parseArrayParamType(type);
  if (arrayInfo) {
    return arrayInfo.length * scalarSlotWidthForType(arrayInfo.elementType);
  }
  return scalarSlotWidthForType(type);
}

function safeArrayParamValue(paramViewModel: any, def: any): any[] {
  const expectedSlots = totalSlotWidthForType(def?.type);
  return paramViewModel.getArrayValue(def, expectedSlots);
}

function safeColorParamValue(paramViewModel: any, def: any): [number, number, number, number] {
  return paramViewModel.getColorValue(def);
}

function attachArrayPager(
  container: HTMLElement,
  arrayLength: number,
  renderElement: (host: HTMLElement, index: number) => void,
  options: { simpleGrid?: boolean; pageSize?: number } = {}
) {
  const { simpleGrid = false, pageSize = ARRAY_PAGE_SIZE } = options;

  let columns = 1;
  let effectivePageSize = pageSize;
  let pageCount = Math.max(1, Math.ceil(arrayLength / effectivePageSize));
  let pageIndex = 0;

  const pageBody = document.createElement("div");
  pageBody.className = "param-array-page";
  if (simpleGrid) {
    pageBody.classList.add("param-array-page--simple");
    pageBody.style.setProperty("--array-rows", String(pageSize));
  }
  container.appendChild(pageBody);

  const pager = document.createElement("div");
  pager.className = "param-array-pager";
  pager.setAttribute("hidden", "");

  const prevBtn = document.createElement("button");
  prevBtn.type = "button";
  prevBtn.className = "param-array-pager-btn";
  prevBtn.textContent = "Prev";

  const nextBtn = document.createElement("button");
  nextBtn.type = "button";
  nextBtn.className = "param-array-pager-btn";
  nextBtn.textContent = "Next";

  const pageLabel = document.createElement("span");
  pageLabel.className = "param-array-pager-label";

  prevBtn.addEventListener("click", () => {
    pageIndex = Math.max(0, pageIndex - 1);
    renderPage();
  });
  nextBtn.addEventListener("click", () => {
    pageIndex = Math.min(pageCount - 1, pageIndex + 1);
    renderPage();
  });

  pager.appendChild(prevBtn);
  pager.appendChild(pageLabel);
  pager.appendChild(nextBtn);
  container.appendChild(pager);

  function recomputePagination() {
    const prevPageSize = effectivePageSize;
    if (simpleGrid) {
      const nextColumns = Math.max(
        SIMPLE_ARRAY_MIN_COLUMNS,
        Math.min(
          SIMPLE_ARRAY_MAX_COLUMNS,
          Math.floor((container.clientWidth || SIMPLE_ARRAY_COLUMN_MIN_WIDTH_PX) / SIMPLE_ARRAY_COLUMN_MIN_WIDTH_PX)
        )
      );
      columns = nextColumns;
      pageBody.style.setProperty("--array-cols", String(columns));
      effectivePageSize = pageSize * columns;
    } else {
      columns = 1;
      pageBody.style.removeProperty("--array-cols");
      effectivePageSize = pageSize;
    }

    if (effectivePageSize !== prevPageSize) {
      const firstVisible = pageIndex * prevPageSize;
      pageIndex = Math.floor(firstVisible / effectivePageSize);
    }
    pageCount = Math.max(1, Math.ceil(arrayLength / effectivePageSize));
    pageIndex = Math.min(pageIndex, pageCount - 1);
    pager.toggleAttribute("hidden", pageCount <= 1);
  }

  function renderPage() {
    recomputePagination();
    pageBody.innerHTML = "";
    const start = pageIndex * effectivePageSize;
    const end = Math.min(arrayLength, start + effectivePageSize);
    for (let i = start; i < end; i += 1) {
      renderElement(pageBody, i);
    }
    pageLabel.textContent = `${start + 1}-${end} / ${arrayLength}`;
    prevBtn.disabled = pageIndex === 0;
    nextBtn.disabled = pageIndex >= pageCount - 1;
  }

  if (simpleGrid && typeof ResizeObserver !== "undefined") {
    const resizeObserver = new ResizeObserver(() => renderPage());
    resizeObserver.observe(container);
  }

  if (simpleGrid && typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => renderPage());
  }

  renderPage();
}

export function renderShaderParamsUi({ shaderParamsEl, renderer, paramDefs, textureDefs, updateParamsLayoutMode }: any) {
  const paramViewModel = createShaderParamViewModel(renderer);
  shaderParamsEl.innerHTML = "";
  const safeParamDefs = Array.isArray(paramDefs) ? paramDefs : [];
  const safeTextureDefs = Array.isArray(textureDefs) ? textureDefs : [];
  const totalRows = safeParamDefs.length + safeTextureDefs.length;
  shaderParamsEl.classList.toggle(
    "shader-params-compact",
    totalRows > 0 && totalRows <= COMPACT_PANEL_ROW_MAX
  );
  shaderParamsEl.classList.add("active");
  let rowOrdinal = 0;
  for (const def of safeParamDefs) {
    const type = canonicalizeParamType(def.type);
    const paramType = def.paramType ?? null;
    const isDynamicArray = paramType
      ? paramType.name === "array" && paramType.size == null
      : parseDynamicArrayParamType(def.type) !== null;
    const dynamicElementType = isDynamicArray
      ? (paramType?.params?.[0] ?? parseDynamicArrayParamType(def.type)?.elementType ?? "f32")
      : null;
    const arrayInfo = parseArrayParamType(def.type);
    const isArray = arrayInfo !== null;
    const baseType = arrayInfo ? arrayInfo.elementType : type;
    const matrixInfo = matrixDimsForType(baseType);
    const componentWidth = scalarSlotWidthForType(baseType);
    const row = document.createElement("div");
    row.className = "param-row";
    if (rowOrdinal % 2 === 1) {
      row.classList.add("param-row--alt");
    }
    rowOrdinal += 1;

    const label = document.createElement("label");
    label.textContent = def.name;
    row.appendChild(label);

    if (isDynamicArray) {
      // Dynamic (runtime-sized) array backed by a storage buffer.
      const elementType = dynamicElementType!;
      const elemComponentWidth = scalarSlotWidthForType(elementType) || 1;
      const isElemColor = isColorLikeParamType(elementType);
      const isElemBoolean = elementType === "bool";
      const isElemInteger = isIntegerParamType(elementType);
      const host = document.createElement("div");
      row.classList.add("param-row--array", "param-row--dynamic-array");

      const getCurrentValues = (): number[] => {
        const raw = renderer.paramValues?.get(def.name);
        return Array.isArray(raw) ? raw : [];
      };

      const editorHandle = renderDynamicArrayParamEditor(host, {
        elementType,
        componentWidth: elemComponentWidth,
        isColor: isElemColor,
        isBoolean: isElemBoolean,
        isInteger: isElemInteger,
        min: typeof def.min === "number" ? def.min : undefined,
        max: typeof def.max === "number" ? def.max : undefined,
        step: isElemInteger ? "1" : "0.01",
        paramName: def.name,
        values: getCurrentValues(),
        onChange: (next) => {
          renderer.setParamValue(def, next);
        }
      });

      row.appendChild(host);
    } else if (arrayInfo && baseType === "color") {
      const initial = safeArrayParamValue(paramViewModel, def);
      const editor = document.createElement("div");
      editor.className = "param-array-editor";
      row.classList.add("param-row--array");
      attachArrayPager(editor, arrayInfo.length, (host, i) => {
        const item = document.createElement("div");
        item.className = "param-array-item";

        const itemLabel = document.createElement("div");
        itemLabel.className = "param-array-item-index";
        itemLabel.textContent = `[${i}]`;

        const controls = document.createElement("div");
        controls.className = "param-array-item-controls";

        const editorHandle = renderColorParamEditor(controls, {
          className: "param-array-item-controls param-color-inputs",
          ariaPrefix: `${def.name}[${i}]`,
          value: initial.slice(i * 4, i * 4 + 4),
          onChange: (rgba) => {
          const next = safeArrayParamValue(paramViewModel, def);
          next.splice(i * 4, 4, ...rgba);
          paramViewModel.setValue(def, next);
          }
        });

        item.appendChild(itemLabel);
        item.appendChild(controls);
        host.appendChild(item);
      });
      row.appendChild(editor);
    } else if (arrayInfo && matrixInfo) {
      const initial = safeArrayParamValue(paramViewModel, def);
      const editor = document.createElement("div");
      editor.className = "param-array-editor";
      row.classList.add("param-row--array");
      attachArrayPager(editor, arrayInfo.length, (host, matrixIndex) => {
        const group = document.createElement("div");
        group.className = "param-matrix-group";

        const groupLabel = document.createElement("div");
        groupLabel.className = "param-matrix-group-label";
        groupLabel.textContent = `[${matrixIndex}]`;
        group.appendChild(groupLabel);

        const matrixHost = document.createElement("div");
        const matrixOffset = matrixIndex * matrixInfo.width;
        const editorHandle = renderMatrixParamEditor(matrixHost, {
          values: initial.slice(matrixOffset, matrixOffset + matrixInfo.width),
          cols: matrixInfo.cols,
          rows: matrixInfo.rows,
          min: typeof def.min === "number" ? def.min : undefined,
          max: typeof def.max === "number" ? def.max : undefined,
          step: "0.01",
          ariaPrefix: `${def.name}[${matrixIndex}]`,
          onChange: (values) => {
            const next = safeArrayParamValue(paramViewModel, def);
            for (let localIndex = 0; localIndex < matrixInfo.width; localIndex += 1) {
              next[matrixOffset + localIndex] = String(values[localIndex] ?? 0);
            }
            paramViewModel.setValue(def, next);
          }
        });

        group.appendChild(matrixHost);
        host.appendChild(group);
      });
      row.appendChild(editor);
    } else if (arrayInfo) {
      const initial = safeArrayParamValue(paramViewModel, def);
      const editor = document.createElement("div");
      editor.className = "param-array-editor";
      const simpleGrid = componentWidth === 1;
      if (simpleGrid) {
        row.classList.add("param-row--array", "param-row--array-simple");
      } else {
        row.classList.add("param-row--array");
      }
      attachArrayPager(editor, arrayInfo.length, (host, elementIndex) => {
        const item = document.createElement("div");
        item.className = "param-array-item";
        if (simpleGrid) {
          item.classList.add("param-array-item--simple");
        }

        const itemLabel = document.createElement("div");
        itemLabel.className = "param-array-item-index";
        itemLabel.textContent = `[${elementIndex}]`;

        const controls = document.createElement("div");
        controls.className = "param-array-item-controls";

        const explicitMin = typeof def.min === "number" ? def.min : undefined;
        const explicitMax = typeof def.max === "number" ? def.max : undefined;
        const defaultMin = baseType === "u32" ? 0 : undefined;
        const min = typeof explicitMin === "number" ? explicitMin : defaultMin;
        const max = explicitMax;
        const start = elementIndex * componentWidth;
        const end = start + componentWidth;

        const editorHandle = renderArrayItemParamEditor(controls, {
          className: "param-array-item-controls",
          ariaPrefix: `${def.name}[${elementIndex}]`,
          values: initial.slice(start, end),
          componentCount: componentWidth,
          isBoolean: baseType === "bool",
          step: isIntegerParamType(baseType) ? "1" : "0.01",
          min,
          max,
          onChange: (values) => {
            const next = safeArrayParamValue(paramViewModel, def);
            for (let i = 0; i < componentWidth; i += 1) {
              const slotIndex = start + i;
              const value = values[i];
              next[slotIndex] = baseType === "bool" ? Boolean(value) : String(Number(value ?? 0));
            }
            paramViewModel.setValue(def, next);
          }
        });

        item.appendChild(itemLabel);
        item.appendChild(controls);
        host.appendChild(item);
      }, { simpleGrid });
      row.appendChild(editor);
    } else if (isColorLikeParamType(type)) {
      const inputs = document.createElement("div");
      const initial = safeColorParamValue(paramViewModel, def);

      const editorHandle = renderColorParamEditor(inputs, {
        className: "param-inputs param-color-inputs",
        ariaPrefix: def.name,
        value: initial,
        onChange: (rgba) => {
          paramViewModel.setValue(def, rgba);
        }
      });

      row.appendChild(inputs);
    } else if (matrixInfo) {
      const initial = safeArrayParamValue(paramViewModel, def);
      const matrixHost = document.createElement("div");
      const editorHandle = renderMatrixParamEditor(matrixHost, {
        values: initial.slice(0, matrixInfo.width),
        cols: matrixInfo.cols,
        rows: matrixInfo.rows,
        step: "0.01",
        ariaPrefix: def.name,
        onChange: (values) => {
          const next = safeArrayParamValue(paramViewModel, def);
          for (let i = 0; i < matrixInfo.width; i += 1) {
            next[i] = String(values[i] ?? 0);
          }
          paramViewModel.setValue(def, next);
        }
      });
      row.appendChild(matrixHost);
    } else if (componentWidth > 1) {
      const initial = safeArrayParamValue(paramViewModel, def);
      const inputs = document.createElement("div");
      const explicitMin = typeof def.min === "number" ? def.min : undefined;
      const explicitMax = typeof def.max === "number" ? def.max : undefined;
      const defaultMin = baseType === "u32" ? 0 : undefined;
      const min = typeof explicitMin === "number" ? explicitMin : defaultMin;
      const max = explicitMax;

      const editorHandle = renderVectorParamEditor(inputs, {
        className: "param-inputs",
        ariaPrefix: def.name,
        values: initial.slice(0, componentWidth),
        componentCount: componentWidth,
        step: isIntegerParamType(baseType) ? "1" : "0.01",
        min,
        max,
        onChange: (values) => {
          const next = safeArrayParamValue(paramViewModel, def);
          for (let i = 0; i < componentWidth; i += 1) {
            next[i] = String(values[i] ?? 0);
          }
          paramViewModel.setValue(def, next);
        }
      });
      row.appendChild(inputs);
    } else if (type === "f32") {
      const initial = Number(
        renderer.paramValues.get(def.name) ?? renderer.defaultValueForType(def.type, def.default)
      );
      const hasRange = typeof def.min === "number" && typeof def.max === "number";

      if (hasRange) {
        const inputs = document.createElement("div");
        const editorHandle = renderNumberParamEditor(inputs, {
          className: "param-inputs",
          ariaLabel: def.name,
          value: Number.isFinite(initial) ? initial : 0,
          min: def.min,
          max: def.max,
          step: "0.01",
          withSlider: true,
          fixed2: true,
          onChange: (next) => {
            renderer.setParamValue(def, String(next));
          }
        });
        row.appendChild(inputs);
      } else {
        const host = document.createElement("div");
        const editorHandle = renderNumberParamEditor(host, {
          className: "param-inputs",
          ariaLabel: def.name,
          value: Number.isFinite(initial) ? initial : 0,
          step: "0.01",
          onChange: (next) => {
            renderer.setParamValue(def, String(next));
          }
        });
        row.appendChild(host);
      }
    } else if (isIntegerParamType(type)) {
      const initial = renderer.normalizeParamValue(def, renderer.paramValues.get(def.name));
      const host = document.createElement("div");
      const explicitMin = typeof def.min === "number" ? def.min : undefined;
      const explicitMax = typeof def.max === "number" ? def.max : undefined;
      const defaultMin = type === "u32" ? 0 : undefined;
      const min = typeof explicitMin === "number" ? explicitMin : defaultMin;
      const max = explicitMax;
      const editorHandle = renderNumberParamEditor(host, {
        className: "param-inputs",
        ariaLabel: def.name,
        value: Number.isFinite(Number(initial)) ? Number(initial) : 0,
        step: "1",
        min,
        max,
        onChange: (next) => {
          renderer.setParamValue(def, String(next));
        }
      });
      row.appendChild(host);
    } else if (type === "bool") {
      const initial = renderer.normalizeParamValue(def, renderer.paramValues.get(def.name));
      const host = document.createElement("div");
      const editorHandle = renderBooleanParamEditor(host, {
        ariaLabel: def.name,
        value: Boolean(initial),
        onChange: (next) => {
          renderer.setParamValue(def, next);
        }
      });
      row.appendChild(host);
    }

    shaderParamsEl.appendChild(row);
  }

  for (const textureDef of safeTextureDefs) {
    const row = document.createElement("div");
    row.className = "param-row";
    if (rowOrdinal % 2 === 1) {
      row.classList.add("param-row--alt");
    }
    rowOrdinal += 1;

    const label = document.createElement("label");
    label.textContent = `${textureDef.name} texture`;
    row.appendChild(label);

    const select = document.createElement("select");
    const options = Array.isArray(textureDef.options) ? textureDef.options : [];
    for (const optionDef of options) {
      const option = document.createElement("option");
      const isDefault = optionDef.url === textureDef.defaultUrl;
      option.value = optionDef.url;
      option.textContent = isDefault
        ? `${optionDef.relPath} (default)`
        : optionDef.relPath;
      select.appendChild(option);
    }

    const initialValue = textureDef.selectedUrl || textureDef.defaultUrl || "";
    if (initialValue) {
      select.value = initialValue;
    }
    if (options.length <= 1) {
      select.disabled = true;
      select.title = "Only the default asset is currently available for this texture parameter.";
    }
    select.addEventListener("change", async () => {
      try {
        await renderer.setTextureSelection(textureDef.name, select.value);
      } catch (err: any) {
        renderer.reportRuntimeIssue(err?.message || err, "texture selection");
      }
    });
    row.appendChild(select);
    shaderParamsEl.appendChild(row);
  }

  updateParamsLayoutMode(safeParamDefs, safeTextureDefs);
}
