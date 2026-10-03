import { createHash } from 'node:crypto';
import fs from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { pairBrowser, restartColab, restartRemote, startDoor } from './harness/browser.js';
import {
  askEntry,
  askState,
  clientId,
  createPage,
  freePort,
  openPage,
  previewAsk,
  run,
  selectInRenderer,
  send,
} from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import type { AcceptanceWorld } from './harness/world.js';

// #1110 Ask agent real-binary acceptance. The Ask UI, Remote operations (#1497),
// the page (`tmt colab page create`, #1522), the recipient and `tmt reply` are all
// real; the page exists before the paired browsers register. The bodies pass against
// a build that includes #1517 and the Remote restart recovery; the cases are disabled
// here only until #1517 (the Remote operations SDK) is on this branch (AWAITS_1517).
// The held case waits for a Remote-provided hold fixture.
//
// Architecture: no native bridge ledger. The asker's browser calls Remote
// operations as its paired device and records the ask, its states and the
// reply in its own Colab stream. v1 is owner-only.

const AWAITS_1517 = ' (disabled until #1517, the Remote operations SDK, is on this branch)';
const PAGE_HTML = '<h1>Ask acceptance</h1><p id="quote">Exact selected sentence for the agent.</p>';

/** Recover the paired session after a Remote restart through the page's own Reconnect. */
async function reconnect(page: Page) {
  await page.getByRole('button', { name: 'Reconnect', exact: true }).click();
  await expect(page.getByTestId('ask-action')).toBeVisible({ timeout: 60_000 });
}

