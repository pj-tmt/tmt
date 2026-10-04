import { expect, test, type Page, type BrowserContext } from '@playwright/test';
import * as c from '@tmt/colab-client';

const mount = '/r/abcd/x/colab/';
const device = '00000000-0000-4000-8000-000000000100';
const member = '00000000-0000-4000-8000-000000000101';
const pageId = '00000000-0000-4000-8000-000000000102';
const json = (v: unknown) => c.text(JSON.stringify(v));
async function fixture(page: Page | BrowserContext, tamper = false, empty = false) {
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
  await page.route('**/sdk/remote-v1.js*', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `
    export async function reopenSession() {sessionStorage.setItem('test:reopens',String(Number(sessionStorage.getItem('test:reopens')??0)+1));}
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
        pages: empty
          ? []
          : [{ pageId, epoch: '1', sharing: 'private', history: 'current', archived: false }],
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
      issuedAt: Date.now() - 1000,
      expiresAt: Date.now() + 60000,
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
  const f = await fixture(page);
  await page.goto(mount);
  await expect(page.getByRole('heading', { name: 'Colab', exact: true })).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`#space=${f.space}$`));
  await expect(page.getByRole('link', { name: new RegExp(pageId) })).toBeVisible();
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
  const f = await fixture(page);
  await page.goto(mount);
  await expect(page).toHaveURL(new RegExp(`#space=${f.space}$`));
  await page.goto(`${mount}#space=${'a'.repeat(32)}`);
  await expect(page.getByRole('alert')).toContainText('does not match the pinned space');
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
  await page.getByRole('link', { name: new RegExp(pageId) }).click();
  await expect(page.getByRole('alert')).toContainText('no verified page key');
  await expect(page.locator('iframe')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Source', exact: true })).toHaveCount(0);
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
      return record(`pin:${location.origin}/r/abcd/x/colab/`);
    }),
  ).toBeUndefined();
});

test('a newer Use here cancels queued takeover and ignores an old registration completing after inactivity', async ({
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
    await expect(page.getByTestId('colab-inactive')).toBeVisible();
    expect(await reopens(page)).toBe(1);
    expect(await reopens(other)).toBe(0);
    await page.getByRole('button', { name: 'Use here' }).click();
    await expect(other.getByTestId('colab-inactive')).toBeVisible();
    release();
    await expect(page.getByRole('heading', { name: 'Colab', exact: true })).toBeVisible();
    await expect(other.getByTestId('colab-inactive')).toBeVisible();
    expect(await reopens(page)).toBe(2);
    expect(await reopens(other)).toBe(0);
    expect(requests).toBe(2);
    await other.getByRole('button', { name: 'Use here' }).click();
    await expect(page.getByTestId('colab-inactive')).toBeVisible();
    await expect(other.getByRole('heading', { name: 'Colab', exact: true })).toBeVisible();
    expect(await reopens(page)).toBe(2);
    expect(await reopens(other)).toBe(1);
    expect(requests).toBe(3);
  } finally {
    release();
    await other.close();
  }
});

