import { expect } from '@playwright/test';
import { test, signIn } from './browser-session.js';
import {
  setTester,
  writeAgentGrantFields,
  writeBlockFields,
  readBlockDocument,
} from './firestore-fixture.js';

test('owner discovers a revoked grant, edits its retained block and keeps home independent', async ({
  openSession,
}) => {
  const { page } = await openSession();
  await signIn(page, 'Space owner');
  const uid = (await page.getByText(/^UID: /).innerText()).replace(/^UID: /, '').trim();
  await setTester(uid, true);
  await page.getByLabel('World name').fill('Retained studio');
  await page.getByRole('button', { name: 'Create world', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Retained studio' })).toBeVisible();
  const worldId = new URL(page.url()).pathname.split('/').at(-1)!;
  await page.getByRole('button', { name: 'Add plant', exact: true }).click();
  await page.getByRole('button', { name: 'Save layout', exact: true }).click();
  await expect(page.getByText('Saved · revision 1', { exact: true })).toBeVisible();
  const home = await readBlockDocument(worldId);
  const blockId = crypto.randomUUID();
  const identityId = crypto.randomUUID();
  const principalUid = `office-agent:${crypto.randomUUID()}`;
  await writeAgentGrantFields(worldId, principalUid, {
    version: { integerValue: '1' },
    ownerUid: { stringValue: uid },
    installationId: { stringValue: crypto.randomUUID() },
    identityId: { stringValue: identityId },
    blockId: { stringValue: blockId },
    capabilities: { arrayValue: { values: [{ stringValue: 'layout.read' }] } },
    enabled: { booleanValue: false },
    createdAt: { timestampValue: new Date(Date.now() - 120_000).toISOString() },
    expiresAt: { timestampValue: new Date(Date.now() - 60_000).toISOString() },
  });
  await writeBlockFields(worldId, blockId, {
    version: { integerValue: '1' },
    revision: { integerValue: '1' },
    objects: { arrayValue: { values: [{ stringValue: 'd000' }] } },
    updatedAt: { timestampValue: new Date().toISOString() },
  });
  await page.getByRole('button', { name: 'Refresh spaces', exact: true }).click();
  await expect(page.getByText(identityId, { exact: true })).toBeVisible();
  await expect(page.getByText(/^Revoked · Lease expires/)).toBeVisible();
  await page.getByRole('button', { name: `Open block ${blockId}`, exact: true }).click();
  await expect(page.getByRole('button', { name: 'Desk 1', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Add chair', exact: true }).click();
  await page.getByRole('button', { name: 'Save layout', exact: true }).click();
  await expect(page.getByText('Saved · revision 2', { exact: true })).toBeVisible();
  expect(await readBlockDocument(worldId, blockId)).toMatchObject({
    fields: {
      revision: { integerValue: '2' },
      objects: { arrayValue: { values: [{ stringValue: 'd000' }, { stringValue: 'c0ee' }] } },
    },
  });
  expect(await readBlockDocument(worldId)).toEqual(home);
  await page.getByRole('button', { name: 'Open home block', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Plant 1', exact: true })).toBeVisible();
  await page.getByRole('button', { name: `Open block ${blockId}`, exact: true }).click();
  await expect(page.getByRole('button', { name: 'Chair 2', exact: true })).toBeVisible();
  await page
    .getByRole('region', { name: 'Agent spaces' })
    .screenshot({ path: test.info().outputPath('retained-spaces-desktop.png') });
  await page.setViewportSize({ width: 390, height: 844 });
  await page
    .getByRole('region', { name: 'Agent spaces' })
    .screenshot({ path: test.info().outputPath('retained-spaces-mobile.png') });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true
  );
  await setTester(uid, false);
  await expect(page.getByRole('heading', { name: 'Waiting for access' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Agent spaces' })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Office block editor' })).toHaveCount(0);
});
