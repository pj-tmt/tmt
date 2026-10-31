import { test, expect } from '@playwright/test';
import { createServer, type ViteDevServer } from 'vite';
import { mkdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';

// Presentation uses the real retained model with an isolated signed-port stand-in.
let server: ViteDevServer;
let origin: string;
type Fixture = {
  admin: boolean;
  machineReads: number;
  effects: unknown[];
  originals: string[];
  known: boolean;
  failLookup: boolean;
  role: 'owner' | 'user';
  routes:
    | { state: 'unavailable' | 'not-recorded' }
    | { state: 'recorded'; publicationPending: boolean; removalPending: boolean };
};
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
  const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><style>${css}</style></head><body><main class="tmt-page settings-page"><h1>People</h1><div id="people"></div><p id="observations" hidden></p></main><script type="module">
    import { MembersPage, mountMembersPage } from '/src/members-page.ts';
    const remoteId='11111111-1111-4111-8111-111111111111';
    const member={remoteId,memberId:'person-1',role:'user',displayName:'Guest',sendingEnabled:false,revision:4,createdAtMs:1,revokedAtMs:null};
    const fixture=window.peopleFixture={admin:true,reads:0,machineReads:0,effects:[],originals:[],external:false,known:false,failLookup:false,role:'user',routes:{state:'not-recorded'}};
    const client={clientId:'current',
      async list(){fixture.reads++;return {remoteId,administrator:fixture.admin,displayName:'Remote',members:location.search.includes('hide=1')?[]:[{...member,role:fixture.role}],nextCursor:null,invitation:fixture.admin?{state:'no-current-offer'}:null,routes:fixture.routes}},
      async machines({memberId}){fixture.machineReads++;return {remoteId,memberId,machines:[{clientId:'browser-1',name:'Laptop',kind:'browser',revision:1,revoked:false,expiresAtMs:null,sendingScope:false,delivery:'direct',recipientAccess:'all',grantLive:true,lastUsedAtMs:null,thisBrowser:true,liveSessionCount:null,lastActivityAtMs:null},{clientId:'browser-2',name:'Old browser',kind:'browser',revision:1,revoked:false,expiresAtMs:1,sendingScope:false,delivery:'direct',recipientAccess:'all',grantLive:false,lastUsedAtMs:null,thisBrowser:false,liveSessionCount:0,lastActivityAtMs:null},{clientId:'browser-3',name:'Removed browser',kind:'browser',revision:1,revoked:true,expiresAtMs:null,sendingScope:false,delivery:'direct',recipientAccess:'all',grantLive:false,lastUsedAtMs:null,thisBrowser:false,liveSessionCount:null,lastActivityAtMs:null}],nextCursor:null}},
      async rename(input){fixture.effects.push({kind:'rename',...input});return {operationId:input.operationId,state:'unknown',reason:'effect_outcome_unconfirmed'}},
      async sending(input){fixture.effects.push({kind:'sending',...input});return {operationId:input.operationId,state:'unknown',reason:'effect_outcome_unconfirmed'}},
      async revoke(input){fixture.effects.push({kind:'revoke',...input});return {operationId:input.operationId,state:'unknown',reason:'effect_outcome_unconfirmed'}},
      async operation(id){fixture.originals.push(id);if(fixture.failLookup){fixture.failLookup=false;throw new Error('lookup unavailable')}const effect=fixture.effects[0];return fixture.known?{operationId:id,state:'committed',result:{member:{...member,sendingEnabled:effect.kind==='sending'?effect.enabled:member.sendingEnabled,displayName:effect.kind==='rename'?effect.name:member.displayName,revision:5}},sessionEnded:false}:{operationId:id,state:'unknown',reason:'effect_outcome_unconfirmed'}}
    };
    const controller=new AbortController();const page=new MembersPage(client,async()=>client);
    const view=mountMembersPage(document.getElementById('people'),page,controller.signal,()=>fixture.external,()=>{},document.getElementById('observations'));
    fixture.refresh=()=>view.refresh();fixture.paint=()=>view.render();fixture.stop=()=>controller.abort();view.refresh();
  </script></body></html>`;
  server = await createServer({
    configFile: false,
    root: new URL('../', import.meta.url).pathname,
    appType: 'custom',
    server: { host: '127.0.0.1', port: 0 },
  });
  server.middlewares.use((request, response, next) => {
    if (!request.url?.startsWith('/remote/members')) return next();
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

async function captureExisting(page: import('@playwright/test').Page, state: string) {
  const directory = process.env.REMOTE_PEOPLE_CAPTURE_DIR;
  if (!directory) return;
  await mkdir(directory, { recursive: true });
  // Design review pairs each state in both schemes; the test continues in light.
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await page.screenshot({ path: join(directory, `${state}-${colorScheme}.png`), fullPage: true });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

async function openRename(page: import('@playwright/test').Page) {
  await page.getByRole('button', { name: 'Actions for Guest' }).click();
  await page.getByRole('menuitem', { name: 'Rename' }).click();
}
async function beginSending(page: import('@playwright/test').Page) {
  await page.getByRole('button', { name: 'Actions for Guest' }).click();
  await page.getByRole('menuitem', { name: 'Enable sending' }).click();
}

test('People keeps drafts on refresh and reads machines only when expanded', async ({ page }) => {
  await page.goto(`${origin}/remote/members`);
  await expect(page.getByRole('cell', { name: 'Guest', exact: true })).toBeVisible();
  const sending = page.getByText('Sending off', { exact: true });
  await expect(sending).toBeVisible();
  const pill = sending.locator('..');
  await expect(pill).toHaveCSS('font-size', '10px');
  const dot = pill.locator('.tmt-ui-state-mark');
  await expect(dot).toHaveCSS('width', '7px');
  expect(await dot.evaluate((node) => getComputedStyle(node).backgroundColor)).toBe(
    await pill.evaluate((node) => getComputedStyle(node).color),
  );
  expect(
    await sending.evaluate(
      (element) => element.parentElement!.scrollWidth <= element.parentElement!.clientWidth,
    ),
  ).toBe(true);
  await captureExisting(page, 'refresh');
  await openRename(page);
  const name = page.getByRole('textbox', { name: 'Person name for Guest' });
  await name.fill('Guest draft');
  await captureExisting(page, 'draft');
  await page.getByRole('button', { name: 'Refresh people' }).click();
  await expect(name).toHaveValue('Guest draft');
  expect(
    await page.evaluate(
      () => (window as unknown as { peopleFixture: Fixture }).peopleFixture.machineReads,
    ),
  ).toBe(0);
  await page.getByRole('button', { name: 'Show machines for Guest' }).click();
  await expect(page.getByText(/Laptop.*Activity unavailable/)).toBeVisible();
  await expect(page.getByText(/Old browser.*Expiry passed locally/)).toBeVisible();
  await expect(page.getByText(/Removed browser.*Revoked/)).toBeVisible();
  expect(
    await page.evaluate(
      () => (window as unknown as { peopleFixture: Fixture }).peopleFixture.machineReads,
    ),
  ).toBe(1);
  await page.evaluate(() => {
    (window as unknown as { peopleFixture: Fixture }).peopleFixture.admin = false;
  });
  await page.getByRole('button', { name: 'Refresh people' }).click();
  await expect(name).toHaveValue('Guest draft');
  await page.getByRole('button', { name: 'Actions for Guest' }).click();
  await expect(page.getByRole('menuitem', { name: 'Enable sending' })).toBeDisabled();
  await expect(page.getByRole('menuitem', { name: 'Remove person' })).toBeDisabled();
  await expect(page.getByText(/Only a designated admin/)).toBeVisible();
});

test('an unsaved person name remains visible after reload until restored', async ({ page }) => {
  await page.goto(`${origin}/remote/members`);
  await openRename(page);
  await page.getByRole('textbox', { name: 'Person name for Guest' }).fill('Guest draft');
  await page.reload();
  const name = page.getByRole('textbox', { name: 'Person name for Guest' });
  await expect(name).toBeVisible();
  await expect(name).toHaveValue('Guest draft');
  await page.getByRole('button', { name: 'Restore name for Guest' }).click();
  await page.reload();
  await expect(name).toBeHidden();
});

test('an unsent name remains visible if its person is absent after reload', async ({ page }) => {
  await page.goto(`${origin}/remote/members`);
  await openRename(page);
  await page.getByRole('textbox', { name: 'Person name for Guest' }).fill('Guest draft');
  await page.goto(`${origin}/remote/members?hide=1`);
  await expect(
    page.getByText(
      'This person is not in the current page. Your unsent name is kept; restore it to continue.',
    ),
  ).toBeVisible();
  await expect(page.getByLabel('Person name', { exact: true })).toHaveValue('Guest draft');
  await page.getByRole('button', { name: 'Restore name', exact: true }).click();
  await expect(page.getByLabel('Person name', { exact: true })).toHaveCount(0);
});

test('invalid names are refused locally without a member effect', async ({ page }) => {
  await page.goto(`${origin}/remote/members`);
  await openRename(page);
  const name = page.getByRole('textbox', { name: 'Person name for Guest' });
  await name.fill('   ');
  await page.getByRole('button', { name: 'Save name for Guest' }).click();
  await expect(page.getByText(/Use a nonblank name/)).toBeVisible();
  expect(
    await page.evaluate(
      () => (window as unknown as { peopleFixture: Fixture }).peopleFixture.effects.length,
    ),
  ).toBe(0);
});

test('People confirms sending once and reads the original without replay', async ({ page }) => {
  await page.goto(`${origin}/remote/members`);
  await beginSending(page);
  const send = page.locator('button[aria-label="Enable sending for Guest"]');
  await expect(send).toBeEnabled();
  await expect(page.getByText(/Press the same action again to confirm/)).toBeVisible();
  await captureExisting(page, 'confirmation');
  expect(
    await page.evaluate(
      () => (window as unknown as { peopleFixture: Fixture }).peopleFixture.effects.length,
    ),
  ).toBe(0);
  await send.click();
  await expect(page.getByText(/This change is unconfirmed/)).toBeVisible();
  await expect(send).toBeDisabled();
  await page.getByRole('button', { name: 'Refresh people' }).click();
  await expect(page.getByText(/Original change/)).toBeVisible();
  await captureExisting(page, 'recovery');
  const checkOriginal = page.getByRole('button', { name: 'Check original outcome' });
  await checkOriginal.click();
  await expect(page.getByText(/This change is unconfirmed/)).toBeVisible();
  await expect(checkOriginal).toBeEnabled();
  await expect(send).toBeDisabled();
  await page.evaluate(() => {
    (window as unknown as { peopleFixture: Fixture }).peopleFixture.known = true;
  });
  await checkOriginal.click();
  await expect(page.getByText('The change was saved.')).toBeVisible();
  await expect(send).toBeEnabled();
  expect(
    await page.evaluate(() => ({
      effects: (window as unknown as { peopleFixture: Fixture }).peopleFixture.effects.length,
      originals: (window as unknown as { peopleFixture: Fixture }).peopleFixture.originals,
    })),
  ).toMatchObject({ effects: 1, originals: [expect.any(String), expect.any(String)] });
  const originals = await page.evaluate(
    () => (window as unknown as { peopleFixture: Fixture }).peopleFixture.originals,
  );
  expect(originals[0]).toBe(originals[1]);
});

test('People retains its original ID after a failed lookup and permits another check', async ({
  page,
}) => {
  await page.goto(`${origin}/remote/members`);
  await beginSending(page);
  const send = page.locator('button[aria-label="Enable sending for Guest"]');
  await expect(send).toBeEnabled();
  await send.click();
  const original = page.getByText(/Original change/);
  await expect(original).toBeVisible();
  const originalText = await original.textContent();
  await page.evaluate(() => {
    (window as unknown as { peopleFixture: Fixture }).peopleFixture.failLookup = true;
  });
  const check = page.getByRole('button', { name: 'Check original outcome' });
  await check.click();
  await expect(original).toHaveText(originalText!);
  await expect(check).toBeEnabled();
  await page.evaluate(() => {
    (window as unknown as { peopleFixture: Fixture }).peopleFixture.known = true;
  });
  await check.click();
  await expect(page.getByText('The change was saved.')).toBeVisible();
  const facts = await page.evaluate(() => {
    const f = (window as unknown as { peopleFixture: Fixture }).peopleFixture;
    return { originals: f.originals, effects: f.effects.length };
  });
  expect(facts.effects).toBe(1);
  expect(facts.originals).toHaveLength(2);
  expect(facts.originals[0]).toBe(facts.originals[1]);
});

test('owner rows do not offer sending or removal, while a member retains both', async ({
  page,
}) => {
  await page.goto(`${origin}/remote/members`);
  await page.getByRole('button', { name: 'Actions for Guest' }).click();
  await expect(page.getByRole('menuitem', { name: 'Enable sending' })).toBeEnabled();
  await page.keyboard.press('Escape');
  await page.evaluate(() => {
    (window as unknown as { peopleFixture: Fixture }).peopleFixture.role = 'owner';
  });
  await page.getByRole('button', { name: 'Refresh people' }).click();
  await expect(page.getByRole('cell', { name: 'Owner' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Enable sending for Guest' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Remove Guest' })).toHaveCount(0);
  await page.locator('.people-person button[aria-label="Remove Guest"]').evaluate((button) => {
    (button as HTMLButtonElement).click();
  });
  expect(
    await page.evaluate(
      () => (window as unknown as { peopleFixture: Fixture }).peopleFixture.effects.length,
    ),
  ).toBe(0);
});

test('person name is stacked and the save action stays next to the field on wide screens', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${origin}/remote/members`);
  await openRename(page);
  const input = page.getByRole('textbox', { name: 'Person name for Guest' });
  const label = page.locator('.people-name-actions label');
  const save = page.getByRole('button', { name: 'Save name for Guest' });
  await expect(input).toBeVisible();
  const boxes = await Promise.all([label.boundingBox(), input.boundingBox(), save.boundingBox()]);
  expect(boxes.every(Boolean)).toBe(true);
  expect(boxes[0]!.y + boxes[0]!.height).toBeLessThanOrEqual(boxes[1]!.y + 1);
  expect(boxes[2]!.x - (boxes[1]!.x + boxes[1]!.width)).toBeGreaterThanOrEqual(0);
  expect(boxes[2]!.x - (boxes[1]!.x + boxes[1]!.width)).toBeLessThanOrEqual(16);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
});

