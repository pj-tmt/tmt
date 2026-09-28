import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, type Page } from '@playwright/test';
import { signIn } from './browser-session.js';
import { setTester } from './firestore-fixture.js';
import { createPairingEmulatorFixture } from '../../../services/office/functions/test/emulator-fixture.js';
import {
  runCli,
  withSandbox,
  type Sandbox,
} from '../../../../../../typescript/test/support/cli-process.js';
import { clearOfficeScopes, installNativeOffice } from './native-office-fixture.js';

type Admin = ReturnType<typeof createPairingEmulatorFixture>;
type Document = ReturnType<ReturnType<Admin['db']['collection']>['doc']>;
type Office = (command: string[], extra?: string[]) => ReturnType<typeof runCli>;

export type OpenSession = (url?: string) => Promise<{ page: Page }>;

export interface NativeDecorationContext {
  blockId: string;
  blockRef: Document;
  grantRef: Document;
  office: Office;
  page: Page;
  sandbox: Sandbox;
  worldId: string;
}

async function pairNative(
  sandbox: Sandbox,
  admin: Admin,
  openSession: OpenSession,
  readOnly = false
): Promise<NativeDecorationContext> {
  const prefix = await installNativeOffice(sandbox);
  const identity = readOnly ? 'ReadOnlyDecorationNative' : 'DecorationNative';
  expect((await runCli(sandbox, ['identity', 'create', identity, '--json'])).status).toBe(0);
  const worldId = admin.db.collection('worlds').doc().id;
  const world = `http://127.0.0.1:4173/worlds/${worldId}`;
  const office: Office = (command, extra = []) =>
    runCli(
      sandbox,
      [
        'office',
        ...command,
        '--prefix',
        prefix,
        '--world',
        world,
        '--identity',
        identity,
        '--emulator',
        '--json',
        ...extra,
      ],
      { deadlineMs: 35_000 }
    );
  const pairFlags = (timeout: string) => [
    ...(readOnly ? ['--read-only'] : []),
    '--timeout',
    timeout,
  ];
  const pending = await office(['pair'], pairFlags('5'));
  expect(pending.status, pending.stdout).toBe(1);
  expect(JSON.parse(pending.stdout).error.code).toBe('OFFICE_PAIRING_PENDING');
  const link = pending.stderr.trim();
  expect(link.startsWith(`${world}/pair#tmt-pair=`)).toBe(true);
  const request = JSON.parse(
    Buffer.from(link.split('#tmt-pair=')[1]!, 'base64url').toString('utf8')
  ) as { pairingId: string; capabilities: string[] };
  expect(request.capabilities).toEqual(
    readOnly ? ['layout.read'] : ['layout.read', 'layout.write']
  );

  const { page } = await openSession(link);
  await signIn(page, readOnly ? 'Native read-only owner' : 'Native decoration owner');
  const ownerUid = (await page.getByText(/^UID: /).innerText()).replace(/^UID: /, '').trim();
  await admin.db
    .collection('worlds')
    .doc(worldId)
    .set({ version: 1, name: 'Native decoration office', ownerUid, createdAt: new Date() });
  await setTester(ownerUid, true);
  await expect(page.getByRole('region', { name: 'Agent pairing request' })).toBeVisible();
  await page.getByRole('checkbox', { name: 'I recognize this agent and installation.' }).check();
  await page.getByRole('button', { name: 'Approve pairing', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Approved. Return' })).toBeVisible();

  const paired = await office(['pair'], pairFlags('25'));
  expect(paired.status, paired.stdout).toBe(0);
  expect(JSON.parse(paired.stdout).state).toBe('credential');
  const pairing = admin.db.collection('officePairings').doc(request.pairingId);
  const remote = (await pairing.get()).data()!;
  const blockId = String(remote.blockId);
  const grantRef = admin.db
    .collection('worlds')
    .doc(worldId)
    .collection('agentGrants')
    .doc(String(remote.principalUid));
  expect((await grantRef.get()).data()).toMatchObject({
    enabled: true,
    blockId,
    capabilities: request.capabilities,
  });
  const blockRef = admin.db.collection('worlds').doc(worldId).collection('blocks').doc(blockId);
  return { blockId, blockRef, grantRef, office, page, sandbox, worldId };
}

export async function withNativeDecoration(
  openSession: OpenSession,
  readOnly: boolean,
  run: (context: NativeDecorationContext) => Promise<void>
): Promise<void> {
  const admin = createPairingEmulatorFixture();
  try {
    await withSandbox(async (sandbox) => {
      try {
        await run(await pairNative(sandbox, admin, openSession, readOnly));
      } finally {
        await clearOfficeScopes(sandbox);
      }
    });
  } finally {
    await admin.dispose();
  }
}

export function layoutFile(sandbox: Sandbox, name: string, objects: readonly unknown[]): string {
  const file = path.join(sandbox.root, name);
  writeFileSync(file, `${JSON.stringify({ objects })}\n`);
  return file;
}

export function rawFile(sandbox: Sandbox, name: string, contents: string): string {
  const file = path.join(sandbox.root, name);
  writeFileSync(file, contents);
  return file;
}

export async function openAssignedBlock(
  page: Page,
  worldId: string,
  blockId: string
): Promise<void> {
  await page.getByRole('link', { name: 'Office', exact: true }).click();
  await page.getByLabel('World ID', { exact: true }).fill(worldId);
  await page.getByRole('button', { name: 'Open world', exact: true }).click();
  const open = page.getByRole('button', { name: `Open block ${blockId}`, exact: true });
  await expect(open).toBeVisible();
  await open.click();
  await expect(page.getByRole('region', { name: 'Office block editor' })).toBeVisible();
}
