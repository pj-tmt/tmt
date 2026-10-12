import { describe, expect, it, vi } from 'vite-plus/test';
import { SessionEvictedError } from '../src/ask-remote.js';
import { ConnectionController } from '../src/connection-controller.js';
import { ReadmitBudget } from '../src/readmit.js';
import { ReopenFailedError, jsonResponse } from '../src/registration.js';
import { RecoveryRequiredError } from '../src/session-recovery.js';
import { terminalFailure } from '../src/terminal-failure.js';
import { text } from '../src/strings.js';

const lost = () => new RecoveryRequiredError(new Error('Sync disconnected'));
const refusal = (code: string) => new Error(code);
// Waits yield one macrotask instead of a clock, so a loop that never ends (the dispose test)
// cannot starve the test; `settled` gives every pending attempt twenty of them.
const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
const settled = async () => {
  for (let turn = 0; turn < 20; turn++) await tick();
};
const instant = { wait: tick, visible: async () => {}, jitter: () => 0.5, reopenWait: tick };

function fixture(
  overrides: {
    reconnect?: (controller: ConnectionController) => Promise<boolean>;
    reopen?: (signal: AbortSignal) => Promise<unknown>;
    budget?: ReadmitBudget;
  } = {},
) {
  const failed = vi.fn();
  const binding = {
    reconnect: vi.fn(async () => (overrides.reconnect ? overrides.reconnect(controller) : true)),
    close: vi.fn(),
  };
  const opened = vi.fn();
  const reopen = vi.fn(overrides.reopen ?? (async () => ({ id: 'next' })));
  const controller: ConnectionController = new ConnectionController({
    binding: () => binding,
    reopen,
    opened,
    failed,
    budget: overrides.budget,
    ...instant,
  });
  return { controller, binding, opened, reopen, failed };
}

async function httpRefusal(code?: string): Promise<Error> {
  return jsonResponse(
    new Response(JSON.stringify(code === undefined ? {} : { code }), { status: 403 }),
    8192,
  ).catch((error: unknown) => error as Error) as Promise<Error>;
}

describe('HTTP admission reopens', () => {
  it('a 403 then successful admission replaces the binding once without a transport replay', async () => {
    const refusal = await httpRefusal('DENIED');
    let attempts = 0;
    const { controller, reopen, opened, binding } = fixture({
      reopen: async () => {
        if (++attempts === 1) throw await httpRefusal('DENIED');
        return { id: 'admitted' };
      },
    });
    controller.report(refusal);
    expect(controller.state).toMatchObject({ phase: 'reconnecting', kind: 'epoch' });
    await settled();
    expect(reopen).toHaveBeenCalledTimes(2);
    expect(opened).toHaveBeenCalledExactlyOnceWith({ id: 'admitted' });
    expect(binding.close).toHaveBeenCalledOnce();
    expect(binding.reconnect).not.toHaveBeenCalled();
    expect(controller.state).toEqual({ phase: 'connected' });
    controller.dispose();
  });

  it('a 403 with a named revoke ends immediately with its own reason', async () => {
    const { controller, reopen, opened } = fixture();
    controller.report(await httpRefusal('REMOTE_DEVICE_REVOKED'));
    await settled();
    expect(controller.state).toMatchObject({ phase: 'ended', reason: 'access', action: 'pair' });
    expect(reopen).not.toHaveBeenCalled();
    expect(opened).not.toHaveBeenCalled();
    if (controller.state.phase !== 'ended') throw new Error('Expected named ending');
    expect(terminalFailure(controller.state.error).sentence).toBe(text.failureAccessEnded);
  });

  it('repeated unclassified 403 stops after the existing five attempts with reconnect failure copy', async () => {
    const { controller, reopen, opened, binding } = fixture({
      reopen: async () => {
        throw await httpRefusal();
      },
    });
    controller.report(await httpRefusal());
    await settled();
    expect(reopen).toHaveBeenCalledTimes(5);
    expect(controller.state).toMatchObject({ phase: 'ended', reason: 'generic', action: 'none' });
    expect(opened).not.toHaveBeenCalled();
    expect(binding.reconnect).not.toHaveBeenCalled();
    if (controller.state.phase !== 'ended') throw new Error('Expected bounded ending');
    expect(terminalFailure(controller.state.error).sentence).toBe(text.reconnectFailed);
    controller.report(await httpRefusal());
    await settled();
    expect(reopen).toHaveBeenCalledTimes(5);
  });
});

