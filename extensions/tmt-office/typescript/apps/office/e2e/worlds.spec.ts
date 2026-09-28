import { expect } from '@playwright/test';
import { test, signIn } from './browser-session.js';
import { setTester, ownedWorlds, TRANSACTION_FAILURE_TIMEOUT_MS } from './firestore-fixture.js';

test('transport failure cannot queue a create across reconnect; explicit retry creates exactly once', async ({
  openSession,
}) => {
  const { page, context } = await openSession();
  await signIn(page, 'Offline');
  const uid = (await page.getByText(/^UID: /).innerText()).replace(/^UID: /, '').trim();
  await setTester(uid, true);
  await expect(page.getByRole('button', { name: 'Create world', exact: true })).toBeVisible();
  expect(await ownedWorlds(uid)).toEqual([]);
  const transport = 'http://127.0.0.1:8080/**';
  let abortedReads = 0;
  await context.route(transport, (route) => {
    if (route.request().url().includes(':batchGet')) abortedReads++;
    return route.abort('internetdisconnected');
  });
  await page.getByLabel('World name').fill('Reconnect room');
  await page.getByRole('button', { name: 'Create world', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('could not be confirmed', {
    timeout: TRANSACTION_FAILURE_TIMEOUT_MS,
  });
  expect(abortedReads).toBeGreaterThan(0);
  expect(await ownedWorlds(uid)).toEqual([]);
  await context.unroute(transport);
  expect(await ownedWorlds(uid)).toEqual([]);
  await page.getByRole('button', { name: 'Retry creation', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Reconnect room' })).toBeVisible();
  expect(await ownedWorlds(uid)).toHaveLength(1);
});

test('Console-style grant enables creation; another user is denied; revocation clears the live world', async ({
  openSession,
}) => {
  const alice = await openSession();
  const bob = await openSession();
  await signIn(alice.page, 'Alice');
  await expect(alice.page.getByRole('heading', { name: 'Waiting for access' })).toBeVisible();
  const aliceUid = (await alice.page.getByText(/^UID: /).innerText()).replace(/^UID: /, '').trim();
  await setTester(aliceUid, true);
  await expect(
    alice.page.getByRole('heading', { name: 'Create your private world' })
  ).toBeVisible();
  await alice.page.getByLabel('World name').fill('Alice private room');
  await alice.page.getByRole('button', { name: 'Create world', exact: true }).click();
  await expect(alice.page.getByRole('heading', { name: 'Alice private room' })).toBeVisible();
  const address = alice.page.url();
  await alice.page.getByRole('link', { name: 'Office', exact: true }).click();
  await alice.page
    .getByLabel('World ID', { exact: true })
    .fill(new URL(address).pathname.split('/').at(-1)!);
  await alice.page.getByRole('button', { name: 'Open world', exact: true }).click();
  await expect(alice.page.getByRole('heading', { name: 'Alice private room' })).toBeVisible();
  await signIn(bob.page, 'Bob');
  const bobUid = (await bob.page.getByText(/^UID: /).innerText()).replace(/^UID: /, '').trim();
  await setTester(bobUid, true);
  await expect(bob.page.getByRole('heading', { name: 'Create your private world' })).toBeVisible();
  await bob.page
    .getByLabel('World ID', { exact: true })
    .fill(new URL(address).pathname.split('/').at(-1)!);
  await bob.page.getByRole('button', { name: 'Open world', exact: true }).click();
  await expect(bob.page.getByRole('heading', { name: 'World unavailable' })).toBeVisible();
  await expect(bob.page.getByText('Alice private room', { exact: true })).toHaveCount(0);
  await setTester(aliceUid, false);
  await expect(alice.page.getByRole('heading', { name: 'Waiting for access' })).toBeVisible();
  await expect(alice.page.getByRole('heading', { name: 'Alice private room' })).toHaveCount(0);
  await setTester(aliceUid, true);
  await expect(alice.page.getByRole('heading', { name: 'Alice private room' })).toBeVisible();
  await alice.page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await expect(alice.page.getByRole('heading', { name: 'Alice private room' })).toHaveCount(0);
  await alice.page.reload();
  await expect(alice.page.getByRole('button', { name: /Sign in/ })).toBeVisible();
  await expect(alice.page.getByRole('heading', { name: 'Alice private room' })).toHaveCount(0);
});
