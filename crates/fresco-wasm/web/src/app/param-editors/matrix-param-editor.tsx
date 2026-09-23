/** @jsxImportSource preact */

import { render } from "preact";
import { CommittedNumberInput } from "./committed-number-input";

type MatrixEditorOptions = {
  values: ArrayLike<number> | null | undefined;
  cols: number;
  rows: number;
  onChange(next: number[]): void;
  className?: string;
  ariaPrefix?: string;
  min?: number;
  max?: number;
  step?: string;
};

type MatrixEditorHandle = {
  update(values: ArrayLike<number> | null | undefined): void;
  dispose(): void;
};

function normalizeVector(values: ArrayLike<number> | null | undefined, size: number): number[] {
  const next: number[] = [];
  for (let i = 0; i < size; i += 1) {
    const n = Number(values?.[i]);
    next.push(Number.isFinite(n) ? n : 0);
  }
  return next;
}

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

function numberToString(value: number): string {
  return Number.isFinite(value) ? String(value) : "0";
}

export function renderMatrixParamEditor(host: HTMLElement, options: MatrixEditorOptions): MatrixEditorHandle {
  const slotCount = options.cols * options.rows;
  let current = normalizeVector(options.values, slotCount);
  const className = options.className || "param-matrix-grid";
  const ariaPrefix = String(options.ariaPrefix || "matrix");
  const step = options.step || "0.01";

  const emit = (index: number, nextRaw: number) => {
    const next = [...current];
    next[index] = clampFinite(nextRaw, options.min, options.max);
    current = next;
    options.onChange(next);
    draw();
  };

  const draw = () => {
    render(
      <div className={className} style={{ "--matrix-cols": String(options.cols) }}>
        {current.map((value, index) => {
          const rowIndex = Math.floor(index / options.cols);
          const colIndex = index % options.cols;
          return (
            <CommittedNumberInput
              key={index}
              min={typeof options.min === "number" ? String(options.min) : undefined}
              max={typeof options.max === "number" ? String(options.max) : undefined}
              step={step}
              value={numberToString(value)}
              aria-label={`${ariaPrefix} r${rowIndex} c${colIndex}`}
              onCommit={(event) => {
                const input = event.currentTarget as HTMLInputElement;
                emit(index, Number(input.value));
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
      current = normalizeVector(values, slotCount);
      draw();
    },
    dispose() {
      render(null, host);
    },
  };
}
