/** The Remote session-limit refusal, recognised without registration or key access, so the public
 * entry and the app tell the same limit apart from an unpaired browser. */
/** A fresh open refused because the paired device is at its Remote session limit (#2585). Remote
 * never evicts an attached session, so nothing changes until a session frees. */
export class SessionLimitError extends Error {
  constructor(
    readonly limit: number,
    readonly settingsUrl?: string,
  ) {
    super('REMOTE_SESSION_LIMIT');
  }
}
/** The settings link is shown as an anchor, so only a web URL is kept. */
function webUrl(value: unknown): string | undefined {
  if (typeof value !== 'string' || !URL.canParse(value)) return undefined;
  const { protocol } = new URL(value);
  return protocol === 'https:' || protocol === 'http:' ? value : undefined;
}
/**
 * The verified limit refusal behind a failed open, or undefined. The boot open throws the SDK's
 * `RefusalError` itself; a bounded re-admission wraps it as the `cause` of its `capacity`
 * `ReopenSessionError`. Both classes arrive at run time, so they are recognised by name, and a
 * limit that is not a positive integer is not trusted.
 */
export function sessionLimit(error: unknown): SessionLimitError | undefined {
  for (let depth = 0, next = error; depth < 3 && next instanceof Error; depth++) {
    if (next.name === 'RefusalError') {
      const { code, limit, settingsUrl } = next as Error & Record<string, unknown>;
      return code === 'REMOTE_SESSION_LIMIT' && Number.isSafeInteger(limit) && (limit as number) > 0
        ? new SessionLimitError(limit as number, webUrl(settingsUrl))
        : undefined;
    }
    next = next.cause;
  }
  return undefined;
}