describe('connection controller: a lost transport', () => {
  it('reconnects on its own and goes back to connected, with no click', async () => {
    const { controller, binding } = fixture();
    expect(controller.state).toEqual({ phase: 'connected' });
    controller.report(lost());
    expect(controller.state).toMatchObject({ phase: 'reconnecting', kind: 'transport' });
    await settled();
    expect(binding.reconnect).toHaveBeenCalledOnce();
    expect(controller.state).toEqual({ phase: 'connected' });
  });

  it('keeps trying while each attempt fails in a recoverable way', async () => {
    let attempts = 0;
    const { controller, binding } = fixture({
      // Live publishes a fresh failure when a try fails; the third try connects.
      reconnect: async (c) => {
        attempts += 1;
        if (attempts < 3) {
          c.report(lost());
          return false;
        }
        return true;
      },
    });
    controller.report(lost());
    await settled();
    expect(binding.reconnect).toHaveBeenCalledTimes(3);
    expect(controller.state).toEqual({ phase: 'connected' });
  });

  it.each([
    ['REMOTE_DEVICE_REVOKED', 'access', 'pair'],
    ['REMOTE_CLOSED', 'access', 'pair'],
    ['Page unavailable', 'gone', 'pages'],
  ] as const)(
    'ends with a named reason when the fresh open is refused (%s)',
    async (code, reason, action) => {
      const { controller, binding } = fixture({
        reconnect: async (c) => {
          c.report(refusal(code));
          return false;
        },
      });
      controller.report(lost());
      await settled();
      expect(controller.state).toMatchObject({ phase: 'ended', reason, action });
      expect(binding.reconnect).toHaveBeenCalledOnce();
    },
  );

  it('ends a page that is no longer paired and keeps the typed reason', async () => {
    const { controller } = fixture({
      reconnect: async (c) => {
        c.report(new ReopenFailedError('unpaired'));
        return false;
      },
    });
    controller.report(lost());
    await settled();
    expect(controller.state).toMatchObject({ phase: 'ended', reason: 'unpaired', action: 'pair' });
  });

  it('keeps trying through a transient fresh-open refusal', async () => {
    let attempts = 0;
    const { controller, binding } = fixture({
      reconnect: async (c) => {
        attempts += 1;
        if (attempts === 1) {
          c.report(new ReopenFailedError('transient'));
          return false;
        }
        return true;
      },
    });
    controller.report(lost());
    await settled();
    expect(binding.reconnect).toHaveBeenCalledTimes(2);
    expect(controller.state).toEqual({ phase: 'connected' });
  });

  it('ends a lost transport at once when the binding cannot reconnect', async () => {
    const failed = vi.fn();
    const controller = new ConnectionController({
      binding: () => ({ close: vi.fn() }),
      reopen: async () => ({}),
      opened: vi.fn(),
      failed,
      ...instant,
    });
    const seen: string[] = [];
    controller.subscribe(() => seen.push(controller.state.phase));
    controller.report(lost());
    expect(controller.state).toMatchObject({ phase: 'ended', reason: 'generic', action: 'none' });
    expect(seen).toEqual(['ended']);
    expect(failed).toHaveBeenCalledOnce();
    await settled();
    expect(controller.state.phase).toBe('ended');
  });

  it('ends when a series gives up without connecting, with the failure it stopped on', async () => {
    // Live.reconnect() returns false and publishes nothing new: the failure is unchanged.
    const { controller, binding } = fixture({ reconnect: async () => false });
    controller.report(lost());
    expect(controller.state.phase).toBe('reconnecting');
    await settled();
    expect(binding.reconnect).toHaveBeenCalledOnce();
    expect(controller.state).toMatchObject({ phase: 'ended', reason: 'generic', action: 'none' });
    controller.report(lost());
    await settled();
    expect(binding.reconnect).toHaveBeenCalledOnce();
  });

  it('starts a new series when an attempt connects while a fresh failure arrived', async () => {
    let attempts = 0;
    const { controller, binding } = fixture({
      reconnect: async (c) => {
        attempts += 1;
        // The first try reports a new failure and then claims success: the old failure is
        // not the current one, so the page is still down and needs another series.
        if (attempts === 1) c.report(lost());
        return true;
      },
    });
    controller.report(lost());
    await settled();
    expect(binding.reconnect).toHaveBeenCalledTimes(2);
    expect(controller.state).toEqual({ phase: 'connected' });
  });
});

