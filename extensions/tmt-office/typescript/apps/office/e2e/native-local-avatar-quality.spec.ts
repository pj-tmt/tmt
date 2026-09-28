import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { readFileSync, symlinkSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice } from './native-office-fixture.js';

test('avatar authoring warnings stay advisory and read-only through the real companion', async () => {
  await withSandbox(async (sandbox) => {
    const prefix = path.join(sandbox.root, 'office-prefix');
    const file = path.join(sandbox.root, 'bot.tmtavatar.json');
    const pixels = Array.from({ length: 24 }, () => '0000000000000000');
    pixels[12] = '1000000000000000';
    const pack = {
      formatVersion: 1,
      label: 'Small silhouette',
      credit: 'TMT',
      license: 'MIT',
      palette: ['#00000000', '#ffffffff', '#223344ff'],
      avatars: [{ key: 'bot', label: 'Bot', pixels }],
    };
    const bytes = JSON.stringify(pack);
    writeFileSync(file, bytes);
    const office = (args: string[], json = true) =>
      runCli(sandbox, [
        'office',
        '--prefix',
        prefix,
        'avatar',
        ...args,
        ...(json ? ['--json'] : []),
      ]);
    const missing = await office(['validate', '--file', file]);
    expect(missing.status).toBe(1);
    expect(JSON.parse(missing.stdout).error.code).toBe('OFFICE_NOT_INSTALLED');
    expect(await installNativeOffice(sandbox)).toBe(prefix);
    const identity = await runCli(sandbox, ['identity', 'create', 'Artist', '--json']);
    expect(identity.status, identity.stdout).toBe(0);
    const state = () => {
      const db = new Database(sandbox.database, { readonly: true });
      try {
        return [
          'office_avatar_catalog',
          'office_avatar_packs',
          'office_local_profiles',
          'request_attempts',
        ].map((table) => db.prepare(`SELECT * FROM ${table}`).all());
      } finally {
        db.close();
      }
    };
    const before = state();
    let digest = '';
    for (const json of [true, false]) {
      const result = await office(['validate', '--file', file], json);
      expect(result.status, result.stdout).toBe(0);
      const output = JSON.parse(result.stdout);
      digest = output.digest;
      expect(output.avatars).toEqual([
        { key: 'bot', label: 'Bot', raster: { width: 16, height: 24 } },
      ]);
      expect(
        output.warnings.map((warning: { avatar: string; code: string }) => [
          warning.avatar,
          warning.code,
        ])
      ).toEqual([
        ['bot', 'opaque-edge'],
        ['bot', 'small-silhouette'],
        ['bot', 'single-color'],
      ]);
    }
    const quietFile = path.join(sandbox.root, 'padded.tmtavatar.json');
    const padded = structuredClone(pack);
    padded.avatars[0]!.pixels = Array.from({ length: 24 }, (_, row) =>
      row >= 6 && row < 18 ? '0000001221000000' : '0000000000000000'
    );
    writeFileSync(quietFile, JSON.stringify(padded));
    const quiet = await office(['validate', '--file', quietFile]);
    expect(quiet.status, quiet.stdout).toBe(0);
    expect(JSON.parse(quiet.stdout).warnings).toEqual([]);
    const invalidFile = path.join(sandbox.root, 'invalid.tmtavatar.json');
    writeFileSync(
      invalidFile,
      bytes.replace('"formatVersion":1', '"formatVersion":1,"formatVersion":1')
    );
    const invalid = await office(['validate', '--file', invalidFile]);
    expect(invalid.status).toBe(1);
    expect(JSON.parse(invalid.stdout).error.code).toBe('OFFICE_AVATAR_INVALID');
    expect(JSON.parse(invalid.stdout)).not.toHaveProperty('warnings');
    const linked = path.join(sandbox.root, 'linked.tmtavatar.json');
    symlinkSync(file, linked);
    expect((await office(['validate', '--file', linked])).status).toBe(1);
    expect(state()).toEqual(before);
    expect(readFileSync(file, 'utf8')).toBe(bytes);
    // The warning is not a hidden admission gate; the same source is installable.
    const installed = await office(['install', '--local', '--file', file, '--if-revision', '0']);
    expect(installed.status, installed.stdout).toBe(0);
    expect(JSON.parse(installed.stdout)).toMatchObject({
      digest,
      changed: true,
      catalogRevision: 1,
    });
    expect(JSON.parse(installed.stdout)).not.toHaveProperty('warnings');
    const shown = await office(['show', '--local', digest]);
    expect(shown.status, shown.stdout).toBe(0);
    expect(JSON.parse(shown.stdout).digest).toBe(digest);
    expect(JSON.parse(shown.stdout)).not.toHaveProperty('warnings');
    const after = state();
    expect(after[1]).toHaveLength(1);
    expect(after.slice(2)).toEqual(before.slice(2));
  });
});
