import { homeList } from '../test/home-list.js';
import { pageAction } from '../test/page-actions.js';
import { generateKeyPairSync, randomUUID } from 'node:crypto';
import { mkdirSync } from 'node:fs';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { openReaderLink, pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, composeChat, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

/** Every mutation below traverses paired Remote admission and the real Colab engine. */
async function manage(page: Page) {
  await (await pageAction(page, 'Manage page')).click();
  const dialog = page.locator('dialog.management-dialog');
  await expect(dialog.getByRole('button', { name: /^Who can open this page/ })).toBeVisible();
  return dialog;
}

// The modal also hosts the page's reply announcer; only this body owns
// management progress and signed-log verification outcomes.
const managementStatus = (dialog: Locator) =>
  dialog.locator('.management-dialog-body').getByRole('status');

async function choose(dialog: Locator, label: string, option: string) {
  await dialog.getByRole('button', { name: new RegExp(`^${label}`) }).click();
  await dialog
    .getByRole('menu', { name: label })
    .getByRole('menuitemradio', { name: option })
    .click();
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
    // No cloud copy was ever confirmed in this world, so the page is only local: no cloud expiry
    // to show and nothing to warn of.
    expect(initialPage.expiresAtMs).toBeNull();
    expect(initialPage.warnings).toEqual([]);
    await expect(dialog.locator('.retention-hint')).toHaveText('local only');
    await expect(dialog.locator('.retention-hint')).not.toHaveAttribute('title');
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

    await dialog.getByLabel('Keep on this computer until I delete it').check();
    await dialog.getByRole('button', { name: 'Set access expiry', exact: true }).click();
    await dialog.getByRole('button', { name: 'Set access expiry', exact: true }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
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
    await choose(dialog, 'Who can open this page', 'Anyone with the link');
    await dialog.getByRole('button', { name: 'Allow access by link' }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await dialog.getByRole('button', { name: 'Create a sharing link…', exact: true }).click();
    await expect(dialog).toContainText('the browser opens it read-only whatever its permission.');
    const captureCopy = async (phase: 'confirm' | 'verified') => {
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 900 });
        for (const theme of ['light', 'dark']) {
          await page.evaluate((value) => {
            document.documentElement.dataset.theme = value;
          }, theme);
          await page.screenshot({
            path: testInfo.outputPath(`link-copy-${phase}-${width}-${theme}.png`),
            mask: [dialog.getByLabel('Share link', { exact: true })],
          });
        }
      }
      await page.setViewportSize({ width: 1280, height: 900 });
      await page.evaluate(() => {
        document.documentElement.dataset.theme = 'light';
      });
    };
    await captureCopy('confirm');
    await expect(dialog.getByLabel('Share link', { exact: true })).toHaveCount(0);
    await dialog.getByRole('button', { name: 'Create a sharing link', exact: true }).click();
    await expect(dialog.getByLabel('Share link', { exact: true })).toBeVisible();
    const shared = new URL(await dialog.getByLabel('Share link', { exact: true }).inputValue());
    expect(shared.origin).toBe(door.origin);
    expect(shared.pathname).toMatch(/^\/read\/[0-9a-f-]{36}$/);
    const capability = new URLSearchParams(shared.hash.slice(1));
    expect(capability.get('link')).toBe(shared.pathname.slice(6));
    expect(capability.get('page')).toBe(first.pageId);
    expect(capability.get('seed')).toMatch(/^[A-Za-z0-9_-]{43}$/);
    expect(shared.search).toBe('');
    await expect(dialog.getByLabel('Link seed')).toHaveCount(0);
    await expect(dialog.getByLabel('Link ID', { exact: true })).toHaveCount(0);
    const copiedReader = await openReaderLink(world, door, shared.href, 'copied-reader');
    await expect(copiedReader.page.frameLocator('iframe').locator('#quote')).toHaveText(
      'Management selection.',
    );
    expect(copiedReader.page.url()).toBe(`${shared.origin}${shared.pathname}`);
    await captureCopy('verified');
    const oldLink = shared.pathname.slice(6);
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await dialog.getByRole('button', { name: 'Replace link…', exact: true }).click();
    await dialog.getByRole('button', { name: 'Replace link', exact: true }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
    const reset = new URL(await dialog.getByLabel('Share link', { exact: true }).inputValue());
    expect(reset.pathname.slice(6)).not.toBe(oldLink);
    expect(reset.origin).toBe(door.origin);
    await expect(
      copiedReader.page.getByRole('heading', { name: 'Access ended', level: 2 }),
    ).toBeVisible();
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await choose(dialog, 'Who can open this page', 'Named members only');
    await expect(dialog).toContainText('Existing links stop working on these pages');
    await dialog.getByRole('button', { name: 'Stop access through existing links' }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(page.locator('iframe')).toBeVisible();
    await page.reload();
    await expect(page.getByRole('button', { name: /^Discussion \(/ })).toBeVisible();
    expect(recipient.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );

    dialog = await manage(page);
    await dialog.getByRole('button', { name: 'Archive and stop edits…', exact: true }).click();
    await dialog.getByRole('button', { name: 'Archive and stop edits', exact: true }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
    // Navigate explicitly: the archived page has no editing route.
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await page.goto(`${door.address}/x/colab/`);
    await expect(homeList(page)).toBeVisible();
    await page.getByRole('tab', { name: /^Archived/ }).click();
    const row = page.locator(`ul.pages li[data-page-id="${first.pageId}"]`);
    await expect(row).toContainText('Archived');
    await row.getByRole('button', { name: /^Actions for/ }).click();
    await page.getByRole('menuitem', { name: 'Manage page' }).click();
    dialog = page.locator('dialog.management-dialog');
    await expect(
      dialog.getByRole('button', { name: 'Archive and stop edits…', exact: true }),
    ).toBeDisabled();
    await dialog.getByRole('button', { name: 'Delete page…', exact: true }).click();
    await expect(dialog).toContainText('Copies already made cannot be taken back');
    await dialog.getByRole('button', { name: 'Delete page', exact: true }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await page.getByRole('tab', { name: 'All', exact: true }).click();
    const remaining = page.locator(`ul.pages li[data-page-id="${second.pageId}"]`);
    await remaining.getByRole('button', { name: /^Actions for/ }).click();
    await page.getByRole('menuitem', { name: 'Manage page' }).click();
    dialog = page.locator('dialog.management-dialog');
    await expect(dialog.getByRole('button', { name: 'Delete page…', exact: true })).toBeVisible();
    await dialog.getByRole('button', { name: 'Delete page…', exact: true }).click();
    await dialog.getByRole('button', { name: 'Delete page', exact: true }).click();
    await expect(dialog.getByRole('alert')).toContainText(
      'The owner accepted the deletion, but it is not confirmed yet',
    );
    await dialog.getByRole('button', { name: 'Check again' }).click();
    await expect(dialog.getByRole('alert')).toContainText(
      'The owner accepted the deletion, but it is not confirmed yet',
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
      await dialog.getByRole('button', { name: 'Give this member access…', exact: true }).click();
      await dialog.getByRole('button', { name: 'Give this member access', exact: true }).click();
      await expect(managementStatus(dialog)).toContainText('Done.');
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
    await choose(dialog, 'What this member can do', 'Edit');
    await dialog.getByRole('button', { name: 'Change member access', exact: true }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
    expect(detail().members.find((m: { id: string }) => m.id === shared).role).toBe('editor');
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await dialog.getByRole('button', { name: 'Remove this member’s access…', exact: true }).click();
    await dialog.getByRole('button', { name: "Confirm remove this member's access" }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
    expect(detail().members.find((m: { id: string }) => m.id === shared).revoked).toBe(true);
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await choose(dialog, 'What people who join later see', 'Current page only');
    await dialog.getByRole('button', { name: 'Change history', exact: true }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
    expect(detail().page.history).toBe('current');
    const beforeCurrent = BigInt(detail().page.epoch);
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await add();
    expect(BigInt(detail().page.epoch)).toBe(beforeCurrent + 1n);
    await dialog.getByRole('button', { name: 'Start new history…', exact: true }).click();
    await dialog.getByRole('button', { name: 'Start new history' }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
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
    await dialog.getByLabel('Keep on this computer until I delete it').check();
    await dialog.getByRole('button', { name: 'Set access expiry', exact: true }).click();
    await dialog.getByRole('button', { name: 'Set access expiry', exact: true }).click();
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
    await dialog.getByRole('button', { name: 'Retry this request' }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
    expect(bodies).toHaveLength(2);
    expect(bodies[1]).toBe(bodies[0]); // Includes original ID, signed framing, expiry and selected payload.
    expect(
      JSON.parse(run(world, world.binaries.colab, ['show', created.pageId, '--json']))
        .membershipHead,
    ).toEqual(committed.membershipHead);
    expect(world.coreCalls().filter((c) => c.operation === 'dispatch.create')).toHaveLength(0);
  });
});

test('archive commit reopens paired tabs read-only while retaining unsent Chat drafts', async () => {
  const info = test.info();
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const recipient = await world.startAgent('archive-recipient');
    const device = await pairBrowser(world, 'archive-browser');
    const created = createPage(
      world,
      'Archived readable page',
      '<p id="quote">Still readable after archive.</p>',
    );
    const page = await openPage(door, device, created);
    await composeChat(page, recipient, 'Keep this unsent archive draft');
    const chat = page.getByTestId('chat-panel');
    const expected = '@archive-recipient Keep this unsent archive draft';
    const second = await openPage(door, device, created);
    // Commit through the actual management modal; the other tab receives the sync invalidation.
    const dialog = await manage(second);
    await dialog.getByRole('button', { name: 'Archive and stop edits…', exact: true }).click();
    await dialog.getByRole('button', { name: 'Archive and stop edits', exact: true }).click();
    await expect(managementStatus(dialog)).toContainText('Done.');
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    for (const tab of [page, second]) {
      await expect(tab.locator('.status')).toContainText('Archived · Read only');
      await expect(tab.getByRole('button', { name: 'Agents', exact: true })).toHaveCount(0);
      await expect(tab.getByRole('button', { name: 'Selection mode', exact: true })).toHaveCount(0);
      await expect(
        tab.frameLocator('iframe').getByText('Still readable after archive.'),
      ).toBeVisible();
    }
    await expect(chat.getByText('Unsent draft', { exact: true })).toBeVisible();
    await expect(chat.locator('.archive-unsent-draft pre')).toHaveText(expected);
    await expect(
      chat.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
    ).toHaveCount(0);
    await page.evaluate(() => {
      Object.defineProperty(navigator, 'clipboard', {
        configurable: true,
        value: {
          writeText: async (value: string) => {
            (window as any).archiveCopy = value;
          },
        },
      });
    });
    await chat.getByRole('button', { name: 'Copy', exact: true }).click();
    expect(await page.evaluate(() => (window as any).archiveCopy)).toBe(expected);
    for (const [width, theme] of [
      [1440, 'light'],
      [390, 'dark'],
    ] as const) {
      await page.setViewportSize({ width, height: 900 });
      await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
      await page.screenshot({ path: info.outputPath(`archived-draft-${width}-${theme}.png`) });
    }
    await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
    for (const [width, theme] of [
      [1440, 'light'],
      [390, 'dark'],
    ] as const) {
      await page.setViewportSize({ width, height: 900 });
      await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
      await expect(page.locator('.status')).toContainText('Archived · Read only');
      await page.screenshot({ path: info.outputPath(`archived-page-${width}-${theme}.png`) });
    }
    await (await pageAction(second, 'Source')).click();
    await expect(second.getByRole('textbox', { name: 'Source' })).toHaveAttribute('readonly', '');
    await expect(second.getByRole('button', { name: 'Save', exact: true })).toHaveCount(0);
    expect(recipient.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
    await second.reload();
    await expect(second.locator('.status')).toContainText('Archived · Read only');
  });
});
