/** @jsxImportSource preact */

import { render } from "preact";
import { CommittedNumberInput } from "./committed-number-input";
import { hexToRgba, rgbaToHex } from "../../preview/helpers";

type RgbaTuple = [number, number, number, number];

type RenderColorParamEditorOptions = {
  value: ArrayLike<number> | null | undefined;
  onChange(next: RgbaTuple): void;
  className?: string;
  ariaPrefix?: string;
};

type RenderColorParamEditorHandle = {
  update(value: ArrayLike<number> | null | undefined): void;
  dispose(): void;
};

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, Number.isFinite(value) ? value : 0));
}

function normalizeRgba(value: ArrayLike<number> | null | undefined): RgbaTuple {
  return [
    clamp01(Number(value?.[0]) || 0),
    clamp01(Number(value?.[1]) || 0),
    clamp01(Number(value?.[2]) || 0),
    clamp01(Number(value?.[3] ?? 1)),
  ];
}

function formatChannel(value: number): string {
  return clamp01(value).toFixed(2);
}

export function renderColorParamEditor(
  host: HTMLElement,
  options: RenderColorParamEditorOptions,
): RenderColorParamEditorHandle {
  let current = normalizeRgba(options.value);

  const className = options.className || "param-color-inputs";
  const ariaPrefix = String(options.ariaPrefix || "color");

  const applyAndNotify = (next: RgbaTuple) => {
    current = normalizeRgba(next);
    options.onChange(current);
    draw();
  };

  const onPickerInput = (event: Event) => {
    const input = event.currentTarget as HTMLInputElement;
    applyAndNotify(hexToRgba(input.value, current[3]));
  };

  const onPickerClick = (event: Event) => {
    const input = event.currentTarget as HTMLInputElement;
    const showPicker = (input as any).showPicker;
    if (typeof showPicker === "function") {
      try {
        showPicker.call(input);
      } catch {
        // Browser refused programmatic picker open; native click behavior still applies.
      }
    }
  };

  const onChannelInput = (index: number) => (event: Event) => {
    const input = event.currentTarget as HTMLInputElement;
    const next = [...current] as RgbaTuple;
    next[index] = clamp01(Number(input.value));
    applyAndNotify(next);
  };

  const draw = () => {
    render(
      <div className={className}>
        <input
          type="color"
          value={rgbaToHex(current)}
          aria-label={`${ariaPrefix} picker`}
          onInput={onPickerInput}
          onClick={onPickerClick}
        />
        <CommittedNumberInput
          min="0"
          max="1"
          step="0.01"
          value={formatChannel(current[0])}
          aria-label={`${ariaPrefix} red`}
          onCommit={onChannelInput(0)}
          onFocus={(event) => {
            (event.currentTarget as HTMLInputElement).select();
          }}
        />
        <CommittedNumberInput
          min="0"
          max="1"
          step="0.01"
          value={formatChannel(current[1])}
          aria-label={`${ariaPrefix} green`}
          onCommit={onChannelInput(1)}
          onFocus={(event) => {
            (event.currentTarget as HTMLInputElement).select();
          }}
        />
        <CommittedNumberInput
          min="0"
          max="1"
          step="0.01"
          value={formatChannel(current[2])}
          aria-label={`${ariaPrefix} blue`}
          onCommit={onChannelInput(2)}
          onFocus={(event) => {
            (event.currentTarget as HTMLInputElement).select();
          }}
        />
        <CommittedNumberInput
          min="0"
          max="1"
          step="0.01"
          value={formatChannel(current[3])}
          aria-label={`${ariaPrefix} alpha`}
          onCommit={onChannelInput(3)}
          onFocus={(event) => {
            (event.currentTarget as HTMLInputElement).select();
          }}
        />
      </div>,
      host,
    );
  };

  draw();

  return {
    update(value) {
      current = normalizeRgba(value);
      draw();
    },
    dispose() {
      render(null, host);
    },
  };
}
