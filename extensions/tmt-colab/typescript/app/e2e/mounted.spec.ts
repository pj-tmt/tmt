import { expect, test, type Page } from '@playwright/test';
import * as c from '@tmt/colab-client';

const mount = '/r/abcd/x/colab/';
const device = '00000000-0000-4000-8000-000000000100';
const member = '00000000-0000-4000-8000-000000000101';
const pageId = '00000000-0000-4000-8000-000000000102';
const json = (v: unknown) => c.text(JSON.stringify(v));
async function fixture(page: Page, tamper = false) {
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
    export async function reopenSession() {}
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
        pages: [{ pageId, epoch: '1', sharing: 'private', history: 'current', archived: false }],
      },
    }),
  );
  await page.route(`**${mount}api/devices/register`, async (route) => {
    const v = route.request().postDataJSON();
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
  return { space, ownerKey, genesis, shared, head };
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
        membershipRevision: '0',
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
            statements: [c.encodeBinary(f.genesis.toJson()), c.encodeBinary(f.shared.toJson())],
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
