/** Bounded admission only. The caller owns connection state and original effect IDs. */
export type ReopenSessionReason =
  | 'unreachable'
  | 'unconfirmed'
  | 'transient'
  | 'mismatch'
  | 'unpaired'
  | 'revoked'
  | 'expired';
export type ReopenSessionDetail =
  | 'budget-exhausted'
  | 'cancelled'
  | 'pairing-unavailable'
  | 'identity-mismatch'
  | 'unverifiable-response'
  | 'admission-unconfirmed';
export interface ReopenSessionOptions {
  retry?: 'bounded';
  /** This is the connection owner's signal; abort cancels its joined admission series. */
  signal?: AbortSignal;
}
export class ReopenSessionError extends Error {
  constructor(
    readonly reason: ReopenSessionReason,
    readonly detail: ReopenSessionDetail,
  ) {
    super(`Remote session admission: ${reason} (${detail}).`);
    this.name = 'ReopenSessionError';
  }
}
const DEADLINE = 20_000;
const ATTEMPT = 4_000;
const DELAYS = [250, 500, 1000] as const;
/** Test clock/wait/jitter ports stay internal; browser callers cannot change the limits. */
export interface AdmissionRuntime {
  now(): number;
  later(callback: () => void, ms: number): ReturnType<typeof setTimeout>;
  clear(timer: ReturnType<typeof setTimeout>): void;
  jitter(): number;
}
const runtime: AdmissionRuntime = {
  now: () => performance.now(),
  later: (callback, ms) => setTimeout(callback, ms),
  clear: (timer) => clearTimeout(timer),
  jitter: () => crypto.getRandomValues(new Uint32Array(1))[0]! / 0x1_0000_0000,
};
export function active(signal?: AbortSignal): void {
  if (signal?.aborted) throw new ReopenSessionError('transient', 'cancelled');
}
/** Bounds even a provider that ignores AbortSignal; abandoned work cannot publish a Session. */
function bounded<T>(
  action: (signal: AbortSignal) => Promise<T>,
  ms: number,
  owner: AbortSignal | undefined,
  clock: AdmissionRuntime,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const child = new AbortController();
    const end = clock.now() + ms;
    let settled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const finish = (value?: T, error?: unknown): void => {
      if (settled) return;
      settled = true;
      if (timer !== undefined) clock.clear(timer);
      owner?.removeEventListener('abort', cancel);
      child.abort();
      if (error !== undefined) reject(error);
      else if (clock.now() >= end)
        reject(new ReopenSessionError('unreachable', 'budget-exhausted'));
      else resolve(value!);
    };
    const cancel = (): void => finish(undefined, new ReopenSessionError('transient', 'cancelled'));
    if (owner?.aborted) return cancel();
    owner?.addEventListener('abort', cancel, { once: true });
    timer = clock.later(
      () => finish(undefined, new ReopenSessionError('unreachable', 'budget-exhausted')),
      ms,
    );
    void Promise.resolve()
      .then(() => action(child.signal))
      .then(
        (value) => finish(value),
        (error: unknown) => finish(undefined, error),
      );
  });
}
export async function boundedAdmission<T>(
  attempt: (signal: AbortSignal) => Promise<T>,
  signal?: AbortSignal,
  clock: AdmissionRuntime = runtime,
): Promise<T> {
  const end = clock.now() + DEADLINE;
  let last = new ReopenSessionError('unreachable', 'budget-exhausted');
  let ambiguous = false;
  for (let index = 0; index < 4; index++) {
    active(signal);
    const remaining = end - clock.now();
    if (remaining <= 0) break;
    try {
      const value = await bounded(attempt, Math.min(ATTEMPT, remaining), signal, clock);
      active(signal);
      return value;
    } catch (error) {
      if (!(error instanceof ReopenSessionError))
        throw new ReopenSessionError('transient', 'admission-unconfirmed');
      if (
        ['mismatch', 'unpaired', 'revoked', 'expired'].includes(error.reason) ||
        ['cancelled', 'unverifiable-response'].includes(error.detail)
      )
        throw error;
      last = error;
      ambiguous ||= error.reason === 'unconfirmed';
    }
    const delay = DELAYS[index];
    if (delay === undefined) break;
    const wait = Math.min(Math.floor(delay + delay * 0.25 * clock.jitter()), end - clock.now());
    if (wait <= 0) break;
    try {
      await bounded(
        (waiting) =>
          new Promise<void>((resolve) => {
            const timer = clock.later(() => {
              waiting.removeEventListener('abort', cancel);
              resolve();
            }, wait);
            const cancel = (): void => clock.clear(timer);
            waiting.addEventListener('abort', cancel, { once: true });
          }),
        end - clock.now(),
        signal,
        clock,
      );
    } catch (error) {
      active(signal);
      if (!(error instanceof ReopenSessionError) || error.detail === 'cancelled') throw error;
      break;
    }
  }
  active(signal);
  throw new ReopenSessionError(ambiguous ? 'unconfirmed' : last.reason, 'budget-exhausted');
}
