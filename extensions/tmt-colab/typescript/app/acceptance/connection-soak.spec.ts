import { createHash } from 'node:crypto';
import {
  expect,
  test,
  type BrowserContext,
  type Page,
  type WebSocketRoute,
} from '@playwright/test';
import {
  pairBrowser,
  restartColab,
  restartRemote,
  startDoor,
  type Door,
  type PairedBrowser,
} from './harness/browser.js';
import {
  annotationInput,
  askEntry,
  clientId,
  composeChat,
  createPage,
  expectLive,
  freePort,
  openPage,
  run,
  sendChat,
  type CreatedPage,
} from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import type { AcceptanceWorld, Agent } from './harness/world.js';

test.afterEach(disposeActiveWorlds);

interface OpenOwner {
  world: AcceptanceWorld;
  door: Door;
  browser: PairedBrowser;
  agent: Agent;
  created: CreatedPage;
  page: Page;
  navigations: string[];
  sockets: Set<object>;
  openedSockets(): number;
}

async function owner(
  world: AcceptanceWorld,
  configure?: (context: BrowserContext) => Promise<void>,
): Promise<OpenOwner> {
  const door = await startDoor(world, await freePort());
  const agent = await world.startAgent('soak-agent');
  const browser = await pairBrowser(world, 'soak-owner', { talk: true });
  await configure?.(browser.context);
  const sockets = new Set<object>();
  let opened = 0;
  // Observe from page creation, before any SDK or mounted sync transport can attach.
  browser.context.on('page', (next) => {
    next.on('websocket', (socket) => {
      opened++;
      sockets.add(socket);
      socket.once('close', () => sockets.delete(socket));
    });
  });
  const created = createPage(world, 'Connection soak', '<h1 id="source">Before</h1>', agent.pane);
  const page = await openPage(door, browser, created);
  await expect(page.frameLocator('iframe').locator('#source')).toHaveText('Before');
  const navigations: string[] = [];
  page.on('framenavigated', (frame) => {
    if (frame === page.mainFrame()) navigations.push(frame.url());
  });
  return {
    world,
    door,
    agent,
    browser,
    created,
    page,
    navigations,
    sockets,
    openedSockets: () => opened,
  };
}

/** A new source and reply must cross the recovered connection, not a retained screenshot. */
async function readSendReceive(s: OpenOwner, label: string) {
  const receivedBefore = s.agent.received().length;
  const dispatches = () =>
    s.world.coreCalls().filter((call) => call.operation === 'dispatch.create');
  const dispatchesBefore = dispatches().length;
  const source = `After ${label}`;
  run(
    s.world,
    s.world.binaries.colab,
    ['page', 'write', s.created.pageId, '--file', '-', '--json'],
    `<h1 id="source">${source}</h1>`,
  );
  await expect(s.page.frameLocator('iframe').locator('#source')).toHaveText(source, {
    timeout: 60_000,
  });
  await expect(s.page.locator('.status.live .status-label')).toHaveText('Live');
  const question = `Reply after ${label}.`;
  const composed = await composeChat(s.page, s.agent, question);
  // Enter is not an auto-waiting button click: readiness must precede it.
  await expect(
    s.page
      .getByTestId('chat-panel')
      .getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
  ).toBeEnabled();
  const ask = await sendChat(s.page, composed);
  await expect.poll(() => s.agent.received().length, { timeout: 60_000 }).toBe(receivedBefore + 1);
  const message = composed.delivered();
  expect(message.startsWith(`[remote: ${s.browser.name}]\n`)).toBe(true);
  expect(message).toContain(question);
  expect(message).toContain(`\nLink: ${s.door.origin}/p/${s.created.pageId.slice(0, 8)}\n`);
  const reply = `ask-reply:${createHash('sha256').update(message).digest('hex').slice(0, 16)}`;
  const entry = askEntry(s.page, ask.operationId);
  await expect(entry.getByTestId('ask-reply')).toHaveText(reply, { timeout: 60_000 });
  await expect(entry.getByTestId('ask-reply-attribution')).toContainText(s.agent.name);
  expect(s.agent.received()).toHaveLength(receivedBefore + 1);
  expect(new Set(s.agent.received().map((row) => row.requestId)).size).toBe(receivedBefore + 1);
  expect(dispatches()).toHaveLength(dispatchesBefore + 1);
  expect(s.navigations).toEqual([]);
}

