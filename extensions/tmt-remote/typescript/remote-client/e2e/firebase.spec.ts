import { expect, test } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
const origin = 'https://demo-remote-1.web.app';
const config = {
  apiKey: 'public-key',
  appId: '1:123:web:mine',
  authDomain: 'demo-remote-1.firebaseapp.com',
  projectId: 'demo-remote-1',
};
const descriptor =
  '{"checksum":"42deaf4f32ad7a7986ef7963b589cf88a0c1ac6158f75730f5c2d098f7da9dcf","kind":"firestore","publicWebConfig":{"apiKey":"public-key","appId":"1:123:web:mine","authDomain":"demo-remote-1.firebaseapp.com","projectId":"demo-remote-1"},"version":1}';
const link = `${origin}/pair#${Buffer.from(descriptor).toString('base64url')}`;
const bytes = JSON.stringify({
  version: 1,
  extension: 'fixture',
  backend: 'firestore',
  resources: [],
  admission: { artifact: 'admission.rules', digest: '0'.repeat(64), entryPoint: 'admitted' },
});
const declaration = { bytes, digest: createHash('sha256').update(bytes).digest('hex') };

test('hosted anonymous bootstrap lazy-loads only same-origin code and fixed Auth origins under CSP', async ({
  page,
}) => {
  const requests: string[] = [];
  const violations: string[] = [];
  page.on('console', (message) => {
    if (message.text().includes('Content Security Policy')) violations.push(message.text());
  });
  await page.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    requests.push(url.href);
    if (url.origin === origin) {
      if (url.pathname === '/')
        return route.fulfill({
          contentType: 'text/html',
          headers: {
            'Content-Security-Policy':
              "default-src 'none'; script-src 'self'; connect-src 'self' https://identitytoolkit.googleapis.com https://securetoken.googleapis.com https://firestore.googleapis.com",
          },
          body: '<!doctype html><title>Fixture</title>',
        });
      if (url.pathname === '/__/firebase/init.json') return route.fulfill({ json: config });
      if (url.pathname === '/sdk/remote-v1.js')
        return route.fulfill({
          contentType: 'text/javascript',
          body: await readFile(
            new URL('../../../rust/tmt-remote/assets/remote-v1.js', import.meta.url),
            'utf8',
          ),
        });
      if (url.pathname === '/sdk/remote-firebase-v1.js')
        return route.fulfill({
          contentType: 'text/javascript',
          body: await readFile(new URL('../dist/remote-firebase-v1.js', import.meta.url), 'utf8'),
        });
    }
    if (
      url.origin === 'https://identitytoolkit.googleapis.com' &&
      url.pathname.endsWith('/accounts:signUp')
    )
      return route.fulfill({
        headers: { 'Access-Control-Allow-Origin': origin },
        json: {
          localId: 'fixture-anonymous-uid',
          idToken: 'fixture-token',
          refreshToken: 'fixture-refresh',
          expiresIn: '3600',
        },
      });
    if (
      url.origin === 'https://identitytoolkit.googleapis.com' &&
      url.pathname.endsWith('/accounts:lookup')
    )
      return route.fulfill({
        headers: { 'Access-Control-Allow-Origin': origin },
        json: {
          users: [
            {
              localId: 'fixture-anonymous-uid',
              providerUserInfo: [],
              createdAt: '1000',
              lastLoginAt: '1000',
            },
          ],
        },
      });
    throw new Error(`Unexpected provider request: ${url.origin}${url.pathname}`);
  });
  await page.goto(origin);
  await page.evaluate(async () => {
    await import('/sdk/remote-v1.js');
  });
  expect(requests).toEqual([`${origin}/`, `${origin}/sdk/remote-v1.js`]);
  const uid = await page.evaluate(
    async ({ link, declaration }) => {
      const sdk = await import('/sdk/remote-v1.js');
      const auth = await sdk.firebaseRemote.connect(link, declaration);
      try {
        return await auth.signInAnonymous();
      } finally {
        await auth.close();
      }
    },
    { link, declaration },
  );
  expect(uid).toBe('fixture-anonymous-uid');
  expect(requests.some((url) => url === `${origin}/sdk/remote-firebase-v1.js`)).toBe(true);
  expect(
    requests.every((url) =>
      [
        origin,
        'https://identitytoolkit.googleapis.com',
        'https://securetoken.googleapis.com',
        'https://firestore.googleapis.com',
      ].includes(new URL(url).origin),
    ),
  ).toBe(true);
  expect(page.frames()).toHaveLength(1);
  expect(violations).toEqual([]);
});
