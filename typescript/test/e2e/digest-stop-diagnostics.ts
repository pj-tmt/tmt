/** Bounded, content-free evidence for the two Digest Stop scenarios, not delivery authority. */
export function digestStopDiagnostics(
  phase: {
    provider: 'claude' | 'codex';
    channel: boolean;
    setup: boolean;
    resume: boolean;
    legacy: boolean;
    step: string;
  },
  learning: readonly unknown[],
  events: readonly Record<string, unknown>[]
): Record<string, unknown> {
  const short = (value: unknown): string | undefined =>
    typeof value === 'string' ? value.slice(0, 96) : undefined;
  const rows = learning.slice(-8).map((value) => {
    const row = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
    return {
      status: short(row.status),
      step: short(row.step),
      reason: short(row.reason),
      detail: short(row.detail),
    };
  });
  const handlers = events
    .filter((event) => ['digest-handler', 'hook-outcome'].includes(String(event.event)))
    .slice(-8)
    .map((event) => ({
      event: short(event.event),
      hookEvent: short(event.hookEvent),
      step: short(event.step),
      ok: typeof event.ok === 'boolean' ? event.ok : undefined,
      exitCode: Number.isSafeInteger(event.exitCode) ? event.exitCode : undefined,
      signal:
        typeof event.signal === 'string'
          ? short(event.signal)
          : Number.isSafeInteger(event.signal)
            ? event.signal
            : undefined,
      stdoutBytes:
        typeof event.stdoutBytes === 'number'
          ? event.stdoutBytes
          : typeof event.stdout === 'string'
            ? Buffer.byteLength(event.stdout)
            : undefined,
      stderrBytes:
        typeof event.stderrBytes === 'number'
          ? event.stderrBytes
          : typeof event.stderr === 'string'
            ? Buffer.byteLength(event.stderr)
            : undefined,
    }));
  return {
    phase: {
      provider: phase.provider,
      channel: phase.channel,
      setup: phase.setup,
      resume: phase.resume,
      legacy: phase.legacy,
      step: phase.step.slice(0, 96),
    },
    learning: rows,
    handlers,
  };
}
