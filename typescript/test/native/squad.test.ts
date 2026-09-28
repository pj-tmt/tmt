import Database from 'better-sqlite3';
import { existsSync, mkdirSync, readFileSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';

// Scenario-local selector: the built squad extension, never an installed copy.
const squadExecutable =
  process.env.TMT_TEST_SQUAD ??
  fileURLToPath(new URL('../../../rust/target/debug/tmt-squad', import.meta.url));

/** Puts `tmt-squad` and its `tmt-sq` alias link on the sandbox PATH, as shipped. */
function installSquad(sandbox: Sandbox): string {
  if (!path.isAbsolute(squadExecutable) || !statSync(squadExecutable).isFile()) {
    throw new Error(`Build tmt-squad first (cargo build -p tmt-squad): ${squadExecutable}`);
  }
  const bin = path.join(sandbox.root, 'bin');
  mkdirSync(bin);
  symlinkSync(squadExecutable, path.join(bin, 'tmt-squad'));
  symlinkSync('tmt-squad', path.join(bin, 'tmt-sq'));
  sandbox.env.PATH = `${bin}${path.delimiter}${sandbox.env.PATH ?? ''}`;
  return bin;
}

async function squad(sandbox: Sandbox, args: string[]) {
  const result = await runCli(sandbox, ['squad', ...args, '--json']);
  return { status: result.status, body: JSON.parse(result.stdout), stderr: result.stderr };
}

async function identity(sandbox: Sandbox, name: string): Promise<string> {
  const result = await runCli(sandbox, ['identity', 'create', name, '--json']);
  expect(result.status).toBe(0);
  return JSON.parse(result.stdout).identity.id;
}

// Independent observation of core state; squad has no store of its own.
function observe(sandbox: Sandbox) {
  const db = new Database(sandbox.database, { readonly: true });
  try {
    return {
      rooms: db.prepare('SELECT name, retired FROM office_meeting_rooms ORDER BY name').all() as {
        name: string;
        retired: number;
      }[],
      members: db
        .prepare(
          `SELECT r.name AS room, i.name AS identity FROM office_meeting_members m
           JOIN office_meeting_rooms r ON r.room_id = m.room_id
           JOIN identities i ON i.id = m.identity_id ORDER BY room, identity`
        )
        .all(),
      metadata: db
        .prepare(
          `SELECT i.name AS identity, m.key, m.value FROM identity_metadata m
           JOIN identities i ON i.id = m.identity_id ORDER BY identity, key`
        )
        .all(),
    };
  } finally {
    db.close();
  }
}

describe('squad extension', () => {
  it('behaves identically through tmt squad, tmt sq and both help paths', async () => {
    await withSandbox(async (sandbox) => {
      const bin = installSquad(sandbox);
      const pairs = [
        [
          ['squad', '--help'],
          ['sq', '--help'],
        ],
        [
          ['help', 'squad'],
          ['help', 'sq'],
        ],
        [
          ['squad', 'status', '--json'],
          ['sq', 'status', '--json'],
        ],
        [
          ['squad', 'bogus'],
          ['sq', 'bogus'],
        ],
      ];
      for (const [long, short] of pairs) {
        const [a, b] = [await runCli(sandbox, long), await runCli(sandbox, short)];
        expect({ status: b.status, stdout: b.stdout, stderr: b.stderr }).toEqual({
          status: a.status,
          stdout: a.stdout,
          stderr: a.stderr,
        });
      }
      const help = await runCli(sandbox, ['squad', '--help']);
      expect(help.stdout).toContain('Usage: tmt squad [OPTIONS] <COMMAND>');
      const status = await squad(sandbox, ['status']);
      expect(status).toMatchObject({ status: 1, body: { error: { code: 'SQUAD_NOT_FOUND' } } });
      // Completion v1: core invokes `tmt-<name> __complete -- <words>` directly.
      const completions = [];
      for (const name of ['tmt-squad', 'tmt-sq']) {
        const direct = { ...sandbox, cli: { executable: path.join(bin, name), args: [] } };
        completions.push(await runCli(direct, ['__complete', '--', 's']));
      }
      expect(completions[0].stdout).toBe('set\nskill\nstatus\n');
      expect(completions[1].stdout).toBe(completions[0].stdout);
      const skill = await runCli(sandbox, ['sq', 'skill', 'show']);
      expect(skill.stdout).toBe(
        readFileSync(
          fileURLToPath(
            new URL('../../../extensions/tmt-squad/skills/tmt-squad/SKILL.md', import.meta.url)
          ),
          'utf8'
        )
      );
    });
  });

  it('initializes only after settling who the user is, preserving squad.toml', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'Ben');
      const squadToml = path.join(sandbox.globalDir, 'squad.toml');

      const refused = await squad(sandbox, ['init', 'product']);
      expect(refused).toMatchObject({ status: 1, body: { error: { code: 'SQUAD_ME_REQUIRED' } } });
      expect(observe(sandbox).rooms).toEqual([]);
      expect(existsSync(squadToml)).toBe(false);
      const unknown = await squad(sandbox, ['init', 'product', '--me', 'Nobody']);
      expect(unknown.body.error.code).toBe('NAME_NOT_FOUND');
      expect((await squad(sandbox, ['init', 'Product', '--me', 'Ben'])).body.error.code).toBe(
        'SQUAD_NAME_INVALID'
      );

      const userText = '# mine\n[squad.product]\nlayout = "crew" # keep\n';
      writeFileSync(squadToml, userText);
      const created = await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      expect(created).toMatchObject({ status: 0, body: { created: true, me: 'Ben' } });
      const written = readFileSync(squadToml, 'utf8');
      expect(written).toContain(userText);
      expect(written).toContain('me = "Ben"');
      const again = await squad(sandbox, ['init', 'product']);
      expect(again).toMatchObject({ status: 0, body: { created: false, me: 'Ben' } });
      expect(readFileSync(squadToml, 'utf8')).toBe(written);
      expect(observe(sandbox).rooms).toEqual([{ name: 'squad-product', retired: 0 }]);
    });
  });

  it('manages lead and members through core rooms and namespaced metadata only', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['Ben', 'Sol', 'Rin', 'auth-fix', 'docs-sweep'])
        await identity(sandbox, name);
      expect((await squad(sandbox, ['init', 'product', '--me', 'Ben'])).status).toBe(0);
      await runCli(sandbox, ['identity', 'meta', 'set', 'team', 'core', '--identity', 'auth-fix']);

      expect((await squad(sandbox, ['lead', 'Sol'])).body.lead.name).toBe('Sol');
      const added = await squad(sandbox, ['add', 'auth-fix', 'docs-sweep', 'ghost']);
      expect(added.status).toBe(1);
      expect(added.body.results).toEqual([
        expect.objectContaining({ name: 'auth-fix', stateSet: 'working' }),
        expect.objectContaining({ name: 'docs-sweep', stateSet: 'working' }),
        { name: 'ghost', error: expect.objectContaining({ code: 'NAME_NOT_FOUND' }) },
      ]);
      const readded = await squad(sandbox, ['add', 'auth-fix']);
      expect(readded.body.results[0].stateSet).toBeNull();

      const before = observe(sandbox);
      const invalid = await squad(sandbox, ['set', 'auth-fix', 'state=blocked', 'Bad=x']);
      expect(invalid.body.error.code).toBe('SQUAD_FIELD_INVALID');
      expect(observe(sandbox)).toEqual(before);
      const outsider = await squad(sandbox, ['set', 'Rin', 'state=working']);
      expect(outsider.body.error.code).toBe('SQUAD_NOT_MEMBER');
      const set = await squad(sandbox, [
        'set',
        'auth-fix',
        'state=blocked',
        'pending=approve the token rotation plan',
        'note=needs a login-vs-sweep call',
      ]);
      expect(set.body.applied).toEqual([
        'squad.product.state',
        'squad.product.pending',
        'squad.product.note',
      ]);

      const status = await squad(sandbox, ['status']);
      expect(status.status).toBe(0);
      expect(status.body.squad).toMatchObject({
        name: 'product',
        layout: 'crew',
        lead: { name: 'Sol' },
      });
      expect(status.body.sections).toHaveLength(1);
      expect(status.body.sections[0].title).toBeNull();
      const rows = status.body.sections[0].rows;
      expect(rows.map((row: { name: string }) => row.name)).toEqual(['auth-fix', 'docs-sweep']);
      expect(rows[0]).toMatchObject({
        state: 'blocked',
        pending: 'approve the token rotation plan',
        note: 'needs a login-vs-sweep call',
        presence: 'offline',
        fields: { state: 'blocked' },
      });
      expect(rows[1]).toMatchObject({ state: 'working', pending: null, note: null });
      // The lead skill documents this row shape; it must not drift silently.
      const skill = readFileSync(
        fileURLToPath(
          new URL('../../../extensions/tmt-squad/skills/tmt-squad/SKILL.md', import.meta.url)
        ),
        'utf8'
      );
      const documented = skill.slice(
        skill.indexOf('- Each row has'),
        skill.indexOf('- A row with')
      );
      const documentedFields = [...documented.matchAll(/`([a-z]+)`/g)]
        .map((match) => match[1])
        .filter((field) => !['active', 'offline', 'unknown'].includes(field));
      expect(documentedFields.sort()).toEqual(Object.keys(rows[0]).sort());
      expect(Object.keys(status.body.squad).sort()).toEqual(['layout', 'lead', 'name', 'roomId']);
      const text = await runCli(sandbox, ['sq', 'status']);
      expect(text.stdout).toContain('◆ auth-fix');
      expect(text.stdout).toContain('    waiting on you: approve the token rotation plan\n');

      expect((await squad(sandbox, ['set', 'auth-fix', 'pending='])).status).toBe(0);
      expect((await squad(sandbox, ['lead', 'Rin'])).body.replaced).toEqual(['Sol']);
      const removed = await squad(sandbox, ['remove', 'auth-fix']);
      expect(removed.body.cleared.sort()).toEqual(['note', 'state']);
      expect((await squad(sandbox, ['remove', 'auth-fix'])).body.cleared).toEqual([]);

      const after = observe(sandbox);
      expect(after.members).toEqual([
        { room: 'squad-product', identity: 'Rin' },
        { room: 'squad-product', identity: 'Sol' },
        { room: 'squad-product', identity: 'docs-sweep' },
      ]);
      expect(after.metadata).toEqual([
        { identity: 'Rin', key: 'squad.product.role', value: 'lead' },
        { identity: 'auth-fix', key: 'team', value: 'core' },
        { identity: 'docs-sweep', key: 'squad.product.state', value: 'working' },
      ]);
      expect(after.rooms).toEqual([{ name: 'squad-product', retired: 0 }]);
    });
  });

  it('keeps several squads apart and requires a choice when ambiguous', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['Ben', 'worker']) await identity(sandbox, name);
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      await squad(sandbox, ['init', 'reviews']);
      writeFileSync(
        path.join(sandbox.globalDir, 'squad.toml'),
        'me = "Ben"\n[squad.reviews]\nlayout = "pr-queue"\n'
      );
      expect((await squad(sandbox, ['status'])).body.error.code).toBe('SQUAD_AMBIGUOUS');
      await squad(sandbox, ['add', 'worker', '--squad', 'product']);
      await squad(sandbox, ['add', 'worker', '--squad', 'reviews']);
      await squad(sandbox, ['remove', 'worker', '--squad', 'product']);
      expect(observe(sandbox).metadata).toEqual([
        { identity: 'worker', key: 'squad.reviews.state', value: 'preparing' },
      ]);
      const reviews = await squad(sandbox, ['status', '--squad', 'reviews']);
      expect(reviews.body.squad.layout).toBe('pr-queue');
      expect(reviews.body.sections[0].rows[0]).toMatchObject({
        name: 'worker',
        state: 'preparing',
      });
    });
  });
});