describe('connection controller: an epoch advance', () => {
  it('reopens the page and hands the new one over', async () => {
    const { controller, binding, opened, reopen } = fixture();
    controller.report(refusal('STALE_EPOCH'));
    expect(controller.state).toMatchObject({ phase: 'reconnecting', kind: 'epoch' });
    await settled();
    expect(reopen).toHaveBeenCalledOnce();
    expect(binding.close).toHaveBeenCalledOnce();
    expect(opened).toHaveBeenCalledWith({ id: 'next' });
    expect(controller.state).toEqual({ phase: 'connected' });
  });

  it('stops the transport loop when the reopen starts', async () => {
    const { controller, binding, opened } = fixture({
      // The transport loop keeps failing recoverably, publishing a fresh failure each try.
      reconnect: async (c) => {
        c.report(lost());
        return false;
      },
      reopen: () => new Promise((resolve) => setTimeout(() => resolve({ id: 'next' }), 100)),
    });
    controller.report(lost());
    await settled();
    const calls = binding.reconnect.mock.calls.length;
    expect(calls).toBeGreaterThan(0);
    controller.report(refusal('STALE_EPOCH'));
    expect(controller.state).toMatchObject({ phase: 'reconnecting', kind: 'epoch' });
    await settled();
    expect(binding.reconnect.mock.calls.length).toBe(calls);
    await new Promise((resolve) => setTimeout(resolve, 150));
    await settled();
    expect(opened).toHaveBeenCalledOnce();
    expect(binding.reconnect.mock.calls.length).toBe(calls);
    expect(controller.state).toEqual({ phase: 'connected' });
  });

  it('waits out a lagging epoch', async () => {
    let tries = 0;
    const { controller, opened } = fixture({
      reopen: async () => {
        tries += 1;
        if (tries < 3) throw refusal('STALE_EPOCH');
        return { id: 'next' };
      },
    });
    controller.report(refusal('STALE_EPOCH'));
    await settled();
    expect(tries).toBe(3);
    expect(opened).toHaveBeenCalledOnce();
  });

  it('ends when the fresh open is refused', async () => {
    const { controller, opened } = fixture({
      reopen: async () => {
        throw refusal('REMOTE_DEVICE_REVOKED');
      },
    });
    controller.report(refusal('DENIED'));
    await settled();
    expect(opened).not.toHaveBeenCalled();
    expect(controller.state).toMatchObject({ phase: 'ended', reason: 'access', action: 'pair' });
  });

  it('stops reopening a page that keeps failing', async () => {
    const budget = new ReadmitBudget(2, 60_000);
    const { controller, reopen } = fixture({ budget });
    for (let round = 0; round < 3; round++) {
      controller.reset();
      controller.report(refusal('EXPIRED'));
      await settled();
    }
    expect(reopen).toHaveBeenCalledTimes(2);
    expect(controller.state).toMatchObject({ phase: 'ended', reason: 'access', action: 'pair' });
  });
});

describe('connection controller: lifecycle', () => {
  it('tells the page about each failure that is not an epoch advance, once', async () => {
    const { controller, failed } = fixture();
    controller.report(refusal('STALE_EPOCH'));
    await settled();
    expect(failed).not.toHaveBeenCalled();
    controller.report(refusal('Access ended'));
    expect(failed).toHaveBeenCalledOnce();
    controller.report(refusal('Page unavailable'));
    expect(failed).toHaveBeenCalledOnce();
  });

  it('ends an eviction with its own reason and never retries it', async () => {
    const { controller, binding, reopen } = fixture();
    controller.report(new SessionEvictedError(8));
    await settled();
    expect(controller.state).toMatchObject({ phase: 'ended', reason: 'evicted', action: 'none' });
    expect(binding.reconnect).not.toHaveBeenCalled();
    expect(reopen).not.toHaveBeenCalled();
  });

  it('keeps the first ending: a later failure does not change the card', async () => {
    const { controller } = fixture();
    controller.report(refusal('Access ended'));
    controller.report(refusal('Page unavailable'));
    expect(controller.state).toMatchObject({ phase: 'ended', reason: 'access' });
  });

  it('goes back to connected on reset and ignores what the old loop reports late', async () => {
    let release: (value: boolean) => void = () => {};
    const { controller, binding } = fixture({
      reconnect: () => new Promise<boolean>((resolve) => (release = resolve)),
    });
    controller.report(lost());
    await settled();
    expect(binding.reconnect).toHaveBeenCalledOnce();
    controller.reset();
    expect(controller.state).toEqual({ phase: 'connected' });
    release(false);
    await settled();
    expect(controller.state).toEqual({ phase: 'connected' });
  });

  it('stops everything on dispose', async () => {
    const { controller, binding } = fixture({
      reconnect: async (c) => {
        c.report(lost());
        return false;
      },
    });
    controller.report(lost());
    await settled();
    const calls = binding.reconnect.mock.calls.length;
    controller.dispose();
    await settled();
    expect(binding.reconnect.mock.calls.length).toBe(calls);
  });

  it('tells subscribers about each change and about nothing else', async () => {
    const { controller } = fixture();
    const seen: string[] = [];
    controller.subscribe(() => seen.push(controller.state.phase));
    controller.report(lost());
    await settled();
    controller.reset();
    expect(seen).toEqual(['reconnecting', 'connected']);
  });
});
