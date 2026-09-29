import Database from 'better-sqlite3';
import {
  chmodSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  statSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
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

/** The version tmt-squad reports: its package version. */
const squadVersion = /^version = "([^"]+)"$/m.exec(
  readFileSync(
    fileURLToPath(
      new URL('../../../extensions/tmt-squad/rust/tmt-squad/Cargo.toml', import.meta.url)
    ),
    'utf8'
  )
)?.[1];

describe('squad extension', () => {
  // The Squad release proof (native-runtime-proof.mjs) expects this exact
  // line; PR CI never runs that proof, so this pins it.
  it('prints exactly squad <version> for --version and -V, directly and through tmt', async () => {
    await withSandbox(async (sandbox) => {
      const bin = installSquad(sandbox);
      expect(squadVersion).toBeTruthy();
      const direct = { ...sandbox, cli: { executable: path.join(bin, 'tmt-squad'), args: [] } };
      for (const [target, args] of [
        [direct, ['--version']],
        [direct, ['-V']],
        [sandbox, ['squad', '--version']],
      ] as const) {
        const result = await runCli(target, [...args]);
        expect(result, args.join(' ')).toMatchObject({
          status: 0,
          stdout: `squad ${squadVersion}\n`,
          stderr: '',
        });
      }
    });
  });

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
        // `help <command>` prints exactly what `<command> --help` prints.
        [
          ['squad', 'help', 'hotkeys', 'install'],
          ['sq', 'hotkeys', 'install', '--help'],
        ],
        [
          ['squad', 'help', 'bogus'],
          ['sq', 'help', 'bogus'],
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
      expect(help.stdout).toContain('Usage: tmt squad [OPTIONS] [COMMAND]');
      const routed = await runCli(sandbox, ['squad', 'help', 'hotkeys', 'install']);
      expect(routed.status).toBe(0);
      expect(routed.stdout).toContain('Usage: tmt squad hotkeys install [OPTIONS]');
      expect(routed.stdout).toContain('\nExamples:\n  # See the bindings and the line');
      const unknown = await runCli(sandbox, ['squad', 'help', 'bogus']);
      expect(unknown.status).toBe(2);
      expect(unknown.stderr).toContain("unrecognized subcommand 'bogus'");
      const status = await squad(sandbox, ['status']);
      expect(status).toMatchObject({ status: 1, body: { error: { code: 'SQUAD_NOT_FOUND' } } });
      // Completion v1: core invokes `tmt-<name> __complete -- <words>` directly.
      const completions = [];
      for (const name of ['tmt-squad', 'tmt-sq']) {
        const direct = { ...sandbox, cli: { executable: path.join(bin, name), args: [] } };
        completions.push(await runCli(direct, ['__complete', '--', 's']));
      }
      expect(completions[0].stdout).toBe('set\nskill\n');
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

  it('follows a renamed user through me_id, with hooks off and then on', async () => {
    await withSandbox(async (sandbox) => {
      const bin = installSquad(sandbox);
      const ada = await identity(sandbox, 'ada');
      const rin = await identity(sandbox, 'rin');
      const squadToml = path.join(sandbox.globalDir, 'squad.toml');
      const me = () => {
        const text = readFileSync(squadToml, 'utf8');
        return {
          me: /^me = "([^"]*)"$/m.exec(text)?.[1],
          id: /^me_id = "([^"]*)"$/m.exec(text)?.[1],
        };
      };
      expect((await squad(sandbox, ['init', 'product', '--me', 'ada'])).status).toBe(0);
      expect(me()).toEqual({ me: 'ada', id: ada });

      // Hooks off: the next command that needs `me` repairs it.
      expect((await runCli(sandbox, ['rename', 'ada', 'ada-2', '--json'])).status).toBe(0);
      expect(me()).toEqual({ me: 'ada', id: ada });
      const healed = await squad(sandbox, ['status']);
      expect(healed.status).toBe(0);
      expect(healed.stderr).toBe('');
      expect(me()).toEqual({ me: 'ada-2', id: ada });

      // The UUID decides: a reused old name never moves the user.
      expect((await runCli(sandbox, ['rename', 'ada-2', 'ada-3', '--json'])).status).toBe(0);
      await identity(sandbox, 'ada-2');
      const reused = await squad(sandbox, ['status']);
      expect(reused.status).toBe(0);
      expect(reused.stderr).toContain('still acting as ada-3');
      expect(me()).toEqual({ me: 'ada-3', id: ada });

      // A hand edit naming someone else is reported, not followed.
      writeFileSync(
        squadToml,
        readFileSync(squadToml, 'utf8').replace('me = "ada-3"', 'me = "rin"')
      );
      const edited = await squad(sandbox, ['status']);
      expect(edited.status).toBe(0);
      expect(edited.stderr).toContain(
        "warning: squad.toml named 'rin' as you, but me_id is ada-3; still acting as ada-3"
      );
      expect(edited.stderr).toContain('hint: tmt squad me rin');
      expect(me()).toEqual({ me: 'ada-3', id: ada });
      expect((await squad(sandbox, ['me', 'rin'])).status).toBe(0);
      expect(me()).toEqual({ me: 'rin', id: rin });

      // Hooks on: the rename observation follows the user at once.
      const capabilities = await runCli(sandbox, ['squad', '__tmt-hooks', '1', 'capabilities']);
      expect(capabilities.stdout).toBe('TMT-HOOKS/1\nlifecycle_observations_v1\n');
      const enabled = await runCli(sandbox, ['extension', 'hooks', 'enable', 'squad', '--json']);
      expect(enabled.status, enabled.stdout + enabled.stderr).toBe(0);
      expect(realpathSync(path.join(bin, 'tmt-squad'))).toBe(realpathSync(squadExecutable));
      expect((await runCli(sandbox, ['rename', 'rin', 'rin-2', '--json'])).status).toBe(0);
      expect(me()).toEqual({ me: 'rin-2', id: rin });

      // Someone else's rename leaves the file alone.
      const before = readFileSync(squadToml, 'utf8');
      await identity(sandbox, 'sol');
      expect((await runCli(sandbox, ['rename', 'sol', 'sol-2', '--json'])).status).toBe(0);
      expect(readFileSync(squadToml, 'utf8')).toBe(before);
    });
  });

  it('initializes without asking; --me records the user only after it resolves', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'Ben');
      const squadToml = path.join(sandbox.globalDir, 'squad.toml');

      // No --me and no terminal question: the room exists, squad.toml does not.
      const created = await squad(sandbox, ['init', 'product']);
      expect(created).toMatchObject({ status: 0, body: { created: true, me: null } });
      expect(observe(sandbox).rooms).toEqual([{ name: 'squad-product', retired: 0 }]);
      expect(existsSync(squadToml)).toBe(false);
      const text = await runCli(sandbox, ['sq', 'init', 'reviews']);
      expect(text).toMatchObject({ status: 0, stderr: '' });
      expect(text.stdout).toBe('✓ Created squad reviews (room squad-reviews)\n');

      // --me is checked before any effect.
      const unknown = await squad(sandbox, ['init', 'docs', '--me', 'Nobody']);
      expect(unknown.body.error.code).toBe('NAME_NOT_FOUND');
      expect((await squad(sandbox, ['init', 'Product', '--me', 'Ben'])).body.error.code).toBe(
        'SQUAD_NAME_INVALID'
      );
      expect(observe(sandbox).rooms.map((room) => room.name)).toEqual([
        'squad-product',
        'squad-reviews',
      ]);

      const userText = '# mine\n[squad.product]\nlayout = "crew" # keep\n';
      writeFileSync(squadToml, userText);
      const recorded = await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      expect(recorded).toMatchObject({ status: 0, body: { created: false, me: 'Ben' } });
      const written = readFileSync(squadToml, 'utf8');
      expect(written).toContain(userText);
      expect(written).toContain('me = "Ben"');
      const again = await squad(sandbox, ['init', 'product']);
      expect(again).toMatchObject({ status: 0, body: { created: false, me: 'Ben' } });
      expect(readFileSync(squadToml, 'utf8')).toBe(written);
    });
  });

  it('shows, records and clears who you are with sq me, never asking', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const ben = await identity(sandbox, 'Ben');
      const squadToml = path.join(sandbox.globalDir, 'squad.toml');
      await squad(sandbox, ['init', 'product']);

      // Nobody yet: outside a pane, nothing is recorded or derived.
      const nobody = await squad(sandbox, ['me']);
      expect(nobody.body).toMatchObject({ action: 'show', me: null, source: null });
      const nobodyText = await runCli(sandbox, ['sq', 'me']);
      expect(nobodyText.stdout).toBe(
        'No identity is recorded as you, and this pane has no saved identity.\nhint: tmt squad me <name>\n'
      );
      const status = await squad(sandbox, ['status']);
      expect(status.body.you).toBeNull();
      expect((await runCli(sandbox, ['sq', 'status'])).stdout).toContain(
        '◆ needs to know who you are: tmt squad me <name>'
      );

      // Recording needs a saved identity and changes nothing else.
      const set = await squad(sandbox, ['me', 'Ben']);
      expect(set.body).toMatchObject({
        action: 'set',
        me: { id: ben, name: 'Ben' },
        source: 'recorded',
        path: squadToml,
      });
      expect(readFileSync(squadToml, 'utf8')).toBe(`me = "Ben"\nme_id = "${ben}"\n`);
      const shown = await runCli(sandbox, ['sq', 'me']);
      expect(shown.stdout).toBe(`You are Ben (recorded in ${squadToml}).\n`);
      expect((await squad(sandbox, ['status'])).body.you).toEqual({
        id: ben,
        name: 'Ben',
        source: 'recorded',
      });
      expect((await runCli(sandbox, ['sq', 'status'])).stdout).not.toContain('◆ needs');
      const missing = await squad(sandbox, ['me', 'Nobody']);
      expect(missing.body.error.code).toBe('NAME_NOT_FOUND');

      const cleared = await squad(sandbox, ['me', '--clear']);
      expect(cleared.body).toMatchObject({ action: 'clear', changed: true, me: null });
      expect(readFileSync(squadToml, 'utf8')).toBe('');
      const repeat = await runCli(sandbox, ['sq', 'me', '--clear']);
      expect(repeat.stdout).toBe('No identity was recorded; nothing changed.\n');
      const both = await runCli(sandbox, ['sq', 'me', 'Ben', '--clear']);
      expect(both.status).toBe(2);
    });
  });

  it('sends as --identity, else the recorded user, else refuses in one line', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const ids: Record<string, string> = {};
      for (const name of ['Ben', 'Sol', 'auth-fix']) ids[name] = await identity(sandbox, name);
      await squad(sandbox, ['init', 'product']);
      await squad(sandbox, ['lead', 'Sol']);
      await squad(sandbox, ['add', 'auth-fix']);
      const api = async (operation: string, input: object) =>
        JSON.parse(
          (
            await runCli(sandbox, ['api'], {
              stdin: JSON.stringify({ version: 1, operation, input }),
            })
          ).stdout
        );
      const sender = async (requestId: string) =>
        (await api('requests.show', { requestId })).sender.identityId;
      const requests = async () =>
        (await api('requests.list', { recipientId: ids['auth-fix'], limit: 50 })).items.length;

      // No pane identity and no recorded user: nothing is sent.
      for (const args of [
        ['talk', 'auth-fix', 'hello'],
        ['annotate', 'auth-fix', 'split it'],
        ['reply', 'auth-fix', 'yes'],
        ['replies'],
      ]) {
        const refused = await squad(sandbox, args);
        expect(refused, args.join(' ')).toMatchObject({
          status: 1,
          body: { error: { code: 'SQUAD_SENDER_UNKNOWN' } },
        });
      }
      expect(await requests()).toBe(0);
      const text = await runCli(sandbox, ['sq', 'talk', 'auth-fix', 'hello']);
      expect(text.stderr).toBe(
        'error: Who is sending? This pane has no identity, and no user is recorded\n' +
          'hint: Name this pane with tmt this <name>, or record yourself with tmt squad me <name>\n'
      );

      // An explicit identity speaks for itself, even with a user recorded.
      await squad(sandbox, ['me', 'Ben']);
      const asLead = await squad(sandbox, [
        'talk',
        'auth-fix',
        'rebase first',
        '--identity',
        'Sol',
      ]);
      expect(asLead.body).toMatchObject({ to: 'auth-fix', as: 'Sol' });
      expect(await sender(asLead.body.requestId)).toBe(ids.Sol);
      const unknown = await squad(sandbox, ['talk', 'auth-fix', 'x', '--identity', 'Nobody']);
      expect(unknown.body.error.code).toBe('NAME_NOT_FOUND');

      // Without one, the recorded user sends.
      const asUser = await runCli(sandbox, ['sq', 'talk', 'auth-fix', 'status?']);
      expect(asUser.stdout).toMatch(/^✓ Sent to auth-fix as Ben \(req_[0-9a-f-]+\)\n$/);
      const annotated = await squad(sandbox, ['annotate', 'auth-fix', 'split it']);
      expect(annotated.body).toMatchObject({ to: 'Sol', as: 'Ben' });
      expect(await sender(annotated.body.requestId)).toBe(ids.Ben);
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
      const documentedFields = [...documented.matchAll(/`([a-z][A-Za-z]*)`/g)]
        .map((match) => match[1])
        .filter((field) => !['active', 'offline', 'unknown'].includes(field));
      expect(documentedFields.sort()).toEqual(Object.keys(rows[0]).sort());
      expect(Object.keys(status.body.squad).sort()).toEqual(['layout', 'lead', 'name', 'roomId']);
      // Without a terminal, the board is exactly status, in text and JSON.
      const statusText = await runCli(sandbox, ['squad', 'status']);
      expect(await runCli(sandbox, ['squad', 'board'])).toEqual(statusText);
      expect((await squad(sandbox, ['board'])).body).toEqual(status.body);
      const text = await runCli(sandbox, ['sq', 'status']);
      // One leading mark: ◆ when the member waits on you; the state has its column.
      expect(text.stdout).toMatch(
        /\n {2}◆ {2}auth-fix +blocked +waiting on you: approve the token rotation plan/
      );

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

  it('reports lead, add, set and remove as text without --json', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['Ben', 'Sol', 'Rin', 'coder', 'outsider']) await identity(sandbox, name);
      expect((await squad(sandbox, ['init', 'product', '--me', 'Ben'])).status).toBe(0);
      const text = (args: string[]) => runCli(sandbox, ['sq', ...args]);

      expect(await text(['lead', 'Sol'])).toMatchObject({
        status: 0,
        stdout: '✓ Sol leads squad product\n',
        stderr: '',
      });
      expect(await text(['lead', 'Rin'])).toMatchObject({
        status: 0,
        stdout: '✓ Rin leads squad product (replaces Sol)\n',
      });
      // A partial add keeps its successes on stdout and each failure on stderr.
      expect(await text(['add', 'coder', 'ghost'])).toMatchObject({
        status: 1,
        stdout: '✓ Added coder to squad product (state working)\n',
        stderr: "error: Could not add ghost: Identity 'ghost' was not found\n",
      });
      expect((await text(['add', 'coder'])).stdout).toBe('✓ Added coder to squad product\n');
      expect(await text(['set', 'coder', 'state=blocked', 'note=needs review'])).toMatchObject({
        status: 0,
        stdout: '✓ Set state, note on coder\n',
        stderr: '',
      });
      expect(await text(['set', 'outsider', 'state=working'])).toMatchObject({
        status: 1,
        stdout: '',
        stderr: "error: 'outsider' is not in squad product; add it first\n",
      });
      expect(await text(['remove', 'coder'])).toMatchObject({
        status: 0,
        stdout: '✓ Removed coder from squad product; cleared note, state\n',
        stderr: '',
      });
    });
  });

  it('lists members with ls; status and a bare tmt sq without a terminal are the same list', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['Ben', 'auth-fix']) await identity(sandbox, name);
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      await squad(sandbox, ['add', 'auth-fix']);
      await squad(sandbox, ['set', 'auth-fix', 'task=rotate session tokens']);
      const ls = await runCli(sandbox, ['sq', 'ls']);
      expect(ls.status).toBe(0);
      // The board's default columns: member, state, task, PR.
      expect(ls.stdout).toContain('auth-fix  working  rotate session tokens');
      for (const args of [['sq', 'status'], ['sq'], ['squad'], ['sq', 'board']]) {
        const same = await runCli(sandbox, args);
        expect({ status: same.status, stdout: same.stdout }, args.join(' ')).toEqual({
          status: 0,
          stdout: ls.stdout,
        });
      }
      const json = await squad(sandbox, ['ls']);
      expect(json.body.columns.map((column: { field: string }) => column.field)).toEqual([
        'member',
        'state',
        'task',
        'pr_link',
      ]);
      for (const args of [
        ['sq', 'status', '--json'],
        ['sq', '--json'],
      ]) {
        const same = await runCli(sandbox, args);
        expect(JSON.parse(same.stdout), args.join(' ')).toEqual(json.body);
      }
      const help = await runCli(sandbox, ['sq', '--help']);
      expect(help.stdout).toContain('Usage: tmt squad [OPTIONS] [COMMAND]');
      expect(help.stdout).toMatch(/\n {2}ls +List the members/);
      expect(help.stdout).not.toMatch(/\n {2}status /);
    });
  });

  it('keeps several squads apart: ls lists each, changes require a choice', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['Ben', 'worker']) await identity(sandbox, name);
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      await squad(sandbox, ['init', 'reviews']);
      writeFileSync(
        path.join(sandbox.globalDir, 'squad.toml'),
        'me = "Ben"\n[squad.reviews]\nlayout = "pr-queue"\n'
      );
      const both = await squad(sandbox, ['ls']);
      expect(both.status).toBe(0);
      expect(both.body.squads.map((one: { squad: { name: string } }) => one.squad.name)).toEqual([
        'product',
        'reviews',
      ]);
      const bothText = (await runCli(sandbox, ['sq', 'ls'])).stdout;
      expect(bothText).toMatch(/^squad product · .*\n[\s\S]*\n\nsquad reviews · /);
      expect((await squad(sandbox, ['add', 'worker'])).body.error.code).toBe('SQUAD_AMBIGUOUS');
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
  it('renders user-defined sections from squad.toml and rejects invalid filters', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['Ben', 'Sol', 'auth-fix', 'docs-sweep', 'perf-cache']) {
        await identity(sandbox, name);
      }
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      await squad(sandbox, ['lead', 'Sol']);
      await squad(sandbox, ['add', 'auth-fix', 'docs-sweep', 'perf-cache']);
      await squad(sandbox, ['set', 'auth-fix', 'state=blocked', 'pending=approve the plan']);
      await squad(sandbox, ['set', 'docs-sweep', 'state=review']);
      const squadToml = path.join(sandbox.globalDir, 'squad.toml');
      const base = readFileSync(squadToml, 'utf8');
      writeFileSync(
        squadToml,
        `${base}
[[squad.product.section]]
title = "Needs me"
filter = "pending or state = blocked"

[[squad.product.section]]
title = "Everyone"
sort = ["-name"]
`
      );
      const status = await squad(sandbox, ['status']);
      expect(status.status).toBe(0);
      const sections = status.body.sections.map(
        (section: { title: string; rows: { name: string }[] }) => [
          section.title,
          section.rows.map((row) => row.name),
        ]
      );
      expect(sections).toEqual([
        ['Needs me', ['auth-fix']],
        ['Everyone', ['perf-cache', 'docs-sweep', 'auth-fix']],
      ]);
      const text = await runCli(sandbox, ['sq', 'status']);
      expect(text.stdout).toMatch(/\nNEEDS ME 1\n {2}◆ {2}auth-fix /);

      writeFileSync(
        squadToml,
        `${base}\n[[squad.product.section]]\ntitle = "Bad"\nfilter = "state ="\n`
      );
      const refused = await squad(sandbox, ['status']);
      expect(refused).toMatchObject({
        status: 1,
        body: { error: { code: 'SQUAD_CONFIG_INVALID' } },
      });
      expect(refused.body.error.message).toContain('squad.product.section[0].filter');

      // Section bindings are validated whenever sections load.
      for (const [bind, place] of [
        ['o = "launch {name}"', 'squad.product.section[0].bind.o'],
        ['q = "refresh"', 'squad.product.section[0].bind.q'],
        ['o = "open {pr link}"', 'squad.product.section[0].bind.o'],
        ['o = "run ./script {name}"', 'squad.product.section[0].bind.o'],
        ['o = "run {program} x"', 'squad.product.section[0].bind.o'],
        ['hold = "jump"', 'squad.product.section[0].bind.hold'],
      ]) {
        writeFileSync(
          squadToml,
          `${base}\n[[squad.product.section]]\ntitle = "Mine"\n[squad.product.section.bind]\n${bind}\n`
        );
        const invalid = await squad(sandbox, ['status']);
        expect(invalid.body.error.code, bind).toBe('SQUAD_CONFIG_INVALID');
        expect(invalid.body.error.message, bind).toContain(place);
      }
      writeFileSync(
        squadToml,
        `${base}\n[[squad.product.section]]\ntitle = "Mine"\n[squad.product.section.bind]\n` +
          `double-click = "run code -- {cwd}"\nclick = "notes"\n`
      );
      expect((await squad(sandbox, ['status'])).status).toBe(0);
    });
  });
  it('orders status by the configured state sort before the layout default', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['Ben', 'a-work', 'b-review', 'c-parked']) await identity(sandbox, name);
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      await squad(sandbox, ['add', 'a-work', 'b-review', 'c-parked']);
      await squad(sandbox, ['set', 'b-review', 'state=review']);
      await squad(sandbox, ['set', 'c-parked', 'state=parked']);
      const order = async () =>
        (await squad(sandbox, ['status'])).body.sections[0].rows.map(
          (row: { name: string }) => row.name
        );
      expect(await order()).toEqual(['a-work', 'b-review', 'c-parked']);
      const squadToml = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(
        squadToml,
        `${readFileSync(squadToml, 'utf8')}\n[squad.product.states]\nreview = { sort = 0 }\nparked = { sort = 0, color = "dim" }\n`
      );
      expect(await order()).toEqual(['b-review', 'c-parked', 'a-work']);
      writeFileSync(squadToml, 'me = "Ben"\n[squad.product.states]\nreview = { sort = -1 }\n');
      expect((await squad(sandbox, ['status'])).body.error.code).toBe('SQUAD_CONFIG_INVALID');
    });
  });

  it('opens and copies a member through configured argv programs, never a shell', async () => {
    await withSandbox(async (sandbox) => {
      const bin = installSquad(sandbox);
      for (const name of ['Ben', 'Sol', 'auth-fix', 'Rin']) await identity(sandbox, name);
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      await squad(sandbox, ['lead', 'Sol']);
      await squad(sandbox, ['add', 'auth-fix']);
      const task = 'rotate; $(touch pwned) `id` "quoted" *.rs';
      await squad(sandbox, [
        'set',
        'auth-fix',
        `task=${task}`,
        'pr_link=https://example.com/pull/412?x=1&y=2',
        'issue_link=https://example.com/issues/9',
        'doc_link=file:///etc/passwd',
      ]);
      // Recorders: one argument per line, or stdin verbatim.
      const opened = path.join(sandbox.root, 'opened');
      const copied = path.join(sandbox.root, 'copied');
      const opener = path.join(bin, 'record-open');
      const clipboard = path.join(bin, 'record-copy');
      writeFileSync(
        opener,
        `#!/bin/sh\nprintf '%s\\n' "$@" > '${opened}.tmp'\nmv '${opened}.tmp' '${opened}'\n`
      );
      writeFileSync(clipboard, `#!/bin/sh\ncat > '${copied}'\n`);
      chmodSync(opener, 0o755);
      chmodSync(clipboard, 0o755);
      const squadToml = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(
        squadToml,
        `${readFileSync(squadToml, 'utf8')}\nopener = ["record-open", "--new-tab"]\nclipboard = ["${clipboard}"]\n`
      );
      const waitForOpened = () =>
        vi.waitFor(() => readFileSync(opened, 'utf8'), { timeout: 5000, interval: 25 });

      const open = await squad(sandbox, ['open', 'auth-fix']);
      expect(open).toMatchObject({
        status: 0,
        body: { member: 'auth-fix', opened: 'https://example.com/pull/412?x=1&y=2' },
      });
      expect(await waitForOpened()).toBe('--new-tab\nhttps://example.com/pull/412?x=1&y=2\n');
      await squad(sandbox, ['open', 'auth-fix', '--link', 'issue_link']);
      await vi.waitFor(() => expect(readFileSync(opened, 'utf8')).toContain('/issues/9'), {
        timeout: 5000,
        interval: 25,
      });

      for (const [args, code] of [
        [['open', 'auth-fix', '--link', 'doc_link'], 'SQUAD_ACTION_REFUSED'],
        [['open', 'Sol'], 'SQUAD_ACTION_REFUSED'],
        [['open', 'Rin'], 'SQUAD_NOT_MEMBER'],
        [['copy', 'auth-fix', '--format', '{pending}'], 'SQUAD_ACTION_REFUSED'],
        [['copy', 'auth-fix', '--format', '{bad field}'], 'SQUAD_ACTION_REFUSED'],
        // Outside tmux there is no client to show; core's refusal passes through.
        [['jump', 'auth-fix'], 'HOST_UNSUPPORTED'],
        [['back'], 'HOST_UNSUPPORTED'],
        [['jump', 'Rin'], 'SQUAD_NOT_MEMBER'],
      ] as const) {
        const refused = await squad(sandbox, [...args]);
        expect(refused.status, args.join(' ')).toBe(1);
        expect(refused.body.error.code, args.join(' ')).toBe(code);
      }
      expect(readFileSync(opened, 'utf8')).toContain('/issues/9');
      expect(existsSync(copied)).toBe(false);

      const copy = await squad(sandbox, ['copy', 'auth-fix']);
      expect(copy).toMatchObject({
        status: 0,
        body: { copied: `auth-fix: ${task} (working)`, to: 'program' },
      });
      expect(readFileSync(copied, 'utf8')).toBe(`auth-fix: ${task} (working)`);
      expect(
        (await squad(sandbox, ['copy', 'Sol', '--format', '- [{name}]({member})'])).body
      ).toMatchObject({ member: 'Sol' });
      expect(readFileSync(copied, 'utf8')).toBe('- [Sol](Sol)');
      expect(existsSync(path.join(sandbox.root, 'pwned'))).toBe(false);
      expect((await runCli(sandbox, ['sq', 'copy', 'auth-fix'])).stdout).toBe(
        '✓ Copied with the configured clipboard program\n'
      );
    });
  });

  it('talks, annotates and replies as the user through core requests only', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const ids: Record<string, string> = {};
      for (const name of ['Ben', 'Sol', 'auth-fix', 'docs'])
        ids[name] = await identity(sandbox, name);
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      await squad(sandbox, ['lead', 'Sol']);
      await squad(sandbox, ['add', 'auth-fix', 'docs']);
      const api = async (operation: string, input: object) => {
        const result = await runCli(sandbox, ['api'], {
          stdin: JSON.stringify({ version: 1, operation, input }),
        });
        return JSON.parse(result.stdout);
      };
      const room = (await runCli(sandbox, ['room', 'show', 'squad-product', '--json'])).stdout;
      const roomId = JSON.parse(room).room.id as string;
      const roomRequests = async () =>
        (await api('requests.list', { roomId, limit: 50 })).items as {
          requestId: string;
        }[];
      const prompt = async (requestId: string) => api('requests.show', { requestId });
      const notebook = async () => (await api('notes.read', { identityId: ids.Sol })).error?.code;
      expect(await notebook()).toBe('NOTEBOOK_NOT_FOUND');

      // talk: detached, untagged, in the squad room, from the user.
      const text = '-rf; $(touch pwned) "quoted"';
      const talk = await squad(sandbox, ['talk', 'auth-fix', text]);
      expect(talk.status).toBe(0);
      const talked = await prompt(talk.body.requestId);
      expect(talked).toMatchObject({
        roomId,
        recipientId: ids['auth-fix'],
        sender: { identityId: ids.Ben },
        kind: 'request',
        final: { status: 'not_submitted' },
        prompt: { message: text },
      });
      const before = (await roomRequests()).length;
      for (const args of [
        ['talk', 'auth-fix', '   '],
        ['annotate', 'auth-fix', ''],
      ]) {
        const empty = await squad(sandbox, args);
        expect(empty.body.error.code, args.join(' ')).toBe('SQUAD_ACTION_REFUSED');
      }
      expect((await roomRequests()).length, 'empty input sends nothing').toBe(before);

      // annotate: tagged, to the lead by default or to the member.
      const toLead = await squad(sandbox, ['annotate', 'auth-fix', 'split the job']);
      expect(toLead.body).toMatchObject({ to: 'Sol', row: 'auth-fix' });
      expect(await prompt(toLead.body.requestId)).toMatchObject({
        recipientId: ids.Sol,
        roomId,
        prompt: { message: '[product · auth-fix] split the job' },
      });
      const toMember = await squad(sandbox, ['annotate', 'docs', 'add examples', '--to', 'member']);
      expect(await prompt(toMember.body.requestId)).toMatchObject({
        recipientId: ids.docs,
        prompt: { message: '[product · docs] add examples' },
      });

      // The marker is derived from request state on every read.
      const marker = async () => {
        const rows = (await squad(sandbox, ['status'])).body.sections[0].rows;
        return Object.fromEntries(
          rows.map((row: { name: string; annotation: unknown }) => [row.name, row.annotation])
        );
      };
      expect(await marker()).toEqual({
        'auth-fix': { requestId: toLead.body.requestId, to: 'Sol', text: 'split the job' },
        docs: { requestId: toMember.body.requestId, to: 'docs', text: 'add examples' },
      });
      expect((await runCli(sandbox, ['sq', 'status'])).stdout).toContain('✎ to Sol: split the job');
      const incoming = async (who: string, requestId: string) =>
        JSON.parse(
          (
            await runCli(sandbox, [
              'x',
              'show',
              requestId,
              '--incoming',
              '--identity',
              who,
              '--json',
            ])
          ).stdout
        ).exchange;
      const leadView = await incoming('Sol', toLead.body.requestId);
      const answered = await runCli(sandbox, [
        'reply',
        toLead.body.requestId,
        '--receipt',
        leadView.reply.receipt,
        '--message',
        'done',
        '--json',
      ]);
      expect(answered.status).toBe(0);
      expect((await marker())['auth-fix'], 'gone after the final').toBeNull();
      expect((await marker()).docs).not.toBeNull();

      // reply: the user chooses among open requests; nothing is acknowledged.
      const asks: string[] = [];
      for (const question of ['approve the plan?', 'which database?']) {
        const asked = await runCli(sandbox, [
          'talk',
          'Ben',
          question,
          '--identity',
          'auth-fix',
          '--inbox',
          '--detach',
          '--json',
        ]);
        asks.push(JSON.parse(asked.stdout).requestId);
      }
      const waiting = (await squad(sandbox, ['status'])).body.sections[0].rows[0].waitingOnYou;
      expect(waiting.map((item: { requestId: string }) => item.requestId).sort()).toEqual(
        [...asks].sort()
      );
      const ambiguous = await squad(sandbox, ['reply', 'auth-fix', 'postgres']);
      expect(ambiguous.body.error.code).toBe('SQUAD_ACTION_REFUSED');
      for (const ask of asks) expect(ambiguous.body.error.message).toContain(ask);
      const stranger = await squad(sandbox, ['reply', 'docs', 'x', '--request', asks[0]]);
      expect(stranger.body.error.code).toBe('SQUAD_ACTION_REFUSED');

      const chosen = await squad(sandbox, ['reply', 'auth-fix', '-postgres', '--request', asks[0]]);
      expect(chosen.body).toEqual({
        requestId: asks[0],
        from: 'auth-fix',
        as: 'Ben',
        replied: true,
      });
      const result = JSON.parse((await runCli(sandbox, ['result', asks[0], '--json'])).stdout);
      expect(JSON.stringify(result)).toContain('-postgres');
      const only = await squad(sandbox, ['reply', 'auth-fix', 'yes']);
      expect(only.body.requestId, 'one open request needs no choice').toBe(asks[1]);
      for (const ask of asks) {
        expect((await incoming('Ben', ask)).acknowledged, 'reply never acknowledges').toBe(false);
      }
      expect((await squad(sandbox, ['reply', 'auth-fix', 'more'])).body.error.message).toBe(
        'auth-fix is not waiting on you.'
      );

      expect(await notebook(), 'no notebook was created or written').toBe('NOTEBOOK_NOT_FOUND');
      expect(existsSync(path.join(sandbox.root, 'pwned'))).toBe(false);
    });
  });

  it('lists finals to the requests the user sent in the squad, newest first, without acknowledging them', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['Ben', 'Sol', 'auth-fix']) await identity(sandbox, name);
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      await squad(sandbox, ['lead', 'Sol']);
      await squad(sandbox, ['add', 'auth-fix']);
      const empty = await squad(sandbox, ['replies']);
      expect(empty.body).toMatchObject({ squad: 'product', replies: [] });

      const talk = (await squad(sandbox, ['talk', 'auth-fix', 'status of the retry path?'])).body;
      const note = (await squad(sandbox, ['annotate', 'auth-fix', 'split the job'])).body;
      const answer = async (who: string, requestId: string, message: string) => {
        const shown = JSON.parse(
          (
            await runCli(sandbox, [
              'x',
              'show',
              requestId,
              '--incoming',
              '--identity',
              who,
              '--json',
            ])
          ).stdout
        );
        const replied = await runCli(sandbox, [
          'reply',
          requestId,
          '--receipt',
          shown.exchange.reply.receipt,
          '--message',
          message,
          '--json',
        ]);
        expect(replied.status).toBe(0);
      };
      await answer('Sol', note.requestId, 'agreed, splitting');
      await new Promise((resolve) => setTimeout(resolve, 20));
      await answer('auth-fix', talk.requestId, 'retry passes\n\u001b[31mred\u001b[0m');

      const attention = async () =>
        JSON.parse((await runCli(sandbox, ['x', 'list', '--identity', 'Ben', '--json'])).stdout)
          .items.map((item: { requestId: string; revision: number }) => [
            item.requestId,
            item.revision,
          ])
          .sort();
      const before = await attention();
      expect(before.length).toBe(2);

      const listed = (await squad(sandbox, ['replies'])).body;
      expect(listed.replies.map((reply: { to: string }) => reply.to)).toEqual(['auth-fix', 'Sol']);
      expect(listed.replies[0]).toMatchObject({
        requestId: talk.requestId,
        prompt: 'status of the retry path?',
        status: 'retained',
        response: 'retry passes\n\u001b[31mred\u001b[0m',
      });
      expect(listed.replies[1]).toMatchObject({
        requestId: note.requestId,
        prompt: '[product · auth-fix] split the job',
        response: 'agreed, splitting',
      });
      const text = (await runCli(sandbox, ['sq', 'replies'])).stdout;
      // One row per request: its ID and the reply's first line; the whole
      // body stays exact behind tmt result.
      expect(text).toMatch(
        new RegExp(`\\n {2}✓ {2}auth-fix .* ${talk.requestId} {2}retry passes\\n`)
      );
      expect(text).not.toContain('red');
      expect(text).not.toContain('\u001b');
      expect(text).toContain(`hint: tmt result ${talk.requestId}\n`);

      // Reading replies acknowledges nothing: the originator's attention is unchanged.
      expect(await attention()).toEqual(before);
    });
  });

  it('lists, shows, installs and removes playbooks through core only, with consent', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const source = readFileSync(
        fileURLToPath(
          new URL('../../../extensions/tmt-squad/playbooks/tmux-squad/SKILL.md', import.meta.url)
        ),
        'utf8'
      );
      // Claude keeps skills under .claude and Codex under .agents; core also
      // publishes optional skills into its other provider roots.
      mkdirSync(path.join(sandbox.home, '.claude'));
      mkdirSync(path.join(sandbox.home, '.codex'));
      const playbook = (args: string[]) => squad(sandbox, ['playbook', ...args]);
      const snapshotState = () =>
        existsSync(sandbox.database) ? readFileSync(sandbox.database) : null;

      const listed = await playbook(['list']);
      expect(listed.body.playbooks).toEqual([
        { name: 'tmux-squad', description: expect.stringContaining('Propose a tmux layout') },
      ]);
      const shown = await runCli(sandbox, ['sq', 'playbook', 'show', 'tmux-squad']);
      expect(shown.stdout, 'the exact embedded bytes').toBe(source);
      expect((await playbook(['show', 'tmux-squad'])).body.content).toBe(source);
      expect((await playbook(['show', 'nope'])).body.error.code).toBe('SQUAD_PLAYBOOK_UNKNOWN');

      const claudeSkill = path.join(sandbox.home, '.claude/skills/tmux-squad');
      const agentsSkill = path.join(sandbox.home, '.agents/skills/tmux-squad');
      const before = snapshotState();
      const printed = await playbook(['install', 'tmux-squad', '--print']);
      expect(printed.body).toMatchObject({ name: 'tmux-squad', owner: 'squad' });
      const refused = await playbook(['install', 'tmux-squad']);
      expect(refused.body.error.code, 'no terminal and no --yes').toBe('SQUAD_CONSENT_REQUIRED');
      expect(existsSync(claudeSkill) || existsSync(agentsSkill)).toBe(false);
      expect(snapshotState()).toEqual(before);

      // A user skill of the same name that tmt does not manage is never replaced.
      mkdirSync(claudeSkill, { recursive: true });
      writeFileSync(path.join(claudeSkill, 'SKILL.md'), 'my own playbook');
      const conflict = await playbook(['install', 'tmux-squad', '--yes']);
      expect(conflict.body.error.code).toBe('SKILL_CONFLICT');
      expect(conflict.body.error.message).toContain('--force');
      expect(readFileSync(path.join(claudeSkill, 'SKILL.md'), 'utf8')).toBe('my own playbook');
      expect(existsSync(agentsSkill), 'nothing is published after a conflict').toBe(false);

      const forced = await playbook(['install', 'tmux-squad', '--yes', '--force']);
      expect(forced.body).toMatchObject({ name: 'tmux-squad', owner: 'squad', changed: true });
      const backups = forced.body.published.filter(
        (item: { backup: string | null }) => item.backup
      );
      expect(backups).toHaveLength(1);
      expect(readFileSync(path.join(backups[0].backup, 'SKILL.md'), 'utf8')).toBe(
        'my own playbook'
      );
      for (const target of [claudeSkill, agentsSkill]) {
        expect(lstatSync(target).isSymbolicLink()).toBe(true);
        expect(readFileSync(path.join(target, 'SKILL.md'), 'utf8')).toBe(source);
      }
      const again = await playbook(['install', 'tmux-squad', '--yes']);
      expect(again.body.changed, 'a repeat is a no-op').toBe(false);
      expect(
        (await runCli(sandbox, ['sq', 'playbook', 'install', 'tmux-squad', '--yes'])).stdout
      ).toBe('Already installed; nothing changed.\n');

      // The lead skill shares the owner; removing the playbook must not touch it.
      const lead = await runCli(sandbox, ['api'], {
        stdin: JSON.stringify({
          version: 1,
          operation: 'skills.install',
          input: {
            owner: 'squad',
            consent: true,
            skills: [{ name: 'tmt-squad', files: [{ path: 'SKILL.md', content: 'lead skill' }] }],
          },
        }),
      });
      expect(JSON.parse(lead.stdout).owner).toBe('squad');
      const userSkill = path.join(sandbox.home, '.claude/skills/mine');
      mkdirSync(userSkill);
      writeFileSync(path.join(userSkill, 'SKILL.md'), 'user skill');

      expect((await playbook(['remove', 'tmux-squad'])).body.error.code).toBe(
        'SQUAD_CONSENT_REQUIRED'
      );
      expect(existsSync(claudeSkill)).toBe(true);
      const removed = await playbook(['remove', 'tmux-squad', '--yes']);
      expect(removed.body).toMatchObject({ name: 'tmux-squad', changed: true, kept: [] });
      const published: string[] = forced.body.published.map(
        (item: { target: string }) => item.target
      );
      expect(published).toEqual(expect.arrayContaining([claudeSkill, agentsSkill]));
      expect(removed.body.removed.sort()).toEqual([...published].sort());
      expect(published.some((target) => existsSync(target))).toBe(false);
      for (const target of ['.claude/skills/tmt-squad', '.agents/skills/tmt-squad']) {
        expect(readFileSync(path.join(sandbox.home, target, 'SKILL.md'), 'utf8')).toBe(
          'lead skill'
        );
      }
      expect(readFileSync(path.join(userSkill, 'SKILL.md'), 'utf8')).toBe('user skill');
      expect((await playbook(['remove', 'tmux-squad', '--yes'])).body.changed).toBe(false);
      // Playbooks never touch identity storage or tmux.
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });

  it('installs tmux hotkeys only with consent, through the stable launcher, and removes only its line', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'Ben');
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      // A stable launcher on PATH that resolves to the tmt under test.
      const launcherDir = path.join(sandbox.root, 'launcher');
      mkdirSync(launcherDir);
      const launcher = path.join(launcherDir, 'tmt');
      symlinkSync(sandbox.cli.executable, launcher);
      sandbox.env.PATH = `${launcherDir}${path.delimiter}${sandbox.env.PATH ?? ''}`;
      const conf = path.join(sandbox.home, '.tmux.conf');
      const squadFile = path.join(sandbox.globalDir, 'squad.tmux.conf');
      const hotkeys = (args: string[]) => squad(sandbox, ['hotkeys', ...args]);

      const printed = await hotkeys(['install', '--print']);
      expect(printed.body).toMatchObject({ target: conf, creates: true, collisions: [] });
      expect(printed.body.bindings).toContain(
        `bind-key -N "tmt squad popup" S display-popup -E -w 90% -h 85% "exec '${launcher}' squad board --popup"`
      );
      expect(printed.body.bindings).not.toContain(sandbox.cli.executable);
      expect(existsSync(conf) || existsSync(squadFile), '--print changes nothing').toBe(false);
      const refused = await hotkeys(['install']);
      expect(refused.body.error.code, 'no terminal and no --yes').toBe('SQUAD_CONSENT_REQUIRED');
      expect(existsSync(conf) || existsSync(squadFile)).toBe(false);

      // An existing binding for a chosen key refuses the install.
      const original = '# mine\r\nset -g mouse on\nbind S choose-tree -s';
      writeFileSync(conf, original);
      const taken = await hotkeys(['install', '--yes']);
      expect(taken.body.error.code).toBe('SQUAD_HOTKEY_TAKEN');
      expect(taken.body.error.message).toContain('bind S choose-tree -s');
      expect(readFileSync(conf, 'utf8')).toBe(original);

      const squadToml = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(
        squadToml,
        `${readFileSync(squadToml, 'utf8')}\n[tmux]\npopup = "C-s"\nback = "b"\n`
      );
      const installed = await hotkeys(['install', '--yes']);
      expect(installed.body).toMatchObject({ installed: true, changed: true, creates: false });
      const line = `source-file -q '${squadFile}' # tmt squad hotkeys`;
      expect(readFileSync(conf, 'utf8')).toBe(`${original}\n${line}\n`);
      expect(readFileSync(installed.body.backup, 'utf8'), 'byte-exact backup').toBe(original);
      const bindings = readFileSync(squadFile, 'utf8');
      expect(bindings).toContain(`bind-key -N "tmt squad popup" C-s display-popup`);
      expect(bindings).toContain(
        `bind-key -N "tmt squad back" b run-shell "'${launcher}' squad back"`
      );
      const again = await hotkeys(['install', '--yes']);
      expect(again.body).toMatchObject({ installed: true, changed: false });
      expect(again.body.backup, 'a no-op writes no backup').toBeUndefined();
      expect(readFileSync(conf, 'utf8')).toBe(`${original}\n${line}\n`);

      const shown = await hotkeys(['show']);
      expect(shown.body).toMatchObject({
        installed: true,
        current: true,
        executable: launcher,
        executableExists: true,
        keys: { popup: 'C-s', pane: 'B', back: 'b' },
      });
      unlinkSync(launcher);
      expect((await hotkeys(['show'])).body.executableExists).toBe(false);
      const report = (await runCli(sandbox, ['sq', 'hotkeys', 'show'])).stdout;
      expect(report).toContain(`${launcher} (no longer exists)`);
      expect(report).toContain('hint: tmt squad hotkeys install\n');

      const removed = await hotkeys(['remove', '--yes']);
      expect(removed.body).toMatchObject({ removed: [conf], changed: true, unbound: [] });
      expect(readFileSync(conf, 'utf8'), 'only the owned line is gone').toBe(`${original}\n`);
      expect((await hotkeys(['remove', '--yes'])).body.changed).toBe(false);
    });
  });

  it('keeps a dotfile-managed tmux.conf a link and refuses a dangling one', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'Ben');
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      const dotfiles = path.join(sandbox.root, 'dotfiles');
      mkdirSync(dotfiles);
      const real = path.join(dotfiles, 'tmux.conf');
      writeFileSync(real, 'set -g mouse on\n');
      const conf = path.join(sandbox.home, '.tmux.conf');
      symlinkSync('../dotfiles/tmux.conf', conf);
      const hotkeys = (args: string[]) => squad(sandbox, ['hotkeys', ...args]);

      const printed = await hotkeys(['install', '--print']);
      expect(printed.body).toMatchObject({ target: conf, creates: false });
      expect(realpathSync(printed.body.resolved)).toBe(realpathSync(real));
      const installed = await hotkeys(['install', '--yes']);
      expect(installed.body.changed).toBe(true);
      expect(lstatSync(conf).isSymbolicLink(), 'the link stays a link').toBe(true);
      expect(readFileSync(real, 'utf8')).toMatch(
        /^set -g mouse on\nsource-file -q .* # tmt squad hotkeys\n$/
      );
      expect(realpathSync(path.dirname(installed.body.backup))).toBe(realpathSync(dotfiles));
      const removed = await hotkeys(['remove', '--yes']);
      expect(removed.body.changed).toBe(true);
      expect(lstatSync(conf).isSymbolicLink()).toBe(true);
      expect(readFileSync(real, 'utf8')).toBe('set -g mouse on\n');

      // A dangling link: install refuses and creates nothing; --print still helps.
      unlinkSync(conf);
      symlinkSync(path.join(sandbox.root, 'missing', 'tmux.conf'), conf);
      const dangling = await hotkeys(['install', '--yes']);
      expect(dangling.body.error.code).toBe('SQUAD_ACTION_REFUSED');
      expect(dangling.body.error.message).toContain('--print');
      expect(existsSync(path.join(sandbox.root, 'missing'))).toBe(false);
      expect(lstatSync(conf).isSymbolicLink()).toBe(true);
      const help = await hotkeys(['install', '--print']);
      expect(help.status).toBe(0);
      expect(help.body.resolved).toBeNull();
    });
  });
});
