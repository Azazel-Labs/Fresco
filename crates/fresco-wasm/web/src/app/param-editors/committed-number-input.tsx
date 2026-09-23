/** @jsxImportSource preact */

import type { JSX } from "preact";
import { useReducer, useRef } from "preact/hooks";

type Props = Omit<JSX.InputHTMLAttributes<HTMLInputElement>, "value" | "onInput" | "onBlur" | "onKeyDown"> & {
  value: string;
  onCommit(event: Event): void;
};

/** Keep unfinished text separate from the parameter value, including during parent updates. */
export function CommittedNumberInput({ value, onCommit, ...props }: Props) {
  const draft = useRef<string | null>(null);
  const [, redraw] = useReducer((version: number) => version + 1, 0);

  const commit = (event: Event) => {
    if (draft.current === null) return;
    const text = draft.current.trim();
    const parsed = Number(text);
    draft.current = null;
    if (text !== "" && Number.isFinite(parsed)) {
      let next = parsed;
      if (props.min !== undefined) next = Math.max(Number(props.min), next);
      if (props.max !== undefined) next = Math.min(Number(props.max), next);
      (event.currentTarget as HTMLInputElement).value = String(next);
      onCommit(event);
    }
    redraw(undefined);
  };

  return (
    <input
      {...props}
      type="text"
      inputMode="decimal"
      data-number-editor=""
      value={draft.current ?? value}
      onInput={(event) => {
        draft.current = event.currentTarget.value;
        redraw(undefined);
      }}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter" && !event.isComposing) {
          event.preventDefault();
          commit(event);
        }
      }}
    />
  );
}
