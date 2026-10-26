// Hands the rate-limit windows Claude Code measures to tmt (#5187). It reads nothing, writes no
// file and changes no setting: one bounded `tmt __allowance claude` call per push, any failure
// swallowed. tmt decodes strictly, stamps the receive time and keeps the newest observation.

/** @typedef {{kind: string, percentUsed: number, resetsAt?: string}} Limit */
/** @typedef {{process: {run: (argv: readonly string[], init?: {stdin?: string, timeoutMs?: number}) => Promise<unknown>}, clock: {now: () => Promise<number>}, session: {usage: () => Promise<{rateLimits: readonly Limit[]}>}}} Kit */

const PUSH_AFTER_MS = 5 * 60_000;
const BUDGET_MS = 1_500;
const KINDS = new Set(['five_hour', 'seven_day']);

let lastPushMs = -Infinity;

/**
 * @param {Kit} $
 * @param {readonly Limit[] | undefined} rateLimits
 */
async function push($, rateLimits) {
  const windows = (rateLimits ?? []).filter((limit) => KINDS.has(limit?.kind));
  if (windows.length === 0) return;
  lastPushMs = await $.clock.now();
  await $.process.run(['tmt', '__allowance', 'claude'], {
    stdin: JSON.stringify({ rateLimits: windows }),
    timeoutMs: BUDGET_MS,
  });
}

/** @param {(event: string, handler: (...args: any[]) => Promise<unknown>) => void} on */
export const register = (on) => {
  on('session.start', async ($, e, next) => {
    const result = await next(e);
    try {
      await push($, (await $.session.usage()).rateLimits);
    } catch {}
    return result;
  });
  on('session.measure', async ($, e, next) => {
    try {
      // A window moved, or a steady window is old enough that tmt would age it out.
      if (e.changed?.includes('rateLimits') || (await $.clock.now()) - lastPushMs >= PUSH_AFTER_MS) {
        await push($, e.rateLimits);
      }
    } catch {}
    return next(e);
  });
};
