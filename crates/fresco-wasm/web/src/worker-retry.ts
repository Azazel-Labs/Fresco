const RETRYABLE_WORKER_ERROR_PATTERNS = [
  /worker\s+(?:crashed|message decode failed|timed out)/i,
  /failed to fetch dynamically imported module/i,
  /error loading dynamically imported module/i,
  /importing a module script failed/i,
  /networkerror/i,
  /load failed/i,
  /script error/i,
  /module script/i
];

export const TRANSIENT_WORKER_RETRY_DELAY_MS = 120;

export function isRetryableWorkerFailure(error: unknown): boolean {
  const message = String((error as { message?: string } | undefined)?.message || error || "").trim();
  if (!message) {
    return false;
  }
  return RETRYABLE_WORKER_ERROR_PATTERNS.some((pattern) => pattern.test(message));
}
