import { test as base, expect } from '@playwright/test';
import type { BrowserContext, Page } from '@playwright/test';

interface BrowserSession {
  context: BrowserContext;
  page: Page;
  requests: URL[];
}
export const test = base.extend<{ openSession: (url?: string) => Promise<BrowserSession> }>({
  openSession: async ({ browser }, use) => {
    const sessions: BrowserSession[] = [];
    const unexpected: string[] = [];
    await use(async (url = 'http://127.0.0.1:4173') => {
      const context = await browser.newContext();
      const requests: URL[] = [];
      context.on('request', (request) => {
        requests.push(new URL(request.url()));
      });
      await context.route('**/*', async (route) => {
        const target = new URL(route.request().url());
        if (target.hostname === '127.0.0.1') return route.continue();
        // GAPI occasionally emits this optional telemetry beacon. Always block
        // it; it is neither a required library nor a production auth request.
        if (target.hostname === 'apis.google.com' && target.pathname === '/js/gen_204') {
          return route.abort();
        }
        // The official popup transport requires Google's public iframe library even
        // with Auth Emulator. No production auth/API endpoints are allowed.
        if (
          target.protocol === 'https:' &&
          target.hostname === 'apis.google.com' &&
          (target.pathname.startsWith('/js/') || target.pathname.startsWith('/_/scs/')) &&
          route.request().method() === 'GET' &&
          route.request().resourceType() === 'script'
        )
          return route.continue();
        // Firebase's emulator widget requests optional CDN presentation assets.
        // Block them, but do not stub its local authentication logic or responses.
        if (!['unpkg.com', 'fonts.googleapis.com', 'fonts.gstatic.com'].includes(target.hostname)) {
          unexpected.push(target.origin + target.pathname);
        }
        await route.abort();
      });
      const page = await context.newPage();
      const session = { context, page, requests };
      sessions.push(session);
      await page.goto(url);
      return session;
    });
    for (const session of sessions) await session.context.close();
    expect(unexpected).toEqual([]);
  },
});

export async function openPopup(page: Page): Promise<Page> {
  const opened = page.waitForEvent('popup');
  await page.getByRole('button', { name: 'Sign in with Google (emulator)' }).click();
  const popup = await opened;
  await expect(popup.getByRole('button', { name: 'Add new account' })).toBeVisible();
  return popup;
}

export async function submitNewAccount(popup: Page, name: string): Promise<void> {
  await popup.getByRole('button', { name: 'Add new account' }).click();
  // The upstream emulator labels are not associated with inputs; use its stable IDs.
  await popup
    .locator('#email-input')
    .fill(`${name.toLowerCase()}-${crypto.randomUUID()}@example.test`);
  await popup.locator('#display-name-input').fill(name);
  await popup.getByRole('button', { name: 'Sign in with Google.com', exact: true }).click();
}

export async function signIn(page: Page, name: string): Promise<void> {
  await submitNewAccount(await openPopup(page), name);
  await expect(page.getByText(`Signed in as ${name}`, { exact: true })).toBeVisible();
  await expect(page.getByText(/^UID: /)).toContainText(/UID: \S+/);
}
