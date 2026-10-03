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
import { withWorld } from './harness/with-world.js';
import type { AcceptanceWorld } from './harness/world.js';

// #1110 Ask agent real-binary acceptance. The ask UI, Remote operations
// (#1497), the recipient and `tmt reply` are all real. Every case is
// test.fixme only because v1 has no product `tmt colab page create` yet
// (createPage in harness/ask.ts fails visibly); the bodies are the acceptance
// and are enabled by removing fixme once that command exists, never by a stand-in.
//
// Architecture: no native bridge ledger. The asker's browser calls Remote
// operations as its paired device and records the ask, its states and the
// reply in its own Colab stream. v1 is owner-only.

const PENDING = 'tmt colab page create';
const PAGE_HTML = '<h1>Ask acceptance</h1><p id="quote">Exact selected sentence for the agent.</p>';

async function scenario(world: AcceptanceWorld, options: { gated?: boolean } = {}) {
  const door = await startDoor(world, await freePort());
  const recipient = await world.startAgent('ask-recipient', options);
  const pageId = createPage(world, PAGE_HTML);
  const asker = await pairBrowser(world, 'asker-browser');
  const viewer = await pairBrowser(world, 'viewer-browser');
  const askerPage = await openPage(world, door, asker, pageId);
  return { door, recipient, pageId, asker, viewer, askerPage };
}

const replyBody = (message: string) =>
  `ask-reply:${createHash('sha256').update(message).digest('hex').slice(0, 16)}`;
const dispatches = (world: AcceptanceWorld) =>
  world.coreCalls().filter((call) => call.operation === 'dispatch.create');
const askerName = 'asker-browser';

test.describe('Ask agent real-binary acceptance (#1110)', () => {
  test.fixme(`direct send: previewed bytes reach the recipient exactly once and the reply shows in a second viewer (needs ${PENDING})`, async () => {
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
      const second: Page = await openPage(world, s.door, s.viewer, s.pageId);
      const mirrored = askEntry(second, ask.operationId);
      await expect(mirrored.getByTestId('ask-reply')).toHaveText(reply);
      await expect(mirrored.getByTestId('ask-reply-attribution')).toContainText(s.recipient.name);
      // agents.list has presence only in v1: no delivery state is shown.
      await expect(
        s.askerPage.locator('[data-testid=ask-agent-option][data-delivery=unavailable]'),
      ).toHaveCount(0);
    });
  });

  test.fixme(`browser reload restores the ask from its own stream with the same operation ID and no second wake (needs ${PENDING})`, async () => {
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

  test.fixme(`remote restart after the core accepted recovers via operation.show with no second wake (needs ${PENDING})`, async () => {
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
      await expect(askState(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-state',
        'uncertain',
      );
      await restartRemote(world, s.door);
      // Read-only re-check by the same operation ID: accepted, never a resend.
      await s.askerPage.getByRole('button', { name: 'Re-check delivery' }).click();
      await expect(askState(s.askerPage, ask.operationId)).toHaveAttribute(
        'data-state',
        'accepted',
      );
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      expect(s.recipient.received()).toHaveLength(1);
      expect(dispatches(world)).toHaveLength(1);
    });
  });

  test.fixme(`remote restart before dispatch stays uncertain with no new dispatch; abandon records MAY_HAVE_BEEN_DELIVERED (needs ${PENDING})`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world);
      await selectInRenderer(s.askerPage, '#quote');
      const ask = await previewAsk(s.askerPage, s.recipient.id, 'Never reaches the core');
      world.armBarrier(ask.operationId, 'before');
      await send(s.askerPage);
      await world.barrierEntered();
      await s.door.remote.kill();
      world.releaseBarrier();
      await restartRemote(world, s.door);
      // operation.show finds nothing: the ask stays uncertain; only re-check or abandon.
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
      expect(s.recipient.received()).toHaveLength(0);
      expect(dispatches(world).filter((c) => c.operationId === ask.operationId)).toHaveLength(0);
    });
  });

  test.fixme(`colab restart keeps the ask and delivers the reply from the own stream once (needs ${PENDING})`, async () => {
    await withWorld(async (world) => {
      const s = await scenario(world, { gated: true });
      await selectInRenderer(s.askerPage, '#quote');
      const ask = await previewAsk(s.askerPage, s.recipient.id, 'Reply after a restart');
      await send(s.askerPage);
      await until(() => s.recipient.received().length === 1, 'recipient received the ask');
      const requestId = s.recipient.received()[0].requestId as string;
      await restartColab(world, s.door);
      fs.writeFileSync(`${s.recipient.gate}/${requestId}.release`, '');
      const reply = askEntry(s.askerPage, ask.operationId).getByTestId('ask-reply');
      await expect(reply).toHaveText(replyBody(ask.previewText));
      await expect(askEntry(s.askerPage, ask.operationId).getByTestId('ask-reply')).toHaveCount(1);
      expect(s.recipient.received()).toHaveLength(1);
    });
  });

  test.fixme(`revoking the asker device refuses a later send and creates no recipient work (needs ${PENDING})`, async () => {
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

  test.fixme(`two tabs of one paired browser stay live, share asks, and a Colab restart in one does not end the other's Remote session (needs ${PENDING} and one Remote session shared between tabs)`, async () => {
    // Remote allows one session per device (tabs.spec.ts pins it), so tabs must
    // share a single session; today each tab opens its own and they end each
    // other's. Steps: open the page in tab A and tab B of the same paired
    // browser; send an ask in A and see its entry (same operation ID) in B;
    // restart tmt-colab; both tabs reconnect; B can still send a second ask
    // accepted by Remote (its session was not ended by A's reconnect), and
    // the recipient has exactly two received rows.
  });

  test.fixme(`a held grant shows held until local approval, then accepted (needs ${PENDING} and a hold grant fixture for the device)`, async () => {
    // Needs a way to give the paired device mode "hold": Remote's own tests
    // seed it directly in Remote storage while serve is stopped (a test-only
    // step this suite has not adopted). Then: Send shows held, the recipient
    // has no row, `tmt-remote approve <operationId> --json` confirms the frozen
    // message, and the ask becomes accepted with exactly one received row.
  });
});
