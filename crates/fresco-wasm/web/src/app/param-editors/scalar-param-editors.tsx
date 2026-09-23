/** @jsxImportSource preact */

import { render } from "preact";
import { CommittedNumberInput } from "./committed-number-input";

type NumberEditorHandle = {
  update(value: number): void;
  dispose(): void;
};

type VectorEditorHandle = {
  update(values: ArrayLike<number> | null | undefined): void;
  dispose(): void;
};

type BooleanEditorHandle = {
  update(value: boolean): void;
  dispose(): void;
};

type ArrayItemEditorHandle = {
  update(values: ArrayLike<number | boolean> | null | undefined): void;
  dispose(): void;
};

type NumberEditorOptions = {
  value: number;
  onChange(next: number): void;
  className?: string;
  ariaLabel?: string;
  min?: number;
  max?: number;
  step?: string;
  withSlider?: boolean;
  fixed2?: boolean;
};

type VectorEditorOptions = {
  values: ArrayLike<number> | null | undefined;
  componentCount: number;
  onChange(next: number[]): void;
  className?: string;
  ariaPrefix?: string;
  min?: number;
  max?: number;
  step?: string;
};

type BooleanEditorOptions = {
  value: boolean;
  onChange(next: boolean): void;
  ariaLabel?: string;
};

type ArrayItemEditorOptions = {
  values: ArrayLike<number | boolean> | null | undefined;
  componentCount: number;
  onChange(next: Array<number | boolean>): void;
  isBoolean?: boolean;
  className?: string;
  ariaPrefix?: string;
  min?: number;
  max?: number;
  step?: string;
};

function clampFinite(value: number, min?: number, max?: number): number {
  let next = Number.isFinite(value) ? value : 0;
  if (typeof min === "number") {
    next = Math.max(min, next);
  }
  if (typeof max === "number") {
    next = Math.min(max, next);
  }
  return next;
}

function numberToString(value: number, fixed2 = false): string {
  if (!Number.isFinite(value)) {
    return fixed2 ? "0.00" : "0";
  }
  return fixed2 ? value.toFixed(2) : String(value);
}

function normalizeVector(values: ArrayLike<number> | null | undefined, size: number): number[] {
  const next: number[] = [];
  for (let i = 0; i < size; i += 1) {
    const n = Number(values?.[i]);
    next.push(Number.isFinite(n) ? n : 0);
  }
  return next;
}

function normalizeArrayItemValues(
  values: ArrayLike<number | boolean> | null | undefined,
  size: number,
  isBoolean: boolean,
): Array<number | boolean> {
  const next: Array<number | boolean> = [];
  for (let i = 0; i < size; i += 1) {
    if (isBoolean) {
      next.push(Boolean(values?.[i]));
      continue;
    }
    const n = Number(values?.[i]);
    next.push(Number.isFinite(n) ? n : 0);
  }
  return next;
}

export function renderNumberParamEditor(host: HTMLElement, options: NumberEditorOptions): NumberEditorHandle {
  let current = clampFinite(Number(options.value), options.min, options.max);
  const className = options.withSlider
    ? `${options.className || "param-inputs"} param-inputs--slider`
    : (options.className || "param-inputs");
  const step = options.step || "0.01";
  const ariaLabel = options.ariaLabel || "number param";

  const emit = (nextRaw: number) => {
    current = clampFinite(nextRaw, options.min, options.max);
    options.onChange(current);
    draw();
  };

  const onNumberInput = (event: Event) => {
    const input = event.currentTarget as HTMLInputElement;
    emit(Number(input.value));
  };

  const onRangeInput = (event: Event) => {
    const input = event.currentTarget as HTMLInputElement;
    emit(Number(input.value));
  };

  const draw = () => {
    render(
      <div className={className}>
        {options.withSlider ? (
          <input
            type="range"
            min={typeof options.min === "number" ? String(options.min) : undefined}
            max={typeof options.max === "number" ? String(options.max) : undefined}
            step={step}
            value={numberToString(current)}
            aria-label={`${ariaLabel} slider`}
            onInput={onRangeInput}
          />
        ) : null}
        <CommittedNumberInput
          min={typeof options.min === "number" ? String(options.min) : undefined}
          max={typeof options.max === "number" ? String(options.max) : undefined}
          step={step}
          value={numberToString(current, Boolean(options.fixed2))}
          aria-label={ariaLabel}
          onCommit={onNumberInput}
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
      current = clampFinite(Number(value), options.min, options.max);
      draw();
    },
    dispose() {
      render(null, host);
    },
  };
}