test('pending and unavailable access observations stay distinct without an idle duplicate', async ({
  page,
}) => {
  await page.goto(`${origin}/remote/members`);
  await expect(page.getByText('People visible to this browser.')).toBeVisible();
  const observation = page.locator('#observations');
  await expect(observation).toBeHidden();
  await page.evaluate(() => {
    (window as unknown as { peopleFixture: Fixture }).peopleFixture.routes = {
      state: 'recorded',
      publicationPending: true,
      removalPending: true,
    };
  });
  await page.getByRole('button', { name: 'Refresh people' }).click();
  await expect(observation).toHaveText(
    'An access update is awaiting delivery. An access removal is awaiting cleanup.',
  );
  await page.evaluate(() => {
    (window as unknown as { peopleFixture: Fixture }).peopleFixture.routes = {
      state: 'unavailable',
    };
  });
  await page.getByRole('button', { name: 'Refresh people' }).click();
  await expect(observation).toHaveText('Access update status is unavailable.');
});

test('phone People rows keep the machines action in the row menu', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await page.goto(`${origin}/remote/members`);
  const row = page.locator('.people-person').first();
  await expect(row.getByRole('button', { name: 'Show machines for Guest' })).toBeHidden();
  await row.getByRole('button', { name: 'Actions for Guest' }).click();
  await page.getByRole('menuitem', { name: 'Show machines' }).click();
  await expect(row).toContainText('Laptop');
  expect(
    await page.evaluate(
      () => (window as unknown as { peopleFixture: Fixture }).peopleFixture.machineReads,
    ),
  ).toBe(1);
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(row.getByRole('button', { name: 'Show machines for Guest' })).toBeVisible();
});
