import { shouldAutoSuggest } from "./completion-auto-trigger";
import "monaco-editor/editor/browser/coreCommands.js";
import "monaco-editor/editor/contrib/clipboard/browser/clipboard.js";
import "monaco-editor/editor/contrib/contextmenu/browser/contextmenu.js";
import "monaco-editor/editor/contrib/find/browser/findController.js";
import "monaco-editor/editor/contrib/parameterHints/browser/parameterHints.js";
import "monaco-editor/editor/contrib/snippet/browser/snippetController2.js";
import "monaco-editor/editor/contrib/suggest/browser/suggestController.js";
import * as monaco from "monaco-editor/editor/editor.api.js";
import editorWorker from "monaco-editor/editor/editor.worker?worker";
import "../node_modules/monaco-editor/min/vs/editor/editor.main.css";

type CompletionItemSource = {
	label?: unknown;
	insertText?: unknown;
	snippet?: unknown;
	signature?: unknown;
	detail?: unknown;
	documentation?: unknown;
	info?: unknown;
	type?: unknown;
	boost?: unknown;
};

type Diagnostic = {
	severity?: unknown;
	from?: unknown;
	to?: unknown;
	message?: unknown;
};

type SyntaxSpan = {
	from?: unknown;
	to?: unknown;
	className?: unknown;
	beforeContent?: unknown;
	beforeClassName?: unknown;
};

type EditorOptions = {
	value?: unknown;
	readOnly?: boolean;
	fontSize?: number;
	onChange?: null | (() => void);
	provideCompletions?: null | ((modelApi: ModelApi, position: { lineNumber: number; column: number }) => Promise<CompletionItemSource[]> | CompletionItemSource[]);
	provideSyntaxHighlights?: null | ((modelApi: ModelApi, text: string) => SyntaxSpan[]);
};

type ModelApi = {
	getValue(): string;
	getValueLength(): number;
	getPositionAt(offset: number): monaco.Position;
	getOffsetAt(position: monaco.Position): number;
	getLineMaxColumn(lineNumber: number): number;
	getValueInRange(range: monaco.IRange): string;
	pushEditOperations(beforeSelections: unknown, edits: Array<{ range: monaco.IRange; text?: string }>, cursorStateComputer: unknown): void;
};

type EditorFacade = {
	getValue(): string;
	setValue(text: string): void;
	focus(): void;
	getModel(): ModelApi;
	getDomNode(): HTMLElement | null;
	changeViewZones(callback: (accessor: any) => void): any;
	onDidChangeModelContent(listener: () => void): { dispose(): void };
	setPosition(position: monaco.IPosition): void;
	revealRangeInCenter(range: monaco.IRange): void;
	setScrollPosition(position: { scrollTop?: number; scrollLeft?: number }): void;
	onMouseMove(listener: (event: any) => void): { dispose(): void };
	onMouseDown(listener: (event: any) => void): { dispose(): void };
	onMouseLeave(listener: () => void): { dispose(): void };
	setDiagnostics(diags: unknown[]): void;
	setCompletionProvider(provider: EditorOptions["provideCompletions"]): void;
	triggerSuggest(): void;
	setSyntaxHighlighter(provider: EditorOptions["provideSyntaxHighlights"]): void;
	destroy(): void;
};

export type { EditorFacade, EditorOptions, ModelApi, CompletionItemSource, Diagnostic, SyntaxSpan };

const FRESCO_OWNER = "fresco";
const FRESCO_LANGUAGE_ID = "fresco";
const FRESCO_THEME = "fresco-theme";

declare global {
	interface Window {
		MonacoEnvironment?: {
			getWorker(): Worker;
		};
	}
}

if (!self.MonacoEnvironment) {
	self.MonacoEnvironment = {
		getWorker() {
			return new editorWorker();
		}
	};
}

