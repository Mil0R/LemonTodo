type DebugDetails = Record<string, unknown>;

const startedAt = Date.now();

export function debugLog(scope: string, event: string, details?: DebugDetails): void {
  if (!__DEV__) {
    return;
  }
  const elapsed = String(Date.now() - startedAt).padStart(6, "0");
  if (details) {
    console.debug(`[LemonTodo +${elapsed}ms] [${scope}] ${event}`, details);
  } else {
    console.debug(`[LemonTodo +${elapsed}ms] [${scope}] ${event}`);
  }
}

export function debugError(scope: string, event: string, error: unknown, details?: DebugDetails): void {
  if (!__DEV__) {
    return;
  }
  console.error(`[LemonTodo] [${scope}] ${event}`, {
    ...details,
    error: error instanceof Error ? error.message : String(error),
    errorName: error instanceof Error ? error.name : typeof error,
  });
}

export function debugEmail(value: string): string {
  const normalized = value.trim().toLowerCase();
  const separator = normalized.indexOf("@");
  if (separator <= 0) {
    return normalized ? `${normalized.slice(0, 1)}***` : "<empty>";
  }
  return `${normalized.slice(0, 1)}***${normalized.slice(separator)}`;
}

export function debugServerUrl(value: string): string {
  try {
    const url = new URL(value);
    return `${url.protocol}//${url.host}`;
  } catch {
    return "<invalid-url>";
  }
}
