import { createHash } from 'node:crypto';
import fs from 'node:fs';
import { expect, test, type BrowserContext, type Page, type Response } from '@playwright/test';
import { pairBrowser, restartColab, restartRemote, startDoor } from './harness/browser.js';
import {
  annotationInput,
  selectInRenderer,
  askEntry,
  askState,
  clientId,
  createPage,
  freePort,
  openPage,
  composeChat,
  openChat,
  run,
  sendChat,
} from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import type { AcceptanceWorld } from './harness/world.js';

// #1110 Ask agent real-binary acceptance. The Ask UI, Remote operations, the page
// (`tmt colab page create`), the recipient and `tmt reply` are all real; the page exists
// before the paired browsers register. The held case waits for a Remote-provided hold
// fixture; held behavior is also covered by unit tests.
//
// Architecture: no native bridge ledger. The asker's browser calls Remote
// operations as its paired device and records the ask, its states and the
// reply in its own Colab stream. v1 is owner-only.

const PAGE_HTML = '<h1>Ask acceptance</h1><p id="quote">Exact selected sentence for the agent.</p>';

/** Arm before Remote dies; a fresh mounted registration proves post-restart recovery. */
function restartRecovery(page: Page) {
  const control = page.getByRole('button', { name: 'Reconnect', exact: true });
  const content = page.frameLocator('iframe').getByRole('heading', { name: 'Ask acceptance' });
  let registered = false;
  const observe = (response: Response) => {
    if (
      new URL(response.url()).pathname.endsWith('/api/devices/register') &&
      response.request().method() === 'POST' &&
      response.status() === 200
    )
      registered = true;
  };
  page.on('response', observe);
  return async () => {
    try {
      await expect
        .poll(async () => registered || (await control.isVisible()), { timeout: 60_000 })
        .toBe(true);
      if (await control.isVisible()) await control.click();
      await expect.poll(() => registered, { timeout: 60_000 }).toBe(true);
      await expect(page.locator('.status.live .status-label')).toHaveText('Live');
      await expect(page.getByRole('heading', { name: 'Preview stopped', exact: true })).toHaveCount(
        0,
      );
      await expect(content).toBeVisible();
      await openChat(page);
    } finally {
      page.off('response', observe);
    }
  };
}

/** Parks only the browser's fresh SDK `listAgents` read while `__holdFreshDirectory` is set, so a
 * test can observe the composer between Reconnect and the directory answer. Production binaries,
 * transport and dispatch are untouched. */
async function holdableDirectory(context: BrowserContext) {
  await context.route('**/sdk/remote-v1.js', async (route) => {
    const response = await route.fetch();
    let body = await response.text();
    body = body.replace(
      'function operations(session, options = {})',
      'function nativeOperations(session, options = {})',
    );
    if (!body.includes('function nativeOperations('))
      throw new Error('SDK operations owner not found');
    body += `\nfunction operations(session, options = {}) { const port = nativeOperations(session, options); return { ...port, listAgents: async () => { if (globalThis.__holdFreshDirectory) { globalThis.__directoryHeld = true; await new Promise((resolve) => (globalThis.__heldDirectoryReads ??= []).push(resolve)); } return port.listAgents(); } }; }\n`;
    await route.fulfill({ response, body });
  });
}
async function scenario(
  world: AcceptanceWorld,
  options: { gated?: boolean; holdDirectory?: boolean } = {},
) {
  const door = await startDoor(world, await freePort());
  const recipient = await world.startAgent('ask-recipient', options);
  const asker = await pairBrowser(world, 'asker-browser', { talk: true });
  if (options.holdDirectory) await holdableDirectory(asker.context);
  const viewer = await pairBrowser(world, 'viewer-browser');
  // The page is created first; each paired device registers when it opens it.
  const page = createPage(world, 'Ask acceptance', PAGE_HTML);
  const askerPage = await openPage(door, asker, page);
  return { door, recipient, page, asker, viewer, askerPage };
}

const replyBody = (message: string) =>
  `ask-reply:${createHash('sha256').update(message).digest('hex').slice(0, 16)}`;
const dispatches = (world: AcceptanceWorld) =>
  world.coreCalls().filter((call) => call.operation === 'dispatch.create');
const askerName = 'asker-browser';

