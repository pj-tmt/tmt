import { pageAction } from '../test/page-actions.js';
import { generateKeyPairSync, randomUUID } from 'node:crypto';
import { mkdirSync } from 'node:fs';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, composeChat, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

/** Every mutation below traverses paired Remote admission and the real Colab engine. */
async function manage(page: Page) {
  await (await pageAction(page, 'Manage page')).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('combobox', { name: /^Audience/ })).toBeVisible();
  return dialog;
}

async function choose(dialog: Locator, label: string, option: string) {
  await dialog.getByRole('combobox', { name: new RegExp(`^${label}`) }).click();
  await dialog.getByRole('listbox', { name: label }).getByRole('option', { name: option }).click();
}

test.afterEach(disposeActiveWorlds);
test('native sharing and lifecycle verification preserve Ask, page recovery and last-page uncertainty', async () => {
  const testInfo = test.info();
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const recipient = await world.startAgent('management-recipient');
    const device = await pairBrowser(world, 'management-browser');
    const first = createPage(world, 'Managed page', '<p id="quote">Management selection.</p>');
    const second = createPage(world, 'Remaining page', '<p>Still readable.</p>');
    const page = await openPage(door, device, first);
    await composeChat(page, recipient, 'Do not send this draft');
    // Narrow page actions must open outside the header menu and return focus there.
    await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
    await page.setViewportSize({ width: 390, height: 844 });
    await page.getByRole('button', { name: 'More', exact: true }).click();
    let dialog = await manage(page);
    await expect(page.getByRole('button', { name: 'More', exact: true })).toHaveAttribute(
      'aria-expanded',
      'false',
    );
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(page.getByRole('button', { name: 'More', exact: true })).toBeFocused();
    await page.setViewportSize({ width: 1280, height: 900 });
    dialog = await manage(page);
    const initialCatalog = JSON.parse(run(world, world.binaries.colab, ['ls', '--json']));
    const initialPage = initialCatalog.pages.find(
      (item: { pageId: string }) => item.pageId === first.pageId,
    );
    expect(Number.isSafeInteger(initialPage.lastUpdateAtMs)).toBe(true);
    expect(initialPage.expiresAtMs).toBe(initialPage.lastUpdateAtMs + 30 * 86400000);
    expect(initialPage.warnings).toEqual([]);
    const expiry = await page.evaluate((value: number) => {
      const parts = new Intl.DateTimeFormat('en-US', {
        weekday: 'short',
        month: '2-digit',
        day: '2-digit',
        hour: '2-digit',
        minute: '2-digit',
        hourCycle: 'h23',
      }).formatToParts(new Date(value));
      const part = (type: Intl.DateTimeFormatPartTypes) =>
        parts.find((p) => p.type === type)?.value;
      return `${part('weekday')} ${part('month')}-${part('day')} ${part('hour')}:${part('minute')}`;
    }, initialPage.expiresAtMs);
    await expect(dialog.locator('.retention-hint')).toHaveAttribute('title', expiry);
    await expect(dialog.locator('.retention-hint')).toHaveText(/expires in (29|30) days/);
    for (const theme of ['light', 'dark']) {
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.screenshot({
        path: testInfo.outputPath(`management-${theme}.png`),
        fullPage: true,
      });
    }
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: testInfo.outputPath('management-mobile.png'), fullPage: true });
    expect(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
    const scrollCaptures = process.env.COLAB_MANAGE_SCROLL_CAPTURE_DIR;
    if (scrollCaptures) {
      mkdirSync(scrollCaptures, { recursive: true });
      const body = dialog.locator('.management-dialog-body');
      const close = dialog.getByRole('button', { name: 'Close', exact: true });
      for (const [width, theme] of [
        [1440, 'light'],
        [390, 'dark'],
      ] as const) {
        await page.setViewportSize({ width, height: 900 });
        await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
        await body.evaluate((node) => (node.scrollTop = 0));
        await expect(close).toBeInViewport({ ratio: 1 });
        await page.screenshot({ path: `${scrollCaptures}/manage-top-${width}-${theme}.png` });
        await body.evaluate((node) => (node.scrollTop = node.scrollHeight));
        expect(await body.evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
        await expect(close).toBeInViewport({ ratio: 1 });
        await page.screenshot({ path: `${scrollCaptures}/manage-bottom-${width}-${theme}.png` });
      }
      await body.evaluate((node) => (node.scrollTop = 0));
    }
    await page.setViewportSize({ width: 1280, height: 900 });

    await dialog.getByLabel('Keep forever').check();
    await dialog.getByRole('button', { name: 'Set retention', exact: true }).click();
    await dialog.getByRole('button', { name: 'Confirm set retention', exact: true }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    const forever = JSON.parse(
      run(world, world.binaries.colab, ['show', first.pageId, '--json']),
    ).page;
    expect(forever.lastUpdateAtMs).toBe(initialPage.lastUpdateAtMs);
    expect(forever.expiresAtMs).toBeNull();
    expect(forever.warnings).toEqual([]);
    await expect(page.getByTestId('ask-preview')).toHaveCount(0);
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(page.locator('iframe')).toBeVisible();
    expect(recipient.received()).toHaveLength(0);

    dialog = await manage(page);
    await choose(dialog, 'Audience', 'Link');
    await dialog.getByRole('button', { name: 'Confirm make link' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await dialog.getByRole('button', { name: 'Create link', exact: true }).click();
    await expect(dialog).toContainText('Shared links open read-only, whatever their role.');
    const captureCopy = async (phase: 'confirm' | 'verified') => {
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 900 });
        for (const theme of ['light', 'dark']) {
          await page.evaluate((value) => {
            document.documentElement.dataset.theme = value;
          }, theme);
          await page.screenshot({
            path: testInfo.outputPath(`link-copy-${phase}-${width}-${theme}.png`),
            mask: [dialog.getByLabel('Link seed')],
          });
        }
      }
      await page.setViewportSize({ width: 1280, height: 900 });
      await page.evaluate(() => {
        document.documentElement.dataset.theme = 'light';
      });
    };
    await captureCopy('confirm');
    await expect(dialog.getByLabel('Link seed')).toHaveCount(0);
    await dialog.getByRole('button', { name: 'Confirm create link' }).click();
    await expect(dialog.getByLabel('Link seed')).toHaveValue(/^[A-Za-z0-9_-]{43}$/);
    await expect(dialog).toContainText('This dialog does not create a URL to open.');
    await captureCopy('verified');
    const oldLink = await dialog.getByLabel('Link ID', { exact: true }).inputValue();
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await dialog.getByRole('button', { name: 'Reset link', exact: true }).click();
    await dialog.getByRole('button', { name: 'Confirm reset link' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    expect(await dialog.getByLabel('Link ID', { exact: true }).inputValue()).not.toBe(oldLink);
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await choose(dialog, 'Audience', 'Private');
    await expect(dialog).toContainText('links are revoked and affected pages rotate');
    await dialog.getByRole('button', { name: 'Confirm make private' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(page.locator('iframe')).toBeVisible();
    await page.reload();
    await expect(page.getByRole('button', { name: /^Discussion \(/ })).toBeVisible();
    expect(recipient.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );

    dialog = await manage(page);
    await dialog.getByRole('button', { name: 'Archive page', exact: true }).click();
    await dialog.getByRole('button', { name: 'Confirm archive page' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    // Navigate explicitly: the archived page has no editing route.
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await page.goto(`${door.address}/x/colab/`);
    await page.getByRole('button', { name: 'Show archived', exact: true }).click();
    const row = page.locator('ul.pages li').filter({ hasText: first.pageId });
    await expect(row).toContainText('Archived');
    await row.locator('summary').click();
    await row.getByRole('button', { name: 'Manage page' }).click();
    dialog = page.getByRole('dialog');
    await expect(dialog.getByRole('button', { name: 'Archive page', exact: true })).toBeDisabled();
    await dialog.getByRole('button', { name: 'Delete page', exact: true }).click();
    await expect(dialog).toContainText('Copies already made cannot be recalled');
    await dialog.getByRole('button', { name: 'Confirm delete page' }).click();
    await expect(dialog.getByRole('status')).toContainText('Deletion verified');
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await page.getByRole('button', { name: 'Show archived', exact: true }).click();
    const remaining = page.locator('ul.pages li').filter({ hasText: second.pageId });
    await remaining.locator('summary').click();
    await remaining.getByRole('button', { name: 'Manage page' }).click();
    dialog = page.getByRole('dialog');
    await expect(dialog.getByRole('button', { name: 'Delete page', exact: true })).toBeVisible();
    await dialog.getByRole('button', { name: 'Delete page', exact: true }).click();
    await dialog.getByRole('button', { name: 'Confirm delete page' }).click();
    await expect(dialog.getByRole('alert')).toContainText(
      'Deletion acknowledged, awaiting verification',
    );
    await dialog.getByRole('button', { name: 'Verify signed log' }).click();
    await expect(dialog.getByRole('alert')).toContainText(
      'Deletion acknowledged, awaiting verification',
    );
    const catalog = JSON.parse(run(world, world.binaries.colab, ['ls', '--archived', '--json']));
    expect(catalog.pages).toHaveLength(0);
    expect(recipient.received()).toHaveLength(0);
  });
});

// L6: browser intent is checked against the real owner's verified CLI projection.
test('member roles, shared/current joins and epoch advance admit the same owner policy', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const device = await pairBrowser(world, 'member-policy-browser');
    const created = createPage(
      world,
      'Member policy',
      '<p>Current baseline survives policy changes.</p>',
    );
    const page = await openPage(door, device, created);
    const detail = () =>
      JSON.parse(run(world, world.binaries.colab, ['show', created.pageId, '--json']));
    let dialog = await manage(page);
    const initialEpoch = BigInt(detail().page.epoch);
    const add = async () => {
      const id = randomUUID();
      const key = (type: 'ed25519' | 'x25519') =>
        (type === 'ed25519'
          ? generateKeyPairSync('ed25519')
          : generateKeyPairSync('x25519')
        ).publicKey
          .export({ type: 'spki', format: 'der' })
          .subarray(-32)
          .toString('base64url');
      await dialog.getByLabel('Member ID', { exact: true }).fill(id);
      await dialog.getByLabel('Signing public key').fill(key('ed25519'));
      await dialog.getByLabel('Encryption public key').fill(key('x25519'));
      await dialog.getByRole('button', { name: 'Add member', exact: true }).click();
      await dialog.getByRole('button', { name: 'Confirm add member', exact: true }).click();
      await expect(dialog.getByRole('status')).toContainText('Change verified');
      expect(detail().members.find((m: { id: string }) => m.id === id)).toMatchObject({
        role: 'viewer',
        pages: [created.pageId],
        revoked: false,
      });
      await dialog.getByRole('button', { name: 'Manage another change' }).click();
      return id;
    };
    const shared = await add();
    expect(BigInt(detail().page.epoch)).toBe(initialEpoch);
    await choose(dialog, 'Member role', 'editor');
    await dialog.getByRole('button', { name: 'Confirm change member role' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    expect(detail().members.find((m: { id: string }) => m.id === shared).role).toBe('editor');
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await dialog.getByRole('button', { name: 'Remove member', exact: true }).click();
    await dialog.getByRole('button', { name: 'Confirm remove member' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    expect(detail().members.find((m: { id: string }) => m.id === shared).revoked).toBe(true);
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await choose(dialog, 'History mode', 'Current');
    await dialog.getByRole('button', { name: 'Confirm change history' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    expect(detail().page.history).toBe('current');
    const beforeCurrent = BigInt(detail().page.epoch);
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await add();
    expect(BigInt(detail().page.epoch)).toBe(beforeCurrent + 1n);
    await dialog.getByRole('button', { name: 'Advance epoch', exact: true }).click();
    await dialog.getByRole('button', { name: 'Confirm advance epoch' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    expect(BigInt(detail().page.epoch)).toBe(beforeCurrent + 2n);
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await page.reload();
    await expect(
      page.frameLocator('iframe').getByText('Current baseline survives policy changes.'),
    ).toBeVisible();
    expect(world.coreCalls().filter((c) => c.operation === 'dispatch.create')).toHaveLength(0);
  });
});

test('a lost management reply permits only an explicit byte-identical retry of the original operation', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const device = await pairBrowser(world, 'management-retry-browser');
    const created = createPage(world, 'Frozen management retry', '<p>No agent work.</p>');
    const page = await openPage(door, device, created);
    const dialog = await manage(page);
    const before = JSON.parse(run(world, world.binaries.colab, ['show', created.pageId, '--json']));
    const bodies: string[] = [];
    let acknowledgment:
      | { operationId: string; membershipHead: { revision: string; statementHash: string } }
      | undefined;
    await device.context.route('**/api/management', async (route) => {
      bodies.push(route.request().postData()!);
      const response = await route.fetch(); // Commit at the real owner before losing the first reply.
      expect(response.status()).toBe(200);
      const result = await response.json();
      if (bodies.length === 1) {
        acknowledgment = result;
        await route.abort('failed');
      } else {
        expect(result).toEqual(acknowledgment);
        await route.fulfill({ response });
      }
    });
    await dialog.getByLabel('Keep forever').check();
    await dialog.getByRole('button', { name: 'Set retention', exact: true }).click();
    await dialog.getByRole('button', { name: 'Confirm set retention', exact: true }).click();
    await expect(dialog.getByRole('alert')).toContainText('Result unknown');
    expect(bodies).toHaveLength(1);
    const committed = JSON.parse(
      run(world, world.binaries.colab, ['show', created.pageId, '--json']),
    );
    expect(committed.page.retentionDays).toBeNull();
    expect(committed.membershipHead).toEqual(acknowledgment!.membershipHead);
    expect(BigInt(committed.membershipHead.revision)).toBe(
      BigInt(before.membershipHead.revision) + 1n,
    );
    await dialog.getByRole('button', { name: 'Retry exact request' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    expect(bodies).toHaveLength(2);
    expect(bodies[1]).toBe(bodies[0]); // Includes original ID, signed framing, expiry and selected payload.
    expect(
      JSON.parse(run(world, world.binaries.colab, ['show', created.pageId, '--json']))
        .membershipHead,
    ).toEqual(committed.membershipHead);
    expect(world.coreCalls().filter((c) => c.operation === 'dispatch.create')).toHaveLength(0);
  });
});
