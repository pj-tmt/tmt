/**
 * The owner browser reads (contracts/remote-channel-v1.md, Owner browser reads): read-only door GETs
 * under the owner session, siblings of the composed shell's `x/-/shell`. One client for every page
 * that shows them (Ops, Digest), and the refresh timer that runs only while the route is visible.
 */
import { shellAddress } from '../../../../../design/browser-ui/src/shell.js';

export type OwnerReadRange = '24h' | '7d' | '30d';
export type OwnerReadName = 'ops' | 'limits' | `digest/${OwnerReadRange}`;

/** The reasons the contract names; a client reads any other reason as `unknown`. */
export const OWNER_READ_REASONS = [
  'not_installed',
  'core_unavailable',
  'timeout',
  'read_failed',
  'too_large',
  'not_sampled',
] as const;
export type OwnerReadReason = (typeof OWNER_READ_REASONS)[number] | 'unknown';

/**
 * A read's outcome. `ok` carries the route's payload (the answer without `version`, `state` and
 * `observedAtMs`); `unavailable` is the owner's own answer with its reason; `unreachable` is no
 * answer at all: no owner session in this browser, a door without the reads, or no network.
 */
export type OwnerReadAnswer<T> =
  | { state: 'ok'; observedAtMs: number; payload: T }
  | { state: 'unavailable'; observedAtMs: number; reason: OwnerReadReason }
  | { state: 'unreachable' };

const UNREACHABLE = { state: 'unreachable' } as const;

/** One answer document checked against the contract's envelope; anything else is unreachable. */
export function ownerAnswer<T>(document: unknown): OwnerReadAnswer<T> {
  if (typeof document !== 'object' || document === null || Array.isArray(document))
    return UNREACHABLE;
  const { version, state, observedAtMs, reason, ...payload } = document as Record<string, unknown>;
  if (version !== 1 || !Number.isSafeInteger(observedAtMs) || (observedAtMs as number) <= 0)
    return UNREACHABLE;
  const at = observedAtMs as number;
  if (state === 'ok') return { state, observedAtMs: at, payload: payload as T };
  if (state !== 'unavailable') return UNREACHABLE;
  const known = OWNER_READ_REASONS.find((name) => name === reason);
  return { state, observedAtMs: at, reason: known ?? 'unknown' };
}

let address: Promise<string | undefined> | undefined;

/** Read `<address>/x/-/<name>` once. The mount address is found once per page and found again
 * after a read that had none. */
export async function ownerRead<T>(
  name: OwnerReadName,
  signal: AbortSignal,
): Promise<OwnerReadAnswer<T>> {
  address ??= shellAddress();
  const base = await address;
  if (base === undefined) {
    address = undefined;
    return UNREACHABLE;
  }
  try {
    const response = await fetch(`${base}/x/-/${name}`, {
      credentials: 'same-origin',
      cache: 'no-store',
      signal,
    });
    if (response.status !== 200) return UNREACHABLE;
    return ownerAnswer<T>(await response.json());
  } catch {
    return UNREACHABLE;
  }
}

/** The longest wait after failures: the cadence doubles per failed read up to this many times. */
const BACKOFF_STEPS = 3;

/**
 * Run `tick` now and every `intervalMs` while the page is visible, until `signal` aborts (the route
 * unmounts). A hidden page reads nothing; showing it again reads at once. A tick that returns false
 * (a failed read) doubles the wait, up to eight times the cadence, and the next success resets it.
 */
export function whileVisible(
  intervalMs: number,
  signal: AbortSignal,
  tick: () => Promise<boolean>,
): void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let failures = 0;
  let running = false;
  const schedule = () => {
    clearTimeout(timer);
    if (signal.aborted || document.visibilityState !== 'visible') return;
    timer = setTimeout(run, intervalMs * 2 ** Math.min(failures, BACKOFF_STEPS));
  };
  const run = async () => {
    clearTimeout(timer);
    if (running || signal.aborted || document.visibilityState !== 'visible') return;
    running = true;
    try {
      failures = (await tick()) ? 0 : failures + 1;
    } catch {
      failures += 1;
    } finally {
      running = false;
    }
    schedule();
  };
  document.addEventListener(
    'visibilitychange',
    () => (document.visibilityState === 'visible' ? void run() : clearTimeout(timer)),
    { signal },
  );
  signal.addEventListener('abort', () => clearTimeout(timer), { once: true });
  void run();
}