test('empty owner space names the product page-create command', async ({ page }) => {
  await fixture(page, false, true);
  await page.goto(mount);
  await expect(page.getByRole('heading', { name: 'Colab', exact: true })).toBeVisible();
  await expect(page.getByText('No pages in this space yet.', { exact: false })).toContainText(
    'tmt colab page create --title "Notes"',
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
        pages: [{ pageId, epoch: '1', sharing, history: 'shared', archived: false }],
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
  await page.getByRole('button', { name: 'Manage page' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByLabel('Audience')).toBeVisible();
  await expect(dialog.getByText('Active', { exact: true })).toBeVisible();
  const revision = dialog.getByText('Verified revision 2', { exact: true });
  await expect(revision).toBeHidden();
  await dialog.getByText('Details', { exact: true }).click();
  await expect(revision).toBeVisible();
  await dialog.getByText('Details', { exact: true }).click();
  await expect(dialog).toContainText('64 most recent epochs');
  await expect(dialog).toContainText('Editors can change');
  expect(await page.locator('iframe').count()).toBe(0);
  for (const theme of ['light', 'dark']) {
    await page.evaluate((theme) => {
      document.documentElement.dataset.theme = theme;
    }, theme);
    await page.screenshot({ path: testInfo.outputPath(`share-${theme}.png`) });
  }
  await dialog.getByLabel('Audience').selectOption('link');
  await dialog.getByRole('button', { name: 'Confirm make link' }).click();
  await expect(dialog.getByRole('alert')).toContainText('unavailable');
  await dialog.getByRole('button', { name: 'Refresh and review' }).click();
  await dialog.getByLabel('Audience').selectOption('link');
  await dialog.getByRole('button', { name: 'Confirm make link' }).click();
  await expect(dialog.getByRole('alert')).toContainText('Result unknown');
  await dialog.getByRole('button', { name: 'Retry exact request' }).click();
  await expect(dialog.getByRole('status')).toContainText('Change verified');
  await dialog.getByRole('button', { name: 'Manage another change' }).click();
  await dialog.getByRole('button', { name: 'Create link' }).click();
  await expect(dialog.getByLabel('Link seed')).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Confirm create link' }).click();
  await expect(dialog.getByLabel('Link seed')).toHaveValue(/^[A-Za-z0-9_-]{43}$/);
  await expect(dialog).toContainText(
    'Opening shared links in this browser app is not available yet',
  );
  await dialog.getByRole('button', { name: 'Manage another change' }).click();
  await dialog.getByLabel('Audience').selectOption('private');
  await expect(dialog).toContainText('links are revoked and affected pages rotate');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: testInfo.outputPath('share-mobile.png') });
  expect(await dialog.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Manage page' })).toBeFocused();
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
      { pageId: id, epoch: '1', sharing: 'private', history: 'shared', archived: false },
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
  const row = (id: string) => page.locator('.pages li').filter({ hasText: id });
  await row(pageId).getByRole('button', { name: 'Manage page' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Expiry time unavailable');
  await dialog.getByLabel('Retention days').fill('14');
  await dialog.getByRole('button', { name: 'Set retention', exact: true }).click();
  await expect(dialog).toContainText('14 days after the last page update');
  await dialog.getByRole('button', { name: 'Confirm set retention' }).click();
  await expect(dialog.getByRole('status')).toContainText('Change verified');
  await dialog.getByRole('button', { name: 'Manage another change' }).click();
  await expect(dialog.getByLabel('Retention days')).toHaveValue('14');
  await dialog.getByLabel('Keep forever').check();
  await expect(dialog.getByLabel('Retention days')).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Set retention', exact: true }).click();
  await dialog.getByRole('button', { name: 'Confirm set retention' }).click();
  await expect(dialog.getByRole('status')).toContainText('Change verified');
  await dialog.getByRole('button', { name: 'Manage another change' }).click();
  await expect(dialog.getByLabel('Keep forever')).toBeChecked();
  await dialog.getByRole('button', { name: 'Archive page', exact: true }).click();
  await expect(dialog).toContainText('freeze writes');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(requests).toBe(2);
  await dialog.getByRole('button', { name: 'Archive page', exact: true }).click();
  await dialog.getByRole('button', { name: 'Confirm archive page' }).click();
  await expect(dialog.getByRole('status')).toContainText('Change verified');
  expect(contexts.at(-1)).toBe(pageId);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(row(pageId)).toHaveCount(0);
  await expect(row(other)).toBeVisible();
  await page.getByLabel('Show archived pages').check();
  await expect(row(pageId)).toContainText('Archived');
  await expect(row(pageId)).toContainText('Retention: forever');
  await expect(row(pageId).getByRole('link')).toHaveCount(0);
  await row(pageId).getByRole('button', { name: 'Manage page' }).click();
  await expect(dialog.getByRole('button', { name: 'Archive page', exact: true })).toBeDisabled();
  for (const theme of ['light', 'dark']) {
    await page.evaluate((theme) => {
      document.documentElement.dataset.theme = theme;
    }, theme);
    await page.screenshot({ path: testInfo.outputPath(`lifecycle-${theme}.png`) });
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: testInfo.outputPath('lifecycle-mobile.png') });
  expect(await dialog.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  await dialog.getByRole('button', { name: 'Delete page', exact: true }).click();
  await expect(dialog).toContainText('Copies already made cannot be recalled');
  await expect(dialog).toContainText(pageId);
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(requests).toBe(3);
  await dialog.getByRole('button', { name: 'Delete page', exact: true }).click();
  await dialog.getByRole('button', { name: 'Confirm delete page' }).click();
  await expect(dialog.getByRole('alert')).toContainText(
    'Deletion acknowledged, awaiting verification',
  );
  await expect(dialog).not.toContainText('Management access was denied');
  expect(contexts.at(-1)).toBe(other);
  withheld = false;
  await dialog.getByRole('button', { name: 'Verify signed log' }).click();
  await expect(dialog.getByRole('status')).toContainText('Deletion verified');
  await expect(dialog.getByRole('alert')).toHaveCount(0);
  expect(requests).toBe(4);
  await expect(dialog.getByRole('button', { name: 'Manage another change' })).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page.getByText('No archived pages.', { exact: true })).toBeVisible();
  await page.getByLabel('Show archived pages').uncheck();
  await row(other).getByRole('button', { name: 'Manage page' }).click();
  await dialog.getByRole('button', { name: 'Delete page', exact: true }).click();
  await dialog.getByRole('button', { name: 'Confirm delete page' }).click();
  await expect(dialog.getByRole('alert')).toContainText(
    'Deletion acknowledged, awaiting verification',
  );
  const sockets = contexts.length;
  await dialog.getByRole('button', { name: 'Verify signed log' }).click();
  await expect(dialog.getByRole('alert')).toContainText(
    'Deletion acknowledged, awaiting verification',
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
