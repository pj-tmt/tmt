import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';

const inputLog = { mode: 'input-log' } as const;

describe('saved identity notes through verified tmux callers', { concurrent: false }, () => {
  it('uses the verified saved identity implicitly and remains available explicitly offline', async () => {
    await withE2EFixture(async (fixture) => {
      const bound = await fixture.runJsonCli<{ id: string }>(['name', 'Saved Researcher', '-s']);
      expect(bound).toMatchObject({ code: 0, json: { id: expect.any(String) } });
      const expected = path.join(fixture.globalDir, 'notes', bound.json!.id, 'notes.md');

      expect(await fixture.runJsonCli(['notes', 'path'])).toMatchObject({
        code: 0,
        json: { identityId: bound.json!.id, path: expected, created: true },
      });
      fs.writeFileSync(expected, '# durable local context\n');
      expect(
        await fixture.runJsonCli(['notes', 'path', '--identity', 'Saved Researcher'], {
          withoutTmux: true,
        })
      ).toMatchObject({
        code: 0,
        json: { identityId: bound.json!.id, path: expected, created: false },
      });
      expect(fs.readFileSync(expected, 'utf8')).toBe('# durable local context\n');
    }, inputLog);
  });

  it('rejects temporary and unverifiable implicit callers before creating a notebook', async () => {
    await withE2EFixture(async (fixture) => {
      const bound = await fixture.runJsonCli<{ id: string }>(['name', 'Temporary']);
      expect(bound.code).toBe(0);
      expect(await fixture.runJsonCli(['notes', 'path'])).toMatchObject({
        code: 1,
        json: { error: { code: 'NOTES_SAVED_IDENTITY_REQUIRED' } },
      });
      expect(fs.existsSync(path.join(fixture.globalDir, 'notes', bound.json!.id))).toBe(false);

      const metadata = JSON.parse(fixture.paneMetadata());
      metadata.globalIdentity.bindingId = 'different-binding';
      fixture.tmux([
        'set-option',
        '-p',
        '-t',
        fixture.pane,
        '@tmux-team.agent',
        JSON.stringify(metadata),
      ]);
      expect(await fixture.runJsonCli(['notes', 'path'])).toMatchObject({
        code: 1,
        json: { error: { code: 'IDENTITY_REQUIRED' } },
      });
      expect(fs.existsSync(path.join(fixture.globalDir, 'notes'))).toBe(false);
    }, inputLog);
  });
});