export function renderVectorParamEditor(host: HTMLElement, options: VectorEditorOptions): VectorEditorHandle {
  let current = normalizeVector(options.values, options.componentCount);
  const className = options.className || "param-inputs";
  const step = options.step || "0.01";
  const ariaPrefix = String(options.ariaPrefix || "vector");

  const emit = (index: number, nextRaw: number) => {
    const next = [...current];
    next[index] = clampFinite(nextRaw, options.min, options.max);
    current = next;
    options.onChange(next);
    draw();
  };

  const draw = () => {
    render(
      <div className={className}>
        {current.map((value, index) => (
          <CommittedNumberInput
            key={index}
            min={typeof options.min === "number" ? String(options.min) : undefined}
            max={typeof options.max === "number" ? String(options.max) : undefined}
            step={step}
            value={numberToString(value)}
            aria-label={`${ariaPrefix}[${index}]`}
            onCommit={(event) => {
              const input = event.currentTarget as HTMLInputElement;
              emit(index, Number(input.value));
            }}
            onFocus={(event) => {
              (event.currentTarget as HTMLInputElement).select();
            }}
          />
        ))}
      </div>,
      host,
    );
  };

  draw();

  return {
    update(values) {
      current = normalizeVector(values, options.componentCount);
      draw();
    },
    dispose() {
      render(null, host);
    },
  };
}

export function renderBooleanParamEditor(host: HTMLElement, options: BooleanEditorOptions): BooleanEditorHandle {
  let current = Boolean(options.value);
  const ariaLabel = options.ariaLabel || "boolean param";

  const draw = () => {
    render(
      <input
        type="checkbox"
        checked={current}
        aria-label={ariaLabel}
        onChange={(event) => {
          const input = event.currentTarget as HTMLInputElement;
          current = Boolean(input.checked);
          options.onChange(current);
        }}
      />,
      host,
    );
  };

  draw();

  return {
    update(value) {
      current = Boolean(value);
      draw();
    },
    dispose() {
      render(null, host);
    },
  };
}

export function renderArrayItemParamEditor(host: HTMLElement, options: ArrayItemEditorOptions): ArrayItemEditorHandle {
  const isBoolean = Boolean(options.isBoolean);
  let current = normalizeArrayItemValues(options.values, options.componentCount, isBoolean);
  const className = options.className || "param-array-item-controls";
  const step = options.step || "0.01";
  const ariaPrefix = String(options.ariaPrefix || "array item");

  const emit = (index: number, nextRaw: number | boolean) => {
    const next = [...current];
    if (isBoolean) {
      next[index] = Boolean(nextRaw);
    } else {
      next[index] = clampFinite(Number(nextRaw), options.min, options.max);
    }
    current = next;
    options.onChange(next);
    draw();
  };

  const draw = () => {
    render(
      <div className={className}>
        {current.map((value, index) => {
          const suffix = options.componentCount > 1 ? `.${index}` : "";
          if (isBoolean) {
            return (
              <input
                key={index}
                type="checkbox"
                checked={Boolean(value)}
                aria-label={`${ariaPrefix}${suffix}`}
                onChange={(event) => {
                  const input = event.currentTarget as HTMLInputElement;
                  emit(index, Boolean(input.checked));
                }}
              />
            );
          }
          return (
            <CommittedNumberInput
              key={index}
              min={typeof options.min === "number" ? String(options.min) : undefined}
              max={typeof options.max === "number" ? String(options.max) : undefined}
              step={step}
              value={numberToString(Number(value))}
              aria-label={`${ariaPrefix}${suffix}`}
              onCommit={(event) => {
                const input = event.currentTarget as HTMLInputElement;
                emit(index, Number(input.value));
              }}
              onFocus={(event) => {
                (event.currentTarget as HTMLInputElement).select();
              }}
            />
          );
        })}
      </div>,
      host,
    );
  };

  draw();

  return {
    update(values) {
      current = normalizeArrayItemValues(values, options.componentCount, isBoolean);
      draw();
    },
    dispose() {
      render(null, host);
    },
  };
}