const triggers: Array<{ name: string; apply(s: OpenOwner): Promise<void> }> = [
  {
    name: 'in-place source write',
    async apply(s) {
      run(
        s.world,
        s.world.binaries.colab,
        ['page', 'write', s.created.pageId, '--file', '-', '--json'],
        '<h1 id="source">Trigger write</h1>',
      );
      await expect(s.page.frameLocator('iframe').locator('#source')).toHaveText('Trigger write');
    },
  },
  {
    name: 'Colab restart',
    async apply(s) {
      await restartColab(s.world, s.door, async () => {
        // Do not mistake the old Live status for the replacement's catch-up.
        await expect(s.page.locator('.status')).toContainText('Reconnecting…', {
          timeout: 30_000,
        });
      });
      // Mounted stdout proves process readiness, not the browser's verified fold.
      await expect(s.page.locator('.status.live .status-label')).toHaveText('Live', {
        timeout: 60_000,
      });
    },
  },
  {
    name: 'Remote restart',
    async apply(s) {
      await s.door.remote.kill();
      await expect(s.page.locator('.status')).toContainText('Reconnecting…', { timeout: 30_000 });
      await restartRemote(s.world, s.door);
    },
  },
  {
    name: 'page epoch advance',
    async apply(s) {
      const colab = (args: string[]) =>
        JSON.parse(run(s.world, s.world.binaries.colab, [...args, '--json'])) as Record<
          string,
          unknown
        >;
      colab(['share', 'mode', s.created.pageId, 'link', '--yes']);
      const link = colab(['share', 'link', 'add', s.created.pageId, '--yes']);
      expect(typeof link.linkId).toBe('string');
      colab(['share', 'link', 'remove', s.created.pageId, link.linkId as string, '--yes']);
    },
  },
  {
    name: 'grant revision change',
    async apply(s) {
      const id = clientId(s.world, s.browser.name);
      const change = (enabled: string) =>
        JSON.parse(
          run(s.world, s.world.binaries.remote, ['devices', 'talk', id, enabled, '--json']),
        ) as { device: { revision: number; scopes: string[] } };
      const off = change('off');
      expect(off.device.scopes).not.toContain('talk');
      const on = change('on');
      expect(on.device.scopes).toContain('talk');
      expect(on.device.revision).toBeGreaterThan(off.device.revision);
    },
  },
];

for (const trigger of triggers) {
  test(`connection soak: ${trigger.name} preserves reads, one Send and its reply without reload`, async () => {
    test.setTimeout(240_000);
    await withWorld(async (world) => {
      let holdCatchup = false;
      const held: Array<() => void> = [];
      const s = await owner(
        world,
        trigger.name === 'Colab restart'
          ? async (context) => {
              // Delay real replacement frames, never fabricate readiness or payloads.
              await context.routeWebSocket(/\/sync(?:\?|$)/, (page) => {
                const server = page.connectToServer();
                const replacement = holdCatchup;
                page.onMessage((message) => server.send(message));
                server.onMessage((message) => {
                  if (replacement && holdCatchup) held.push(() => page.send(message));
                  else page.send(message);
                });
              });
            }
          : undefined,
      );
      await readSendReceive(s, 'positive control');
      const old = s.door.colab;
      holdCatchup = trigger.name === 'Colab restart';
      let finished = false;
      const applying = trigger.apply(s).finally(() => {
        finished = true;
      });
      // Teardown still owns a failed attempt; this observer does not consume its error.
      void applying.catch(() => {});
      if (holdCatchup) {
        await expect.poll(() => s.door.colab !== old).toBe(true);
        await expect.poll(() => held.length).toBeGreaterThan(0);
        expect(finished, 'process mounted must not complete browser recovery').toBe(false);
        await expect(s.page.locator('.status')).toContainText('Reconnecting…');
        holdCatchup = false;
        held.splice(0).forEach((send) => send());
      }
      await applying;
      await readSendReceive(s, trigger.name);
    });
  });
}

// Remote's public detached-session grace is 60 seconds, with one-second maintenance.
const detachedGraceMs = 60_000;

for (const beyondGrace of [false, true]) {
  test(`connection soak: offline ${beyondGrace ? 'past detached grace' : 'and back online'} recovers without reload`, async () => {
    test.setTimeout(240_000);
    await withWorld(async (world) => {
      let offline = false;
      const routes = new Set<{ page: WebSocketRoute; server: WebSocketRoute }>();
      const s = await owner(world, async (context) => {
        // Chromium's offline emulation leaves established sockets open; cut the real passthrough too.
        await context.routeWebSocket(/.*/, (page) => {
          if (offline) {
            void page.close();
            return;
          }
          const server = page.connectToServer();
          routes.add({ page, server });
        });
      });
      await readSendReceive(s, 'online positive control');
      expect(s.openedSockets()).toBeGreaterThan(0);
      offline = true;
      await s.browser.context.setOffline(true);
      // As in page-save's lost-reply case, default close (1000) is transient, not an ended frame.
      await Promise.all([...routes].flatMap(({ page, server }) => [page.close(), server.close()]));
      routes.clear();
      await expect.poll(() => s.sockets.size, { timeout: 30_000 }).toBe(0);
      await expect(s.page.locator('.status')).toContainText('Reconnecting…', { timeout: 30_000 });
      if (beyondGrace) {
        // This elapsed-time wait is the property under test, after proven transport closure.
        await s.page.waitForTimeout(detachedGraceMs + 5_000);
        expect(s.sockets.size).toBe(0);
      }
      offline = false;
      await s.browser.context.setOffline(false);
      await readSendReceive(s, beyondGrace ? 'expired detached session' : 'offline recovery');
      expect(s.openedSockets()).toBeGreaterThan(1);
    });
  });
}

