import { expect, test, type Page, type BrowserContext, type Locator } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
import { homeList, showAllPages } from '../test/home-list.js';
import { text } from '../src/strings.js';
import * as c from '@tmt/colab-client';

// Keep only error diagnostics: other console levels can include fixture data.
const pageErrors = new WeakMap<Page, string[]>();
test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  pageErrors.set(page, errors);
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
});
test.afterEach(async ({ page }, info) => {
  if (info.status !== info.expectedStatus) {
    await info.attach('page-console-errors', {
      body: (pageErrors.get(page) ?? []).join('\n'),
      contentType: 'text/plain',
    });
  }
});

const mount = '/r/abcdefghijklmnop/x/colab/';
const device = '00000000-0000-4000-8000-000000000100';
const member = '00000000-0000-4000-8000-000000000101';
const pageId = '00000000-0000-4000-8000-000000000102';
const json = (v: unknown) => c.text(JSON.stringify(v));
async function choose(dialog: Locator, label: string, option: string) {
  await dialog.getByRole('button', { name: new RegExp(`^${label}`) }).click();
  await dialog
    .getByRole('menu', { name: label })
    .getByRole('menuitemradio', { name: option })
    .click();
}
async function captureListbox(page: Page, name: string) {
  const directory = process.env.COLAB_LISTBOX_CAPTURE_DIR;
  if (!directory) return;
  await mkdir(directory, { recursive: true });
  for (const theme of ['light', 'dark'] as const) {
    await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      await page.screenshot({ path: `${directory}/${name}-${theme}-${width}.png`, fullPage: true });
    }
  }
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.evaluate(() => (document.documentElement.dataset.theme = 'light'));
}
async function fixture(
  page: Page | BrowserContext,
  tamper = false,
  empty = false,
  clock = Date.now,
) {
  const owner = (await crypto.subtle.generateKey('Ed25519', false, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  const ownerKey = new Uint8Array(await crypto.subtle.exportKey('raw', owner.publicKey));
  const issuer = (await crypto.subtle.generateKey('Ed25519', false, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  const issuerKey = new Uint8Array(await crypto.subtle.exportKey('raw', issuer.publicKey));
  const space = await c.deriveSpaceId(ownerKey);
  const signed = async (operation: string, payload: unknown, previous: c.statement.Head | null) => {
    const bytes = json(payload),
      header = c.statement.input({
        space,
        operation,
        revision: previous ? String(previous.revision + 1n) : '1',
        previousHash: previous?.hash ?? new Uint8Array(32),
        payloadDigest: await c.digest(bytes),
      });
    return c.statement.Envelope.fromJson(
      json({
        statement: c.encodeBinary(header),
        payload: c.encodeBinary(bytes),
        signature: c.encodeBinary(await c.sign(owner.privateKey, header)),
      }),
    );
  };
  const genesis = await signed(
    'member.add',
    {
      memberId: member,
      role: 'editor',
      signKey: c.encodeBinary(issuerKey),
      encKey: c.encodeBinary(new Uint8Array(32).fill(9)),
      pages: [],
    },
    null,
  );
  const g = await genesis.verifyNext(space, ownerKey, null);
  const shared = await signed('page.share', { pageId, mode: 'private', epoch: '1' }, g.head);
  const head = await shared.verifyNext(space, ownerKey, g.head);
  await page.addInitScript((mount) => {
    const install = () => {
      const meta = document.createElement('meta');
      meta.name = 'tmt-colab-mount';
      meta.content = mount;
      document.head.append(meta);
    };
    if (document.head) install();
    else document.addEventListener('readystatechange', install, { once: true });
  }, mount);
  await page.route('**/sdk/remote-v1.js*', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `
    export async function reopenSession() {sessionStorage.setItem('test:reopens',String(Number(sessionStorage.getItem('test:reopens')??0)+1));}
    export function transportUrl(_session,url){return String(url);}
    export async function certifyKey(purpose, bytes) {
      return {publicKey:btoa(String.fromCharCode(...bytes)).replaceAll('+','-').replaceAll('/','_').replace(/=+$/,''),
        issuedAtMs:Date.now(),signature:'${c.encodeBinary(new Uint8Array(64))}'};
    }`,
    }),
  );
  await page.route(`**${mount}api/session`, (route) =>
    route.fulfill({
      json: {
        deviceId: device,
        publicKey: c.encodeBinary(ownerKey),
        grantRevision: '1',
        name: 'Test browser',
      },
    }),
  );
  await page.route(`**${mount}api/pages`, (route) =>
    route.fulfill({
      json: {
        spaceId: space,
        ownerKey: c.encodeBinary(ownerKey),
        revision: '2',
        projects: [],
        pageIds: empty ? [] : [{ pageId, deleted: false }],
        pages: empty
          ? []
          : [
              {
                pageId,
                epoch: '1',
                sharing: 'private',
                history: 'current',
                archived: false,
                retentionDays: 30,
                lastUpdateAtMs: null,
                expiresAtMs: null,
                warnings: [],
                projects: [],
              },
            ],
      },
    }),
  );
  let deviceKey: Uint8Array;
  await page.route(`**${mount}api/devices/register`, async (route) => {
    const v = route.request().postDataJSON();
    deviceKey = c.binary(v.sign.publicKey, 32, 32);
    expect(Object.keys(v).sort()).toEqual(['deviceId', 'enc', 'sign']);
    const input = c.certificate.input({
      space,
      issuerKind: 'member',
      issuerId: member,
      deviceId: device,
      signingKey: c.binary(v.sign.publicKey, 32, 32),
      encryptionKey: c.binary(v.enc.publicKey, 32, 32),
      membershipRevision: '1',
      issuedAt: clock() - 1000,
      expiresAt: clock() + 60000,
    });
    await route.fulfill({
      json: {
        issuerStatement: JSON.parse(c.decodeText(genesis.toJson())),
        chain: {
          version: 1,
          issuerStatement: c.encodeBinary(g.head.hash),
          deviceCertificate: c.encodeBinary(input),
          issuerSignature: c.encodeBinary(
            tamper ? new Uint8Array(64) : await c.sign(issuer.privateKey, input),
          ),
        },
      },
    });
  });
  await page.routeWebSocket(`**${mount}sync`, (socket) =>
    socket.onMessage((message) => {
      const hello = JSON.parse(String(message));
      if (hello.type !== 'hello') return;
      socket.send(
        JSON.stringify({
          version: 1,
          type: 'catchup',
          space,
          page: pageId,
          epoch: '1',
          streams: [],
          more: true,
          baseline: null,
          membershipHead: {
            revision: '2',
            statementHash: c.encodeBinary(head.head.hash),
            ownerKey: c.encodeBinary(ownerKey),
            statements: [genesis, shared]
              .slice(Number(hello.membershipRevision))
              .map((v) => c.encodeBinary(v.toJson())),
            more: false,
          },
        }),
      );
    }),
  );
  return { space, ownerKey, genesis, shared, head, signed, deviceKey: () => deviceKey };
}

test('mounted owner discovers a pinned space; registration failure stays blocked', async ({
  page,
}) => {
  await fixture(page);
  await page.goto(mount);
  await expect(homeList(page)).toBeVisible();
  await expect(page).toHaveURL(/\/colab\/$/);
  await showAllPages(page);
  await expect(page.locator(`[data-page-id="${pageId}"] a`)).toBeVisible();
  await page.route(`**${mount}api/devices/register`, (route) =>
    route.fulfill({ status: 403, json: { code: 'DENIED' } }),
  );
  await page.reload();
  await expect(page.getByRole('alert')).toContainText('Could not open this paired space');
  await expect(page.getByRole('button', { name: 'Reload' })).toBeVisible();
  await expect(page.locator('iframe')).toHaveCount(0);
});

test('discovery cannot silently replace an existing mount pin or conflicting fragment', async ({
  page,
}) => {
  await fixture(page);
  await page.goto(mount);
  await expect(page).toHaveURL(/\/colab\/$/);
  await showAllPages(page);
  await expect(page.locator(`[data-page-id="${pageId}"] a`)).toBeVisible();
  await page.goto(`/colab/#space=${'a'.repeat(32)}`);
  await expect(page.getByRole('alert')).toContainText('does not match the pinned space');
  await expect(page.locator('iframe')).toHaveCount(0);
  await page.goto(`${mount}#space=${'a'.repeat(32)}`);
  await expect(page.getByRole('alert')).toContainText('Could not open this paired space');
  await expect(page.locator('iframe')).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole('alert')).toContainText('Could not open this paired space');
});

test('verified metadata with no wraps keeps the page blocked and opens no renderer', async ({
  page,
}) => {
  const f = await fixture(page);
  await page.routeWebSocket(`**${mount}sync`, (socket) => {
    socket.onMessage((message) => {
      const hello = JSON.parse(String(message));
      if (hello.type === 'ack') return;
      if (hello.type === 'subscribe') {
        expect(hello.cursors).toEqual([]);
        return;
      }
      expect(hello).toEqual({
        version: 1,
        type: 'hello',
        space: f.space,
        page: pageId,
        epoch: '1',
        device,
        membershipRevision: hello.membershipRevision,
        cursors: [],
      });
      const common = { version: 1, type: 'catchup', space: f.space, page: pageId, epoch: '1' };
      socket.send(
        JSON.stringify({
          ...common,
          membershipHead: {
            revision: '2',
            statementHash: c.encodeBinary(f.head.head.hash),
            ownerKey: c.encodeBinary(f.ownerKey),
            statements: [f.genesis, f.shared]
              .slice(Number(hello.membershipRevision))
              .map((v) => c.encodeBinary(v.toJson())),
            more: false,
          },
          baseline: null,
          streams: [],
          more: true,
        }),
      );
      socket.send(JSON.stringify({ ...common, wraps: [], streams: [], more: true }));
      socket.send(JSON.stringify({ ...common, streams: [], more: false }));
    });
  });
  await page.goto(mount);
  await showAllPages(page);
  await page.locator(`[data-page-id="${pageId}"] a`).click();
  await expect(page.getByRole('alert')).toContainText('no verified page key');
  // The visible location keeps the short page path, with no space or route in the fragment.
  expect(new URL(page.url()).pathname).toBe(`/p/${pageId.slice(0, 8)}`);
  expect(new URL(page.url()).hash).toBe('');
  await expect(page.locator('iframe')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Source', exact: true })).toHaveCount(0);
  await expect(page.locator('.page-header-actions')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Export page' })).toHaveCount(0);
});

test('a successful HTTP registration with a forged certificate remains blocked', async ({
  page,
}) => {
  await fixture(page, true);
  await page.goto(mount);
  await expect(page.getByRole('alert')).toContainText('Could not open this paired space');
  await expect(page.locator('iframe')).toHaveCount(0);
  expect(
    await page.evaluate(async () => {
      const path = '/src/storage.ts';
      const { record } = await import(path);
      return record(`pin:${location.origin}/r/abcdefghijklmnop/x/colab/`);
    }),
  ).toBeUndefined();
});

test('another tab opens while the first registration is still pending', async ({
  page,
  context,
}) => {
  await fixture(context);
  let started!: () => void, release!: () => void;
  const registered = new Promise<void>((resolve) => {
    started = resolve;
  });
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let requests = 0;
  await context.route(`**${mount}api/devices/register`, async (route) => {
    if (++requests === 1) {
      started();
      await gate;
    }
    await route.fallback();
  });
  const other = await context.newPage();
  const reopens = (tab: Page) =>
    tab.evaluate(() => Number(sessionStorage.getItem('test:reopens') ?? 0));
  try {
    await page.goto(mount);
    await registered;
    await other.goto(mount);
    await expect(homeList(other)).toBeVisible();
    await expect(page.getByText('Opening space')).toBeVisible();
    expect(await reopens(page)).toBe(1);
    expect(await reopens(other)).toBe(1);
    release();
    await expect(homeList(page)).toBeVisible();
    await expect(homeList(other)).toBeVisible();
    expect(await reopens(page)).toBe(1);
    expect(await reopens(other)).toBe(1);
    expect(requests).toBe(2);
  } finally {
    release();
    await other.close();
  }
});

test('empty owner space names the product page-create command', async ({ page }) => {
  await fixture(page, false, true);
  await page.goto(mount);
  await expect(homeList(page)).toBeVisible();
  await expect(page.getByText('No pages in this space yet.', { exact: false })).toContainText(
    'tmt colab page create --title "Notes" --layout column',
  );
  await expect(page.locator('ul.pages')).toHaveCount(0);
  await expect(page.locator('iframe')).toHaveCount(0);
});

test('trusted sharing confirms narrowing, retries frozen bytes and exposes a new seed only after verification', async ({
  page,
}, testInfo) => {
  const f = await fixture(page),
    log = [f.genesis, f.shared];
  let head = f.head.head,
    sharing = 'private',
    rejected = false,
    lost = false;
  await page.route(`**${mount}api/pages`, (route) =>
    route.fulfill({
      json: {
        spaceId: f.space,
        ownerKey: c.encodeBinary(f.ownerKey),
        revision: String(head.revision),
        projects: [],
        pageIds: [{ pageId, deleted: false }],
        pages: [
          {
            pageId,
            epoch: '1',
            sharing,
            history: 'shared',
            archived: false,
            retentionDays: 30,
            lastUpdateAtMs: null,
            expiresAtMs: null,
            warnings: [],
            projects: [],
          },
        ],
      },
    }),
  );
  await page.routeWebSocket(`**${mount}sync`, (socket) =>
    socket.onMessage((message) => {
      const hello = JSON.parse(String(message));
      if (hello.type !== 'hello') return;
      socket.send(
        JSON.stringify({
          version: 1,
          type: 'catchup',
          space: f.space,
          page: pageId,
          epoch: '1',
          streams: [],
          more: true,
          baseline: null,
          membershipHead: {
            revision: String(head.revision),
            statementHash: c.encodeBinary(head.hash),
            ownerKey: c.encodeBinary(f.ownerKey),
            statements: log
              .slice(Number(hello.membershipRevision))
              .map((v) => c.encodeBinary(v.toJson())),
            more: false,
          },
        }),
      );
    }),
  );
  let previous = '',
    result: unknown;
  await page.route(`**${mount}api/management`, async (route) => {
    const body = route.request().postData()!,
      request = JSON.parse(body),
      bytes = c.binary(request.request, 1024),
      fields = c.fields(bytes, 11),
      operation = c.decodeText(fields[6]),
      selection = JSON.parse(c.decodeText(c.binary(request.payload, 16384)));
    expect(await c.strictVerify(f.deviceKey(), c.binary(request.signature, 64, 64), bytes)).toBe(
      true,
    );
    expect(c.decodeText(fields[2])).toBe(f.space);
    expect(c.decodeText(fields[3])).toBe(pageId);
    if (!rejected) {
      rejected = true;
      await route.fulfill({ status: 503, json: { code: 'UNAVAILABLE' } });
      return;
    }
    if (body === previous) {
      await route.fulfill({ json: result });
      return;
    }
    expect(c.decodeText(fields[4])).toBe(String(head.revision));
    const payload =
      operation === 'link.add'
        ? {
            linkId: selection.linkId,
            role: selection.role,
            pages: selection.pages,
            linkSignKey: c.encodeBinary(f.ownerKey),
            linkEncKey: c.encodeBinary(new Uint8Array(32).fill(9)),
          }
        : { ...selection, epoch: '1' };
    const envelope = await f.signed(operation, payload, head);
    head = (await envelope.verifyNext(f.space, f.ownerKey, head)).head;
    log.push(envelope);
    if (operation === 'page.share') sharing = selection.mode;
    previous = body;
    result = {
      operationId: c.decodeText(fields[5]),
      membershipHead: { revision: String(head.revision), statementHash: c.encodeBinary(head.hash) },
    };
    if (!lost) {
      lost = true;
      await route.abort();
    } else await route.fulfill({ json: result });
  });
  await page.goto(mount);
  await expect(homeList(page)).toBeVisible();
  await showAllPages(page);
  await page
    .locator('.pages li')
    .getByRole('button', { name: /^Actions for/ })
    .click();
  await page.getByRole('menuitem', { name: 'Manage page' }).click();
  const dialog = page.locator('dialog.management-dialog');
  const audience = dialog.getByRole('button', { name: /^Who can open this page/ });
  await expect(audience).toBeVisible();
  const managementBody = dialog.locator('.management-dialog-body');
  const closeManagement = dialog.getByRole('button', { name: 'Close', exact: true });
  const originalViewport = page.viewportSize()!;
  const scrollCaptures = process.env.COLAB_MANAGE_SCROLL_CAPTURE_DIR;
  if (scrollCaptures) await mkdir(scrollCaptures, { recursive: true });
  for (const [width, theme] of [
    [1440, 'light'],
    [390, 'dark'],
  ] as const) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
    await managementBody.evaluate((node) => (node.scrollTop = 0));
    await expect(closeManagement).toBeInViewport({ ratio: 1 });
    const closeAtTop = await closeManagement.boundingBox();
    if (scrollCaptures)
      await page.screenshot({ path: `${scrollCaptures}/manage-top-${width}-${theme}.png` });
    await managementBody.evaluate((node) => (node.scrollTop = node.scrollHeight));
    expect(await managementBody.evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
    await expect(closeManagement).toBeInViewport({ ratio: 1 });
    expect(await closeManagement.boundingBox()).toEqual(closeAtTop);
    expect(await dialog.evaluate((node) => node.scrollTop)).toBe(0);
    if (scrollCaptures)
      await page.screenshot({ path: `${scrollCaptures}/manage-bottom-${width}-${theme}.png` });
    // The visible action must actually close the scrolled dialog and restore its trigger.
    await closeManagement.click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole('button', { name: /^Actions for/ })).toBeFocused();
    await page.getByRole('button', { name: /^Actions for/ }).click();
    await page.getByRole('menuitem', { name: 'Manage page' }).click();
    await expect(audience).toBeVisible();
  }
  await page.setViewportSize(originalViewport);
  await expect(dialog.locator('select')).toHaveCount(0);
  await audience.click();
  const menu = dialog.getByRole('menu', { name: 'Who can open this page' });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole('menuitemradio')).toHaveCount(3);
  await captureListbox(page, 'open-list');
  await menu.press('End');
  await expect(menu.getByRole('menuitemradio', { name: 'Anyone on this computer' })).toBeFocused();
  await captureListbox(page, 'keyboard-focus');
  await menu.press('ArrowUp');
  await expect(menu.getByRole('menuitemradio', { name: 'Anyone with the link' })).toBeFocused();
  await menu.press('Home');
  await expect(menu.getByRole('menuitemradio', { name: 'Named members only' })).toBeFocused();
  await menu.press('Escape');
  await expect(menu).toBeHidden();
  await audience.click();
  // The top menu catches a press outside both layers; Manage must remain open.
  await page.mouse.click(1, 1);
  await expect(menu).toBeHidden();
  const history = dialog.getByRole('button', { name: /^What people who join later see/ });
  await history.click();
  await expect(dialog.getByRole('menu', { name: 'What people who join later see' })).toBeVisible();
  await dialog.getByRole('menu', { name: 'What people who join later see' }).press('Escape');
  const memberRole = dialog.getByRole('button', { name: /^Permission for this new recipient/ });
  await choose(dialog, 'Permission for this new recipient', 'Edit');
  await expect(memberRole).toContainText('Edit');
  await captureListbox(page, 'selected');
  await choose(dialog, 'Permission for this new recipient', 'Read');
  await expect(dialog.getByText('Active', { exact: true })).toBeVisible();
  const revision = dialog.getByText(/^Last edit:/);
  await expect(revision).toBeHidden();
  await dialog.getByText('Details', { exact: true }).click();
  await expect(revision).toBeVisible();
  await dialog.getByText('Details', { exact: true }).click();
  await expect(dialog).toContainText(
    'New readers also get recent history, including deleted text, snapshots and comments. Very old history is not included. Nothing already shared is taken back.',
  );
  const technicalHistory = dialog.getByText(
    'Later joins receive the current key and up to 63 earlier keys.',
    { exact: true },
  );
  await expect(technicalHistory).toBeHidden();
  await dialog.getByText('Details', { exact: true }).click();
  await expect(technicalHistory).toBeVisible();
  await dialog.getByText('Details', { exact: true }).click();
  await dialog.getByRole('button', { name: 'Start new history…', exact: true }).click();
  await expect(
    dialog.getByRole('heading', { name: "Start the page's history from now?" }),
  ).toBeVisible();
  await expect(dialog).toContainText(
    'Existing links keep working, and anything already shared stays shared.',
  );
  await expect(
    dialog.getByRole('button', { name: 'Start new history', exact: true }),
  ).toBeVisible();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toContainText('Editors can change');
  await expect(dialog).toContainText(text.frameLimit);
  expect(await page.locator('iframe').count()).toBe(0);
  for (const theme of ['light', 'dark']) {
    await page.evaluate((theme) => {
      document.documentElement.dataset.theme = theme;
    }, theme);
    await page.screenshot({ path: testInfo.outputPath(`share-${theme}.png`) });
  }
  await audience.press('ArrowDown');
  await audience.press('ArrowDown');
  await expect(menu.getByRole('menuitemradio', { name: 'Anyone with the link' })).toBeFocused();
  await expect(menu.getByRole('menuitemradio', { name: 'Anyone with the link' })).toHaveAttribute(
    'aria-checked',
    'false',
  );
  await menu.getByRole('menuitemradio', { name: 'Anyone with the link' }).press('Enter');
  await dialog.getByRole('button', { name: 'Allow access by link' }).click();
  await expect(dialog.getByRole('alert')).toContainText('unavailable');
  await dialog.getByRole('button', { name: 'Refresh and review' }).click();
  await choose(dialog, 'Who can open this page', 'Anyone with the link');
  await dialog.getByRole('button', { name: 'Allow access by link' }).click();
  await expect(dialog.getByRole('alert')).toContainText('Result unknown');
  await dialog.getByRole('button', { name: 'Retry this request' }).click();
  await expect(dialog.getByRole('status')).toContainText('Done.');
  await dialog.getByRole('button', { name: 'Manage another change' }).click();
  await dialog.getByRole('button', { name: 'Create a sharing link…' }).click();
  await expect(dialog.getByLabel('Share link', { exact: true })).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Create a sharing link', exact: true }).click();
  await expect(dialog.getByLabel('Share link', { exact: true })).toBeVisible();
  const shared = new URL(await dialog.getByLabel('Share link', { exact: true }).inputValue());
  const capability = new URLSearchParams(shared.hash.slice(1));
  expect(shared.origin).toBe(new URL(page.url()).origin);
  expect(shared.pathname).toBe(`/read/${capability.get('link')}`);
  expect(capability.get('seed')).toMatch(/^[A-Za-z0-9_-]{43}$/);
  expect(capability.get('page')).toBe(pageId);
  await expect(dialog.getByLabel('Link seed')).toHaveCount(0);
  await expect(dialog).toContainText('People with this link can read this page.');
  await dialog.getByRole('button', { name: 'Manage another change' }).click();
  await choose(dialog, 'Who can open this page', 'Named members only');
  await expect(dialog).toContainText('Existing links stop working on these pages');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: testInfo.outputPath('share-mobile.png') });
  expect(await dialog.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page.getByRole('button', { name: /^Actions for/ })).toBeFocused();
});

