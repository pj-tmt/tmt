import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import Database from 'better-sqlite3';
import { withSandbox } from '../support/cli-process.js';
import { cli } from '../support/extension-hooks.js';
import { createArtifact } from '../support/native-artifact.js';
import { workspaceVersion } from '../support/workspace-version.js';

const officeVersion = workspaceVersion('tmt-office');

describe('consented extension hooks', () => {
  it('lets the Office companion adopt the contract once the user enables it', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'isolated office');
      const office = (args: string[]) => cli(sandbox, ['office', '--prefix', prefix, ...args]);
      const artifact = await createArtifact(sandbox, officeVersion, new Uint8Array(), 'office');
      await cli(sandbox, [
        '__native-install',
        '--product',
        'office',
        '--channel',
        'alpha',
        '--prefix',
        prefix,
        '--archive',
        artifact.archive,
        '--manifest',
        artifact.manifest,
      ]);
      await cli(sandbox, ['identity', 'create', 'Bea']);
      await office([
        'board',
        'post',
        '--general',
        '--identity',
        'Bea',
        '--title',
        'T',
        '--body',
        'B',
      ]);
      await office(['storage', 'migrate', '--yes']);
      sandbox.env.PATH = `${path.join(prefix, 'bin')}${path.delimiter}${sandbox.env.PATH ?? ''}`;
      const enabled = await cli(sandbox, ['extension', 'hooks', 'enable', 'office']);
      expect(enabled.enabled).toMatchObject({
        name: 'office',
        capabilities: ['context_v1', 'lifecycle_observations_v1'],
      });
      // Context is a read-only protocol reply; Bea has no desk yet, so no line.
      const bea = ((await cli(sandbox, ['identity', 'show', 'Bea'])).identity as { id: string }).id;
      const context = execFileSync(
        path.join(prefix, 'bin', 'tmt-office'),
        ['__tmt-hooks', '1', 'context'],
        {
          input: JSON.stringify({ version: 1, identityId: bea }),
          env: { ...sandbox.env, TMT_HOOK_DELIVERY: '1' },
        }
      ).toString();
      expect(JSON.parse(context)).toEqual({ summary: null });
      await cli(sandbox, ['rm', 'Bea', '--force']);
      // The retirement observation made Office reconcile before any Office command ran.
      const marker = new Database(path.join(sandbox.globalDir, 'office', 'office.db'), {
        readonly: true,
      });
      try {
        expect(
          marker.prepare('SELECT count(*) AS count FROM office_retired_identities').get()
        ).toEqual({
          count: 1,
        });
      } finally {
        marker.close();
      }
    });
  });
});