interface SdkOperations {
  listAgents(): Promise<unknown[]>;
}
interface SdkFixtureWindow {
  soakOperations?: SdkOperations;
}

/** Retain only the shipped SDK's opaque operations handle; no Remote state is patched. */
async function sdkOpen(page: Page) {
  return page.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as {
      reopenSession(): Promise<unknown>;
      operations(session: unknown): SdkOperations;
    };
    const session = await sdk.reopenSession();
    const ops = sdk.operations(session);
    (window as unknown as SdkFixtureWindow).soakOperations = ops;
    return (await ops.listAgents()).length;
  });
}

async function sdkRead(page: Page) {
  return page.evaluate(async () => {
    try {
      return {
        agents: (await (window as unknown as SdkFixtureWindow).soakOperations!.listAgents()).length,
      };
    } catch (error) {
      const refusal = error as { code?: string; limit?: number };
      return { code: refusal.code, limit: refusal.limit };
    }
  });
}

test('connection soak: detached eviction and a refused new tab leave the attached page usable', async () => {
  test.setTimeout(240_000);
  await withWorld(async (world) => {
    run(world, world.binaries.remote, ['settings', 'sessions-per-device', '2', '--json']);
    const s = await owner(world);
    await readSendReceive(s, 'capacity positive control');
    const sdkPages: Page[] = [];
    for (let index = 0; index < 2; index++) {
      const tab = await s.browser.context.newPage();
      // An SDK-only tab is intentionally detached, as in the existing tabs acceptance control.
      await tab.route('**/colab/assets/*.js', (route) => route.abort());
      await tab.goto(`${s.door.mounts}colab/`);
      expect(await sdkOpen(tab)).toBe(1);
      sdkPages.push(tab);
    }
    await expect
      .poll(() => sdkRead(sdkPages[0]))
      .toMatchObject({ code: 'REMOTE_SESSION_EVICTED', limit: 2 });
    expect(await sdkRead(sdkPages[1])).toEqual({ agents: 1 });
    await readSendReceive(s, 'detached eviction');

    // A second attached owner page takes the detached slot; all capacity is now attached.
    const second = await openPage(s.door, s.browser, s.created);
    await expect(second.frameLocator('iframe').locator('#source')).toHaveText(
      'After detached eviction',
    );
    const refused = await openPage(s.door, s.browser, s.created);
    await expect(refused.getByRole('alert')).toBeVisible({ timeout: 30_000 });
    await expect(refused.locator('iframe')).toHaveCount(0);
    const reason = await refused.evaluate(async () => {
      const sdk = (await import('/sdk/remote-v1.js' as string)) as {
        reopenSession(): Promise<unknown>;
      };
      try {
        await sdk.reopenSession();
        return { opened: true };
      } catch (error) {
        const failure = error as { code?: string; limit?: number };
        return { code: failure.code, limit: failure.limit };
      }
    });
    expect(reason).toEqual({ code: 'REMOTE_SESSION_LIMIT', limit: 2 });
    // Source can be visible before this tab's renderer has reported ready.
    // Make the attached peer current before exercising its next update.
    await second.bringToFront();
    await expect.poll(() => second.evaluate(() => document.visibilityState)).toBe('visible');
    await expectLive(second);
    await readSendReceive(s, 'new-tab capacity refusal');
    // The Send/reply exercise targets the first page; return to the peer being checked.
    await second.bringToFront();
    await expect.poll(() => second.evaluate(() => document.visibilityState)).toBe('visible');
    await expectLive(second);
    await expect(second.frameLocator('iframe').locator('#source')).toHaveText(
      'After new-tab capacity refusal',
    );
  });
});

test('connection soak: explicit revoke ends access and cannot dispatch a retained draft', async () => {
  test.setTimeout(240_000);
  await withWorld(async (world) => {
    const s = await owner(world);
    await readSendReceive(s, 'revoke positive control');
    const panel = s.page.getByTestId('chat-panel');
    const input = await annotationInput(panel);
    await input.fill('@soak-agent Do not dispatch after revoke.');
    const before = world.coreCalls().filter((call) => call.operation === 'dispatch.create').length;
    run(world, world.binaries.remote, [
      'devices',
      'revoke',
      clientId(world, s.browser.name),
      '--json',
    ]);
    await expect(s.page.locator('.tmt-ui-drawer[data-panel=chat]')).toContainText(
      'Your access to this page ended.',
      { timeout: 90_000 },
    );
    await expect(
      panel.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
    ).toBeDisabled();
    await expect(input).toHaveAttribute('contenteditable', 'false');
    await input.press('Enter');
    expect(s.agent.received()).toHaveLength(1);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      before,
    );
    expect(s.navigations).toEqual([]);
  });
});
