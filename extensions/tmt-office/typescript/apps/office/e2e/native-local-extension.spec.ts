import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { readFileSync, symlinkSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice } from './native-office-fixture.js';
import pairs from '../../../../../../contracts/office/extension-pair-vectors.json' with { type: 'json' };

test('extension preflight uses the installed companion, rejects unsafe inputs and never writes Office state', async () => {
  await withSandbox(async (sandbox) => {
    const definitionFile = path.join(sandbox.root, 'definition.json');
    const instanceFile = path.join(sandbox.root, 'instance.json');
    const definition = JSON.stringify(pairs.definition);
    writeFileSync(definitionFile, definition);
    writeFileSync(instanceFile, JSON.stringify(pairs.cases[0]!.instance));
    const prefix = path.join(sandbox.root, 'office-prefix');
    const validate = (file = definitionFile, json = true) =>
      runCli(sandbox, [
        'office',
        '--prefix',
        prefix,
        'extension',
        'validate',
        '--file',
        file,
        '--instance',
        instanceFile,
        ...(json ? ['--json'] : []),
      ]);
    const absent = await validate();
    expect(absent.status).toBe(1);
    expect(JSON.parse(absent.stdout).error.code).toBe('OFFICE_NOT_INSTALLED');
    expect(await installNativeOffice(sandbox)).toBe(prefix);
    const shown = await runCli(sandbox, ['office', '--prefix', prefix, 'layout', 'show', '--json']);
    expect(shown.status, shown.stdout).toBe(0);
    const preview = JSON.parse(shown.stdout);
    const layoutFile = path.join(sandbox.root, 'existing-layout.json');
    writeFileSync(layoutFile, JSON.stringify(preview.layout));
    const saved = await runCli(sandbox, [
      'office',
      '--prefix',
      prefix,
      'layout',
      'apply',
      '--file',
      layoutFile,
      '--if-revision',
      '0',
      '--legacy-basis',
      preview.legacyBasis,
      '--json',
    ]);
    expect(saved.status, saved.stdout).toBe(0);
    const storedState = () => {
      const db = new Database(sandbox.database, { readonly: true });
      try {
        return [
          'office_local_worlds',
          'office_local_blocks',
          'office_local_profiles',
          'office_prop_packs',
          'office_avatar_packs',
          'office_board_entries',
          'office_whiteboards',
          'office_dispatch_operations',
          'request_attempts',
        ].map((table) => db.prepare(`SELECT * FROM ${table}`).all());
      } finally {
        db.close();
      }
    };
    const before = storedState();
    expect(before[0]).toHaveLength(1);
    for (const pair of pairs.cases) {
      writeFileSync(instanceFile, JSON.stringify(pair.instance));
      const result = await validate();
      expect(result.status, pair.name).toBe(pair.problem === null ? 0 : 1);
      expect(result.stderr).toBe('');
      const output = JSON.parse(result.stdout);
      if (pair.problem === null) {
        expect(output).toEqual({
          status: 'valid',
          definition: 'board',
          instance: 'lobby-board',
          scope: 'structureOnly',
        });
      } else {
        expect(output.error.code).toBe('OFFICE_EXTENSION_INVALID');
        expect(output.error.message).toEqual(expect.any(String));
      }
    }
    writeFileSync(instanceFile, JSON.stringify(pairs.cases[0]!.instance));
    const text = await validate(definitionFile, false);
    expect(text.status).toBe(0);
    expect(text.stdout).toContain('Artwork availability and host capabilities were not checked.');
    const badFile = path.join(sandbox.root, 'bad.json');
    const link = path.join(sandbox.root, 'linked.json');
    symlinkSync(definitionFile, link);
    for (const file of [link, sandbox.root]) {
      const result = await validate(file);
      expect(result.status).toBe(1);
      expect(JSON.parse(result.stdout).error.code).toBe('OFFICE_EXTENSION_INPUT_INVALID');
    }
    for (const content of [Buffer.from([0xff]), Buffer.alloc(65_537, ' ')]) {
      writeFileSync(badFile, content);
      const result = await validate(badFile);
      expect(result.status).toBe(1);
      expect(JSON.parse(result.stdout).error.code).toBe('OFFICE_EXTENSION_INPUT_INVALID');
    }
    writeFileSync(
      badFile,
      definition.replace('"formatVersion":1', '"formatVersion":1,"formatVersion":1')
    );
    const duplicate = await validate(badFile);
    expect(duplicate.status).toBe(1);
    expect(JSON.parse(duplicate.stdout).error.code).toBe('OFFICE_EXTENSION_INVALID');
    expect(readFileSync(definitionFile, 'utf8')).toBe(definition);
    expect(storedState()).toEqual(before);
  });
});
