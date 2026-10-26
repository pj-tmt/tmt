import { afterEach, beforeEach, expect, it, vi } from 'vite-plus/test';

type Limit = { kind: string; percentUsed: number; resetsAt?: string };
type Event = { changed?: string[]; rateLimits?: Limit[] };
type Call = { argv: readonly string[]; stdin?: string; timeoutMs?: number };
type Kit = {
  process: { run: (argv: readonly string[], init?: Omit<Call, 'argv'>) => Promise<unknown> };
  clock: { now: () => Promise<number> };
  session: { usage: () => Promise<{ rateLimits: Limit[] }> };
};
type Handler = (kit: Kit, event: Event, next: (event: Event) => Promise<string>) => Promise<string>;

const FIVE: Limit = { kind: 'five_hour', percentUsed: 21, resetsAt: '2026-10-26T01:00:00.000Z' };
const WEEK: Limit = { kind: 'seven_day', percentUsed: 64, resetsAt: '2026-10-29T10:00:00.000Z' };
const SPEND: Limit = { kind: 'spend_limit', percentUsed: 3 };

let handlers: Map<string, Handler>;
let calls: Call[];
let now: number;
let usage: Limit[];
let runFails: boolean;

const kit: Kit = {
  process: {
    run: async (argv, init) => {
      calls.push({ argv, ...init });
      if (runFails) throw new Error('tmt is not on PATH');
      return { exitCode: 0 };
    },
  },
  clock: { now: async () => now },
  session: { usage: async () => ({ rateLimits: usage }) },
};
const next = async () => 'next';

beforeEach(async () => {
  vi.resetModules();
  handlers = new Map();
  calls = [];
  now = 1_000_000;
  usage = [FIVE, WEEK, SPEND];
  runFails = false;
  const mod = await import(
    new URL('../../../drivers/claude/plugins/tmt-allowance/hooks/register.mjs', import.meta.url)
      .href
  );
  mod.register((event: string, handler: Handler) => handlers.set(event, handler));
});
afterEach(() => vi.restoreAllMocks());

const handler = (event: string): Handler => handlers.get(event) as Handler;

it('hands only the two named windows to the bounded tmt intake at session start', async () => {
  await expect(handler('session.start')(kit, {}, next)).resolves.toBe('next');
  expect(calls).toHaveLength(1);
  expect(calls[0].argv).toEqual(['tmt', '__allowance', 'claude']);
  expect(calls[0].timeoutMs).toBeLessThanOrEqual(2_000);
  expect(JSON.parse(calls[0].stdin as string)).toEqual({ rateLimits: [FIVE, WEEK] });
});

it('pushes nothing when Claude reports no window tmt reads', async () => {
  usage = [SPEND];
  await handler('session.start')(kit, {}, next);
  await handler('session.measure')(kit, { changed: ['rateLimits'], rateLimits: [] }, next);
  expect(calls).toEqual([]);
});

it('pushes a measure whose windows moved and skips a steady one until the push ages', async () => {
  const measure = handler('session.measure');
  await expect(measure(kit, { changed: ['rateLimits'], rateLimits: [FIVE] }, next)).resolves.toBe(
    'next'
  );
  expect(calls).toHaveLength(1);
  expect(JSON.parse(calls[0].stdin as string)).toEqual({ rateLimits: [FIVE] });

  now += 60_000;
  await measure(kit, { changed: ['tokens'], rateLimits: [FIVE] }, next);
  expect(calls).toHaveLength(1);

  now += 4 * 60_000;
  await measure(kit, { changed: ['tokens'], rateLimits: [FIVE, WEEK] }, next);
  expect(calls).toHaveLength(2);
  expect(JSON.parse(calls[1].stdin as string)).toEqual({ rateLimits: [FIVE, WEEK] });
});

it('never disturbs Claude when the push fails', async () => {
  runFails = true;
  await expect(handler('session.start')(kit, {}, next)).resolves.toBe('next');
  await expect(
    handler('session.measure')(kit, { changed: ['rateLimits'], rateLimits: [FIVE] }, next)
  ).resolves.toBe('next');
  expect(calls).toHaveLength(2);
});
