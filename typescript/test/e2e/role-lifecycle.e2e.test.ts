import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vitest';
import { withE2EFixture, type E2EFixture, type CliResult } from './harness.js';

interface RoleResult {
  identity: {
    id: string;
    name: string;
    canonicalName: string;
    lifetime: 'temporary' | 'saved';
  };
  role: { content: string; updatedAt: string } | null;
}

function successful(result: CliResult<RoleResult>): RoleResult {
  expect(result.code, result.stderr || result.stdout).toBe(0);
  expect(result.json).toBeDefined();
  return result.json!;
}

function storedProfiles(fixture: E2EFixture): unknown[] {
  const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
  try {
    return database
      .prepare('SELECT identity_id, content, updated_at FROM role_profiles ORDER BY identity_id')
      .all();
  } finally {
    database.close();
  }
}

async function showOffline(fixture: E2EFixture, identity: string): Promise<RoleResult> {
  return successful(
    await fixture.runJsonCli<RoleResult>(['role', 'show', '--identity', identity], {
      withoutTmux: true,
    })
  );
}

describe('durable role profiles', { concurrent: false }, () => {
  it('preserves one identity profile through unbind, pane death, restart, and rebind', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Alice', '-s'])).code).toBe(0);
      const empty = successful(await fixture.runJsonCli<RoleResult>(['role', 'show']));
      expect(empty.role).toBeNull();
      const content = '# Reviewer\n\tPreserve evidence.\n';
      const assigned = successful(await fixture.runJsonCli<RoleResult>(['role', 'set', content]));
      expect(assigned.identity).toEqual(empty.identity);
      expect(assigned.role?.content).toBe(content);
      expect(assigned.role?.updatedAt).toMatch(/^\d{4}-\d{2}-\d{2}T/);
      const talk = await fixture.runJsonCli([
        'talk',
        'Alice',
        'profile-not-injected',
        '--timeout',
        '5',
      ]);
      expect(talk.code, talk.stderr || talk.stdout).toBe(0);
      const request = fixture.events().find((event) => event.event === 'request');
      expect(request?.message).toContain('profile-not-injected');
      expect(request?.message).not.toContain('Preserve evidence.');

      const peer = await fixture.createMockPane('peer');
      expect((await fixture.runJsonCli(['add', peer.pane, 'Bob'])).code).toBe(0);
      const bob = successful(
        await fixture.runJsonCli<RoleResult>([
          'role',
          'set',
          'Independent profile',
          '--identity',
          'Bob',
        ])
      );
      const before = storedProfiles(fixture);
      expect(before).toHaveLength(2);
      expect((await fixture.runJsonCli(['unbind'])).code).toBe(0);
      expect(await showOffline(fixture, 'ALICE')).toEqual(assigned);
      expect(storedProfiles(fixture)).toEqual(before);

      const replacement = await fixture.createMockPane('replacement');
      expect((await fixture.runJsonCli(['add', replacement.pane, 'alice'])).code).toBe(0);
      fixture.tmux(['kill-pane', '-t', replacement.pane]);
      expect(await showOffline(fixture, 'Alice')).toEqual(assigned);
      await fixture.restartServer();
      expect(await showOffline(fixture, 'Alice')).toEqual(assigned);
      expect(await showOffline(fixture, 'Bob')).toEqual(bob);
      expect((await fixture.runJsonCli(['name', 'alice'])).code).toBe(0);
      expect(successful(await fixture.runJsonCli<RoleResult>(['role', 'show']))).toEqual(assigned);

      for (let attempt = 0; attempt < 2; attempt++) {
        const cleared = successful(await fixture.runJsonCli<RoleResult>(['role', 'clear']));
        expect(cleared).toEqual({ identity: assigned.identity, role: null });
      }
      expect(storedProfiles(fixture)).toEqual([
        {
          identity_id: bob.identity.id,
          content: bob.role!.content,
          updated_at: bob.role!.updatedAt,
        },
      ]);
      expect((await fixture.runJsonCli(['whoami'])).json).toEqual({
        interfaceKind: 'container',
        sessionState: 'unknown',
        bound: true,
        id: assigned.identity.id,
        name: 'Alice',
        pane: fixture.pane,
        lifetime: 'saved',
      });
      expect(await showOffline(fixture, 'Bob')).toEqual(bob);
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
    });
  }, 30_000);

  it('shares inline/file normalization and rejects invalid writes without changing stored data', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Writer'])).code).toBe(0);
      const raw = '\uFEFF# Profile\r\n\tIndented text  \rFinal line\r\n';
      const normalized = '# Profile\n\tIndented text  \nFinal line\n';
      const inline = successful(
        await fixture.runJsonCli<RoleResult>(['role', 'set', raw, '--identity', 'Writer'])
      );
      const file = path.join(fixture.workspace, 'profile.md');
      fs.writeFileSync(file, raw);
      const fromFile = successful(
        await fixture.runJsonCli<RoleResult>(
          ['role', '--identity=Writer', 'set', '--file=profile.md'],
          {
            withoutTmux: true,
          }
        )
      );
      expect(inline.role?.content).toBe(normalized);
      expect(fromFile.role?.content).toBe(normalized);
      expect(fromFile.identity).toEqual(inline.identity);
      const human = await fixture.runCli(['role', 'show', '--identity', 'Writer'], {
        withoutTmux: true,
      });
      expect(human.code).toBe(0);
      expect(human.stdout).toContain(normalized);
      const before = storedProfiles(fixture);

      const invalidFiles: Array<[string, Buffer | string, string]> = [
        ['malformed', Buffer.from([0xc3, 0x28]), 'ROLE_INPUT_INVALID'],
        ['binary', Buffer.from([0x61, 0x00, 0x62]), 'ROLE_INPUT_INVALID'],
        ['empty', ' \r\n\t', 'ROLE_INPUT_INVALID'],
        ['large', 'a'.repeat(65_537), 'ROLE_INPUT_TOO_LARGE'],
      ];
      for (const [name, data, code] of invalidFiles) {
        const invalidPath = path.join(fixture.workspace, name);
        fs.writeFileSync(invalidPath, data);
        const failed = await fixture.runJsonCli([
          'role',
          'set',
          '--file',
          invalidPath,
          '--identity',
          'Writer',
        ]);
        expect(failed.code).toBe(1);
        expect(failed.json).toMatchObject({ error: { code } });
        expect(storedProfiles(fixture)).toEqual(before);
      }
      for (const invalidPath of [fixture.workspace, path.join(fixture.workspace, 'missing')]) {
        const failed = await fixture.runJsonCli([
          'role',
          'set',
          '--file',
          invalidPath,
          '--identity',
          'Writer',
        ]);
        expect(failed.code).toBe(1);
        expect(failed.json).toMatchObject({ error: { code: 'ROLE_FILE_ERROR' } });
        expect(storedProfiles(fixture)).toEqual(before);
      }
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
    });
  }, 30_000);

  it('keeps explicit roles offline while rejecting unrelated implicit callers after discovery', async () => {
    await withE2EFixture(async (fixture) => {
      const invalid = await fixture.runJsonCli(['role', 'set', 'inline', '--file', 'file.md']);
      expect(invalid.code).toBe(1);
      expect(fs.existsSync(path.join(fixture.globalDir, 'tmux-team.db'))).toBe(false);
      const missing = await fixture.runJsonCli(['role', 'show', '--identity', 'missing'], {
        withoutTmux: true,
      });
      expect(missing.code).toBe(3);
      expect(missing.json).toMatchObject({ error: { code: 'NAME_NOT_FOUND' } });
      // Implicit selection may perform bounded read-only discovery. Explicit
      // identity access above/below must still never invoke tmux.
      const outside = await fixture.runJsonCli(['role', 'show'], { outsideTmux: true });
      expect(outside.code).toBe(1);
      expect(outside.json).toMatchObject({ error: { code: 'IDENTITY_REQUIRED' } });
      const unbound = await fixture.runJsonCli(['role', 'show']);
      expect(unbound.code).toBe(1);
      expect(unbound.json).toMatchObject({ error: { code: 'IDENTITY_REQUIRED' } });
      expect((await fixture.runJsonCli(['name', 'Empty'])).code).toBe(0);
      const metadataBefore = fixture.paneMetadata();
      const profilesBefore = storedProfiles(fixture);
      const outsideWithBoundDefault = await fixture.runJsonCli(['role', 'show'], {
        outsideTmux: true,
      });
      expect(outsideWithBoundDefault.code).toBe(1);
      expect(outsideWithBoundDefault.json).toMatchObject({ error: { code: 'IDENTITY_REQUIRED' } });
      expect(fixture.paneMetadata()).toBe(metadataBefore);
      expect(storedProfiles(fixture)).toEqual(profilesBefore);
      expect((await showOffline(fixture, 'Empty')).role).toBeNull();
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
      // Calibrate the guard: a command that actually needs tmux must trip it.
      await fixture.runJsonCli(['list'], { withoutTmux: true });
      expect(fs.readFileSync(fixture.forbiddenTmuxLogPath, 'utf8')).toContain(
        'unexpected tmux invocation'
      );
    });
  }, 30_000);

  it('keeps concurrent CLI updates whole and returns each transaction own profile', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Concurrent'])).code).toBe(0);
      const contents = ['A', 'B', 'C', 'D'].map((letter) => `${letter.repeat(4096)}\n${letter}`);
      const results = await Promise.all(
        contents.map((content) =>
          fixture.runJsonCli<RoleResult>(['role', 'set', content, '--identity', 'Concurrent'], {
            withoutTmux: true,
          })
        )
      );
      const committed = results.map(successful);
      committed.forEach((result, index) => expect(result.role?.content).toBe(contents[index]));
      const final = await showOffline(fixture, 'Concurrent');
      expect(committed).toContainEqual(final);
      expect(storedProfiles(fixture)).toEqual([
        {
          identity_id: final.identity.id,
          content: final.role!.content,
          updated_at: final.role!.updatedAt,
        },
      ]);
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
    });
  }, 30_000);
});
