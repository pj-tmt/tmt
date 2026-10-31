import { test, expect, type Page } from '@playwright/test';
import { createServer, type ViteDevServer } from 'vite';
import { mkdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';

// Presentation uses the real retained model with an isolated signed-port stand-in.
let server: ViteDevServer;
let origin: string;
type Fixture = {
  lists: number;
  effects: { kind: string; operationId: string; offerId?: string }[];
  originals: string[];
  create: 'committed' | 'unknown' | string;
  cancel: 'cancelled' | 'not-active';
  fail: boolean;
  empty: boolean;
  secretShown: () => boolean;
};
const fixture = (page: Page) =>
  page.evaluate(() => {
    const value = (window as unknown as { invitesFixture: Fixture }).invitesFixture;
    return { ...value, secretShown: value.secretShown() };
  });
const CODE = 'KQ7M-4TZ2-PRIVATE';
const LINK = `https://studio-mac.example/remote/join#${CODE}`;

test.beforeAll(async () => {
  const assets = new URL('../../../rust/tmt-remote/assets/', import.meta.url);
  const css =
    (await readFile(
      new URL('../../../../../design/browser-ui/generated/static.css', import.meta.url),
      'utf8',
    )) +
    '\n' +
    (await readFile(new URL('pages.css', assets), 'utf8')) +
    '\n' +
    (await readFile(new URL('settings.css', assets), 'utf8'));
  const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><style>${css}</style></head><body><main class="tmt-page settings-page"><h1>Invites</h1><div id="invites"></div></main><script type="module">
    import { IssuedInvitationsPage } from '/src/issued-invitations-page.ts';
    import { mountInvitesPage } from '/src/invites-page.ts';
    const remoteId='11111111-1111-4111-8111-111111111111';
    const now=Date.now();
    const row=(offerId,state,issued,expires)=>({offerId,issuedAtMs:now-issued,expiresAtMs:now-issued+expires,state,localConfirmationKnown:false,cleanup:'not-requested'});
    const rows=[
      row('7f3a0000-0000-4000-8000-000000000001','awaiting-owner',120000,1800000),
      row('52c10000-0000-4000-8000-000000000002','open',600000,1800000),
      row('1e9b0000-0000-4000-8000-000000000003','confirmed',86400000,600000),
      row('a04d0000-0000-4000-8000-000000000004','cancelled',90000000,600000),
      row('9d770000-0000-4000-8000-000000000005','expired',259200000,600000),
      row('c0de0000-0000-4000-8000-000000000006','interrupted',259200000,600000),
    ];
    const created='0ff10000-0000-4000-8000-000000000007';
    const fixture=window.invitesFixture={lists:0,effects:[],originals:[],create:'committed',cancel:'cancelled',fail:false,empty:false,
      secretShown:()=>{const body=document.body.cloneNode(true);body.querySelectorAll('script').forEach((node)=>node.remove());return body.outerHTML.includes('${CODE}')}};
    const client={
      async list(){fixture.lists++;if(fixture.fail)throw new Error('unavailable');return {remoteId,invitations:fixture.empty?[]:rows,nextCursor:null}},
      async create(input){fixture.effects.push({kind:'create',...input});
        if(fixture.create==='unknown')return {operationId:input.operationId,state:'unknown',reason:'effect_outcome_unconfirmed'};
        if(fixture.create!=='committed')return {operationId:input.operationId,state:'refused',reason:fixture.create};
        return {operationId:input.operationId,state:'committed',sessionEnded:false,result:{invitation:{offerId:created,state:'created',expiresAtMs:Date.now()+1800000}},secret:{code:'${CODE}',link:'${LINK}'}}},
      async cancel(input){fixture.effects.push({kind:'cancel',...input});
        return {operationId:input.operationId,state:'committed',sessionEnded:false,result:{invitation:{offerId:input.offerId,state:fixture.cancel,code:fixture.cancel==='cancelled'?'REMOTE_INVITATION_CANCELLED':'REMOTE_INVITATION_NOT_ACTIVE'}}}},
      async operation(id){fixture.originals.push(id);return {operationId:id,state:'committed',sessionEnded:false,result:{invitation:{offerId:created,state:'created',expiresAtMs:Date.now()+1800000}}}},
      async pending(){return {state:'none'}},
      async decide(){throw new Error('not used')},
    };
    const controller=new AbortController();const page=new IssuedInvitationsPage(client,async()=>client);
    fixture.confirmed=0;
    const view=mountInvitesPage(document.getElementById('invites'),page,controller.signal,{otherBusy:()=>false,changed:()=>{},confirm:()=>{fixture.confirmed++}});
    fixture.invite=()=>view.invite();fixture.refresh=()=>view.refresh();fixture.model=()=>({secret:page.secret});
    if(!location.search.includes('idle=1'))view.refresh();
  </script></body></html>`;
  server = await createServer({
    configFile: false,
    root: new URL('../', import.meta.url).pathname,
    appType: 'custom',
    server: { host: '127.0.0.1', port: 0 },
  });
  server.middlewares.use((request, response, next) => {
    if (!request.url?.startsWith('/remote/invites')) return next();
    response.setHeader('Content-Type', 'text/html; charset=utf-8');
    response.end(html);
  });
  await server.listen();
  const address = server.httpServer!.address();
  if (!address || typeof address === 'string') throw new Error('Missing loopback fixture address');
  origin = `http://127.0.0.1:${address.port}`;
});
test.afterAll(async () => {
  await server?.close();
});

async function capture(page: Page, state: string) {
  const directory = process.env.REMOTE_INVITES_CAPTURE_DIR;
  if (!directory) return;
  await mkdir(directory, { recursive: true });
  // Design review pairs each state in both schemes; the test continues in light.
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    // Capture the settled look, never a dialog or theme transition in flight.
    await page.evaluate(() =>
      Promise.all(document.getAnimations().map((a) => a.finished.catch(() => undefined))),
    );
    await page.screenshot({ path: join(directory, `${state}-${colorScheme}.png`), fullPage: true });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

test('Invites lists issued invitation metadata with Mock D labels and phone rows', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${origin}/remote/invites`);
  const table = page.getByRole('table', { name: 'Invitations' });
  await expect(table.getByRole('row')).toHaveCount(7);
  const awaiting = table.getByRole('row').filter({ hasText: 'inv-7f3a' });
  await expect(awaiting).toContainText('Awaiting you');
  await expect(awaiting).toContainText(
    'Someone used this code · confirm or refuse in the row menu',
  );
  await expect(awaiting.getByText('inv-7f3a', { exact: true })).toHaveAttribute(
    'title',
    '7f3a0000-0000-4000-8000-000000000001',
  );
  await expect(awaiting).toContainText('2 min ago');
  await expect(awaiting).toContainText(/in (28|29) min/);
  for (const [id, label] of [
    ['inv-52c1', 'Open'],
    ['inv-1e9b', 'Confirmed'],
    ['inv-a04d', 'Cancelled'],
    ['inv-9d77', 'Expired'],
    ['inv-c0de', 'Interrupted · needs action'],
  ] as const)
    await expect(table.getByRole('row').filter({ hasText: id })).toContainText(label);
  // Ben 10-31: Refused stays neutral grey like Cancelled; interrupted outcomes are amber.
  await expect(
    table.getByRole('row').filter({ hasText: 'inv-a04d' }).locator('[data-tone]'),
  ).toHaveCount(0);
  await expect(
    table.getByRole('row').filter({ hasText: 'inv-c0de' }).locator('[data-tone="waiting"]'),
  ).toHaveText('Interrupted · needs action');
  await expect(table.getByRole('row').filter({ hasText: 'inv-1e9b' })).toContainText('—');
  await expect(page.locator('.invites-list-footer')).toContainText('6 invitations · newest first');
  await expect(table).not.toHaveAttribute('style');
  await expect(page.locator('[style]')).toHaveCount(0);
  await capture(page, 'list-1440');

  await awaiting.getByRole('button', { name: 'Actions for inv-7f3a' }).click();
  await page.getByRole('menuitem', { name: 'Confirm or refuse…' }).click();
  expect((await fixture(page)).effects).toEqual([]);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { invitesFixture: { confirmed: number } }).invitesFixture.confirmed,
    ),
  ).toBe(1);
  // A terminal row offers no cancel.
  await table.getByRole('button', { name: 'Actions for inv-1e9b' }).click();
  await expect(page.getByRole('menuitem', { name: 'Cancel invitation' })).toHaveCount(0);
  await page.keyboard.press('Escape');

  await page.setViewportSize({ width: 390, height: 900 });
  // Hidden phone cells leave the accessibility tree; index the DOM cells instead.
  const cells = awaiting.locator('[role="cell"]');
  const box = async (index: number) => (await cells.nth(index).boundingBox())!;
  const name = await box(1);
  const status = await box(2);
  const expires = await box(4);
  const menu = await box(5);
  expect(Math.abs(name.y - menu.y)).toBeLessThanOrEqual(1);
  expect(menu.x).toBeGreaterThan(name.x);
  expect(status.y).toBeGreaterThan(name.y);
  expect(Math.abs(status.y - expires.y)).toBeLessThanOrEqual(1);
  await expect(cells.nth(3)).toBeHidden();
  const lines = await page.locator('.invites-list-footer span').all();
  expect((await lines[1]!.boundingBox())!.y).toBeGreaterThan((await lines[0]!.boundingBox())!.y);
  await expect(page.locator('html')).toHaveJSProperty('scrollWidth', 390);
  await capture(page, 'list-390');
});

test('a created code is shown once in the dialog and never kept after it closes', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${origin}/remote/invites`);
  await expect(page.getByRole('table', { name: 'Invitations' }).getByRole('row')).toHaveCount(7);
  await page.evaluate(() =>
    (
      window as unknown as { invitesFixture: { invite: () => Promise<void> } }
    ).invitesFixture.invite(),
  );
  const dialog = page.getByRole('dialog', { name: 'Invitation created' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText(CODE);
  await expect(dialog).toContainText(LINK);
  await expect(dialog).toContainText(/expires in (29|30) min/);
  await expect(dialog).toContainText(
    "This code is shown only now. After you close this, it can't be shown again.",
  );
  await expect(dialog.getByRole('button', { name: 'Copy code' })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Copy link' })).toBeVisible();
  // Copy code, Copy link and Done are enabled and legible on their own fills in both themes.
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    for (const name of ['Copy code', 'Copy link', 'Done']) {
      const control = dialog.getByRole('button', { name, exact: true });
      await expect(control).toBeEnabled();
      await expect
        .poll(() =>
          control.evaluate((node) => {
            const rgb = (value: string) =>
              value
                .match(/[\d.]+/g)!
                .slice(0, 3)
                .map(Number);
            const lum = (channels: number[]) =>
              channels
                .map((v) => v / 255)
                .map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
                .reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i]!, 0);
            const style = getComputedStyle(node);
            const [a, b] = [lum(rgb(style.color)), lum(rgb(style.backgroundColor))];
            return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
          }),
        )
        .toBeGreaterThanOrEqual(4.5);
    }
  }
  await page.emulateMedia({ colorScheme: 'light' });
  await capture(page, 'dialog-1440');
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(dialog.getByRole('button', { name: 'Copy link' })).toBeInViewport();
  await capture(page, 'dialog-390');
  await dialog.getByRole('button', { name: 'Done' }).click();
  await expect(dialog).toHaveCount(0);
  const after = await fixture(page);
  expect(after.secretShown).toBe(false);
  expect(after.effects.map((effect) => effect.kind)).toEqual(['create']);
  expect(
    await page.evaluate(() =>
      (
        window as unknown as { invitesFixture: { model: () => { secret?: unknown } } }
      ).invitesFixture.model(),
    ),
  ).toEqual({});
  expect(
    await page.evaluate(
      (code) => JSON.stringify({ ...localStorage, ...sessionStorage }).includes(code),
      CODE,
    ),
  ).toBe(false);
  await expect(page.getByRole('status').first()).toHaveText(
    'Invitation created. Its code is no longer shown; invite again for a new one.',
  );
  // The list re-reads metadata after a committed creation.
  expect(after.lists).toBe(2);
});

