import path from 'node:path';
import { readdirSync } from 'node:fs';
import { expect } from '@playwright/test';
import { createArtifact } from '../../../../../../typescript/test/support/native-artifact.js';
import { runCli, type Sandbox } from '../../../../../../typescript/test/support/cli-process.js';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';

export { unusedLoopbackPort } from '../../../../../../typescript/test/support/loopback-port.mjs';

export const NATIVE_OFFICE_FIXTURE_VERSION = '0.1.0-alpha.4';

export async function installNativeOffice(
  sandbox: Sandbox,
  executable = path.resolve('../../../../../rust/target/debug/tmt-office')
): Promise<string> {
  const artifact = await createArtifact(
    sandbox,
    NATIVE_OFFICE_FIXTURE_VERSION,
    new Uint8Array(),
    'office',
    executable
  );
  const prefix = path.join(sandbox.root, 'office-prefix');
  const installed = await runCli(
    sandbox,
    [
      'office',
      'install',
      '--yes',
      '--prefix',
      prefix,
      '--archive',
      artifact.archive,
      '--manifest',
      artifact.manifest,
      '--json',
    ],
    { deadlineMs: 20_000 }
  );
  expect(installed.status, installed.stdout).toBe(0);
  expect(installed.stderr).toBe('');
  return prefix;
}

export function protectedOfficeRecord(
  sandbox: Sandbox,
  scopeKey: string,
  operation: 'lookup' | 'store' | 'clear',
  input?: string
) {
  return runCli(
    { ...sandbox, cli: { executable: '/usr/bin/secret-tool', args: [] } },
    [
      operation,
      ...(operation === 'store' ? ['--label', 'Native Office fixture'] : []),
      'service',
      'org.tmux-team.office.v1',
      'username',
      scopeKey,
    ],
    input === undefined ? {} : { stdin: input }
  );
}

export function officeScopeKeys(sandbox: Sandbox): string[] {
  try {
    return readdirSync(path.join(sandbox.globalDir, 'office'))
      .filter((name) => /^[0-9a-f]{64}\.lock$/.test(name))
      .map((name) => name.slice(0, -'.lock'.length));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return [];
    throw error;
  }
}

export async function clearOfficeScopes(sandbox: Sandbox): Promise<void> {
  for (const key of officeScopeKeys(sandbox)) {
    expect((await protectedOfficeRecord(sandbox, key, 'clear')).status).toBe(0);
    expect((await protectedOfficeRecord(sandbox, key, 'lookup')).status).toBe(1);
  }
}

/**
 * Writes one board thread into the shared core file, as a pre-extraction Office
 * did. A fresh install now switches to `office.db` on its first Office command, so
 * only rows that already exist keep an install on the legacy store.
 */
export async function seedLegacyOfficeData(sandbox: Sandbox): Promise<void> {
  const initialized = await runCli(sandbox, ['identity', 'list', '--json']);
  expect(initialized.status, initialized.stdout + initialized.stderr).toBe(0);
  const database = new Database(sandbox.database);
  try {
    const id = '11111111-1111-4111-8111-111111111111';
    database
      .prepare(
        `INSERT INTO office_board_entries
           (id, thread_id, is_root, category_kind, category_id, author_kind, author_id,
            author_name, revision, deleted, created_sequence, activity_sequence,
            created_at_ms, updated_at_ms, title, body)
         VALUES (?, ?, 1, 'general', NULL, 'owner', 'owner', NULL, 1, 0, 1, 1, 1, 1, 'Hi', 'Data')`
      )
      .run(id, id);
    database.prepare('UPDATE office_board_state SET revision = 1, next_sequence = 2').run();
  } finally {
    database.close();
  }
}
