import init, { fresco_completion_items } from "../../pkg/fresco_wasm.js";
import { createCodeMirrorEditor } from "../../src/editor-monaco-adapter";
import { createWasmCompletionProvider } from "../../src/completion-provider";
await init();
const provider = createWasmCompletionProvider({ completionItemsFn: fresco_completion_items,
  utf16OffsetToUtf8Byte: (source, offset) => new TextEncoder().encode(source.slice(0, offset)).length });
const editor = createCodeMirrorEditor(document.querySelector<HTMLElement>("#editor")!, { provideCompletions: provider });
Object.assign(window, { editorTest: { setSource(source: string) {
  editor.setValue(source); editor.setPosition(editor.getModel().getPositionAt(source.length)); editor.focus();
}, source() { return editor.getValue(); } } });