test('trusted home manages retention, archive and verified or awaiting deletion', async ({
  page,
}, testInfo) => {
  const f = await fixture(page),
    other = '00000000-0000-4000-8000-000000000103',
    log = [f.genesis, f.shared];
  let head = f.head.head;
  const added = await f.signed('page.share', { pageId: other, mode: 'private', epoch: '1' }, head);
  head = (await added.verifyNext(f.space, f.ownerKey, head)).head;
  log.push(added);
  const states = new Map(
    [pageId, other].map((id) => [
      id,
      {
        pageId: id,
        epoch: '1',
        sharing: 'private',
        history: 'shared',
        archived: false,
        retentionDays: 30,
        lastUpdateAtMs: null,
        expiresAtMs: null,
        warnings: [],
        projects: [],
      },
    ]),
  );
  const contexts: string[] = [];
  let withheld = false,
    requests = 0;
  await page.route(`**${mount}api/pages`, (route) =>
    route.fulfill({
      json: {
        spaceId: f.space,
        ownerKey: c.encodeBinary(f.ownerKey),
        revision: String(head.revision),
        projects: [],
        pageIds: [pageId, other].sort().map((id) => ({ pageId: id, deleted: !states.has(id) })),
        pages: [...states.values()],
      },
    }),
  );
  await page.routeWebSocket(`**${mount}sync`, (socket) =>
    socket.onMessage((message) => {
      const hello = JSON.parse(String(message));
      if (hello.type !== 'hello') return;
      contexts.push(hello.page);
      if (!states.has(hello.page) || withheld) {
        socket.close({ code: 1008, reason: 'DENIED' });
        return;
      }
      socket.send(
        JSON.stringify({
          version: 1,
          type: 'catchup',
          space: f.space,
          page: hello.page,
          epoch: '1',
          streams: [],
          more: true,
          baseline: null,
          membershipHead: {
            revision: String(head.revision),
            statementHash: c.encodeBinary(head.hash),
            ownerKey: c.encodeBinary(f.ownerKey),
            statements: log
              .slice(Number(hello.membershipRevision))
              .map((v) => c.encodeBinary(v.toJson())),
            more: false,
          },
        }),
      );
    }),
  );
  await page.route(`**${mount}api/management`, async (route) => {
    requests++;
    const request = route.request().postDataJSON(),
      bytes = c.binary(request.request, 1024),
      fields = c.fields(bytes, 11),
      operation = c.decodeText(fields[6]),
      selected = JSON.parse(c.decodeText(c.binary(request.payload, 16384)));
    expect(await c.strictVerify(f.deviceKey(), c.binary(request.signature, 64, 64), bytes)).toBe(
      true,
    );
    expect(c.decodeText(fields[4])).toBe(String(head.revision));
    expect(c.decodeText(fields[3])).toBe(selected.pageId);
    const envelope = await f.signed(operation, selected, head);
    head = (await envelope.verifyNext(f.space, f.ownerKey, head)).head;
    log.push(envelope);
    if (operation === 'page.archive') states.get(selected.pageId)!.archived = true;
    if (operation === 'page.delete') {
      states.delete(selected.pageId);
      // A temporarily unavailable other page must not turn an acknowledgment into success.
      withheld = states.size > 0;
    }
    await route.fulfill({
      json: {
        operationId: c.decodeText(fields[5]),
        membershipHead: {
          revision: String(head.revision),
          statementHash: c.encodeBinary(head.hash),
        },
      },
    });
  });
  await page.goto(mount);
  await expect(homeList(page)).toBeVisible();
  await showAllPages(page);
  const row = (id: string) => page.locator(`.pages li[data-page-id="${id}"]`);
  await row(pageId)
    .getByRole('button', { name: /^Actions for/ })
    .click();
  const manage = page.getByRole('menuitem', { name: 'Manage page' });
  await manage.click();
  const dialog = page.locator('dialog.management-dialog');
  await expect(dialog).toContainText('local only');
  await expect(dialog.getByRole('heading', { level: 2 })).toBeFocused();
  await expect(dialog.getByRole('heading', { level: 2 })).toHaveCSS('outline-style', 'none');
  await expect(dialog).not.toBeFocused();
  await page.keyboard.press('Tab');
  const close = dialog.getByRole('button', { name: 'Close', exact: true });
  await expect(close).toBeFocused();
  // An edge-bordered Colab button: one 2px frame of the focus colour drawn over its border.
  await expect(close).toHaveCSS('outline-style', 'solid');
  await expect(close).toHaveCSS('outline-width', '2px');
  await expect(close).toHaveCSS('outline-offset', '-2px');
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(row(pageId).getByRole('button', { name: /^Actions for/ })).toBeFocused();
  await row(pageId)
    .getByRole('button', { name: /^Actions for/ })
    .click();
  await manage.click();
  await expect(dialog.getByRole('heading', { level: 2 })).toBeFocused();
  await expect(close).toBeEnabled();
  await close.click();
  await expect(dialog).toHaveCount(0);
  await expect(row(pageId).getByRole('button', { name: /^Actions for/ })).toBeFocused();
  await row(pageId)
    .getByRole('button', { name: /^Actions for/ })
    .click();
  await manage.click();
  await expect(dialog.getByLabel('Days after the last change')).toBeVisible();
  await dialog.getByLabel('Days after the last change').fill('14');
  await dialog.getByRole('button', { name: 'Set access expiry', exact: true }).click();
  await expect(dialog).toContainText('Access expires after 14 days without a page update');
  await dialog.getByRole('button', { name: 'Set access expiry', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('Done.');
  await dialog.getByRole('button', { name: 'Manage another change' }).click();
  await expect(dialog.getByLabel('Days after the last change')).toHaveValue('14');
  await dialog.getByLabel('Keep on this computer until I delete it').check();
  await expect(dialog.getByLabel('Days after the last change')).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Set access expiry', exact: true }).click();
  await dialog.getByRole('button', { name: 'Set access expiry', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('Done.');
  await dialog.getByRole('button', { name: 'Manage another change' }).click();
  await expect(dialog.getByLabel('Keep on this computer until I delete it')).toBeChecked();
  await dialog.getByRole('button', { name: 'Archive and stop edits…', exact: true }).click();
  await expect(dialog).toContainText('stop new writes');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(requests).toBe(2);
  await dialog.getByRole('button', { name: 'Archive and stop edits…', exact: true }).click();
  await dialog.getByRole('button', { name: 'Archive and stop edits', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('Done.');
  expect(contexts.at(-1)).toBe(pageId);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(row(pageId)).toHaveCount(0);
  await expect(row(other)).toBeVisible();
  await page.getByRole('tab', { name: /^Archived/ }).click();
  await expect(row(pageId)).toContainText('Archived');
  await row(pageId)
    .getByRole('button', { name: /^Actions for/ })
    .click();
  await expect(page.getByRole('menu').locator('.retention-hint')).toHaveText('kept forever');
  await page.getByRole('menu').press('Escape');
  await expect(row(pageId).getByRole('link')).toHaveCount(1);
  // The two catalog IDs differ only in the last character, so the shortest unique prefix is the ID.
  await expect(row(pageId).getByRole('link')).toHaveAttribute('href', `/p/${pageId}`);
  await row(pageId)
    .getByRole('button', { name: /^Actions for/ })
    .click();
  await page.getByRole('menuitem', { name: 'Manage page' }).click();
  await expect(
    dialog.getByRole('button', { name: 'Archive and stop edits…', exact: true }),
  ).toBeDisabled();
  for (const theme of ['light', 'dark']) {
    await page.evaluate((theme) => {
      document.documentElement.dataset.theme = theme;
    }, theme);
    await page.screenshot({ path: testInfo.outputPath(`lifecycle-${theme}.png`) });
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: testInfo.outputPath('lifecycle-mobile.png') });
  expect(await dialog.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  await dialog.getByRole('button', { name: 'Delete page…', exact: true }).click();
  await expect(dialog).toContainText('Copies already made cannot be taken back');
  await expect(dialog).toContainText(pageId);
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(requests).toBe(3);
  await dialog.getByRole('button', { name: 'Delete page…', exact: true }).click();
  await dialog.getByRole('button', { name: 'Delete page', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText(
    'Your machine accepted the deletion, but it is not confirmed yet',
  );
  await expect(dialog).not.toContainText('This device is not allowed to manage this page');
  expect(contexts.at(-1)).toBe(other);
  withheld = false;
  await dialog.getByRole('button', { name: 'Check again' }).click();
  await expect(dialog.getByRole('status')).toContainText('Done.');
  await expect(dialog.getByRole('alert')).toHaveCount(0);
  expect(requests).toBe(4);
  await expect(dialog.getByRole('button', { name: 'Manage another change' })).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page.getByText('No archived pages.', { exact: true })).toBeVisible();
  await page.getByRole('tab', { name: /^All \d+$/ }).click();
  await row(other)
    .getByRole('button', { name: /^Actions for/ })
    .click();
  await page.getByRole('menuitem', { name: 'Manage page' }).click();
  await dialog.getByRole('button', { name: 'Delete page…', exact: true }).click();
  await dialog.getByRole('button', { name: 'Delete page', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText(
    'Your machine accepted the deletion, but it is not confirmed yet',
  );
  const sockets = contexts.length;
  await dialog.getByRole('button', { name: 'Check again' }).click();
  await expect(dialog.getByRole('alert')).toContainText(
    'Your machine accepted the deletion, but it is not confirmed yet',
  );
  expect(requests).toBe(5);
  expect(contexts).toHaveLength(sockets);
  await expect(page.locator('iframe')).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page.locator('.pages li')).toHaveCount(0);
  await expect(page.getByText('No pages in this space yet.', { exact: false })).toContainText(
    'tmt colab page create',
  );
});

test.describe('relative retention evidence', () => {
  test.use({ timezoneId: 'Asia/Tokyo' });
  test('home actions and management keep retention with local-date hover', async ({
    page,
  }, testInfo) => {
    let now = Date.UTC(2026, 9, 4, 12, 34, 56, 789);
    await page.clock.setFixedTime(now);
    const f = await fixture(page, false, false, () => now);
    let warning = 'expires-soon';
    const updated = Date.UTC(2026, 8, 10, 12, 34, 56, 789);
    const expires = updated + 30 * 86400000;
    await page.route(`**${mount}api/pages`, (route) =>
      route.fulfill({
        json: {
          spaceId: f.space,
          ownerKey: c.encodeBinary(f.ownerKey),
          revision: '2',
          projects: [],
          pageIds: [{ pageId, deleted: false }],
          pages: [
            {
              pageId,
              epoch: '1',
              sharing: 'private',
              history: 'shared',
              archived: false,
              retentionDays: 30,
              lastUpdateAtMs: updated,
              expiresAtMs: expires,
              warnings: warning ? [warning] : [],
              projects: [],
            },
          ],
        },
      }),
    );
    await page.goto(mount);
    await expect(homeList(page)).toBeVisible();
    await showAllPages(page);
    const row = page.locator(`.pages li[data-page-id="${pageId}"]`);
    const hint = page.getByRole('menu').locator('.retention-hint');
    await row.getByRole('button', { name: /^Actions for/ }).click();
    await expect(hint).toHaveText('expires in 6 days');
    await expect(hint).toHaveAttribute('title', 'Sat 10-10 21:34');
    await expect(hint.locator('.retention-mark .lucide-clock')).toHaveClass(/lucide-clock/);
    await expect(hint).toHaveCSS(
      'color',
      await page.locator('body').evaluate((node) => getComputedStyle(node).color),
    );
    await expect(row.getByRole('link')).toBeVisible();
    const dialog = page.locator('.management-dialog');
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      for (const theme of ['light', 'dark']) {
        await page.evaluate((theme) => {
          document.documentElement.dataset.theme = theme;
        }, theme);
        if (
          (await row
            .getByRole('button', { name: /^Actions for/ })
            .getAttribute('aria-expanded')) !== 'true'
        )
          await row.getByRole('button', { name: /^Actions for/ }).click();
        await page.screenshot({
          path: testInfo.outputPath(`expiry-home-${width}-${theme}.png`),
          fullPage: true,
        });
        await page.getByRole('menuitem', { name: 'Manage page' }).click();
        await expect(dialog.locator('.retention-hint')).toHaveText('expires in 6 days');
        await expect(dialog.locator('.retention-hint')).toHaveAttribute('title', 'Sat 10-10 21:34');
        await dialog.getByText('Details', { exact: true }).click();
        await expect(dialog).toContainText('Last edit: Thu 09-10 21:34');
        await dialog.getByText('Details', { exact: true }).click();
        await dialog.getByRole('button', { name: /^Who can open this page/ }).click();
        await expect(
          dialog.getByRole('menu', { name: 'Who can open this page', exact: true }),
        ).toBeVisible();
        await expect(dialog.locator('select')).toHaveCount(0);
        expect(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
        await page.screenshot({
          path: testInfo.outputPath(`expiry-dialog-${width}-${theme}.png`),
          fullPage: true,
        });
        await dialog.getByRole('button', { name: /^Who can open this page/ }).press('Escape');
        await dialog.getByRole('button', { name: 'Close', exact: true }).click();
      }
    }
    warning = 'not-in-cloud';
    now = expires + 2 * 86400000;
    await page.clock.setFixedTime(now);
    await page.reload();
    await showAllPages(page);
    await row.getByRole('button', { name: /^Actions for/ }).click();
    await expect(hint).toHaveText('not in the cloud');
    await expect(row.getByRole('link')).toBeVisible();
    await page.getByRole('menuitem', { name: 'Manage page' }).click();
    await expect(dialog.locator('.retention-hint')).toContainText('not in the cloud');
    await expect(
      dialog.getByRole('button', { name: 'Set access expiry', exact: true }),
    ).toBeEnabled();
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    warning = '';
    now = expires - 10 * 86400000;
    await page.clock.setFixedTime(now);
    await page.reload();
    await showAllPages(page);
    await row.getByRole('button', { name: /^Actions for/ }).click();
    await expect(hint).toHaveText('expires in 10 days');
    await expect(hint.locator('.retention-mark .lucide-clock')).toHaveCount(0);
    // An ordinary hint is dim text: the muted colour, read from a probe in the list.
    await expect(hint).toHaveCSS(
      'color',
      await page.locator('section.home').evaluate((home) => {
        const probe = home.appendChild(document.createElement('span'));
        probe.style.color = 'var(--c-muted)';
        const color = getComputedStyle(probe).color;
        probe.remove();
        return color;
      }),
    );
  });
});

// A mounted address names the list as `/colab/` and no more, so the tab it was left on lives in its
// history entry: a reload, Back and Forward return to it, and a fresh visit opens All.
test('the list keeps its tab across a reload, Back and Forward, and a fresh visit opens All', async ({
  page,
}) => {
  await fixture(page);
  await page.goto(mount);
  await showAllPages(page);
  const row = page.locator(`[data-page-id="${pageId}"] a`);
  const href = await row.getAttribute('href');
  // A tab other than the default. The page has not reported its tasks, so it is under no stage and
  // the list leaves for it by its address.
  const discussion = page.getByRole('tab', { name: /^Discussion/ });
  await discussion.click();
  await expect(discussion).toHaveAttribute('aria-selected', 'true');
  await page.reload();
  await expect(discussion).toHaveAttribute('aria-selected', 'true');
  await page.goto(new URL(href!, page.url()).href);
  await expect(page).toHaveURL(/\/p\//);
  await page.goBack();
  await expect(page).toHaveURL(/\/colab\/$/);
  await expect(discussion).toHaveAttribute('aria-selected', 'true');
  await page.goForward();
  await expect(page).toHaveURL(/\/p\//);
  await page.goto(mount);
  await expect(page.getByRole('tab', { name: /^All \d+$/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );
});

// Back from a page to the list reads the catalog again; an ended Remote session is refreshed once
// for the same device, as the page's own read does, instead of ending in a terminal error.
for (const status of [403, 503])
  test(`Back to the list refreshes an ended session once (${status}) and shows the list`, async ({
    page,
  }) => {
    await fixture(page);
    await page.goto(mount);
    await showAllPages(page);
    await page.locator(`[data-page-id="${pageId}"] a`).click();
    await expect(page).toHaveURL(/\/p\//);
    let reads = 0;
    await page.route(`**${mount}api/pages`, (route) =>
      reads++ === 0 ? route.fulfill({ status, json: { code: 'DENIED' } }) : route.fallback(),
    );
    let registrations = 0;
    page.on('request', (request) => {
      if (request.url().endsWith(`${mount}api/devices/register`)) registrations++;
    });
    const registers = page.waitForRequest(`**${mount}api/devices/register`);
    await page.goBack();
    await registers;
    // The list is back on the tab it was left on. The refreshed read is issued after the tab shows,
    // so the read count is polled, not sampled. Other readers of the catalog (a re-read for a page
    // with no summary yet) may add reads on a slow machine, so `once` is the device refresh: one
    // registration, however many reads follow it.
    await expect(page.getByRole('tab', { name: /^All \d+$/ })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await expect.poll(() => reads).toBeGreaterThanOrEqual(2);
    await expect(page.locator(`[data-page-id="${pageId}"] a`)).toBeVisible();
    expect(registrations).toBe(1);
    await expect(page.getByRole('alert')).toHaveCount(0);
    await expect(page).toHaveURL(/\/colab\/$/);
  });
test('Back to the list still ends in the error when a refreshed session is refused again', async ({
  page,
}) => {
  await fixture(page);
  await page.goto(mount);
  await showAllPages(page);
  await page.locator(`[data-page-id="${pageId}"] a`).click();
  await expect(page).toHaveURL(/\/p\//);
  let reads = 0;
  await page.route(`**${mount}api/pages`, (route) => {
    reads++;
    return route.fulfill({ status: 403, json: { code: 'DENIED' } });
  });
  let registrations = 0;
  page.on('request', (request) => {
    if (request.url().endsWith(`${mount}api/devices/register`)) registrations++;
  });
  await page.goBack();
  await expect(page.getByText('DENIED')).toBeVisible();
  // The read and its one refreshed retry. The list's re-read for a page with no summary yet may add
  // a read on a loaded machine, so `once` is the device refresh: one registration.
  expect(reads).toBeGreaterThanOrEqual(2);
  expect(registrations).toBe(1);
});
