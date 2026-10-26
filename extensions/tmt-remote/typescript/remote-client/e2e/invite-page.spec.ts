import { test, expect } from '@playwright/test';
import { createServer, type Server } from 'node:http';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import vector from '../../../rust/tmt-remote/tests/fixtures/invitations/invite-v2.json' with { type: 'json' };

// The exact Remote-owned hosted document/resources, with real CSP and HTTP.
// No provider/door/Colab handler exists in this server: any such call fails.
let server: Server;
let origin: string;
let requests: { path: string; referrer: string }[] = [];
test.beforeAll(async () => {
  const assets = new URL('../../../rust/tmt-remote/assets/', import.meta.url);
  const bootstrap = await readFile(new URL('join.js', assets), 'utf8');
  const html = (await readFile(new URL('join.html', assets), 'utf8')).replace(
    '__TMT_JOIN_BOOTSTRAP__',
    bootstrap,
  );
  const js = await readFile(new URL('invite-v1.js', assets), 'utf8');
  const css =
    (await readFile(
      new URL('../../../../../design/browser-ui/generated/static.css', import.meta.url),
      'utf8',
    )) +
    '\n' +
    (await readFile(new URL('pages.css', assets), 'utf8'));
  const inline = html.split('<script>')[1]!.split('</script>')[0]!;
  const hash = createHash('sha256').update(inline).digest('base64');
  const policy = `default-src 'none'; script-src 'self' 'sha256-${hash}'; style-src 'self'; img-src data:; connect-src 'none'; base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'`;
  server = createServer((request, response) => {
    requests.push({ path: request.url!, referrer: request.headers.referer ?? '' });
    response.setHeader('Cache-Control', 'no-store');
    response.setHeader('Referrer-Policy', 'no-referrer');
    response.setHeader('X-Content-Type-Options', 'nosniff');
    const resources: Record<string, [string, string]> = {
      '/join': ['text/html', html],
      '/sdk/invite-v1.js': ['text/javascript', js],
      '/sdk/invite.css': ['text/css', css],
      '/blank': ['text/html', '<title>Left invitation</title>'],
    };
    const resource = resources[request.url!];
    if (!resource) {
      response.writeHead(404);
      response.end();
      return;
    }
    response.setHeader('Content-Type', resource[0] + '; charset=utf-8');
    if (request.url === '/join') response.setHeader('Content-Security-Policy', policy);
    response.end(resource[1]);
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Missing private HTTP address');
  origin = `http://127.0.0.1:${address.port}`;
});
test.afterAll(async () => {
  await new Promise<void>((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
});

function token(now = Date.now(), duration = 600_000): string {
  const invite = structuredClone(vector.invite);
  invite.context.issued_at_ms = now;
  invite.context.expires_at_ms = now + duration;
  return Buffer.from(JSON.stringify(invite)).toString('base64url');
}

test('erases before resources, copies only on explicit action and keeps no browser persistence', async ({
  page,
  context,
}) => {
  requests = [];
  const code = token();
  const errors: string[] = [];
  page.on('console', (message) => errors.push(message.text()));
  page.on('pageerror', (error) => errors.push(error.message));
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.addInitScript(() => {
    Storage.prototype.setItem = () => {
      throw new Error('Unexpected persistence');
    };
    indexedDB.open = () => {
      throw new Error('Unexpected IndexedDB');
    };
    navigator.serviceWorker.register = () => {
      throw new Error('Unexpected worker');
    };
  });
  let resourceChecks = 0;
  await page.route('**/sdk/*', async (route) => {
    expect(await page.evaluate(() => location.hash)).toBe('');
    resourceChecks++;
    await route.continue();
  });
  await page.goto(`${origin}/join#${code}`);
  const copy = page.getByRole('button', { name: 'Copy join command' });
  await expect(copy).toBeEnabled();
  expect(resourceChecks).toBe(2);
  expect(page.url()).toBe(`${origin}/join`);
  expect(await page.evaluate(() => history.state)).toBeNull();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('');
  await copy.click();
  await expect(page.getByText('Command copied. Run it in your terminal.')).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(`tmt remote join ${code}`);
  expect(await page.content()).not.toContain(code);
  expect(
    await page.evaluate(() => [localStorage.length, sessionStorage.length, document.cookie]),
  ).toEqual([0, 0, '']);
  expect(requests.map((request) => request.path).sort()).toEqual([
    '/join',
    '/sdk/invite-v1.js',
    '/sdk/invite.css',
  ]);
  expect(requests.every((request) => request.referrer === '')).toBe(true);
  expect(JSON.stringify(requests)).not.toContain(code);
  expect(errors).toEqual([]);
  for (const [name, width] of [
    ['desktop', 1100],
    ['mobile', 390],
  ] as const) {
    await page.setViewportSize({ width, height: 850 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({
      path: `/private/tmp/remote-4-4982-invite-${name}.png`,
      fullPage: true,
    });
  }
  await page.reload();
  await expect(copy).toBeDisabled();
  await expect(
    page.getByText('This invitation is missing, invalid or expired.', { exact: false }),
  ).toBeVisible();
});

test('invalid, expired, replaced and departed interactions never retain a usable code', async ({
  page,
  context,
}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto(`${origin}/join#not-a-code`);
  const copy = page.getByRole('button', { name: 'Copy join command' });
  await expect(copy).toBeDisabled();
  await page.evaluate(
    (code) => {
      location.hash = code;
    },
    token(Date.now() - 600_000),
  );
  await expect(copy).toBeDisabled();
  await page.clock.install();
  await page.evaluate((code) => {
    location.hash = code;
  }, token());
  await expect(copy).toBeEnabled();
  await page.clock.fastForward(600_001);
  await expect(copy).toBeDisabled();
  await expect(
    page.getByText('This invitation has reached its expiry.', { exact: false }),
  ).toBeVisible();
  const later = await page.evaluate(() => Date.now());
  await page.evaluate((code) => {
    location.hash = code;
  }, token(later));
  await expect(copy).toBeEnabled();
  await page.goto(`${origin}/blank`);
  await page.goBack();
  await expect(copy).toBeDisabled();
  expect(page.url()).toBe(`${origin}/join`);
});

test('a failed history erase loads no resource and enables no copy', async ({ page }) => {
  requests = [];
  await page.addInitScript(() => {
    history.replaceState = () => {
      throw new Error('History unavailable');
    };
  });
  await page.goto(`${origin}/join#${token()}`);
  await expect(page.getByRole('button', { name: 'Copy join command' })).toBeDisabled();
  expect(requests.map((request) => request.path)).toEqual(['/join']);
});
