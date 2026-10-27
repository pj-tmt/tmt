import { test, expect } from '@playwright/test';
import { createServer, type ViteDevServer } from 'vite';
import { readFile } from 'node:fs/promises';

// UI-only composition: real markup/SDK presenter and local mock port. Native
// signed-message/Store/worker authority is verified by browser_ceremony_tests.rs.
let server: ViteDevServer;
let origin: string;
type Fixture = {
  reads: number;
  decisions: Record<string, unknown>[];
  lookups: number;
  mode: 'normal' | 'lost' | 'refused' | 'late' | 'terminal-won';
  original: 'unknown' | 'confirmed';
  release?: () => void;
  attach: () => void;
  end: () => void;
};
test.beforeAll(async () => {
  const assets = new URL('../../../rust/tmt-remote/assets/', import.meta.url);
  const document = await readFile(new URL('settings.html', assets), 'utf8');
  const start = document.indexOf('<section id="owner-invitation"');
  const end = document.indexOf('</section>', start) + '</section>'.length;
  if (start < 0 || end < start) throw new Error('Missing actual invitation markup');
  const css =
    (await readFile(
      new URL('../../../../../design/browser-ui/generated/static.css', import.meta.url),
      'utf8',
    )) +
    '\n' +
    (await readFile(new URL('pages.css', assets), 'utf8'));
  const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><style>${css}</style></head><body><main class="tmt-page">${document.slice(start, end)}</main><script type="module">
    import { invitationPage } from '/src/invitation-page.ts';
    import { RefusalError } from '/src/operations.ts';
    const fixture = window.invitationFixture = {reads:0,decisions:[],lookups:0,mode:'normal',original:'unknown'};
    const candidate = {clientId:'22222222-2222-4222-8222-222222222222',name:'Guest',words:['apple','birch','cedar','dawn'],snapshot:'BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc'};
    const result = input => ({operationId:input.operationId,state:'committed',sessionEnded:false,result:{invitation:input.matched ? {offerId:input.offerId,clientId:candidate.clientId,memberId:'member',state:'confirmed',sendingEnabled:input.sending} : {offerId:input.offerId,clientId:candidate.clientId,state:'refused'}}});
    const controller = new AbortController();
    const attach = invitationPage(document.getElementById('owner-invitation'),controller.signal,() => ({
      async pending() { fixture.reads++; if(fixture.mode==='terminal-won') return {state:'current',offerId:'11111111-1111-4111-8111-111111111111',expiresAtMs:Date.now()+60000,phase:'confirmed',localConfirmationKnown:true,cleanup:'not-requested',candidate:null}; if(fixture.mode==='late') await new Promise(resolve=>fixture.release=resolve); if(fixture.mode==='refused') throw new RefusalError('REMOTE_MANAGEMENT_READ_ONLY'); return {state:'current',offerId:'11111111-1111-4111-8111-111111111111',expiresAtMs:Date.now()+60000,phase:'awaiting-owner',localConfirmationKnown:false,cleanup:'not-requested',candidate}; },
      async decide(input) { fixture.decisions.push(structuredClone(input)); if(fixture.mode==='terminal-won') return {operationId:input.operationId,state:'refused',reason:'REMOTE_INVITATION_DECIDED'}; if(fixture.mode==='lost') throw new Error('lost answer'); return result(input); },
      async operation(id) { fixture.lookups++; return fixture.original==='unknown' ? {operationId:id,state:'unknown',reason:'effect_outcome_unconfirmed'} : result(fixture.decisions[0]); },
    }));
    fixture.attach=()=>attach({}); fixture.end=()=>controller.abort(); fixture.attach();
  </script></body></html>`;
  server = await createServer({
    configFile: false,
    root: new URL('../', import.meta.url).pathname,
    appType: 'custom',
    server: { host: '127.0.0.1', port: 0 },
  });
  server.middlewares.use((request, response, next) => {
    if (request.url !== '/remote/members') return next();
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

test('words require explicit comparison; sending defaults off and the decision is submitted once', async ({
  page,
}, info) => {
  await page.goto(`${origin}/remote/members`);
  await expect(page.getByText('apple birch cedar dawn')).toBeVisible();
  const confirm = page.getByRole('button', { name: 'Confirm invitation', exact: true });
  await expect(confirm).toBeDisabled();
  await expect(page.getByRole('checkbox', { name: 'Allow this member' })).not.toBeChecked();
  await page.getByRole('checkbox', { name: 'I compared' }).check();
  await expect(confirm).toBeEnabled();
  await page.screenshot({
    path: info.outputPath('owner-invitation-desktop.png'),
    fullPage: true,
    animations: 'disabled',
  });
  await confirm.click();
  await expect(page.getByRole('status')).toContainText('Confirmed; sending off.');
  const decisions = await page.evaluate(
    () => (window as unknown as { invitationFixture: Fixture }).invitationFixture.decisions,
  );
  expect(decisions).toHaveLength(1);
  expect(decisions[0]).toMatchObject({ matched: true, sending: false });
  await expect(page.getByRole('button', { name: 'Check original outcome' })).toBeHidden();
});
test('decline needs no word or sending consent and both choices reset on a new read', async ({
  page,
}) => {
  await page.goto(`${origin}/remote/members`);
  await page.getByRole('checkbox', { name: 'Allow this member' }).check();
  await page.getByRole('button', { name: 'Check invitation', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: 'Allow this member' })).not.toBeChecked();
  await page.getByRole('button', { name: 'Decline invitation', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('Invitation declined.');
  expect(
    await page.evaluate(
      () => (window as unknown as { invitationFixture: Fixture }).invitationFixture.decisions,
    ),
  ).toMatchObject([{ matched: false, sending: false }]);
});
test('lost answer survives tab changes and explicit lookup without resending or another admission', async ({
  page,
}) => {
  await page.goto(`${origin}/remote/members`);
  await expect(page.getByText('apple birch cedar dawn')).toBeVisible();
  await page.evaluate(() => {
    (window as unknown as { invitationFixture: Fixture }).invitationFixture.mode = 'lost';
  });
  await page.getByRole('checkbox', { name: 'I compared' }).check();
  await page.getByRole('button', { name: 'Confirm invitation', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('outcome is unconfirmed');
  await page.evaluate(() => {
    history.pushState({}, '', '/remote/overview');
    window.dispatchEvent(new PopStateEvent('popstate'));
    history.pushState({}, '', '/remote/members');
    window.dispatchEvent(new PopStateEvent('popstate'));
  });
  await page.getByRole('button', { name: 'Check original outcome' }).click();
  await expect(page.getByRole('status')).toContainText('outcome is unconfirmed');
  await page.evaluate(() => {
    (window as unknown as { invitationFixture: Fixture }).invitationFixture.original = 'confirmed';
  });
  await page.getByRole('button', { name: 'Check original outcome' }).click();
  await expect(page.getByRole('status')).toContainText('Confirmed; sending off.');
  expect(
    await page.evaluate(() => {
      const f = (window as unknown as { invitationFixture: Fixture }).invitationFixture;
      return { decisions: f.decisions.length, lookups: f.lookups, reads: f.reads };
    }),
  ).toEqual({ decisions: 1, lookups: 2, reads: 1 });
});
test('refused access and departed late reads disclose no candidate or decision controls', async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${origin}/remote/members`);
  await expect(page.getByText('apple birch cedar dawn')).toBeVisible();
  await page.screenshot({
    path: info.outputPath('owner-invitation-mobile.png'),
    fullPage: true,
    animations: 'disabled',
  });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.evaluate(() => {
    (window as unknown as { invitationFixture: Fixture }).invitationFixture.mode = 'refused';
  });
  await page.getByRole('button', { name: 'Check invitation', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('cannot read the invitation');
  await expect(page.getByText('apple birch cedar dawn')).toBeHidden();
  await expect(page.locator('#invitation-words')).toHaveText('');
  await page.screenshot({
    path: info.outputPath('owner-invitation-mobile-refused.png'),
    fullPage: true,
  });
  await page.evaluate(() => {
    const f = (window as unknown as { invitationFixture: Fixture }).invitationFixture;
    f.mode = 'late';
    f.attach();
  });
  await page.evaluate(() => {
    history.pushState({}, '', '/remote/overview');
    window.dispatchEvent(new PopStateEvent('popstate'));
    (window as unknown as { invitationFixture: Fixture }).invitationFixture.release?.();
  });
  await expect(page.getByText('apple birch cedar dawn')).toBeHidden();
  await expect(page.locator('#invitation-words')).toHaveText('');
  expect(
    await page.evaluate(
      () => (window as unknown as { invitationFixture: Fixture }).invitationFixture.decisions,
    ),
  ).toHaveLength(0);
});

test('a definite terminal-winner refusal unlocks refresh and shows its confirmed outcome', async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${origin}/remote/members`);
  await expect(page.getByText('apple birch cedar dawn')).toBeVisible();
  await page.evaluate(() => {
    (window as unknown as { invitationFixture: Fixture }).invitationFixture.mode = 'terminal-won';
  });
  await page.getByRole('checkbox', { name: 'I compared' }).check();
  await page.getByRole('button', { name: 'Confirm invitation', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('already decided');
  await page.screenshot({
    path: info.outputPath('f1-terminal-won-refused.png'),
    fullPage: true,
    animations: 'disabled',
  });
  await expect(page.getByRole('button', { name: 'Check original outcome' })).toBeHidden();
  await expect(page.getByRole('button', { name: 'Check invitation', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Check invitation', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('confirmed and its receipt was delivered');
  await page.screenshot({
    path: info.outputPath('f1-terminal-won-current.png'),
    fullPage: true,
    animations: 'disabled',
  });
  expect(
    await page.evaluate(() => {
      const f = (window as unknown as { invitationFixture: Fixture }).invitationFixture;
      return { reads: f.reads, decisions: f.decisions.length, lookups: f.lookups };
    }),
  ).toEqual({ reads: 2, decisions: 1, lookups: 0 });
});
