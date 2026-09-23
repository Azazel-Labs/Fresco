export type VirtualFilesMap = Map<string, string>;

export interface ExampleEntry {
  id: string;
  source: string;
}

export interface SourceModeOptions {
  baseline?: string;
  exampleId?: string;
}

export type CompileQueueFiles = VirtualFilesMap | null;

export interface CompileQueueRequest {
  delayMs?: number;
  cancelInFlight?: boolean;
  deferPreviewBuild?: boolean;
}

export interface CompileQueuePayload {
  source: string;
  files: CompileQueueFiles;
  cancelInFlight: boolean;
  deferPreviewBuild: boolean;
  queuedAtMs: number;
  debounceDelayMs: number;
  debounceTimerLagMs: number;
}