function ensureFrescoLanguage() {
	if (!monaco.languages.getLanguages().some((lang) => lang.id === FRESCO_LANGUAGE_ID)) {
		monaco.languages.register({ id: FRESCO_LANGUAGE_ID });
	}

	monaco.languages.setMonarchTokensProvider(FRESCO_LANGUAGE_ID, {
		tokenizer: {
			root: [[/.+/, "source"]]
		}
	});

	monaco.editor.defineTheme(FRESCO_THEME, {
		base: "vs-dark",
		inherit: true,
		rules: [],
		colors: {
			"editor.background": "#1e1e1e",
			"editor.foreground": "#d4d4d4",
			"editorLineNumber.foreground": "#858585",
			"editorLineNumber.activeForeground": "#d4d4d4",
			"editor.selectionBackground": "#264f78",
			"editorCursor.foreground": "#d4d4d4",
			"editorWidget.background": "#252526",
			"editorWidget.border": "#454545",
			"editorSuggestWidget.background": "#252526",
			"editorSuggestWidget.border": "#454545",
			"editorSuggestWidget.foreground": "#d4d4d4",
			"editorSuggestWidget.selectedBackground": "#04395e",
			"editorSuggestWidget.selectedForeground": "#ffffff",
			"editorSuggestWidget.highlightForeground": "#4299e1",
			"editorSuggestWidget.focusHighlightForeground": "#4299e1",
			"editorHoverWidget.background": "#252526",
			"editorHoverWidget.border": "#454545"
		}
	});

	monaco.editor.setTheme(FRESCO_THEME);
}

function mapCompletionKind(type: unknown) {
	const t = String(type || "").toLowerCase();
	if (t.includes("function") || t.includes("method")) return monaco.languages.CompletionItemKind.Function;
	if (t.includes("keyword")) return monaco.languages.CompletionItemKind.Keyword;
	if (t.includes("class")) return monaco.languages.CompletionItemKind.Class;
	if (t.includes("module")) return monaco.languages.CompletionItemKind.Module;
	if (t.includes("field") || t.includes("property")) return monaco.languages.CompletionItemKind.Field;
	if (t.includes("type")) return monaco.languages.CompletionItemKind.TypeParameter;
	if (t.includes("value") || t.includes("variable")) return monaco.languages.CompletionItemKind.Variable;
	return monaco.languages.CompletionItemKind.Text;
}

function mapMarkerSeverity(severity: unknown) {
	const s = String(severity || "error").toLowerCase();
	if (s === "warning") return monaco.MarkerSeverity.Warning;
	if (s === "info") return monaco.MarkerSeverity.Info;
	return monaco.MarkerSeverity.Error;
}

