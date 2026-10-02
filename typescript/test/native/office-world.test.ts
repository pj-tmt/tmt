import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, it } from 'vite-plus/test';
import {
  expectError,
  parseWholeStdout,
  runCli,
  withSandbox,
  type CliResult,
} from '../support/cli-process.js';
import { createArtifact } from '../support/native-artifact.js';

function success(result: CliResult) {
  expect(result.status).toBe(0);
  return parseWholeStdout(result);
}

it(
  'reads and saves one local world without identity or service, with explicit conflict fencing',
  { timeout: 120_000 },
  async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'isolated office');
      const office = (args: string[]) =>
        runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], { deadlineMs: 15_000 });
      expectError(await office(['layout', 'show']), 'OFFICE_NOT_INSTALLED');
      // Retained-install fixture: public Office acquisition is frozen.
      const artifact = await createArtifact(sandbox, '0.1.0-alpha.4', new Uint8Array(), 'office');
      success(
        await runCli(
          sandbox,
          [
            '__native-install',
            '--product',
            'office',
            '--channel',
            'alpha',
            '--prefix',
            prefix,
            '--json',
            '--archive',
            artifact.archive,
            '--manifest',
            artifact.manifest,
          ],
          { deadlineMs: 15_000 }
        )
      );
      // No tmux, identity, credentials or HTTP service is needed. Each invocation is a new process.
      const preview = success(await office(['layout', 'show']));
      expect(preview).toMatchObject({ worldId: null, revision: 0, changed: false });
      expect(preview.legacyBasis).toMatch(/^[a-f0-9]{64}$/);
      expect(success(await office(['layout', 'show']))).toEqual(preview);
      const file = path.join(sandbox.cwd, 'world draft.json');
      writeFileSync(file, JSON.stringify(preview.layout));
      const apply = (revision: number, basis?: string) =>
        office([
          'layout',
          'apply',
          '--file',
          file,
          '--if-revision',
          String(revision),
          ...(basis === undefined ? [] : ['--legacy-basis', basis]),
        ]);
      expectError(await apply(0), 'WORLD_INVALID');
      expect(success(await office(['layout', 'show']))).toEqual(preview);

      const saved = success(await apply(0, preview.legacyBasis as string));
      expect(saved).toMatchObject({
        revision: 1,
        legacyBasis: null,
        changed: true,
        layout: preview.layout,
      });
      expect(saved.worldId).toMatch(/^[a-f0-9-]{36}$/);
      const reopened = success(await office(['layout', 'show']));
      expect(reopened).toEqual({ ...saved, changed: false });
      expect(success(await apply(1))).toEqual(reopened);
      expectError(await apply(0, preview.legacyBasis as string), 'WORLD_REVISION_CONFLICT');
      expect(success(await office(['layout', 'show']))).toEqual(reopened);

      // Malformed candidates must not be accepted merely because the revision is current.
      writeFileSync(file, JSON.stringify({ ...(reopened.layout as object), extra: true }));
      expectError(await apply(1), 'WORLD_INVALID');
      expect(success(await office(['layout', 'show']))).toEqual(reopened);

      // Deleting placement records edits the same layout, not a second per-agent block.
      const empty = { ...(reopened.layout as object), objects: [] };
      writeFileSync(file, JSON.stringify(empty));
      const cleared = success(await apply(1));
      expect(cleared).toMatchObject({
        worldId: saved.worldId,
        revision: 2,
        changed: true,
        layout: empty,
      });
      expect(success(await office(['layout', 'show']))).toEqual({ ...cleared, changed: false });
    });
  }
);
