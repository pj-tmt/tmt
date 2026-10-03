import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

interface SdkWindow {
  session?: unknown;
  ops?: { listAgents(): Promise<unknown[]> };
}

// Remote allows one session per device: a newer `session.open` ends the
// previous session and its tunnels. Two tabs of one paired browser share one
// device, so each tab opening its own session ends the other's. This pins that
// Remote contract with the real door and the served SDK, with the app's own
// scripts blocked so only this test opens sessions. The two-tab Ask case in
// ask.spec.ts depends on the app sharing one session between tabs.
test.fixme('Remote keeps one session per device: a second tab ending the first is the contract (disabled until #1517, the operations SDK, is on this branch)', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world);
    await world.startAgent('ask-recipient');
    const browser = await pairBrowser(world, 'one-device');
    await browser.context.route('**/colab/assets/*.js', (route) => route.abort());
    const first = await browser.context.newPage();
    await first.goto(`${door.mounts}colab/`);
    const second = await browser.context.newPage();
    await second.goto(`${door.mounts}colab/`);

    const open = (page: typeof first) =>
      page.evaluate(async () => {
        const sdk = (await import('/sdk/remote-v1.js' as string)) as {
          reopenSession(): Promise<unknown>;
          operations(session: unknown): { listAgents(): Promise<unknown[]> };
        };
        const w = window as unknown as SdkWindow;
        w.session = await sdk.reopenSession();
        w.ops = sdk.operations(w.session);
        return (await w.ops.listAgents()).length;
      });
    const list = (page: typeof first) =>
      page.evaluate(async () => {
        try {
          return { agents: (await (window as unknown as SdkWindow).ops!.listAgents()).length };
        } catch (error) {
          return { code: (error as { code?: string }).code };
        }
      });

    expect(await open(first)).toBe(1);
    expect(await list(first)).toEqual({ agents: 1 });
    expect(await open(second)).toBe(1);
    // The newer session ended the first tab's session; the second stays live.
    expect(await list(first)).toEqual({ code: 'REMOTE_SESSION_ENDED' });
    expect(await list(second)).toEqual({ agents: 1 });
  });
});