export function createCodeMirrorEditor(container: HTMLElement, options: EditorOptions = {}): EditorFacade {
	ensureFrescoLanguage();

	const {
		value = "",
		readOnly = false,
		fontSize = 14,
		onChange = null,
		provideCompletions = null
	} = options;

	let completionProvider = provideCompletions;
	let completionProviderDisposable: { dispose(): void } | null = null;
	let decorationIds: string[] = [];
	let syntaxHighlightProvider = options.provideSyntaxHighlights;
	const listeners = new Set<() => void>();

	const model = monaco.editor.createModel(String(value ?? ""), FRESCO_LANGUAGE_ID);
	const editor = monaco.editor.create(container, {
		model,
		theme: FRESCO_THEME,
		fixedOverflowWidgets: true,
		readOnly,
		lineNumbers: "on",
		minimap: { enabled: false },
		renderWhitespace: "selection",
		scrollBeyondLastLine: false,
		automaticLayout: true,
		quickSuggestions: {
			other: true,
			comments: false,
			strings: true
		},
		quickSuggestionsDelay: 80,
		suggestOnTriggerCharacters: true,
		acceptSuggestionOnEnter: "on",
		tabCompletion: "on",
		snippetSuggestions: "inline",
		suggest: {
			showStatusBar: true,
			preview: true,
			showInlineDetails: true,
			insertMode: "insert"
		},
		fontFamily: "Monaco, Consolas, 'Courier New', monospace",
		fontSize,
		tabSize: 4,
		insertSpaces: true,
		wordWrap: "off"
	});

	editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Space, () => {
		editor.trigger("keyboard", "editor.action.triggerSuggest", {});
	});

	editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyJ, () => {
		editor.trigger("keyboard", "editor.action.triggerSuggest", {});
	});

	editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Space, () => {
		editor.trigger("keyboard", "editor.action.triggerSuggest", {});
	});

	const modelApi: ModelApi = {
		getValue() {
			return model.getValue();
		},
		getValueLength() {
			return model.getValueLength();
		},
		getPositionAt(offset) {
			return model.getPositionAt(Math.max(0, Number(offset) || 0));
		},
		getOffsetAt(position) {
			return model.getOffsetAt(position);
		},
		getLineMaxColumn(lineNumber) {
			return model.getLineMaxColumn(Math.max(1, Number(lineNumber) || 1));
		},
		getValueInRange(range) {
			return model.getValueInRange(range);
		},
		pushEditOperations(_beforeSelections, edits) {
			editor.executeEdits(
				"fresco-scrub",
				(edits || []).map((edit) => ({
					range: new monaco.Range(
						edit.range.startLineNumber,
						edit.range.startColumn,
						edit.range.endLineNumber,
						edit.range.endColumn
					),
					text: edit.text ?? ""
				}))
			);
		}
	};

	const editorDomNode = editor.getDomNode();
	const onEditorKeyDown = (event: KeyboardEvent) => {
		if (!event) {
			return;
		}
		if ((event.ctrlKey || event.metaKey) && event.code === "Space") {
			event.preventDefault();
			event.stopPropagation();
			editor.focus();
			editor.trigger("keyboard", "editor.action.triggerSuggest", {});
		}
	};
	editorDomNode?.addEventListener("keydown", onEditorKeyDown, true);

	function applySyntaxHighlights() {
		if (typeof syntaxHighlightProvider !== "function") {
			decorationIds = editor.deltaDecorations(decorationIds, []);
			return;
		}

		const spans = syntaxHighlightProvider(modelApi, model.getValue()) || [];
		const nextDecorations: monaco.editor.IModelDeltaDecoration[] = [];
		for (const span of spans) {
			const from = Number(span?.from);
			const to = Number(span?.to);
			const className = String(span?.className || "").trim();
			if (!Number.isFinite(from) || !Number.isFinite(to) || to <= from || !className) {
				continue;
			}
			const beforeContent = typeof span?.beforeContent === "string" ? span.beforeContent : "";
			const beforeClassName = String(span?.beforeClassName || "").trim();
			const options: monaco.editor.IModelDecorationOptions = { inlineClassName: className };
			if (beforeContent && beforeClassName) {
				options.before = {
					content: beforeContent,
					inlineClassName: beforeClassName,
					cursorStops: monaco.editor.InjectedTextCursorStops.None
				};
			}
			nextDecorations.push({
				range: new monaco.Range(
					model.getPositionAt(from).lineNumber,
					model.getPositionAt(from).column,
					model.getPositionAt(to).lineNumber,
					model.getPositionAt(to).column
				),
				options
			});
		}

		decorationIds = editor.deltaDecorations(decorationIds, nextDecorations);
	}

	function installCompletionProvider() {
		completionProviderDisposable?.dispose();
		completionProviderDisposable = monaco.languages.registerCompletionItemProvider(FRESCO_LANGUAGE_ID, {
			triggerCharacters: [".", "(", ",", ":"],
			provideCompletionItems: async (monacoModel, position) => {
				if (typeof completionProvider !== "function") {
					return { suggestions: [] };
				}

				let items: CompletionItemSource[] = [];
				try {
					items = await completionProvider(modelApi, {
						lineNumber: position.lineNumber,
						column: position.column
					});
				} catch {
					return { suggestions: [] };
				}

				const typedWordInfo = monacoModel.getWordUntilPosition(position);
				const typedWord = monacoModel
					.getValueInRange(
						new monaco.Range(
							position.lineNumber,
							typedWordInfo.startColumn,
							position.lineNumber,
							typedWordInfo.endColumn
						)
					)
					.trim()
					.toLowerCase();

				const suggestions = (items || []).map((item) => {
					const wordInfo = monacoModel.getWordUntilPosition(position);
					const startColumn = Math.max(1, Number(wordInfo?.startColumn) || position.column);
					const endColumn = Math.max(startColumn, Number(wordInfo?.endColumn) || position.column);
					const range = new monaco.Range(position.lineNumber, startColumn, position.lineNumber, endColumn);
					const insertText = String(item.insertText ?? item.label ?? "");
					const snippet = typeof item.snippet === "string" && item.snippet.trim() ? item.snippet : null;
					const label = String(item.label || "");
					const loweredLabel = label.toLowerCase();
					const signature = typeof item.signature === "string" && item.signature.trim()
						? item.signature.trim()
						: "";
					const detail = typeof item.detail === "string" && item.detail.trim() ? item.detail.trim() : "";
					const docs = typeof item.documentation === "string" && item.documentation.trim()
						? item.documentation.trim()
						: typeof item.info === "string" && item.info.trim()
							? item.info.trim()
							: "";

					const detailText = [detail, signature].filter(Boolean).join(" | ");
					const documentation = docs
						? {
								value: signature && docs.startsWith(signature)
									? docs
									: signature
										? `**${signature}**\n\n${docs}`
										: docs
							}
						: signature
							? { value: `**${signature}**` }
							: undefined;

					let matchRank = 2;
					if (typedWord) {
						if (loweredLabel.startsWith(typedWord)) {
							matchRank = 0;
						} else if (loweredLabel.includes(typedWord)) {
							matchRank = 1;
						}
					}

					const boostSort = Number.isFinite(Number(item.boost))
						? String(100000 - Number(item.boost)).padStart(6, "0")
						: "999999";

					return {
						label,
						kind: mapCompletionKind(item.type),
						detail: detailText || undefined,
						documentation,
						sortText: `${matchRank}_${boostSort}_${loweredLabel}`,
						preselect: matchRank === 0,
						range,
						insertText: snippet || insertText,
                        command: (label.endsWith(":") || snippet?.includes("("))
                            ? { id: "editor.action.triggerSuggest", title: "Suggest argument values" }
                            : undefined,
						insertTextRules: snippet
							? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet
							: monaco.languages.CompletionItemInsertTextRule.None
					};
				});

				return { suggestions, incomplete: true };
			}
		});
	}

	installCompletionProvider();

    const autoSuggestDisposable = editor.onDidChangeModelContent((event) => {
        if (event.isFlush || event.isUndoing || event.isRedoing) return;
        const deleting = event.changes.some(change => change.text === "" && change.rangeLength > 0);
        if (!deleting && !event.changes.some(change => change.text.length <= 32 && /[\s(:,.]$/.test(change.text))) return;
        queueMicrotask(() => {
            if (!editor.hasTextFocus()) return;
            const position = editor.getPosition();
            if (!position) return;
            const before = model.getValue().slice(0, model.getOffsetAt(position));
            if (shouldAutoSuggest(before, deleting)) editor.trigger("typing", "editor.action.triggerSuggest", {});
        });
    });

	const changeDisposable = editor.onDidChangeModelContent(() => {
		if (typeof onChange === "function") {
			onChange();
		}
		for (const listener of listeners) {
			listener();
		}
	});

	return {
		getValue() {
			return model.getValue();
		},
		setValue(text) {
			model.setValue(String(text ?? ""));
		},
		focus() {
			editor.focus();
		},
		getModel() {
			return modelApi;
		},
		getDomNode() {
			return editor.getDomNode();
		},
		changeViewZones(callback) {
			return editor.changeViewZones(callback);
		},
		onDidChangeModelContent(listener) {
			listeners.add(listener);
			return {
				dispose() {
					listeners.delete(listener);
				}
			};
		},
		setPosition(position) {
			editor.setPosition(position);
		},
		revealRangeInCenter(range) {
			editor.revealRangeInCenter(range);
		},
		setScrollPosition({ scrollTop = 0, scrollLeft = 0 }) {
			editor.setScrollTop(Number(scrollTop) || 0);
			editor.setScrollLeft(Number(scrollLeft) || 0);
		},
		onMouseMove(listener) {
			const disposable = editor.onMouseMove((event) => listener(event));
			return { dispose: () => disposable.dispose() };
		},
		onMouseDown(listener) {
			const disposable = editor.onMouseDown((event) => listener(event));
			return { dispose: () => disposable.dispose() };
		},
		onMouseLeave(listener) {
			const disposable = editor.onMouseLeave(listener);
			return { dispose: () => disposable.dispose() };
		},
		setDiagnostics(diags) {
			const markers = (diags || []).map((diag) => {
				const d = (diag && typeof diag === "object") ? (diag as Record<string, unknown>) : {};
				return {
					severity: mapMarkerSeverity(d.severity),
					startLineNumber: model.getPositionAt(Math.max(0, Number(d.from) || 0)).lineNumber,
					startColumn: model.getPositionAt(Math.max(0, Number(d.from) || 0)).column,
					endLineNumber: model.getPositionAt(Math.max(0, Number(d.to) || 0)).lineNumber,
					endColumn: model.getPositionAt(Math.max(0, Number(d.to) || 0)).column,
					message: String(d.message || "")
				};
			});
			monaco.editor.setModelMarkers(model, FRESCO_OWNER, markers);
		},
		setCompletionProvider(provider) {
			completionProvider = provider;
			installCompletionProvider();
		},
		triggerSuggest() {
			editor.focus();
			editor.trigger("manual", "editor.action.triggerSuggest", {});
		},
		setSyntaxHighlighter(provider) {
			syntaxHighlightProvider = provider;
			applySyntaxHighlights();
		},
		destroy() {
			editorDomNode?.removeEventListener("keydown", onEditorKeyDown, true);
			changeDisposable.dispose();
            autoSuggestDisposable.dispose();
			completionProviderDisposable?.dispose();
			monaco.editor.setModelMarkers(model, FRESCO_OWNER, []);
			decorationIds = editor.deltaDecorations(decorationIds, []);
			editor.dispose();
			model.dispose();
		}
	};
}
