/** @jsxImportSource preact */

import { render } from "preact";
import { CommittedNumberInput } from "./committed-number-input";
import { renderColorParamEditor } from "./color-param-editor";
import { renderArrayItemParamEditor } from "./scalar-param-editors";

type DynamicArrayEditorHandle = {
  update(values: number[] | null | undefined): void;
  dispose(): void;
};

type DynamicArrayEditorOptions = {
  elementType: string;
  componentWidth: number;
  values: number[];
  isColor?: boolean;
  isBoolean?: boolean;
  isInteger?: boolean;
  min?: number;
  max?: number;
  step?: string;
  paramName: string;
  onChange(next: number[]): void;
};

const DEFAULT_ELEMENT_BY_TYPE: Record<string, number[]> = {
  color: [1, 1, 1, 1],
};

function defaultElementForType(componentWidth: number, isColor: boolean): number[] {
  if (isColor) {
    return DEFAULT_ELEMENT_BY_TYPE.color.slice();
  }
  return Array.from({ length: Math.max(1, componentWidth) }, () => 0);
}

export function renderDynamicArrayParamEditor(
  host: HTMLElement,
  options: DynamicArrayEditorOptions,
): DynamicArrayEditorHandle {
  const {
    elementType,
    componentWidth,
    isColor = false,
    isBoolean = false,
    isInteger = false,
    paramName,
  } = options;

  let current = Array.isArray(options.values) ? options.values.slice() : [];

  const elementCount = () => Math.floor(current.length / Math.max(1, componentWidth));

  const emit = (next: number[]) => {
    current = next;
    options.onChange(next);
    draw();
  };

  const addElement = () => {
    const fill = defaultElementForType(componentWidth, isColor);
    emit([...current, ...fill]);
  };

  const removeElement = (index: number) => {
    const start = index * componentWidth;
    const next = [...current];
    next.splice(start, componentWidth);
    emit(next);
  };

  const step = options.step || (isInteger ? "1" : "0.01");

  const draw = () => {
    const count = elementCount();
    render(
      <div class="param-dynamic-array-editor">
        {count > 0 && (
          <div class="param-array-page">
            {Array.from({ length: count }, (_, i) => {
              const start = i * componentWidth;
              const slice = current.slice(start, start + componentWidth);
              return (
                <div key={i} class="param-array-item">
                  <div class="param-array-item-index">[{i}]</div>
                  <div class="param-dynamic-array-item-body">
                    <ItemEditor
                      index={i}
                      slice={slice}
                      componentWidth={componentWidth}
                      isColor={isColor}
                      isBoolean={isBoolean}
                      step={step}
                      min={options.min}
                      max={options.max}
                      paramName={paramName}
                      elementType={elementType}
                      onSliceChange={(values) => {
                        const next = current.slice();
                        for (let j = 0; j < componentWidth; j += 1) {
                          next[start + j] = values[j] ?? 0;
                        }
                        emit(next);
                      }}
                    />
                  </div>
                  <button
                    type="button"
                    class="param-dynamic-array-remove-btn"
                    aria-label={`Remove ${paramName}[${i}]`}
                    onClick={() => removeElement(i)}
                  >
                    ×
                  </button>
                </div>
              );
            })}
          </div>
        )}
        <div class="param-dynamic-array-controls">
          <button
            type="button"
            class="param-dynamic-array-add-btn"
            aria-label={`Add element to ${paramName}`}
            onClick={addElement}
          >
            + Add
          </button>
          {count > 0 && (
            <span class="param-dynamic-array-count">{count} element{count !== 1 ? "s" : ""}</span>
          )}
        </div>
      </div>,
      host,
    );
  };

  draw();

  return {
    update(values) {
      current = Array.isArray(values) ? values.slice() : [];
      draw();
    },
    dispose() {
      render(null, host);
    },
  };
}