test.describe('Ask agent real-binary acceptance (#1110)', () => {
  test.afterEach(disposeActiveWorlds);

  test(`direct Chat send: the turn reaches the recipient exactly once and the reply shows in a second viewer`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world);
      const draft = await composeChat(s.askerPage, s.recipient, 'Explain this sentence');
      expect(new URL(s.askerPage.url()).pathname).toBe(s.page.path);
      expect(new URL(s.askerPage.url()).hash).toBe('');
      const ask = await sendChat(s.askerPage, draft);
      // The input stays ready for another explicit turn; no preview screen exists.
      await expect(s.askerPage.getByTestId('ask-preview')).toHaveCount(0);
      await expect(
        s.askerPage.getByTestId('chat-panel').getByRole('combobox', { name: 'Message' }),
      ).toBeFocused();
      await expect(askEntry(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-ledger-state',
        'accepted',
      );
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      expect(s.recipient.received()).toHaveLength(1);
      // The text starts with Remote's stable device-name line and names the page itself.
      expect(draft.delivered().startsWith(`[remote: ${askerName}]\n`)).toBe(true);
      expect(draft.delivered()).toContain(
        `\nLink: ${s.door.origin}/p/${s.page.pageId.slice(0, 8)}\n`,
      );
      expect(dispatches(world)).toHaveLength(1);
      // The real `tmt reply` shows up on the asker's page, attributed to the agent,
      // and in a second paired viewer.
      const reply = replyBody(draft.delivered());
      const entry = askEntry(s.askerPage, ask.operationId);
      await expect(entry.getByTestId('ask-reply')).toHaveText(reply);
      await expect(entry.getByTestId('ask-reply-attribution')).toContainText(s.recipient.name);
      const second: Page = await openPage(s.door, s.viewer, s.page);
      await openChat(second);
      const mirrored = askEntry(second, ask.operationId);
      await expect(mirrored.getByTestId('ask-reply')).toHaveText(reply);
      await expect(mirrored.getByTestId('ask-reply-attribution')).toContainText(s.recipient.name);
      // agents.list has presence only in v1: no delivery state is shown.
      await expect(
        s.askerPage.locator('[data-testid=ask-agent-option][data-delivery=unavailable]'),
      ).toHaveCount(0);
    });
  });

  test(`browser reload restores the ask from its own stream with the same operation ID and no second wake`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world, { gated: true });
      const draft = await composeChat(s.askerPage, s.recipient, 'Hold the reply');
      const ask = await sendChat(s.askerPage, draft);
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      const requestId = s.recipient.received()[0].requestId as string;
      await s.askerPage.reload();
      await openChat(s.askerPage);
      await expect(askEntry(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-ledger-state',
        'accepted',
      );
      // Releasing the gate lets the real reply flow; still one wake, one dispatch.
      fs.writeFileSync(`${s.recipient.gate}/${requestId}.release`, '');
      await expect(askEntry(s.askerPage, ask.operationId).getByTestId('ask-reply')).toHaveText(
        replyBody(draft.delivered()),
      );
      expect(s.recipient.received()).toHaveLength(1);
      expect(dispatches(world)).toHaveLength(1);
    });
  });

  for (const width of [1440, 390])
    for (const surface of ['Chat', 'annotation'] as const) {
      test(`explicit Reconnect retains the mounted ${surface} draft and caret at ${width} without resending`, async ({
        browserName,
      }, testInfo) => {
        expect(browserName).toBe('chromium');
        await withWorld(async (world) => {
          const s = await scenario(world, { holdDirectory: true });
          const initial = await composeChat(
            s.askerPage,
            s.recipient,
            'One original Ask before recovery',
          );
          const ask = await sendChat(s.askerPage, initial);
          const entry = askEntry(s.askerPage, ask.operationId);
          await until(() => s.recipient.received().length === 1, 'one original recipient wake');
          await expect(entry.getByTestId('ask-reply')).toHaveText(replyBody(initial.delivered()));
          await s.askerPage.setViewportSize({ width, height: 900 });
          if (surface === 'annotation') {
            await s.askerPage.getByRole('button', { name: 'Close Chat', exact: true }).click();
            await selectInRenderer(s.askerPage, '#quote');
            await s.askerPage.getByTestId('selection-ask').click();
            await annotationInput(
              s.askerPage.getByRole('dialog', { name: 'Annotate selection' }),
              s.recipient.name,
            );
          }
          const container =
            surface === 'Chat'
              ? s.askerPage.getByTestId('chat-panel')
              : s.askerPage.getByRole('dialog', { name: 'Annotate selection' });
          const input = container.getByRole('combobox', { name: 'Message', exact: true });
          const action = container.getByRole('button', {
            name: 'Send',
            exact: true,
          });
          await s.door.remote.kill();
          const reconnect = s.askerPage.getByRole('button', { name: 'Reconnect', exact: true });
          await expect(reconnect).toHaveCount(1, { timeout: 60000 });
          const draft = `Keep this draft typed while disconnected. @${s.recipient.name}`;
          await input.fill(draft);
          await input.evaluate((node) => {
            const first = node.ownerDocument
              .createTreeWalker(node, NodeFilter.SHOW_TEXT)
              .nextNode()!;
            const range = node.ownerDocument.createRange();
            range.setStart(first, 5);
            range.collapse(true);
            const selection = node.ownerDocument.getSelection()!;
            selection.removeAllRanges();
            selection.addRange(range);
          });
          const selection = () =>
            input.evaluate((node) => {
              const value = getSelection()!;
              return {
                anchor: value.anchorOffset,
                focus: value.focusOffset,
                inside: node.contains(value.anchorNode) && node.contains(value.focusNode),
              };
            });
          const before = await selection();
          expect(before).toEqual({ anchor: 5, focus: 5, inside: true });
          await input.evaluate((node) => {
            Object.assign(window, { recoveryDraftNode: node });
          });
          await expect(action).toBeDisabled();
          if (surface === 'annotation')
            await expect(
              container.getByRole('button', { name: 'Send', exact: true }),
            ).toBeDisabled();
          if (surface === 'Chat' && width === 390)
            await s.askerPage.getByRole('button', { name: 'Close Chat', exact: true }).click();
          await reconnect.click(); // Door still down: retry remains available, without reload.
          await expect(reconnect).toBeEnabled();
          await expect(
            s.askerPage.getByRole('heading', { name: 'Connection lost', exact: true }),
          ).toBeVisible();
          if (surface === 'Chat') await openChat(s.askerPage);
          await input.focus();
          await expect.poll(selection).toEqual(before);
          await expect(input).toHaveText(draft);
          expect(s.recipient.received()).toHaveLength(1);
          expect(dispatches(world)).toHaveLength(1);
          await expect(entry).toHaveAttribute('data-ledger-state', 'accepted');
          let registrations = 0;
          const navigations: string[] = [];
          const registered = (response: Response) => {
            if (
              new URL(response.url()).pathname.endsWith('/api/devices/register') &&
              response.request().method() === 'POST' &&
              response.status() === 200
            )
              registrations++;
          };
          s.askerPage.on('response', registered);
          s.askerPage.on('framenavigated', (frame) => {
            if (frame === s.askerPage.mainFrame()) navigations.push(frame.url());
          });
          await s.askerPage.evaluate(() => {
            Object.assign(window, { __holdFreshDirectory: true });
          });
          await restartRemote(world, s.door);
          if (surface === 'Chat' && width === 390)
            await s.askerPage.getByRole('button', { name: 'Close Chat', exact: true }).click();
          await reconnect.click();
          await expect(reconnect).toHaveCount(0);
          await expect(s.askerPage.locator('.status.live .status-label')).toHaveText('Live');
          await expect(
            s.askerPage.frameLocator('iframe').getByRole('heading', { name: 'Ask acceptance' }),
          ).toBeVisible();
          await expect.poll(() => registrations).toBe(1);
          if (surface === 'Chat') await openChat(s.askerPage);
          await input.focus();
          await expect.poll(selection).toEqual(before);
          expect(
            await input.evaluate(
              (node) =>
                node === (window as unknown as { recoveryDraftNode: Element }).recoveryDraftNode,
            ),
          ).toBe(true);
          await expect(input).toHaveText(draft);
          // The fresh directory read is parked: nothing can be asked yet (#2066).
          await expect
            .poll(() =>
              s.askerPage.evaluate(
                () => !!(window as unknown as { __directoryHeld?: boolean }).__directoryHeld,
              ),
            )
            .toBe(true);
          await expect(container.locator('.annotation-status-row [role="status"]')).toHaveText(
            'Checking for agents…',
          );
          await expect(action).toBeDisabled();
          for (const theme of ['light', 'dark']) {
            await s.askerPage.evaluate(
              (theme) => (document.documentElement.dataset.theme = theme),
              theme,
            );
            await s.askerPage.screenshot({
              path: testInfo.outputPath(`loading-${surface}-${width}-${theme}.png`),
            });
          }
          await s.askerPage.evaluate(() => delete document.documentElement.dataset.theme);
          await input.press('Enter');
          await expect(container.getByRole('alert')).toHaveCount(0);
          expect(s.recipient.received()).toHaveLength(1);
          expect(dispatches(world)).toHaveLength(1);
          await expect(input).toHaveText(draft);
          // Every parked read is released, and later ones are no longer parked.
          await s.askerPage.evaluate(() => {
            const held = window as unknown as {
              __holdFreshDirectory?: boolean;
              __heldDirectoryReads?: (() => void)[];
            };
            held.__holdFreshDirectory = false;
            for (const release of held.__heldDirectoryReads ?? []) release();
            held.__heldDirectoryReads = [];
          });
          await expect(action).toBeEnabled();
          await expect(container.locator('.annotation-status-row [role="status"]')).toHaveText(
            `Asks @${s.recipient.name}.`,
          );
          await expect(entry).toHaveAttribute('data-ledger-state', 'accepted');
          await expect(entry.getByTestId('ask-reply')).toHaveText(replyBody(initial.delivered()));
          expect(navigations).toEqual([]);
          expect(s.recipient.received()).toHaveLength(1);
          expect(dispatches(world)).toHaveLength(1);
          await input.press('X');
          const edited = draft.slice(0, 5) + 'X' + draft.slice(5);
          await expect(input).toHaveText(edited);
          await s.askerPage.screenshot({
            path: testInfo.outputPath(`reconnected-${surface}-${width}.png`),
            fullPage: true,
          });
          if (surface === 'annotation') {
            const post = container.getByRole('button', { name: 'Send', exact: true });
            await expect(post).toBeEnabled();
            await input.fill(edited.replace(` @${s.recipient.name}`, ''));
            await post.click();
            await expect(input).toHaveText('');
            await expect(
              container.getByText(edited.replace(` @${s.recipient.name}`, ''), { exact: true }),
            ).toBeVisible();
            expect(s.recipient.received()).toHaveLength(1);
            expect(dispatches(world)).toHaveLength(1);
            await input.fill(`@${s.recipient.name} One new explicit Ask after recovery.`);
          }
          await action.click();
          await until(() => s.recipient.received().length === 2, 'one new explicit recipient wake');
          expect(s.recipient.received()).toHaveLength(2);
          expect(dispatches(world)).toHaveLength(2);
          const received = s.recipient.received()[1];
          expect(String(received.message)).toContain(
            surface === 'Chat' ? edited : 'One new explicit Ask after recovery.',
          );
          await expect(input).toHaveText('');
          s.askerPage.off('response', registered);
        });
      });
    }

  test(`remote restart after the core accepted recovers via operation.show with no second wake`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world);
      const draft = await composeChat(s.askerPage, s.recipient, 'Survive a restart');
      world.armNextBarrier('after');
      const ask = await sendChat(s.askerPage, draft);
      const parked = await world.barrierEntered();
      expect(parked.operationId).toBe(ask.operationId);
      // The real core has accepted; Remote dies before it can answer the browser.
      const recover = restartRecovery(s.askerPage);
      await s.door.remote.kill();
      world.releaseBarrier();
      await restartRemote(world, s.door);
      // The mounted owner can replace a verified ended Session automatically.
      // Recovery observes the original operation ID read-only, never a resend.
      await recover();
      await expect(askEntry(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-ledger-state',
        'accepted',
        { timeout: 60_000 },
      );
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      expect(s.recipient.received()).toHaveLength(1);
      expect(dispatches(world)).toHaveLength(1);
    });
  });

  test(`remote restart before dispatch stays uncertain with no new dispatch; abandon records MAY_HAVE_BEEN_DELIVERED`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world);
      const draft = await composeChat(s.askerPage, s.recipient, 'Never reaches the core');
      world.armNextBarrier('before');
      const ask = await sendChat(s.askerPage, draft);
      const parked = await world.barrierEntered();
      expect(parked.operationId).toBe(ask.operationId);
      // Remote dies and its parked core launch is killed before the core acts, so
      // nothing is dispatched (releasing it would let the dispatch run).
      const recover = restartRecovery(s.askerPage);
      await s.door.remote.kill();
      process.kill(parked.pid as number, 'SIGKILL');
      await restartRemote(world, s.door);
      // Recovery preserves the original uncertain state and leaves only read-only
      // recheck or abandon; no recipient effect or second core launch is allowed.
      expect(s.recipient.received()).toHaveLength(0);
      // The harness records the parked launch before core runs; it has no effect.
      expect(dispatches(world)).toHaveLength(1);
      await recover();
      await expect(askState(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-state',
        'uncertain',
        { timeout: 60_000 },
      );
      await s.askerPage.getByRole('button', { name: 'Check again' }).click();
      await expect(askState(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-state',
        'uncertain',
      );
      await expect(
        askEntry(s.askerPage, ask.operationId).getByRole('button', { name: 'Send', exact: true }),
      ).toHaveCount(0);
      await expect(
        s.askerPage
          .getByTestId('chat-panel')
          .getByRole('combobox', { name: 'Message', exact: true }),
      ).toHaveText('');
      await expect(
        s.askerPage.getByTestId('chat-panel').getByRole('button', { name: 'Send', exact: true }),
      ).toBeDisabled();
      await s.askerPage.getByRole('button', { name: 'Abandon tracking' }).click();
      await expect(askState(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-state',
        'abandoned',
      );
      // No effect: the recipient never received anything.
      expect(s.recipient.received()).toHaveLength(0);
      expect(dispatches(world)).toHaveLength(1);
    });
  });

  test(`colab restart keeps the ask and delivers the reply from the own stream once`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world, { gated: true });
      const draft = await composeChat(s.askerPage, s.recipient, 'Reply after a restart');
      const ask = await sendChat(s.askerPage, draft);
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      const requestId = s.recipient.received()[0].requestId as string;
      await restartColab(world, s.door);
      fs.writeFileSync(`${s.recipient.gate}/${requestId}.release`, '');
      // The open page loses its sync tunnel with Colab; reopening it resumes the
      // observer for the unresolved ask, which publishes the reply from Remote.
      await s.askerPage.reload();
      await openChat(s.askerPage);
      const reply = askEntry(s.askerPage, ask.operationId).getByTestId('ask-reply');
      await expect(reply).toHaveText(replyBody(draft.delivered()), { timeout: 30_000 });
      await expect(askEntry(s.askerPage, ask.operationId).getByTestId('ask-reply')).toHaveCount(1);
      expect(s.recipient.received()).toHaveLength(1);
    });
  });

  test(`revoking the asker device refuses a later send and creates no recipient work`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world);
      // Positive control: the same device can send before it is revoked.
      const draft = await composeChat(s.askerPage, s.recipient, 'Before revoke');
      const first = await sendChat(s.askerPage, draft);
      await expect(askEntry(s.askerPage, first.operationId)).toHaveAttribute(
        'data-ledger-state',
        'accepted',
      );
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      run(world, world.binaries.remote, ['devices', 'revoke', clientId(world, askerName)]);
      const before = dispatches(world).length;
      await s.askerPage.reload();
      await expect(s.askerPage.getByTestId('ask-send')).toHaveCount(0);
      expect(dispatches(world)).toHaveLength(before);
      expect(s.recipient.received()).toHaveLength(1);
    });
  });

  test(`two tabs of one paired browser stay live and share Ask replies without duplicate wake`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world);
      // The scenario's tab is A. Tab B is a second tab of the same paired browser.
      const tabA = s.askerPage;
      const tabB = await openPage(s.door, s.asker, s.page);
      await expect(tabA.getByRole('button', { name: /^Discussion \(/ })).toBeVisible();
      await expect(tabB.getByRole('button', { name: /^Discussion \(/ })).toBeVisible();
      const draft = await composeChat(tabB, s.recipient, 'Sent from the second tab');
      const ask = await sendChat(tabB, draft);
      await expect(askEntry(tabB, ask.operationId)).toHaveAttribute(
        'data-ledger-state',
        'accepted',
      );
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      await openChat(tabA);
      await expect(askEntry(tabA, ask.operationId)).toBeVisible();
      await expect(askEntry(tabA, ask.operationId).getByTestId('ask-reply')).toHaveText(
        replyBody(draft.delivered()),
      );
      await expect(tabB.getByRole('button', { name: /^Discussion \(/ })).toBeVisible();
      // Both live tabs see the same signed Ask record; opening either tab never resends.
      expect(s.recipient.received()).toHaveLength(1);
      expect(dispatches(world)).toHaveLength(1);
    });
  });

  test.fixme(`a held grant shows held until local approval, then accepted (waits for a Remote-provided hold fixture; held is covered by unit tests)`, async () => {
    // Needs a way to give the paired device mode "hold": Remote's own tests
    // seed it directly in Remote storage while serve is stopped (a test-only
    // step this suite has not adopted). Then: Send shows held, the recipient
    // has no row, `tmt-remote approve <operationId> --json` confirms the frozen
    // message, and the ask becomes accepted with exactly one received row.
  });
});
