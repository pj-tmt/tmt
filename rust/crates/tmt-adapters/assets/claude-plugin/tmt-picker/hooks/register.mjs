// Typing `tmt/<query>` (or `tmt:<query>`) offers tmt identities, fuzzy-matched.
// Keystrokes never spawn a process: they read a cached target list that a
// background refresh keeps current. The list comes from one source function so
// remote hosts can be added later (e.g. `tmt ls --json` returning host-qualified
// names) without touching matching or rendering.

/** @typedef {{name: string, presence: string, driver: string | null, activity: string}} Target */

const TRIGGER = /^tmt[/:]/;
const REFRESH_MS = 15_000;
const MAX_ROWS = 8;

/** @type {readonly Target[]} */
let targets = [];
let fetchedAt = 0;
/** @type {Promise<void> | undefined} */
let refreshing;

/** @typedef {(argv: readonly string[], init?: {timeoutMs?: number}) => Promise<{exitCode: number, stdout: string}>} Run */

/** @param {Run} run */
async function listTargets(run) {
  const { exitCode, stdout } = await run(['tmt', 'ls', '--json'], { timeoutMs: 5_000 });
  if (exitCode !== 0) throw new Error('Identity listing unavailable');
  const parsed = JSON.parse(stdout);
  if (!Array.isArray(parsed.identities)) throw new Error('Invalid identity listing');
  return parsed.identities
    .filter((i) => i && typeof i.name === 'string' && typeof i.presence === 'string')
    .map((i) => ({
      name: i.name,
      presence: i.presence,
      driver: typeof i.runningDriver === 'string' ? i.runningDriver : null,
      activity:
        typeof i.session?.activity?.state === 'string' ? i.session.activity.state : 'unknown',
    }));
}

/** @param {Run} run */
function refresh(run) {
  if (refreshing) return refreshing;
  fetchedAt = Date.now(); // Failed refreshes also retain the 15-second backoff.
  refreshing = listTargets(run)
    .then((list) => {
      targets = list;
      fetchedAt = Date.now();
    })
    .catch(() => {})
    .finally(() => {
      refreshing = undefined;
    });
  return refreshing;
}

// fzf-like subsequence score: higher is better, undefined when not a match.
/** @param {string} query @param {string} name */
export function score(query, name) {
  const q = query.toLowerCase();
  const n = name.toLowerCase();
  if (q === '') return 0;
  let total = 0;
  let at = 0;
  let previous = -2;
  for (const ch of q) {
    const found = n.indexOf(ch, at);
    if (found < 0) return undefined;
    const boundary = found === 0 || '-_./: '.includes(n[found - 1]);
    total += 1 + (found === previous + 1 ? 3 : 0) + (boundary ? 2 : 0);
    previous = found;
    at = found + 1;
  }
  if (n.startsWith(q)) total += 5;
  return total - n.length * 0.01;
}

/** @param {string} query @param {readonly Target[]} list */
export function rank(query, list) {
  return list
    .map((target) => ({ target, s: score(query, target.name) }))
    .filter((row) => row.s !== undefined)
    .sort(
      (a, b) =>
        Number(b.target.presence === 'active') - Number(a.target.presence === 'active') ||
        b.s - a.s ||
        a.target.name.localeCompare(b.target.name)
    )
    .map((row) => row.target);
}

/** @param {string} prefix @param {Target} target */
function row(prefix, target) {
  const state = target.presence === 'active' ? target.activity : target.presence;
  return {
    text: `${prefix}${target.name} `,
    label: `${prefix}${target.name}`,
    description: [target.driver, state].filter(Boolean).join(' · '),
  };
}

/** @type {import('claude-code').Register} */
export const register = (on) => {
  on('session.start', async ($, e, next) => {
    const started = await next(e);
    void refresh((argv, init) => $.process.run(argv, init));
    return started;
  });

  on('prompt.autocomplete', { token: TRIGGER }, async ($, e, next) => {
    const base = await next(e);
    if (Date.now() - fetchedAt > REFRESH_MS)
      void refresh((argv, init) => $.process.run(argv, init));
    const prefix = e.token.slice(0, 4);
    const query = e.token.slice(4);
    const rows = rank(query, targets)
      .slice(0, MAX_ROWS)
      .map((t) => row(prefix, t));
    return { suggestions: [...rows, ...base.suggestions] };
  });
};
