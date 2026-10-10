import { afterEach, beforeEach, expect, it, vi } from 'vite-plus/test';

type Target = { name: string; presence: string; driver: string | null; activity: string };
type Suggestion = { text: string; label: string; description?: string };
type Run = (
  argv: readonly string[],
  init?: { timeoutMs?: number }
) => Promise<{ exitCode: number; stdout: string }>;
type Event = { token: string };
type Reply = { suggestions: Suggestion[] };
type Handler = (
  engine: { process: { run: Run } },
  event: Event,
  next: (event: Event) => Promise<Reply>
) => Promise<Reply>;
type Picker = {
  score: (query: string, name: string) => number | undefined;
  rank: (query: string, targets: readonly Target[]) => Target[];
  register: (
    on: (event: string, options: Handler | { token: RegExp }, handler?: Handler) => void
  ) => void;
};

let picker: Picker;
beforeEach(async () => {
  vi.resetModules();
  picker = await import(
    new URL(
      '../../../rust/crates/tmt-adapters/assets/claude-plugin/tmt-picker/hooks/register.mjs',
      import.meta.url
    ).href
  );
});
afterEach(() => vi.restoreAllMocks());

const target = (name: string, presence = 'active'): Target => ({
  name,
  presence,
  driver: 'claude',
  activity: 'idle',
});

it('preserves the prototype subsequence scoring and active-first ranking', () => {
  expect(picker.score('c1', 'core-1')).toBeDefined();
  expect(picker.score('cr1', 'core-1')).toBeDefined();
  expect(picker.score('x', 'core-1')).toBeUndefined();
  const list = [
    target('tmt-core-pm'),
    target('core-1'),
    target('colab-1'),
    target('core-2', 'offline'),
  ];
  expect(
    picker
      .rank('co1', list)
      .slice(0, 2)
      .map((row) => row.name)
  ).toEqual(['core-1', 'colab-1']);
  expect(picker.rank('core', list).at(-1)?.name).toBe('core-2');
});

function hooks(run: Run) {
  const handlers = new Map<string, Handler>();
  picker.register((event, options, handler) =>
    handlers.set(event, handler ?? (options as Handler))
  );
  const engine = { process: { run } };
  const next = async () => ({ suggestions: [] });
  return {
    start: () => handlers.get('session.start')!(engine, { token: '' }, next),
    complete: (token: string) => handlers.get('prompt.autocomplete')!(engine, { token }, next),
  };
}

it('serves cached rows while one background refresh is pending, retaining host-qualified names', async () => {
  let finish: (reply: { exitCode: number; stdout: string }) => void = () => {};
  const run = vi.fn<Run>(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  const session = hooks(run);
  await session.start();
  expect(run).toHaveBeenCalledExactlyOnceWith(['tmt', 'ls', '--json'], { timeoutMs: 5_000 });
  expect((await session.complete('tmt/co1')).suggestions).toEqual([]);
  expect(run).toHaveBeenCalledTimes(1);
  finish({
    exitCode: 0,
    stdout: JSON.stringify({
      identities: Array.from({ length: 10 }, (_, i) => ({
        name: `remote/core-${i}`,
        presence: 'active',
        driver: 'herdr',
        runningDriver: 'claude',
        session: { activity: { state: 'idle' } },
      })),
    }),
  });
  await vi.waitFor(async () =>
    expect((await session.complete('tmt:')).suggestions).toHaveLength(8)
  );
  const rows = (await session.complete('tmt:co1')).suggestions;
  expect(rows[0]).toEqual({
    text: 'tmt:remote/core-1 ',
    label: 'tmt:remote/core-1',
    description: 'claude · idle',
  });
  expect(run).toHaveBeenCalledTimes(1);
});

it('retains the cache and backs off when the listing process is unavailable', async () => {
  let now = 20_000;
  vi.spyOn(Date, 'now').mockImplementation(() => now);
  const run = vi
    .fn<Run>()
    .mockResolvedValueOnce({
      exitCode: 0,
      stdout: JSON.stringify({ identities: [{ name: 'core-1', presence: 'offline' }] }),
    })
    .mockResolvedValue({ exitCode: 127, stdout: '' });
  const session = hooks(run);
  await session.start();
  await vi.waitFor(async () =>
    expect((await session.complete('tmt/co1')).suggestions).toHaveLength(1)
  );
  now += 15_001;
  await session.complete('tmt/co1');
  await session.complete('tmt/co1');
  await session.complete('tmt/co1');
  expect(run).toHaveBeenCalledTimes(2);
  expect((await session.complete('tmt/co1')).suggestions[0].description).toBe('offline');
});
