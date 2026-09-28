import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { withE2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';

const inputLog = { mode: 'input-log' } as const;

describe.sequential('read-only identity context through verified callers', () => {
  it('distinguishes a verified empty pane from invalid caller evidence without mutations', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Former Reader', '-s'])).code).toBe(0);
      expect((await fixture.runJsonCli(['unbind'])).code).toBe(0);
      const before = durableState(fixture);
      const metadata = fixture.paneMetadata();
      const context = await fixture.runCli(['whoami', '--context']);
      expect(context).toMatchObject({
        code: 0,
        stderr: '',
        stdout: 'This pane has no TMT identity; run: tmt name <name> (-s to save)\n',
      });
      expect(await fixture.runJsonCli(['whoami', '--context'])).toMatchObject({
        code: 0,
        json: { bound: false, status: 'unbound' },
      });
      expect(
        await fixture.runJsonCli(['whoami', '--context'], { caller: { pane: 'invalid' } })
      ).toMatchObject({
        code: 0,
        json: { bound: false, status: 'unavailable' },
      });
      expect(durableState(fixture)).toEqual(before);
      expect(fixture.paneMetadata()).toBe(metadata);
    }, inputLog);
  });

  for (const saved of [false, true]) {
    it(`observes a ${saved ? 'saved' : 'temporary'} binding without renewing or creating state`, async () => {
      await withE2EFixture(async (fixture) => {
        const bound = await fixture.runJsonCli<{ id: string }>([
          'name',
          'Context Reader',
          ...(saved ? ['-s'] : []),
        ]);
        expect(bound).toMatchObject({ code: 0, json: { id: expect.any(String) } });
        expect(
          (await fixture.runJsonCli(['role', 'set', 'Review implementation and tests.'])).code
        ).toBe(0);
        const before = durableState(fixture);
        const notesRoot = path.join(fixture.globalDir, 'notes');
        expect(fs.existsSync(notesRoot)).toBe(false);

        const context = await fixture.runJsonCli(['whoami', '--context']);
        expect(context).toMatchObject({
          code: 0,
          json: {
            bound: true,
            id: bound.json!.id,
            name: 'Context Reader',
            lifetime: saved ? 'saved' : 'temporary',
            role: 'Review implementation and tests.',
            notesPath: null,
            extensions: [],
            originated: { count: 0, inspect: `tmt x --identity '${bound.json!.id}' --json` },
            incoming: {
              count: 0,
              inspect: `tmt x --incoming --identity '${bound.json!.id}' --json`,
            },
          },
        });
        expect(Buffer.byteLength(context.stdout)).toBeLessThanOrEqual(4096);
        const human = await fixture.runCli(['whoami', '--context']);
        expect(human.code).toBe(0);
        expect(human.stdout).toContain(bound.json!.id);
        expect(human.stdout).toContain('Role: "Review implementation and tests."');
        if (saved) console.info('Rendered identity context example:\n' + human.stdout);
        expect(Buffer.byteLength(human.stdout)).toBeLessThanOrEqual(4096);
        // Retain last_verified_at: context must not reconcile or renew the binding.
        expect(durableState(fixture)).toEqual(before);
        expect(fs.existsSync(notesRoot)).toBe(false);
      }, inputLog);
    });
  }

  it('returns an existing saved notebook path without reading or rewriting its content', async () => {
    await withE2EFixture(async (fixture) => {
      const bound = await fixture.runJsonCli<{ id: string }>(['name', 'Notebook Reader', '-s']);
      expect(bound.code).toBe(0);
      const notebook = await fixture.runJsonCli<{ path: string }>(['notes', 'path']);
      expect(notebook.code).toBe(0);
      const notesPath = notebook.json!.path;
      const content = 'Private notebook content must not be injected.\n';
      fs.writeFileSync(notesPath, content);
      const before = durableState(fixture);
      const stat = fs.statSync(notesPath);
      const context = await fixture.runJsonCli(['whoami', '--context']);
      expect(context).toMatchObject({ code: 0, json: { bound: true, notesPath } });
      expect(context.stdout).not.toContain(content.trim());
      expect(fs.readFileSync(notesPath, 'utf8')).toBe(content);
      expect(fs.statSync(notesPath).mtimeMs).toBe(stat.mtimeMs);
      expect(durableState(fixture)).toEqual(before);
    }, inputLog);
  });
});
