import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import { OWNER_READ_REASONS, ownerAnswer, ownerRead, whileVisible } from '../src/owner-reads.js';

const reopenSession = vi.fn<() => Promise<void>>();
vi.mock('../src/browser.js', () => ({ reopenSession: () => reopenSession() }));

interface Vectors {
  reasons: string[];
  cases: { name: string; target: string; expect: Record<string, unknown> }[];
}
const vectors = JSON.parse(
  readFileSync(new URL('../../../../../contracts/owner-reads-v1.json', import.meta.url), 'utf8'),
) as Vectors;

it('names exactly the contract reasons', () => {
  expect([...OWNER_READ_REASONS]).toEqual(vectors.reasons);
});

it('reads every vector answer: ok carries the payload, unavailable its reason', () => {
  for (const vector of vectors.cases) {
    // The vectors leave out observedAtMs, a positive integer in every answer.
    const { version, state, reason, ...payload } = vector.expect;
    const answer = ownerAnswer({ ...vector.expect, observedAtMs: 1792635300000 });
    expect(version, vector.name).toBe(1);
    expect(answer, vector.name).toEqual(
      state === 'ok'
        ? { state, observedAtMs: 1792635300000, payload }
        : { state, observedAtMs: 1792635300000, reason },
    );
  }
});

it('takes an unknown reason as unknown and anything off the envelope as unreachable', () => {
  const at = { version: 1, observedAtMs: 5 };
  expect(ownerAnswer({ ...at, state: 'unavailable', reason: 'later' })).toEqual({
    state: 'unavailable',
    observedAtMs: 5,
    reason: 'unknown',
  });
  for (const document of [
    null,
    [],
    'ok',
    { ...at, version: 2, state: 'ok' },
    { ...at, observedAtMs: 0, state: 'ok' },
    { ...at, observedAtMs: 1.5, state: 'ok' },
    { version: 1, state: 'ok' },
    { ...at, state: 'pending' },
  ])
    expect(ownerAnswer(document)).toEqual({ state: 'unreachable' });
});

class FakeDocument extends EventTarget {
  visibilityState: 'visible' | 'hidden' = 'visible';
  show(visible: boolean) {
    this.visibilityState = visible ? 'visible' : 'hidden';
    this.dispatchEvent(new Event('visibilitychange'));
  }
}
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('refreshes only while visible, backs off after failures and stops with the route', async () => {
  vi.useFakeTimers();
  const page = new FakeDocument();
  vi.stubGlobal('document', page);
  const route = new AbortController();
  const outcomes = [true, false, false, true, true];
  const at: number[] = [];
  whileVisible(1000, route.signal, async () => {
    at.push(Date.now());
    return outcomes.shift() ?? true;
  });
  const start = Date.now();
  await vi.advanceTimersByTimeAsync(0);
  // Now, then 1 s; two failures double the wait twice; a success resets it.
  await vi.advanceTimersByTimeAsync(1000 + 2000 + 4000 + 1000);
  expect(at.map((time) => time - start)).toEqual([0, 1000, 3000, 7000, 8000]);
  page.show(false);
  await vi.advanceTimersByTimeAsync(10_000);
  expect(at).toHaveLength(5);
  // Showing the page again reads at once.
  page.show(true);
  await vi.advanceTimersByTimeAsync(0);
  expect(at).toHaveLength(6);
  route.abort();
  page.show(false);
  page.show(true);
  await vi.advanceTimersByTimeAsync(10_000);
  expect(at).toHaveLength(6);
});

it('asks to pair only without an owner mount; a mount that fails to answer is unreachable', async () => {
  const signal = new AbortController().signal;
  let mount = 404;
  let read: () => Promise<Response> = async () =>
    Response.json({ version: 1, state: 'ok', observedAtMs: 1 });
  vi.stubGlobal('location', new URL('https://door.test/ops'));
  vi.stubGlobal('fetch', async (input: string) =>
    input === '/sdk/mount'
      ? mount === 200
        ? Response.json({ address: 'https://door.test/r/owner' })
        : new Response('', { status: mount })
      : read(),
  );
  expect(await ownerRead('ops', signal)).toEqual({ state: 'no-session' });
  mount = 200;
  expect(await ownerRead('ops', signal)).toEqual({ state: 'ok', observedAtMs: 1, payload: {} });
  // The door stopped, restarted or answered off the contract: unreachable, never no-session.
  for (const failure of [
    async () => {
      throw new TypeError('Failed to fetch');
    },
    async () => new Response('', { status: 502 }),
    async () => new Response('not json', { status: 200 }),
  ]) {
    read = failure;
    expect(await ownerRead('ops', signal)).toEqual({ state: 'unreachable' });
  }
});

it('a 404 reopens the session from the pairing before it names a missing session', async () => {
  const signal = new AbortController().signal;
  let open = false;
  vi.stubGlobal('location', new URL('https://door.test/ops'));
  vi.stubGlobal('fetch', async (input: string) =>
    input === '/sdk/mount'
      ? Response.json({ address: 'https://door.test/r/owner' })
      : open
        ? Response.json({ version: 1, state: 'ok', observedAtMs: 1 })
        : new Response('', { status: 404 }),
  );
  // A door restart ended the session; the pairing opens a new one and the read is made again.
  reopenSession.mockImplementationOnce(async () => {
    open = true;
  });
  expect(await ownerRead('ops', signal)).toEqual({ state: 'ok', observedAtMs: 1, payload: {} });
  open = false;
  reopenSession.mockRejectedValueOnce(Object.assign(new Error('unpaired'), { reason: 'unpaired' }));
  expect(await ownerRead('ops', signal)).toEqual({ state: 'no-session' });
  reopenSession.mockRejectedValueOnce(
    Object.assign(new Error('unreachable'), { reason: 'unreachable' }),
  );
  expect(await ownerRead('ops', signal)).toEqual({ state: 'unreachable' });
});