// A sub-component that renders a single element editor inline using DOM rendering.
// We do it inline with JSX here for simplicity; the actual rendering delegates
// to the existing scalar / color editors mounted to a ref.
function ItemEditor({
  index,
  slice,
  componentWidth,
  isColor,
  isBoolean,
  step,
  min,
  max,
  paramName,
  elementType,
  onSliceChange,
}: {
  index: number;
  slice: number[];
  componentWidth: number;
  isColor: boolean;
  isBoolean: boolean;
  step: string;
  min?: number;
  max?: number;
  paramName: string;
  elementType: string;
  onSliceChange(values: number[]): void;
}) {
  if (isColor) {
    return (
      <ColorItemEditor
        index={index}
        slice={slice}
        paramName={paramName}
        onSliceChange={onSliceChange}
      />
    );
  }

  if (componentWidth === 1) {
    const value = Number(slice[0]) || 0;
    const ariaLabel = `${paramName}[${index}]`;
    if (isBoolean) {
      return (
        <input
          type="checkbox"
          checked={value >= 0.5}
          aria-label={ariaLabel}
          onChange={(e) => {
            onSliceChange([(e.currentTarget as HTMLInputElement).checked ? 1 : 0]);
          }}
        />
      );
    }
    return (
      <CommittedNumberInput
        class="param-array-item-number"
        value={String(value)}
        step={step}
        min={typeof min === "number" ? String(min) : undefined}
        max={typeof max === "number" ? String(max) : undefined}
        aria-label={ariaLabel}
        onCommit={(e) => {
          const v = Number((e.currentTarget as HTMLInputElement).value);
          onSliceChange([Number.isFinite(v) ? v : 0]);
        }}
        onFocus={(e) => {
          (e.currentTarget as HTMLInputElement).select();
        }}
      />
    );
  }

  return (
    <div class="param-array-item-controls">
      {slice.map((val, j) => (
        <CommittedNumberInput
          key={j}
          class="param-array-item-number"
          value={String(Number(val) || 0)}
          step={step}
          min={typeof min === "number" ? String(min) : undefined}
          max={typeof max === "number" ? String(max) : undefined}
          aria-label={`${paramName}[${index}].${j}`}
          onCommit={(e) => {
            const next = slice.slice();
            next[j] = Number((e.currentTarget as HTMLInputElement).value) || 0;
            onSliceChange(next);
          }}
          onFocus={(e) => {
            (e.currentTarget as HTMLInputElement).select();
          }}
        />
      ))}
    </div>
  );
}

// Color picker rendered into a detached div, mounted by the Preact ref.
function ColorItemEditor({
  index,
  slice,
  paramName,
  onSliceChange,
}: {
  index: number;
  slice: number[];
  paramName: string;
  onSliceChange(values: number[]): void;
}) {
  const rgba: [number, number, number, number] = [
    Number(slice[0]) || 0,
    Number(slice[1]) || 0,
    Number(slice[2]) || 0,
    Number(slice[3] ?? 1) || 0,
  ];

  const hex = rgbaToHex(rgba);
  const alpha = rgba[3];

  const onPickerInput = (e: Event) => {
    const input = e.currentTarget as HTMLInputElement;
    const next = hexToRgba(input.value, alpha);
    onSliceChange([...next]);
  };

  const onPickerClick = (e: Event) => {
    const input = e.currentTarget as HTMLInputElement;
    const showPicker = (input as any).showPicker;
    if (typeof showPicker === "function") {
      try {
        showPicker.call(input);
      } catch {
        // Browser refused programmatic picker open; native click behavior still applies.
      }
    }
  };

  const onChannelInput = (chIdx: number) => (e: Event) => {
    const v = Math.min(1, Math.max(0, Number((e.currentTarget as HTMLInputElement).value) || 0));
    const next = rgba.slice() as [number, number, number, number];
    next[chIdx] = v;
    onSliceChange([...next]);
  };

  return (
    <div class="param-color-inputs">
      <input
        type="color"
        value={hex}
        aria-label={`${paramName}[${index}] picker`}
        onInput={onPickerInput}
        onClick={onPickerClick}
      />
      {(["r", "g", "b", "a"] as const).map((ch, chIdx) => (
        <CommittedNumberInput
          key={ch}
          min="0"
          max="1"
          step="0.01"
          value={rgba[chIdx].toFixed(2)}
          aria-label={`${paramName}[${index}] ${ch}`}
          onCommit={onChannelInput(chIdx)}
          onFocus={(e) => {
            (e.currentTarget as HTMLInputElement).select();
          }}
        />
      ))}
    </div>
  );
}

function rgbaToHex(rgba: [number, number, number, number]): string {
  const clamp = (v: number) => Math.round(Math.min(1, Math.max(0, v)) * 255);
  const hex2 = (n: number) => clamp(n).toString(16).padStart(2, "0");
  return `#${hex2(rgba[0])}${hex2(rgba[1])}${hex2(rgba[2])}`;
}

function hexToRgba(hex: string, alpha = 1): [number, number, number, number] {
  const normalized = String(hex || "").trim().replace(/^#/, "");
  if (!/^[0-9a-fA-F]{6}$/.test(normalized)) {
    return [1, 1, 1, Math.min(1, Math.max(0, alpha))];
  }
  return [
    parseInt(normalized.slice(0, 2), 16) / 255,
    parseInt(normalized.slice(2, 4), 16) / 255,
    parseInt(normalized.slice(4, 6), 16) / 255,
    Math.min(1, Math.max(0, alpha)),
  ];
}
