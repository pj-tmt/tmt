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
      expect(text.stdout).toContain('\nNeeds me\n◆ auth-fix');

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
        'Copied with the configured clipboard program.\n'
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
      expect((await runCli(sandbox, ['sq', 'status'])).stdout).toContain(
        '    ✎ sent to Sol: split the job'
      );
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
      expect(chosen.body).toEqual({ requestId: asks[0], from: 'auth-fix', replied: true });
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
      expect(text).toContain('auth-fix · ');
      expect(text).toContain('  › status of the retry path?\n  retry passes\n  red\n');
      expect(text).not.toContain('\u001b');

      // Reading replies acknowledges nothing: the originator's attention is unchanged.
      expect(await attention()).toEqual(before);
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
      expect((await runCli(sandbox, ['sq', 'hotkeys', 'show'])).stdout).toContain(
        `The recorded tmt ${launcher} no longer exists`
      );

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