async function scenario(world: AcceptanceWorld, options: { gated?: boolean } = {}) {
  const door = await startDoor(world, await freePort());
  const recipient = await world.startAgent('ask-recipient', options);
  const asker = await pairBrowser(world, 'asker-browser');
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

  test.fixme(`direct send: previewed bytes reach the recipient exactly once and the reply shows in a second viewer${AWAITS_1517}`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world);
      await selectInRenderer(s.askerPage, '#quote');
      const ask = await previewAsk(s.askerPage, s.recipient.id, 'Explain this sentence');
      // The delivered preview starts with Remote's stable device-name line.
      expect(ask.previewText.startsWith(`[remote: ${askerName}]\n`)).toBe(true);
      await send(s.askerPage);
      await expect(askState(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-state',
        'accepted',
      );
      // Exact bytes: the recipient's text is the previewed text, once.
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      expect(s.recipient.received()).toHaveLength(1);
      expect(s.recipient.received()[0].message).toBe(ask.previewText);
      expect(dispatches(world)).toHaveLength(1);
      // The real `tmt reply` shows up on the asker's page, attributed to the agent,
      // and in a second paired viewer.
      const reply = replyBody(ask.previewText);
      const entry = askEntry(s.askerPage, ask.operationId);
      await expect(entry.getByTestId('ask-reply')).toHaveText(reply);
      await expect(entry.getByTestId('ask-reply-attribution')).toContainText(s.recipient.name);
      const second: Page = await openPage(s.door, s.viewer, s.page);
      const mirrored = askEntry(second, ask.operationId);
      await expect(mirrored.getByTestId('ask-reply')).toHaveText(reply);
      await expect(mirrored.getByTestId('ask-reply-attribution')).toContainText(s.recipient.name);
      // agents.list has presence only in v1: no delivery state is shown.
      await expect(
        s.askerPage.locator('[data-testid=ask-agent-option][data-delivery=unavailable]'),
      ).toHaveCount(0);
    });
  });

  test.fixme(`browser reload restores the ask from its own stream with the same operation ID and no second wake${AWAITS_1517}`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world, { gated: true });
      await selectInRenderer(s.askerPage, '#quote');
      const ask = await previewAsk(s.askerPage, s.recipient.id, 'Hold the reply');
      await send(s.askerPage);
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      const requestId = s.recipient.received()[0].requestId as string;
      await s.askerPage.reload();
      await expect(askState(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-state',
        'accepted',
      );
      // Releasing the gate lets the real reply flow; still one wake, one dispatch.
      fs.writeFileSync(`${s.recipient.gate}/${requestId}.release`, '');
      await expect(askEntry(s.askerPage, ask.operationId).getByTestId('ask-reply')).toHaveText(
        replyBody(ask.previewText),
      );
      expect(s.recipient.received()).toHaveLength(1);
      expect(dispatches(world)).toHaveLength(1);
    });
  });

  test.fixme(`remote restart after the core accepted recovers via operation.show with no second wake${AWAITS_1517}`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world);
      await selectInRenderer(s.askerPage, '#quote');
      const ask = await previewAsk(s.askerPage, s.recipient.id, 'Survive a restart');
      world.armBarrier(ask.operationId, 'after');
      await send(s.askerPage);
      await world.barrierEntered();
      // The real core has accepted; Remote dies before it can answer the browser.
      await s.door.remote.kill();
      world.releaseBarrier();
      await restartRemote(world, s.door);
      // Remote's sessions are in memory: the page shows "Sync disconnected" and the
      // explicit Reconnect reopens the paired session, then reloads. The restored
      // ask observes the original operation ID (read-only, never a resend).
      await reconnect(s.askerPage);
      await expect(askState(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-state',
        'accepted',
        { timeout: 60_000 },
      );
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      expect(s.recipient.received()).toHaveLength(1);
      expect(dispatches(world)).toHaveLength(1);
    });
  });

  test.fixme(`remote restart before dispatch stays uncertain with no new dispatch; abandon records MAY_HAVE_BEEN_DELIVERED${AWAITS_1517}`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world);
      await selectInRenderer(s.askerPage, '#quote');
      const ask = await previewAsk(s.askerPage, s.recipient.id, 'Never reaches the core');
      world.armBarrier(ask.operationId, 'before');
      await send(s.askerPage);
      const parked = await world.barrierEntered();
      // Remote dies and its parked core launch is killed before the core acts, so
      // nothing is dispatched (releasing it would let the dispatch run).
      await s.door.remote.kill();
      process.kill(parked.pid as number, 'SIGKILL');
      await restartRemote(world, s.door);
      // The in-flight send only times out after the SDK's own deadline, so the user
      // reconnects; the restored ask is uncertain, never resent, and operation.show
      // finds nothing: only re-check or abandon remain.
      await reconnect(s.askerPage);
      await expect(askState(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-state',
        'uncertain',
      );
      await s.askerPage.getByRole('button', { name: 'Re-check delivery' }).click();
      await expect(askState(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-state',
        'uncertain',
      );
      await expect(s.askerPage.getByRole('button', { name: 'Send', exact: true })).toHaveCount(0);
      await s.askerPage.getByRole('button', { name: 'Abandon tracking' }).click();
      await expect(askState(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-state',
        'abandoned',
      );
      // No effect: the recipient never received anything.
      expect(s.recipient.received()).toHaveLength(0);
    });
  });

  test.fixme(`colab restart keeps the ask and delivers the reply from the own stream once${AWAITS_1517}`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world, { gated: true });
      await selectInRenderer(s.askerPage, '#quote');
      const ask = await previewAsk(s.askerPage, s.recipient.id, 'Reply after a restart');
      await send(s.askerPage);
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      const requestId = s.recipient.received()[0].requestId as string;
      await restartColab(world, s.door);
      fs.writeFileSync(`${s.recipient.gate}/${requestId}.release`, '');
      // The open page loses its sync tunnel with Colab; reopening it resumes the
      // observer for the unresolved ask, which publishes the reply from Remote.
      await s.askerPage.reload();
      const reply = askEntry(s.askerPage, ask.operationId).getByTestId('ask-reply');
      await expect(reply).toHaveText(replyBody(ask.previewText), { timeout: 30_000 });
      await expect(askEntry(s.askerPage, ask.operationId).getByTestId('ask-reply')).toHaveCount(1);
      expect(s.recipient.received()).toHaveLength(1);
    });
  });

  test.fixme(`revoking the asker device refuses a later send and creates no recipient work${AWAITS_1517}`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world);
      await selectInRenderer(s.askerPage, '#quote');
      // Positive control: the same device can send before it is revoked.
      const first = await previewAsk(s.askerPage, s.recipient.id, 'Before revoke');
      await send(s.askerPage);
      await expect(askState(s.askerPage, first.operationId)).toHaveAttribute(
        'data-state',
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

  test.fixme(`two tabs of one paired browser: the newer tab takes the session, the older shows the notice, and "Use here" takes it back with no duplicate wake${AWAITS_1517}`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world);
      // The scenario's tab is A. Tab B is a second tab of the same paired browser.
      const tabA = s.askerPage;
      const tabB = await openPage(s.door, s.asker, s.page);
      // Remote keeps one session per device (tabs.spec.ts): B took it, so A stops
      // reconnecting and says so, with no automatic ping-pong.
      await expect(tabA.getByText('Colab is open in another tab')).toBeVisible();
      await expect(tabB.getByText('Colab is open in another tab')).toHaveCount(0);
      // An ask sent in the active tab B is accepted by Remote.
      await selectInRenderer(tabB, '#quote');
      const ask = await previewAsk(tabB, s.recipient.id, 'Sent from the active tab');
      await send(tabB);
      await expect(askState(tabB, ask.operationId)).toHaveAttribute('data-state', 'accepted');
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      // "Use here" in A takes the session back; the ask made in B is visible in A,
      // and B now shows the notice instead.
      await tabA.getByRole('button', { name: 'Use here' }).click();
      await expect(askEntry(tabA, ask.operationId)).toBeVisible();
      await expect(tabB.getByText('Colab is open in another tab')).toBeVisible();
      await expect(askEntry(tabA, ask.operationId).getByTestId('ask-reply')).toHaveText(
        replyBody(ask.previewText),
      );
      // Takeover never resends: one wake, one dispatch.
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