test('an unconfirmed create recovers metadata only and never shows a code', async ({ page }) => {
  await page.goto(`${origin}/remote/invites`);
  await expect(page.getByRole('table', { name: 'Invitations' }).getByRole('row')).toHaveCount(7);
  await page.evaluate(() => {
    const value = (
      window as unknown as { invitesFixture: Fixture & { invite: () => Promise<void> } }
    ).invitesFixture;
    value.create = 'unknown';
    return value.invite();
  });
  await expect(page.getByRole('status').first()).toHaveText(
    'Creating this invitation is unconfirmed. Check its original outcome before inviting again.',
  );
  const recover = page.getByRole('button', { name: 'Check original outcome' });
  await expect(recover).toBeVisible();
  await capture(page, 'unconfirmed');
  await recover.click();
  await expect(page.getByRole('status').first()).toHaveText(
    'Invitation created. Its code is no longer shown; invite again for a new one.',
  );
  const after = await fixture(page);
  expect(after.originals).toHaveLength(1);
  expect(after.effects.map((effect) => effect.kind)).toEqual(['create']);
  expect(after.secretShown).toBe(false);
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('create refusals use the contract reasons and cancel reports a no-longer-open invitation', async ({
  page,
}) => {
  await page.goto(`${origin}/remote/invites`);
  await expect(page.getByRole('table', { name: 'Invitations' }).getByRole('row')).toHaveCount(7);
  for (const [reason, text] of [
    ['REMOTE_INVITATION_BUSY', 'Another invitation request is in progress'],
    ['REMOTE_INVITATION_CAPACITY', 'too many open invitations'],
    [
      'REMOTE_INVITATION_STOPPED',
      'The invitation request stopped before it finished. Nothing was created. Try again.',
    ],
    ['REMOTE_INVITATION_UNAVAILABLE', 'Invitations are unavailable right now'],
    ['REMOTE_MANAGEMENT_READ_ONLY', 'read-only for this Remote'],
    ['REMOTE_INTENT_CONFLICT', 'conflicts with an earlier one'],
  ] as const) {
    await page.evaluate((code) => {
      const value = (
        window as unknown as { invitesFixture: Fixture & { invite: () => Promise<void> } }
      ).invitesFixture;
      value.create = code;
      return value.invite();
    }, reason);
    await expect(page.getByRole('status').first()).toContainText(text);
  }
  await page.evaluate(() => {
    (window as unknown as { invitesFixture: Fixture }).invitesFixture.cancel = 'not-active';
  });
  await page.getByRole('button', { name: 'Actions for inv-52c1' }).click();
  await page.getByRole('menuitem', { name: 'Cancel invitation' }).click();
  await expect(page.getByRole('status').first()).toHaveText(
    'That invitation was no longer open. Nothing changed.',
  );
  expect((await fixture(page)).effects.at(-1)).toMatchObject({
    kind: 'cancel',
    offerId: '52c10000-0000-4000-8000-000000000002',
  });
});

test('empty, loading and error states use the shared list presentation', async ({ page }) => {
  await page.goto(`${origin}/remote/invites?idle=1`);
  await expect(page.getByRole('status').first()).toHaveText('Checking invitations…');
  await expect(page.locator('.invites-list-footer')).toBeHidden();
  await page.evaluate(() => {
    const value = (
      window as unknown as { invitesFixture: Fixture & { refresh: () => Promise<void> } }
    ).invitesFixture;
    value.empty = true;
    return value.refresh();
  });
  await expect(page.getByText('No invitations in the last 7 days.')).toBeVisible();
  // An empty list is its one line: no column header, count or row hint.
  await expect(page.locator('.invites-list-footer')).toBeHidden();
  await expect(page.getByRole('table', { name: 'Invitations' })).toBeHidden();
  await capture(page, 'empty');
  await page.evaluate(() => {
    const value = (
      window as unknown as { invitesFixture: Fixture & { refresh: () => Promise<void> } }
    ).invitesFixture;
    value.fail = true;
    return value.refresh();
  });
  await expect(page.getByRole('status').first()).toHaveText(
    'Invitations could not be confirmed. Use tmt remote invite on this machine.',
  );
  await expect(page.locator('.invites-list-footer')).toBeHidden();
});
