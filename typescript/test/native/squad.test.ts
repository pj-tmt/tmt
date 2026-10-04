import {
  closeSync,
  constants,
  existsSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readdirSync,
  realpathSync,
  statSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import Database from 'better-sqlite3';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vite-plus/test';
import { workspaceVersion } from '../support/workspace-version.js';
import { parseWholeStdout, runCli, withSandbox, type Sandbox } from '../support/cli-process.js';

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
  sandbox.env.XDG_CACHE_HOME = path.join(sandbox.root, 'cache');
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

// Independent observation of the authoritative roster and board metadata.
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
        .all() as { identity: string; key: string; value: string }[],
    };
  } finally {
    db.close();
  }
}

/** Cache setup shared only by the context invocation and deadline scenarios. */
async function reminderFixture(sandbox: Sandbox) {
  installSquad(sandbox);
  const lead = await identity(sandbox, 'Sol');
  expect((await squad(sandbox, ['init', 'product', '--me', 'Sol'])).status).toBe(0);
  expect((await squad(sandbox, ['lead', 'Sol'])).status).toBe(0);
  const config = path.join(sandbox.globalDir, 'squad.toml');
  writeFileSync(
    config,
    readFileSync(config, 'utf8') + '\n[squad.product.reminders]\nenabled=true\nstale_after="1m"\n'
  );
  const notebook = JSON.parse(
    (await runCli(sandbox, ['notes', 'path', '--identity', 'Sol', '--json'])).stdout
  ).path;
  writeFileSync(notebook, 'Current plan');
  const cacheFile = () => {
    const directory = path.join(sandbox.root, 'cache', 'tmt-squad', 'staleness');
    return path.join(
      directory,
      readdirSync(directory).find((name) => name.endsWith('.json'))!
    );
  };
  const age = () => {
    const cache = JSON.parse(readFileSync(cacheFile(), 'utf8'));
    cache.notes.sinceMs = Date.now() - 125_000;
    cache.observedAtMs = cache.notes.sinceMs;
    writeFileSync(cacheFile(), JSON.stringify(cache));
  };
  const context = () =>
    runCli(
      { ...sandbox, cli: { executable: squadExecutable, args: [] } },
      ['__tmt-hooks', '1', 'context'],
      { stdin: JSON.stringify({ version: 1, identityId: lead }) }
    );
  return { lead, cacheFile, age, context };
}

/** Publish once, then assess the executable before a short production deadline. */
async function readyContextFixture(sandbox: Sandbox, file: string, payload: string) {
  writeExecutable(
    file,
    `#!/bin/sh\nif [ "$1" = __tmt_fixture_ready ]; then exit 0; fi\n${payload}\n`,
    0o755
  );
  const ready = await runCli(
    { ...sandbox, cli: { executable: file, args: [] } },
    ['__tmt_fixture_ready'],
    { deadlineMs: 30_000 }
  );
  expect(ready.status, ready.stderr).toBe(0);
  expect(ready.signal).toBeNull();
  expect(ready.stdout).toBe('');
}

/** The version tmt-squad reports: its package version. */
const squadVersion = workspaceVersion('tmt-squad');

describe('squad extension', () => {
  it('config show reports effective sources without writing or executing configured commands', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'settings-probe');
      const marker = path.join(sandbox.root, 'must-not-exist');
      const config = path.join(sandbox.globalDir, 'squad.toml');
      const original = `# keep my comment
opaque = "kept"
[board]
refresh = "1m"
[squad.product.board]
refresh = "off"
[squad.product.fields.probe]
run = ["touch", "${marker}"]
[bind]
o = "run touch ${marker}"
`;
      writeFileSync(config, original);
      const before = observe(sandbox);
      const shown = await squad(sandbox, ['config', 'show', '--squad', 'product']);
      expect(shown.status).toBe(0);
      expect(shown.stderr).toBe('');
      expect(shown.body.path).toBe(config);
      expect(shown.body.entries).toContainEqual({
        key: 'board.refresh',
        value: 'off',
        source: 'squad.product.board.refresh',
        editable: true,
      });
      expect(
        shown.body.entries.find((entry: { key: string }) => entry.key === 'fields.probe').value.run
      ).toEqual(['touch', marker]);
      const text = await runCli(sandbox, ['sq', 'config', 'show', '--squad', 'product']);
      expect(text.status).toBe(0);
      expect(text.stdout).toContain('squad.product.board.refresh');
      expect(text.stdout).toContain('read-only');
      expect((await squad(sandbox, ['config', 'show', '--tab', 'all'])).status).toBe(0);
      expect((await squad(sandbox, ['config', 'show', '--tab', 'missing'])).body.error.code).toBe(
        'SQUAD_TAB_NOT_FOUND'
      );
      expect(
        (await squad(sandbox, ['config', 'show', '--squad', 'product', '--tab', 'all'])).status
      ).not.toBe(0);
      expect(readFileSync(config, 'utf8')).toBe(original);
      expect(existsSync(marker)).toBe(false);
      expect(observe(sandbox)).toEqual(before);
    });
  });

  it('config set validates, preserves unrelated TOML and hides tracks without removing JSON values', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'settings-owner');
      await identity(sandbox, 'settings-worker');
      await squad(sandbox, ['init', 'product', '--me', 'settings-owner']);
      await squad(sandbox, ['add', 'settings-worker', '--squad', 'product']);
      await squad(sandbox, [
        'set',
        'settings-worker',
        'state=working',
        'task=Keep task',
        'pr_link=https://example.com/keep-hidden',
      ]);
      const marker = path.join(sandbox.root, 'must-not-run');
      const config = path.join(sandbox.globalDir, 'squad.toml');
      const original = `# keep settings comments
opaque = "retained"
[squad.product]
layout = "crew"
[squad.product.board]
refresh = "5s" # keep timing comment
[squad.product.rows]
columns = [{name="member"}, {name="state"}, {name="task"}, {name="pr_link"}]
lines = [["member", "state", "task", "pr_link"], [{field="task", span=4}]]
[squad.product.fields.probe]
run = ["touch", "${marker}"]
[bind]
o = "run touch ${marker}"
`;
      writeFileSync(config, original);
      const before = observe(sandbox);
      const edited = await squad(sandbox, [
        'config',
        'set',
        'board.refresh',
        '10s',
        '--squad',
        'product',
      ]);
      expect(edited.status, edited.stderr).toBe(0);
      expect(edited.body.changed).toBe(true);
      expect(edited.body.entries).toContainEqual({
        key: 'board.refresh',
        value: '10s',
        source: 'squad.product.board.refresh',
        editable: true,
      });
      const saved = readFileSync(config, 'utf8');
      expect(saved).toContain('# keep timing comment');
      expect(saved).toContain('opaque = "retained"');
      for (const [key, value] of [
        ['board.refresh', '0s'],
        ['board.hidden_columns', '["unknown"]'],
        ['board.hidden_columns', '["member","state","task","pr_link"]'],
        ['fields.probe', 'run touch /never'],
        ['bind.o', 'refresh'],
      ]) {
        expect(
          (await squad(sandbox, ['config', 'set', key, value, '--squad', 'product'])).status
        ).not.toBe(0);
        expect(readFileSync(config, 'utf8')).toBe(saved);
      }
      expect(
        (await squad(sandbox, ['config', 'set', 'board.refresh', '10s', '--squad', 'product'])).body
          .changed
      ).toBe(false);
      expect(existsSync(marker)).toBe(false);
      expect(observe(sandbox)).toEqual(before);
      // Ordinary roster reads use this provider-free fixture; edits above never ran its command.
      writeFileSync(
        config,
        saved.replace(`[squad.product.fields.probe]\nrun = ["touch", "${marker}"]\n`, '')
      );
      const opening = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(opening.status).toBe(0);
      expect(
        (
          await squad(sandbox, [
            'config',
            'set',
            'board.hidden_columns',
            '["pr_link"]',
            '--squad',
            'product',
          ])
        ).status
      ).toBe(0);
      const hidden = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(hidden.body.hidden_columns).toEqual(['pr_link']);
      expect(hidden.body.columns).toEqual(opening.body.columns);
      expect(hidden.body.lines).toEqual(opening.body.lines);
      expect(
        hidden.body.sections[0].rows.find((row: { name: string }) => row.name === 'settings-worker')
          .fields.pr_link
      ).toBe('https://example.com/keep-hidden');
      const text = await runCli(sandbox, ['sq', 'ls', '--squad', 'product']);
      expect(text.stdout).toContain('Keep task');
      expect(text.stdout).not.toContain('example.com/keep-hidden');
      expect(
        (
          await squad(sandbox, [
            'config',
            'set',
            'board.hidden_columns',
            '[]',
            '--squad',
            'product',
          ])
        ).status
      ).toBe(0);
      expect((await runCli(sandbox, ['sq', 'ls', '--squad', 'product'])).stdout).toContain(
        'example.com/keep-hidden'
      );
      const pinned = await squad(sandbox, [
        'config',
        'set',
        'board.direction',
        'top-bottom',
        '--squad',
        'product',
      ]);
      expect(pinned.status).toBe(0);
      expect(pinned.body.notices.join(' ')).toContain('Saved layout crew and its split');
      expect(observe(sandbox)).toEqual(before);
    });
  });

  it('lists built-in tabs with their board rows, including hidden and empty squads', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const tab of ['leads', 'all']) {
        const empty = await squad(sandbox, ['ls', '--tab', tab]);
        expect(empty.status).toBe(0);
        expect(empty.body.sections).toEqual([{ title: null, rows: [] }]);
      }
      for (const name of ['Ben', 'Sol', 'worker']) await identity(sandbox, name);
      expect((await squad(sandbox, ['init', 'product', '--me', 'Ben'])).status).toBe(0);
      expect((await squad(sandbox, ['lead', 'Sol', '--squad', 'product'])).status).toBe(0);
      expect((await squad(sandbox, ['add', 'worker', '--squad', 'product'])).status).toBe(0);
      expect((await squad(sandbox, ['init', 'quiet'])).status).toBe(0);
      writeFileSync(
        path.join(sandbox.globalDir, 'squad.toml'),
        'me = "Ben"\n[tabs]\norder = ["all", "product", "leads"]\nhide = ["quiet"]\n'
      );
      const leads = await squad(sandbox, ['ls', '--tab', 'leads']);
      expect(leads.status).toBe(0);
      expect(leads.body.sections[0].rows).toMatchObject([
        { name: 'Sol', squad: 'product', fields: { squad: 'product' } },
      ]);
      expect(leads.body.columns.map((column: { field: string }) => column.field)).toEqual([
        'squad',
        'member',
        'state',
        'task',
      ]);
      const all = await squad(sandbox, ['ls', '--tab', 'all']);
      expect(all.status).toBe(0);
      expect(all.body.sections[0].rows).toMatchObject([
        { name: 'product', fields: { lead: 'Sol', members: '1' } },
        { name: 'quiet', fields: { lead: null, members: '0' } },
      ]);
      const text = await runCli(sandbox, ['sq', 'ls', '--tab', 'all']);
      expect(text.status).toBe(0);
      expect(text.stdout).toContain('product');
      expect(text.stdout).toContain('Sol');
      expect(text.stdout).toContain('quiet');
      writeFileSync(
        path.join(sandbox.globalDir, 'squad.toml'),
        '[tabs]\nhide = ["product", "tab:members"]\n[tabs.members]\nfilter = "squad = product"\nsort = ["-name"]\n[[tabs.members.section]]\ntitle = "Leads"\nfilter = "name = Sol"\n'
      );
      const members = await squad(sandbox, ['ls', '--tab', 'members']);
      expect(members.status).toBe(0);
      expect(members.body.tab).toBe('members');
      expect(members.body.sections).toMatchObject([
        { title: 'Leads', rows: [{ name: 'Sol', squad: 'product' }] },
        { title: null, rows: [{ name: 'worker', squad: 'product' }] },
      ]);
      expect(members.body.columns).toEqual(
        leads.body.columns.map((column: { field: string; title: string }) =>
          column.field === 'member' ? { ...column, title: 'MEMBER' } : column
        )
      );
      expect(leads.body.columns[1].title).toBe('LEAD');
      const memberText = await runCli(sandbox, ['sq', 'ls', '--tab', 'members']);
      expect(memberText.status).toBe(0);
      expect(memberText.stdout).toContain('LEADS');
      expect(memberText.stdout).toContain('worker');
      expect((await squad(sandbox, ['ls', '--tab', 'tab:members'])).body).toEqual(members.body);
      const missing = await squad(sandbox, ['ls', '--tab', 'missing']);
      expect(missing.status).not.toBe(0);
      expect(missing.body.error.code).toBe('SQUAD_TAB_NOT_FOUND');
      for (const option of [['--squad', 'product'], ['--refresh-fields']]) {
        expect((await squad(sandbox, ['ls', '--tab', 'all', ...option])).status).not.toBe(0);
      }
    });
  });

  it('reports the resolved default layout in ls JSON for team and legacy simple boards', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'Ben');
      expect((await squad(sandbox, ['init', 'product', '--me', 'Ben'])).status).toBe(0);
      const toml = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(toml, 'me = "Ben"\n');
      expect((await squad(sandbox, ['ls', '--squad', 'product'])).body.squad.layout).toBe('team');
      for (const setting of [
        'direction = "top-bottom"',
        'panes = ["rows", "notes"]',
        'sizes = [60, 40]',
      ]) {
        writeFileSync(toml, `me = "Ben"\n[squad.product.board]\n${setting}\n`);
        const listed = await squad(sandbox, ['ls', '--squad', 'product']);
        expect(listed.status).toBe(0);
        expect(listed.body.squad.layout).toBe('crew');
        expect(listed.body.columns.map((column: { field: string }) => column.field)).toEqual([
          'member',
          'state',
          'task',
          'pr_link',
        ]);
      }
    });
  });

  it('defaults to team rows and refreshes only linked PRs through the existing provider', async () => {
    await withSandbox(async (sandbox) => {
      const bin = installSquad(sandbox);
      for (const name of ['Ben', 'Sol', 'linked', 'unlinked']) await identity(sandbox, name);
      expect((await squad(sandbox, ['init', 'product', '--me', 'Ben'])).status).toBe(0);
      expect((await squad(sandbox, ['lead', 'Sol'])).status).toBe(0);
      const toml = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(toml, 'me = "Ben"\n');
      expect((await squad(sandbox, ['add', 'linked', 'unlinked'])).status).toBe(0);
      expect(
        (
          await squad(sandbox, [
            'set',
            'linked',
            'task=review PR',
            'pr_link=https://example.com/pull/412',
          ])
        ).status
      ).toBe(0);
      expect(
        (await squad(sandbox, ['set', 'unlinked', 'task=write notes', 'pending=approve the plan']))
          .status
      ).toBe(0);
      const calls = path.join(sandbox.root, 'gh-calls');
      const gh = path.join(bin, 'gh');
      writeExecutable(
        gh,
        `#!/bin/sh\nprintf '%s\\n' "$*" >> '${calls}'\nprintf '%s\\n' '{"number":412,"state":"OPEN","isDraft":false,"reviewDecision":"APPROVED"}'\n`,
        0o755
      );
      const metadata = observe(sandbox);
      const listed = await squad(sandbox, ['ls', '--squad', 'product', '--refresh-fields']);
      expect(listed.status).toBe(0);
      expect(listed.body.squad.layout).toBe('team');
      expect(listed.body.columns.map((column: { field: string }) => column.field)).toEqual([
        'member',
        'state',
        'task',
        'pr',
        'model',
      ]);
      expect(listed.body.columns[4].from).toBe('session.model');
      expect(listed.body.lines[1]).toEqual([
        { field: null, span: 1 },
        { field: null, span: 1 },
        { field: 'pending', span: 3, token: 'waiting' },
      ]);
      const rows = listed.body.sections[0].rows;
      expect(rows.map((row: { name: string }) => row.name)).toEqual(['unlinked', 'linked']);
      expect(rows[0]).toMatchObject({
        state: 'working',
        pending: 'approve the plan',
        staleness: { state: 'fresh' },
      });
      expect(rows[1].fields.pr).toContain('#412 open');
      expect(readFileSync(calls, 'utf8').trim().split('\n')).toHaveLength(1);
      const refresh = await squad(sandbox, ['ls', '--squad', 'product', '--refresh-fields']);
      expect(refresh.status).toBe(0);
      expect(readFileSync(calls, 'utf8').trim().split('\n')).toHaveLength(1);
      expect(observe(sandbox)).toEqual(metadata);
      writeFileSync(toml, 'me = "Ben"\n[squad.product]\nlayout = "crew"\n');
      const crew = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(crew.body.columns.map((column: { field: string }) => column.field)).toEqual([
        'member',
        'state',
        'task',
        'pr_link',
      ]);
      expect(crew.body.sections[0].rows[0].staleness.state).toBe('disabled');
    });
  });

  it('dispatches rm and remove identically while retaining the identity and its other metadata', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const id = await identity(sandbox, 'worker');
      expect((await squad(sandbox, ['init', 'product'])).status).toBe(0);
      expect(
        (
          await runCli(sandbox, [
            'identity',
            'meta',
            'set',
            'team',
            'infra',
            '--identity',
            'worker',
          ])
        ).status
      ).toBe(0);
      for (const command of ['rm', 'remove']) {
        expect((await squad(sandbox, ['add', 'worker'])).status).toBe(0);
        expect((await squad(sandbox, ['set', 'worker', 'task=Review'])).status).toBe(0);
        expect((await squad(sandbox, [command, 'worker'])).status).toBe(0);
        expect(observe(sandbox).members).toEqual([]);
        expect(observe(sandbox).metadata).toEqual([
          { identity: 'worker', key: 'team', value: 'infra' },
        ]);
        const shown = await runCli(sandbox, ['identity', 'show', 'worker', '--json']);
        expect(JSON.parse(shown.stdout).identity.id).toBe(id);
      }
    });
  });

  it('delivers a claimed reminder only once and revalidates notes and leadership', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const lead = await identity(sandbox, 'Sol');
      const member = await identity(sandbox, 'Rin');
      expect((await squad(sandbox, ['init', 'product', '--me', 'Sol'])).status).toBe(0);
      expect((await squad(sandbox, ['lead', 'Sol'])).status).toBe(0);
      expect((await squad(sandbox, ['add', 'Rin'])).status).toBe(0);
      const config = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(
        config,
        readFileSync(config, 'utf8') +
          '\n[squad.product.reminders]\nenabled=true\nstale_after="1m"\n'
      );
      const notebook = JSON.parse(
        (await runCli(sandbox, ['notes', 'path', '--identity', 'Sol', '--json'])).stdout
      ).path;
      writeFileSync(notebook, 'Current plan');
      const context = async (identityId = lead) => {
        const result = await runCli(
          { ...sandbox, cli: { executable: squadExecutable, args: [] } },
          ['__tmt-hooks', '1', 'context'],
          {
            stdin: JSON.stringify({ version: 1, identityId }),
          }
        );
        expect(result.status).toBe(0);
        expect(result.stderr).toBe('');
        return JSON.parse(result.stdout).summary as string | null;
      };
      sandbox.env.TMT_EXECUTABLE = sandbox.cli.executable;
      expect(await context()).toBeNull(); // No observation: no core reads or notebook creation.
      expect((await squad(sandbox, ['ls'])).status).toBe(0);
      expect(await context()).toBeNull();
      const directory = path.join(sandbox.root, 'cache', 'tmt-squad', 'staleness');
      const file = path.join(
        directory,
        readdirSync(directory).find((name) => name.endsWith('.json'))!
      );
      const age = () => {
        const cache = JSON.parse(readFileSync(file, 'utf8'));
        cache.notes.sinceMs = Date.now() - 125_000;
        cache.observedAtMs = cache.notes.sinceMs;
        writeFileSync(file, JSON.stringify(cache));
      };
      age();
      const before = readFileSync(file, 'utf8');
      expect(await context(member)).toBeNull();
      expect(readFileSync(file, 'utf8')).toBe(before);
      const summary = await context();
      expect(summary).toContain('Squad product: stale lead notes');
      expect([...summary!].length).toBeLessThanOrEqual(240);
      expect(JSON.parse(readFileSync(file, 'utf8')).notes.claimed).toBe(true);
      expect(await context()).toBeNull();
      writeFileSync(notebook, 'Updated plan');
      expect((await squad(sandbox, ['ls'])).status).toBe(0);
      age();
      await squad(sandbox, ['lead', 'Rin']);
      expect(await context(lead)).toBeNull();
      expect(JSON.parse(readFileSync(file, 'utf8')).notes.claimed).toBe(false);
    });
  });

  it('keeps cold and fresh context silent and invokes core for stale context', async () => {
    await withSandbox(async (sandbox) => {
      const { context, age, cacheFile } = await reminderFixture(sandbox);
      const fake = path.join(sandbox.root, 'sentinel-core');
      await readyContextFixture(
        sandbox,
        fake,
        [
          'root=${0%/*}',
          'printf called > "$root/core-called"',
          `printf '%s\\n' '{"error":{"code":"FIXTURE","message":"core invoked"}}'`,
          'exit 1',
          '',
        ].join('\n')
      );
      sandbox.env.TMT_EXECUTABLE = fake;
      expect(JSON.parse((await context()).stdout)).toEqual({ summary: null });
      expect(existsSync(path.join(sandbox.root, 'core-called'))).toBe(false);
      expect((await squad(sandbox, ['ls'])).status).toBe(0);
      expect(JSON.parse((await context()).stdout)).toEqual({ summary: null });
      expect(existsSync(path.join(sandbox.root, 'core-called'))).toBe(false);
      age();
      const before = readFileSync(cacheFile(), 'utf8');
      const invoked = await context();
      expect(invoked.status, invoked.stderr).toBe(0);
      expect(invoked.signal).toBeNull();
      expect(JSON.parse(invoked.stdout)).toEqual({ summary: null });
      expect(readFileSync(path.join(sandbox.root, 'core-called'), 'utf8')).toBe('called');
      expect(readFileSync(cacheFile(), 'utf8')).toBe(before);
    });
  }, 90_000);

  it('kills context hooks and seeded descendants within local and outer deadlines', async () => {
    await withSandbox(async (sandbox) => {
      const { lead, age, cacheFile } = await reminderFixture(sandbox);
      expect((await squad(sandbox, ['ls'])).status).toBe(0);
      age();
      const before = readFileSync(cacheFile(), 'utf8');
      const gate = path.join(sandbox.root, 'child-gate');
      const ready = path.join(sandbox.root, 'child-ready');
      const fifos = await runCli({ ...sandbox, cli: { executable: '/usr/bin/mkfifo', args: [] } }, [
        gate,
        ready,
      ]);
      expect(fifos.status, fifos.stderr).toBe(0);
      const gateFd = openSync(gate, constants.O_RDWR);
      try {
        const launcher = path.join(sandbox.root, 'context-launcher');
        await readyContextFixture(
          sandbox,
          launcher,
          [
            'root=${0%/*}',
            // The descendant owns its gate before the hook (and its budget) starts.
            '(',
            '  exec 3< "$root/child-gate"',
            '  printf "ready\\n" > "$root/child-ready"',
            '  IFS= read -r release <&3',
            '  printf leaked > "$root/leaked-child"',
            ') &',
            'printf "%s\\n" "$!" > "$root/child-pid"',
            'IFS= read -r ready < "$root/child-ready"',
            '[ "$ready" = ready ] || exit 1',
            'printf "%s\\n" "$$" > "$root/hook-group"',
            // exec preserves runCli's owned process group and leader PID.
            'exec "$TMT_TEST_CONTEXT_EXECUTABLE" "$@"',
            '',
          ].join('\n')
        );
        const fake = path.join(sandbox.root, 'blocked-core');
        await readyContextFixture(
          sandbox,
          fake,
          'root=${0%/*}\nIFS= read -r release < "$root/child-gate"\n'
        );
        for (const witness of ['child-pid', 'hook-group', 'leaked-child']) {
          expect(existsSync(path.join(sandbox.root, witness))).toBe(false);
        }
        sandbox.env.TMT_EXECUTABLE = fake;
        sandbox.env.TMT_TEST_CONTEXT_EXECUTABLE = squadExecutable;
        const context = (deadlineMs = 5_000) =>
          runCli(
            { ...sandbox, cli: { executable: launcher, args: [] } },
            ['__tmt-hooks', '1', 'context'],
            { stdin: JSON.stringify({ version: 1, identityId: lead }), deadlineMs }
          );
        const assertNoDescendant = () => {
          const child = Number(readFileSync(path.join(sandbox.root, 'child-pid'), 'utf8'));
          expect(Number.isSafeInteger(child) && child > 1).toBe(true);
          const group = Number(readFileSync(path.join(sandbox.root, 'hook-group'), 'utf8'));
          expect(Number.isSafeInteger(group) && group > 1).toBe(true);
          // runCli confirms close and group exit before settling; independently
          // check the seeded child and the group recorded after its handshake.
          for (const target of [child, -group]) {
            let gone = false;
            try {
              process.kill(target, 0);
            } catch (error) {
              if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error;
              gone = true;
            }
            expect(gone).toBe(true);
          }
          expect(existsSync(path.join(sandbox.root, 'leaked-child'))).toBe(false);
          expect(readFileSync(cacheFile(), 'utf8')).toBe(before);
        };
        const started = performance.now();
        const timedOut = await context();
        expect(timedOut.signal).toBe('SIGKILL');
        expect(timedOut.stdout).toBe('');
        expect(performance.now() - started).toBeLessThan(1_000);
        assertNoDescendant();
        unlinkSync(path.join(sandbox.root, 'child-pid'));
        unlinkSync(path.join(sandbox.root, 'hook-group'));
        // The outer runner can cut off the hook before its local 300 ms budget.
        const outerStarted = performance.now();
        await expect(context(200)).rejects.toThrow('200 millisecond test bound');
        expect(performance.now() - outerStarted).toBeLessThan(1_000);
        assertNoDescendant();
      } finally {
        closeSync(gateFd);
      }
    });
  }, 90_000);

  it('reports observed age without changing board metadata or creating missing notes', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'Ben');
      const leadId = await identity(sandbox, 'Sol');
      const memberId = await identity(sandbox, 'Rin');
      for (const args of [
        ['init', 'product', '--me', 'Ben'],
        ['lead', 'Sol'],
        ['add', 'Rin'],
        ['set', 'Rin', 'task=review tokens', 'state=working'],
      ])
        expect((await squad(sandbox, args)).status).toBe(0);
      const toml = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(toml, `${readFileSync(toml, 'utf8')}\n[squad.product]\nlayout = "crew"\n`);
      const original = readFileSync(toml, 'utf8');
      const listing = () => squad(sandbox, ['ls', '--squad', 'product']);
      const disabled = await listing();
      expect(disabled.status).toBe(0);
      expect(disabled.body.sections[0].rows[0].staleness).toEqual({
        state: 'disabled',
        unchangedSinceMs: null,
        ageMs: null,
        activityAfterUpdate: false,
        reasons: [],
      });
      const directory = path.join(sandbox.root, 'cache', 'tmt-squad', 'staleness');
      expect(existsSync(directory)).toBe(false);
      expect(readFileSync(toml, 'utf8')).toBe(original);
      const created = await runCli(sandbox, ['notes', 'path', '--identity', 'Sol', '--json']);
      expect(created.status).toBe(0);
      const notebook = JSON.parse(created.stdout).path as string;
      writeFileSync(notebook, '# Current work\nReview token rotation.\n');
      expect(
        (await squad(sandbox, ['config', 'set', 'reminders.enabled', 'true', '--squad', 'product']))
          .status
      ).toBe(0);
      expect(
        (
          await squad(sandbox, [
            'config',
            'set',
            'reminders.stale_after',
            '1m',
            '--squad',
            'product',
          ])
        ).status
      ).toBe(0);
      const metadataBefore = observe(sandbox);
      const first = await listing();
      expect(first.status).toBe(0);
      expect(first.body.sections[0].rows[0].staleness).toMatchObject({ state: 'fresh', ageMs: 0 });
      expect(first.body.squad.notesStaleness).toMatchObject({ state: 'fresh', ageMs: 0 });
      const file = path.join(
        directory,
        readdirSync(directory).find((name) => name.endsWith('.json'))!
      );
      const persisted = JSON.parse(readFileSync(file, 'utf8'));
      // Independent fixture ages retained observations; the public commands
      // must recompute age from these records rather than from display fields.
      const since = Date.now() - 125_000;
      persisted.members[memberId].sinceMs = since;
      persisted.notes.sinceMs = since;
      persisted.observedAtMs = since;
      writeFileSync(file, JSON.stringify(persisted));
      const stale = await listing();
      expect(stale.status).toBe(0);
      expect(stale.stderr).toBe('');
      expect(stale.body.sections[0].rows[0].staleness).toMatchObject({
        state: 'stale',
        unchangedSinceMs: since,
        activityAfterUpdate: false,
      });
      expect(stale.body.squad.notesStaleness.state).toBe('stale');
      // A public config-writing command is unrelated to observed content age.
      expect((await squad(sandbox, ['me', 'Sol'])).status).toBe(0);
      const afterMe = await listing();
      expect(afterMe.body.sections[0].rows[0].staleness.unchangedSinceMs).toBe(since);
      expect(afterMe.body.squad.notesStaleness.unchangedSinceMs).toBe(since);

      const human = await runCli(sandbox, ['sq', 'ls', '--squad', 'product']);
      expect(human.status).toBe(0);
      expect(human.stdout).toContain('lead notes: stale 2m');
      expect(human.stdout).toContain('stale 2m');
      expect(human.stdout).toContain('Rin');
      expect(observe(sandbox)).toEqual(metadataBefore);
      expect((await runCli(sandbox, ['rename', 'Rin', 'NewRin', '--json'])).status).toBe(0);
      const renamed = await listing();
      expect(renamed.body.sections[0].rows[0]).toMatchObject({
        id: memberId,
        name: 'NewRin',
        staleness: { state: 'stale' },
      });
      expect((await squad(sandbox, ['set', 'NewRin', 'state=review'])).status).toBe(0);
      const updated = await listing();
      expect(updated.body.sections[0].rows[0].staleness).toMatchObject({
        state: 'fresh',
        ageMs: 0,
      });
      expect(updated.body.squad.notesStaleness.state).toBe('stale');
      unlinkSync(notebook);
      const missing = await listing();
      expect(missing.body.squad.lead.id).toBe(leadId);
      expect(missing.body.squad.notesStaleness.state).toBe('unknown');
      expect(existsSync(notebook)).toBe(false);
      const beforeDisable = readFileSync(file, 'utf8');
      expect(
        (
          await squad(sandbox, [
            'config',
            'set',
            'reminders.enabled',
            'false',
            '--squad',
            'product',
          ])
        ).status
      ).toBe(0);
      const off = await listing();
      expect(off.body.squad.notesStaleness.state).toBe('disabled');
      expect(readFileSync(file, 'utf8')).toBe(beforeDisable);
      expect(
        (await squad(sandbox, ['config', 'set', 'reminders.enabled', 'true', '--squad', 'product']))
          .status
      ).toBe(0);
      expect(
        (
          await squad(sandbox, [
            'config',
            'set',
            'reminders.stale_after',
            '1m',
            '--squad',
            'product',
          ])
        ).status
      ).toBe(0);
      const reenabled = await listing();
      expect(reenabled.body.sections[0].rows[0].staleness.unchangedSinceMs).toBe(
        updated.body.sections[0].rows[0].staleness.unchangedSinceMs
      );
      const afterReenable = readFileSync(file, 'utf8');
      const validSettings = readFileSync(toml, 'utf8');
      for (const [key, value] of [
        ['reminders.enabled', 'yes'],
        ['reminders.stale_after', '59s'],
        ['reminders.stale_after', '25h'],
      ]) {
        const rejected = await squad(sandbox, ['config', 'set', key, value, '--squad', 'product']);
        expect(rejected.status).toBe(1);
        expect(rejected.body.error.code).toBe('SQUAD_CONFIG_INVALID');
        expect(readFileSync(toml, 'utf8')).toBe(validSettings);
        expect(readFileSync(file, 'utf8')).toBe(afterReenable);
      }
      expect((await squad(sandbox, ['config', 'set', 'reminders.enabled', 'true'])).status).toBe(1);
      expect(readFileSync(toml, 'utf8')).toBe(validSettings);
      const invalid = `${original}\n[squad.product.reminders]\nenabled = true\nstale_after = "59s"\n`;
      writeFileSync(toml, invalid);
      const refused = await listing();
      expect(refused.status).toBe(1);
      expect(refused.body.error.code).toBe('SQUAD_CONFIG_INVALID');
      expect(readFileSync(toml, 'utf8')).toBe(invalid);
      expect(readFileSync(file, 'utf8')).toBe(afterReenable);
    });
  });

  // The Squad release proof (native-runtime-proof.mjs) expects this exact
  // line; PR CI never runs that proof, so this pins it.
  it('prints exactly squad <version> for --version and -V, directly and through tmt', async () => {
    await withSandbox(async (sandbox) => {
      const bin = installSquad(sandbox);
      expect(squadVersion).toBe('0.1.0-dev');
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
      // Without --squad the shape never depends on how many squads exist.
      const none = await squad(sandbox, ['status']);
      expect(none).toMatchObject({ status: 0, body: { squads: [], you: null } });
      expect((await runCli(sandbox, ['sq', 'ls'])).stdout).toBe(
        'No squad exists yet.\nhint: tmt squad init <name>\n'
      );
      const named = await squad(sandbox, ['status', '--squad', 'product']);
      expect(named).toMatchObject({ status: 1, body: { error: { code: 'SQUAD_NOT_FOUND' } } });
      // Completion v1: core invokes `tmt-<name> __complete -- <words>` directly.
      const completions = [];
      for (const name of ['tmt-squad', 'tmt-sq']) {
        const direct = { ...sandbox, cli: { executable: path.join(bin, name), args: [] } };
        completions.push(await runCli(direct, ['__complete', '--', 's']));
      }
      expect(completions[0].stdout).toBe('set\nskill\n');
      expect(completions[1].stdout).toBe(completions[0].stdout);
      const skill = await runCli(sandbox, ['sq', 'skill', 'show']);
      expect(skill.stdout).toContain('`ctrl-r` refreshes the board in squad, leads and all views');
      expect(skill.stdout).toContain('`f5 = "refresh"` binding remains supported');
      expect(skill.stdout).not.toContain('Ctrl-R');
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

  it('lists and edits view layers through real dispatch without changing workflow state', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'worker');
      expect((await squad(sandbox, ['init', 'product'])).status).toBe(0);
      expect((await squad(sandbox, ['add', 'worker', '--squad', 'product'])).status).toBe(0);
      const file = path.join(sandbox.globalDir, 'squad.toml');
      const original =
        "# untouched\n[squad.product]\nlayout = 'crew'\n[squad.product.board]\nview = 'focus' # own\n";
      writeFileSync(file, original);
      const before = await squad(sandbox, ['ls', '--squad', 'product']);
      const metadata = observe(sandbox).metadata;
      const aliases = await Promise.all(
        ['view', 'view ls', 'view list'].map((words) => squad(sandbox, words.split(' ')))
      );
      expect(aliases[0]).toEqual(aliases[1]);
      expect(aliases[1]).toEqual(aliases[2]);
      expect(aliases[0].body.views.map((view: { name: string }) => view.name)).toEqual([
        'team',
        'focus',
        'notes',
        'detail',
        'wide',
      ]);
      expect((await runCli(sandbox, ['sq', 'view', 'ls'])).stdout).toContain('VIEWS 5');
      expect((await squad(sandbox, ['view', 'set', 'notes', '--squad', 'product'])).status).toBe(0);
      expect(readFileSync(file, 'utf8')).toBe(
        original.replace("view = 'focus' # own", 'view = "notes" # own')
      );
      const after = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(after).toEqual(before);
      expect(observe(sandbox).metadata).toEqual(metadata);
      expect((await squad(sandbox, ['view', 'set', 'detail'])).status).toBe(0);
      expect(
        (await squad(sandbox, ['view', 'ls', '--squad', 'product'])).body.effective
      ).toMatchObject({ view: 'notes', source: 'squad', layout: 'crew' });
      expect((await squad(sandbox, ['view', 'rm', '--squad', 'product'])).status).toBe(0);
      expect(
        (await squad(sandbox, ['view', 'ls', '--squad', 'product'])).body.effective
      ).toMatchObject({ view: 'detail', source: 'board' });
      expect((await squad(sandbox, ['view', 'rm'])).status).toBe(0);
      expect(readFileSync(file, 'utf8')).toBe(
        original.replace("view = 'focus' # own\n", '').replace('[squad.product.board]\n', '')
      );
      const custom = original.replace("view = 'focus' # own", "panes = ['rows', 'notes'] # own");
      writeFileSync(file, custom);
      const refused = await squad(sandbox, ['view', 'set', 'wide', '--squad', 'product']);
      expect(refused).toMatchObject({ status: 1, body: { error: { code: 'SQUAD_VIEW_CUSTOM' } } });
      expect(readFileSync(file, 'utf8')).toBe(custom);
      expect((await squad(sandbox, ['view', 'set', 'bogus'])).body.error.code).toBe(
        'SQUAD_VIEW_UNKNOWN'
      );
      const help = await runCli(sandbox, ['sq', 'view', 'set', '--help']);
      expect(help.stdout).toContain('Usage: tmt squad view set');
      expect(help.stdout).toContain('hand-written board.layout or panes');
    });
  });

  it('keeps working when the global theme is wrong', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      mkdirSync(sandbox.globalDir, { recursive: true });
      // Squad finds squad.toml through tmt config show; a bad theme must not
      // stop it (the theme is presentation, reported by config show).
      writeFileSync(sandbox.globalConfig, JSON.stringify({ theme: { waiting: 'orange' } }));
      const none = await squad(sandbox, ['ls']);
      expect(none).toMatchObject({ status: 0, body: { squads: [], you: null } });
      expect((await runCli(sandbox, ['squad', 'init', 'product', '--json'])).status).toBe(0);
      const listed = await squad(sandbox, ['ls']);
      expect(listed.status).toBe(0);
      expect(listed.body.squads[0].squad.name).toBe('product');
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
      expect(capabilities.stdout).toBe('TMT-HOOKS/1\nlifecycle_observations_v1\ncontext_v1\n');
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

      // The conversation verbs moved to core: each refuses with its
      // replacement, sends nothing and never forwards.
      for (const [command, replacement] of [
        ['talk', 'tmt talk <member> "…" --detach'],
        ['reply', 'tmt answer <member> "…"'],
        ['replies', 'tmt x (and tmt result <request-id>)'],
      ]) {
        const removed = await squad(sandbox, [command, 'auth-fix', 'hello']);
        expect(removed, command).toMatchObject({
          status: 2,
          body: { error: { code: 'SQUAD_COMMAND_REMOVED' } },
        });
        const human = await runCli(sandbox, ['sq', command, 'auth-fix', 'hello']);
        expect(human.status).toBe(2);
        expect(human.stderr).toBe(
          `error: tmt squad ${command} was removed\nhint: ${replacement}\n`
        );
        const help = await runCli(sandbox, ['sq', '--help']);
        expect(help.stdout).not.toMatch(new RegExp(`^  ${command} `, 'm'));
      }
      expect(await requests()).toBe(0);

      // No pane identity and no recorded user: nothing is sent.
      const refused = await squad(sandbox, ['annotate', 'auth-fix', 'split it']);
      expect(refused).toMatchObject({
        status: 1,
        body: { error: { code: 'SQUAD_SENDER_UNKNOWN' } },
      });
      expect(await requests()).toBe(0);
      const text = await runCli(sandbox, ['sq', 'annotate', 'auth-fix', 'hello']);
      expect(text.stderr).toBe(
        'error: Who is sending? This pane has no identity, and no user is recorded\n' +
          'hint: Name this pane with tmt this <name>, or record yourself with tmt squad me <name>\n'
      );

      // An explicit identity speaks for itself, even with a user recorded.
      await squad(sandbox, ['me', 'Ben']);
      const asLead = await squad(sandbox, [
        'annotate',
        'auth-fix',
        'rebase first',
        '--to',
        'member',
        '--identity',
        'Sol',
      ]);
      expect(asLead.body).toMatchObject({ to: 'auth-fix', as: 'Sol' });
      expect(await sender(asLead.body.requestId)).toBe(ids.Sol);
      const unknown = await squad(sandbox, ['annotate', 'auth-fix', 'x', '--identity', 'Nobody']);
      expect(unknown.body.error.code).toBe('NAME_NOT_FOUND');

      // Without one, the recorded user sends.
      const asUser = await runCli(sandbox, ['sq', 'annotate', 'auth-fix', 'split it']);
      expect(asUser.stdout).toMatch(/^✓ Sent to Sol as Ben \(req_[0-9a-f-]+\)\n$/);
      const annotated = await squad(sandbox, ['annotate', 'auth-fix', 'split it']);
      expect(annotated.body).toMatchObject({ to: 'Sol', as: 'Ben' });
      expect(await sender(annotated.body.requestId)).toBe(ids.Ben);
    });
  });

  it('keeps leadership separate from role and lead fields through legacy conversion and re-add', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const sol = await identity(sandbox, 'Sol');
      const rin = await identity(sandbox, 'Rin');
      const initialized = await squad(sandbox, ['init', 'product']);
      const room = initialized.body.squad.roomId;
      for (const id of [sol, rin]) {
        expect((await runCli(sandbox, ['room', 'join', room, '--identity', id])).status).toBe(0);
      }
      expect(
        (
          await runCli(sandbox, [
            'identity',
            'meta',
            'set',
            'squad.product.role',
            'lead',
            '--identity',
            sol,
          ])
        ).status
      ).toBe(0);
      const legacy = observe(sandbox);
      for (const command of ['ls', 'board']) {
        const read = await squad(sandbox, [command, '--squad', 'product']);
        expect(read.status).toBe(0);
        expect(read.body.squad.lead.id).toBe(sol);
        expect(observe(sandbox)).toEqual(legacy);
      }

      expect(
        (await squad(sandbox, ['set', 'Sol', 'role=reviews every merge', 'lead=ordinary data']))
          .status
      ).toBe(0);
      let listing = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(listing.body.squad.lead).toMatchObject({
        id: sol,
        fields: { role: 'reviews every merge', lead: 'ordinary data' },
      });
      expect(listing.body.squad.lead.fields).not.toHaveProperty('lead.marker');
      expect(observe(sandbox).metadata).toEqual(
        expect.arrayContaining([
          { identity: 'Sol', key: 'squad.product.lead.marker', value: 'true' },
        ])
      );
      const textListing = await runCli(sandbox, ['sq', 'ls', '--squad', 'product']);
      expect(textListing.status).toBe(0);
      expect(textListing.stdout).not.toContain('lead.marker');
      expect((await squad(sandbox, ['set', 'Sol', 'role='])).status).toBe(0);
      listing = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(listing.body.squad.lead.id).toBe(sol);
      expect(listing.body.squad.lead.fields).not.toHaveProperty('role');

      expect((await squad(sandbox, ['set', 'Rin', 'role=lead', 'lead=true'])).status).toBe(0);
      expect((await squad(sandbox, ['set', 'Sol', 'role=lead'])).status).toBe(0);
      listing = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(listing.body.squad.lead.id).toBe(sol);
      expect(listing.body.sections[0].rows[0]).toMatchObject({
        id: rin,
        fields: { role: 'lead', lead: 'true' },
      });
      for (const row of [listing.body.squad.lead, ...listing.body.sections[0].rows]) {
        expect(row.fields).not.toHaveProperty('lead.marker');
      }
      expect(observe(sandbox).metadata).toEqual(
        expect.arrayContaining([
          { identity: 'Sol', key: 'squad.product.lead.marker', value: 'true' },
          { identity: 'Rin', key: 'squad.product.lead.marker', value: 'false' },
        ])
      );
      expect((await squad(sandbox, ['lead', 'Rin'])).body.replaced).toEqual(['Sol']);
      listing = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(listing.body.squad.lead).toMatchObject({
        id: rin,
        fields: { role: 'lead', lead: 'true' },
      });
      expect(listing.body.sections[0].rows[0]).toMatchObject({
        id: sol,
        fields: { role: 'lead', lead: 'ordinary data' },
      });

      const removed = await squad(sandbox, ['remove', 'Rin']);
      expect(removed.status).toBe(0);
      expect(removed.body.cleared.sort()).toEqual(['lead', 'lead.marker', 'role']);
      expect(
        observe(sandbox).metadata.filter(
          (row: { identity: string; key: string }) =>
            row.identity === 'Rin' && row.key === 'squad.product.lead.marker'
        )
      ).toEqual([]);
      expect((await squad(sandbox, ['add', 'Rin'])).status).toBe(0);
      listing = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(listing.body.squad.lead).toBeNull();
      expect(
        listing.body.sections[0].rows.find((row: { id: string }) => row.id === rin).fields
      ).not.toHaveProperty('lead.marker');
    });
  });

  it('keeps a failed legacy conversion from mutating roles or demoting another legacy lead', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const sol = await identity(sandbox, 'Sol');
      const rin = await identity(sandbox, 'Rin');
      const initialized = await squad(sandbox, ['init', 'product']);
      for (const id of [sol, rin]) {
        expect(
          (await runCli(sandbox, ['room', 'join', initialized.body.squad.roomId, '--identity', id]))
            .status
        ).toBe(0);
        expect(
          (
            await runCli(sandbox, [
              'identity',
              'meta',
              'set',
              'squad.product.role',
              'lead',
              '--identity',
              id,
            ])
          ).status
        ).toBe(0);
      }
      // Fill only Sol's metadata: the extra marker cannot be persisted.
      const db = new Database(sandbox.database);
      try {
        const insert = db.prepare(
          'INSERT INTO identity_metadata (identity_id, key, value) VALUES (?, ?, ?)'
        );
        for (let i = 0; i < 63; i++) insert.run(sol, `squad.product.fixture${i}`, 'value');
      } finally {
        db.close();
      }
      expect((await squad(sandbox, ['set', 'Sol', 'fixture0=updated'])).status).toBe(0);
      const before = observe(sandbox);
      const previousLead = (await squad(sandbox, ['ls', '--squad', 'product'])).body.squad.lead.id;
      expect([sol, rin]).toContain(previousLead);
      const failed = await squad(sandbox, ['set', 'Sol', 'fixture0=not applied', 'role=reviewer']);
      expect(failed.status).toBe(1);
      expect(failed.body.error.code).toBe('IDENTITY_METADATA_INVALID');
      expect(observe(sandbox)).toEqual(before);
      const listing = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(listing.body.squad.lead.id).toBe(previousLead);
      expect(observe(sandbox)).toEqual(before);
    });
  });

  it('masks pre-existing lead data before an addition and leaves a capped addition untouched', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const sol = await identity(sandbox, 'Sol');
      const rin = await identity(sandbox, 'Rin');
      expect((await squad(sandbox, ['init', 'product'])).status).toBe(0);
      for (const id of [sol, rin]) {
        expect(
          (
            await runCli(sandbox, [
              'identity',
              'meta',
              'set',
              'squad.product.role',
              'lead',
              '--identity',
              id,
            ])
          ).status
        ).toBe(0);
      }
      const db = new Database(sandbox.database);
      try {
        const insert = db.prepare(
          'INSERT INTO identity_metadata (identity_id, key, value) VALUES (?, ?, ?)'
        );
        for (let i = 0; i < 63; i++) insert.run(rin, `squad.product.fixture${i}`, 'value');
      } finally {
        db.close();
      }
      const before = observe(sandbox);
      const failed = await squad(sandbox, ['add', 'Rin']);
      expect(failed.status).toBe(1);
      expect(failed.body.results[0].error.code).toBe('IDENTITY_METADATA_INVALID');
      expect(observe(sandbox)).toEqual(before);
      expect((await squad(sandbox, ['add', 'Sol'])).status).toBe(0);
      const listing = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(listing.body.squad.lead).toBeNull();
      expect(listing.body.sections[0].rows[0]).toMatchObject({
        id: sol,
        fields: { role: 'lead' },
      });
    });
  });

  it('leaves a capped old legacy lead as the only lead when replacement cannot record its marker', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const sol = await identity(sandbox, 'Sol');
      await identity(sandbox, 'Rin');
      const initialized = await squad(sandbox, ['init', 'product']);
      expect(
        (await runCli(sandbox, ['room', 'join', initialized.body.squad.roomId, '--identity', sol]))
          .status
      ).toBe(0);
      expect(
        (
          await runCli(sandbox, [
            'identity',
            'meta',
            'set',
            'squad.product.role',
            'lead',
            '--identity',
            sol,
          ])
        ).status
      ).toBe(0);
      // Reach core's real boundary through its public validation path.
      for (let i = 0; i < 63; i++) {
        const filled = await runCli(sandbox, [
          'identity',
          'meta',
          'set',
          `fixture${i}`,
          'value',
          '--identity',
          sol,
          '--json',
        ]);
        expect(filled.status).toBe(0);
      }
      const before = observe(sandbox);
      expect(before.metadata.filter((row) => row.identity === 'Sol')).toHaveLength(64);
      const overflow = await runCli(sandbox, [
        'identity',
        'meta',
        'set',
        'overflow',
        'value',
        '--identity',
        sol,
        '--json',
      ]);
      expect(overflow.status).toBe(1);
      const coreError = parseWholeStdout(overflow).error;
      expect(coreError).toMatchObject({ code: 'IDENTITY_METADATA_INVALID' });
      expect(observe(sandbox)).toEqual(before);
      const failed = await squad(sandbox, ['lead', 'Rin']);
      expect(failed.status).toBe(1);
      expect(failed.body.error).toEqual(coreError);
      expect(observe(sandbox)).toEqual(before);
      const listing = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(listing.body.squad.lead.id).toBe(sol);
      expect(listing.body.sections[0].rows).toEqual([]);
    });
  });

  it('retires row notes without deleting legacy metadata and still accepts an explicit clear', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'coder');
      await squad(sandbox, ['init', 'product']);
      await squad(sandbox, ['add', 'coder']);
      expect(
        (
          await runCli(sandbox, [
            'identity',
            'meta',
            'set',
            'squad.product.note',
            'legacy summary',
            '--identity',
            'coder',
          ])
        ).status
      ).toBe(0);
      const before = observe(sandbox);
      const invalid = await squad(sandbox, ['set', 'coder', 'task=new task', 'note=new summary']);
      expect(invalid.status).toBe(1);
      expect(invalid.body.error).toMatchObject({ code: 'SQUAD_NOTE_RETIRED' });
      expect(invalid.body.error.message).toContain('tmt notes path --identity <member>');
      expect(invalid.body.error.message).toContain('task=');
      expect(invalid.body.error.message).toContain('pending=');
      expect(observe(sandbox)).toEqual(before);
      const human = await runCli(sandbox, ['sq', 'set', 'coder', 'note=new summary']);
      expect(human.status).toBe(1);
      expect(human.stdout).toBe('');
      expect(human.stderr).toContain('error: The per-member note is retired');
      expect(human.stderr).toContain('hint: tmt notes path --identity <member>');
      expect(observe(sandbox)).toEqual(before);
      // A configured legacy note line cannot redisplay the retained metadata.
      writeFileSync(
        path.join(sandbox.globalDir, 'squad.toml'),
        '[squad.product]\nlayout = "crew"\n[squad.product.rows]\ncolumns = [{name = "member"}, {name = "note"}]\nlines = [["member"], ["", "note"]]\n'
      );
      const listed = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(listed.status).toBe(0);
      const row = listed.body.sections[0].rows[0];
      expect(row).not.toHaveProperty('note');
      expect(row.fields).not.toHaveProperty('note');
      const text = await runCli(sandbox, ['sq', 'ls', '--squad', 'product']);
      expect(text.status).toBe(0);
      expect(text.stdout).not.toContain('legacy summary');
      expect(text.stdout).not.toContain('note:');
      expect(observe(sandbox)).toEqual(before);
      const cleared = await squad(sandbox, ['set', 'coder', 'note=']);
      expect(cleared.status).toBe(0);
      expect(cleared.body.applied).toEqual(['squad.product.note']);
      expect(observe(sandbox).metadata).toEqual(
        before.metadata.filter((entry) => entry.key !== 'squad.product.note')
      );
      expect((await squad(sandbox, ['set', 'coder', 'note='])).status).toBe(0);
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
      ]);
      expect(set.body.applied).toEqual(['squad.product.state', 'squad.product.pending']);

      writeFileSync(
        path.join(sandbox.globalDir, 'squad.toml'),
        'me = "Ben"\n[squad.product]\nlayout = "crew"\n'
      );
      const status = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(status.status).toBe(0);
      expect(status.body.squad).toMatchObject({
        name: 'product',
        layout: 'crew',
        lead: { name: 'Sol' },
        // The tab state the board colors, never carried by color alone (#507).
        attention: { state: 'waiting', waiting: 1, blocked: 1 },
      });
      expect(status.body.sections).toHaveLength(1);
      expect(status.body.sections[0].title).toBeNull();
      const rows = status.body.sections[0].rows;
      expect(rows.map((row: { name: string }) => row.name)).toEqual(['auth-fix', 'docs-sweep']);
      expect(rows[0]).toMatchObject({
        state: 'blocked',
        pending: 'approve the token rotation plan',
        presence: 'offline',
        fields: { state: 'blocked' },
      });
      expect(rows[1]).toMatchObject({ state: 'working', pending: null });
      expect(rows[0]).not.toHaveProperty('note');
      expect(rows[1]).not.toHaveProperty('note');
      // The lead skill documents this row shape; it must not drift silently.
      const skill = readFileSync(
        fileURLToPath(
          new URL('../../../extensions/tmt-squad/skills/tmt-squad/SKILL.md', import.meta.url)
        ),
        'utf8'
      );
      const documented = skill.slice(skill.indexOf('- Each row has'), skill.indexOf('- Every row'));
      const documentedFields = [...documented.matchAll(/`([a-z][A-Za-z]*)`/g)]
        .map((match) => match[1])
        .filter((field) => !['active', 'offline', 'unknown'].includes(field));
      expect(documentedFields.sort()).toEqual(
        Object.keys(rows[0])
          .filter((key) => key !== 'colors')
          .sort()
      );
      expect(skill).toContain('- A row has the optional `colors` key');
      expect(rows[0].colors).toEqual({ state: 'blocked' });
      expect(rows[1].colors).toEqual({ state: 'working' });
      const nested = skill.slice(skill.indexOf('- Every row'), skill.indexOf('- A row with'));
      const ageFields = [...nested.matchAll(/`([a-z][A-Za-z]*)`/g)]
        .map((match) => match[1])
        .filter(
          (field) =>
            !['staleness', 'state', 'disabled', 'unknown', 'fresh', 'stale', 'ls'].includes(field)
        );
      expect(['state', ...ageFields].sort()).toEqual(Object.keys(rows[0].staleness).sort());
      expect(Object.keys(status.body.squad.notesStaleness).sort()).toEqual(
        Object.keys(rows[0].staleness).sort()
      );
      expect(Object.keys(status.body.squad).sort()).toEqual([
        'attention',
        'layout',
        'lead',
        'name',
        'notesStaleness',
        'roomId',
      ]);
      // Without a terminal, the board is exactly status, in text and JSON.
      const statusText = await runCli(sandbox, ['squad', 'status']);
      expect(await runCli(sandbox, ['squad', 'board'])).toEqual(statusText);
      expect((await squad(sandbox, ['board', '--squad', 'product'])).body).toEqual(status.body);
      const text = await runCli(sandbox, ['sq', 'status']);
      // One leading mark: ◆ when the member waits on you; the state has its column.
      expect(text.stdout).toMatch(
        /\n {2}◆ {2}auth-fix +blocked +waiting on you: approve the token rotation plan/
      );

      expect((await squad(sandbox, ['set', 'auth-fix', 'pending='])).status).toBe(0);
      expect((await squad(sandbox, ['lead', 'Rin'])).body.replaced).toEqual(['Sol']);
      const removed = await squad(sandbox, ['remove', 'auth-fix']);
      expect(removed.body.cleared.sort()).toEqual(['state']);
      expect((await squad(sandbox, ['remove', 'auth-fix'])).body.cleared).toEqual([]);

      const after = observe(sandbox);
      expect(after.members).toEqual([
        { room: 'squad-product', identity: 'Rin' },
        { room: 'squad-product', identity: 'Sol' },
        { room: 'squad-product', identity: 'docs-sweep' },
      ]);
      expect(after.metadata).toEqual([
        { identity: 'Rin', key: 'squad.product.lead.marker', value: 'true' },
        { identity: 'Sol', key: 'squad.product.lead.marker', value: 'false' },
        { identity: 'auth-fix', key: 'team', value: 'core' },
        { identity: 'docs-sweep', key: 'squad.product.state', value: 'working' },
      ]);
      expect(after.rooms).toEqual([{ name: 'squad-product', retired: 0 }]);
    });
  });

  it('clears leadership without removing members, and explains leaderless recovery', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['Ben', 'Sol', 'Rin']) await identity(sandbox, name);
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      await squad(sandbox, ['lead', 'Sol']);
      await squad(sandbox, ['set', 'Sol', 'role=lead', 'task=Review']);
      const before = observe(sandbox);
      for (const args of [['lead'], ['lead', 'Sol', '--none']]) {
        expect((await squad(sandbox, args)).status).toBe(2);
        expect(observe(sandbox)).toEqual(before);
      }
      const cleared = await squad(sandbox, ['lead', '--none']);
      expect(cleared).toMatchObject({ status: 0, body: { lead: null, replaced: ['Sol'] } });
      const after = observe(sandbox);
      expect(after.members).toEqual(before.members);
      expect(after.metadata).toEqual(
        before.metadata.map((entry) =>
          entry.key === 'squad.product.lead.marker' ? { ...entry, value: 'false' } : entry
        )
      );
      expect(
        (await squad(sandbox, ['ls', '--squad', 'product'])).body.sections[0].rows
      ).toMatchObject([{ name: 'Sol', fields: { role: 'lead', task: 'Review' } }]);
      expect((await squad(sandbox, ['lead', '--none'])).body.replaced).toEqual([]);
      expect(observe(sandbox)).toEqual(after);
      const text = await runCli(sandbox, ['sq', 'ls', '--squad', 'product']);
      expect(text.stdout).toContain('squad product · no lead · layout team');
      expect(text.stdout).not.toContain('hint: tmt squad lead');
      const refused = await runCli(sandbox, ['sq', 'annotate', 'Sol', 'Review this']);
      expect(refused.status).toBe(1);
      expect(refused.stderr).toContain(
        'hint: Set one with tmt squad lead <name> --squad product, or use --to member'
      );
      expect(observe(sandbox)).toEqual(after);
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(db.prepare('SELECT COUNT(*) AS n FROM request_attempts').get()).toEqual({ n: 0 });
      } finally {
        db.close();
      }
      expect(
        (await squad(sandbox, ['annotate', 'Sol', 'Review this', '--to', 'member'])).body
      ).toMatchObject({ to: 'Sol', as: 'Ben' });
      await squad(sandbox, ['lead', 'Rin']);
      const clearedText = await runCli(sandbox, ['sq', 'lead', '--none']);
      expect(clearedText.stdout).toBe('✓ Squad product has no lead; Rin remains a member\n');
      await squad(sandbox, ['init', 'reviews']);
      expect((await squad(sandbox, ['lead', '--none'])).body.error.code).toBe('SQUAD_AMBIGUOUS');
      expect((await squad(sandbox, ['lead', '--none', '--squad', 'product'])).status).toBe(0);
    });
  });

  it('distinguishes new membership, duplicate operands and unchanged re-adds', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'coder');
      await squad(sandbox, ['init', 'product']);
      const added = await squad(sandbox, ['add', 'coder', 'coder']);
      expect(added.status).toBe(0);
      expect(added.body.results).toMatchObject([
        { name: 'coder', added: true, stateSet: 'working' },
        { name: 'coder', added: false, stateSet: null },
      ]);
      await squad(sandbox, ['set', 'coder', 'state=blocked', 'task=Review']);
      const before = observe(sandbox);
      expect((await squad(sandbox, ['add', 'coder'])).body.results).toMatchObject([
        { added: false, stateSet: null },
      ]);
      expect(observe(sandbox)).toEqual(before);
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
        stdout: '✓ Rin leads squad product (replaces Sol; Sol remains a member)\n',
      });
      // A partial add keeps its successes on stdout and each failure on stderr.
      expect(await text(['add', 'coder', 'ghost'])).toMatchObject({
        status: 1,
        stdout: '✓ Added coder to squad product (state working)\n',
        stderr: "error: Could not add ghost: Identity 'ghost' was not found\n",
      });
      expect((await text(['add', 'coder'])).stdout).toBe('coder is already in squad product.\n');
      expect(await text(['set', 'coder', 'state=blocked', 'task=needs review'])).toMatchObject({
        status: 0,
        stdout: '✓ Set state, task on coder\n',
        stderr: '',
      });
      expect(await text(['set', 'outsider', 'state=working'])).toMatchObject({
        status: 1,
        stdout: '',
        stderr: "error: 'outsider' is not in squad product; add it first\n",
      });
      expect(await text(['remove', 'coder'])).toMatchObject({
        status: 0,
        stdout: '✓ Removed coder from squad product; cleared state, task\n',
        stderr: '',
      });
    });
  });

  it('lists members with ls; status and a bare tmt sq without a terminal are the same list', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['Ben', 'auth-fix']) await identity(sandbox, name);
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      const legacyConfig = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(
        legacyConfig,
        `${readFileSync(legacyConfig, 'utf8')}\n[squad.product]\nlayout = "crew"\n`
      );
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
      // Both JSON shapes are pinned: --squad gives that squad's document;
      // without it, always {squads, you}, even with one squad.
      const json = await squad(sandbox, ['ls']);
      expect(Object.keys(json.body).sort()).toEqual(['squads', 'you']);
      expect(json.body.squads).toHaveLength(1);
      const one = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(Object.keys(one.body).sort()).toEqual([
        'columns',
        'lines',
        'olderRequestsNotShown',
        'sections',
        'squad',
        'you',
      ]);
      expect(json.body.squads[0]).toEqual({ ...one.body, you: undefined });
      expect(one.body.columns.map((column: { field: string }) => column.field)).toEqual([
        'member',
        'state',
        'task',
        'pr_link',
      ]);
      // The preset's grid: fixed widths, a growing task, and a link that
      // steps aside first on a narrow board; one line per row.
      expect(one.body.columns[2]).toMatchObject({ field: 'task', width: null, grow: 1 });
      expect(one.body.columns[3]).toMatchObject({ field: 'pr_link', width: 12, priority: 1 });
      expect(one.body.lines).toEqual([
        ['member', 'state', 'task', 'pr_link'].map((field) => ({ field, span: 1 })),
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
      expect(help.stdout).toMatch(/\n {2}ls +List members or a board tab/);
      expect(help.stdout).not.toMatch(/\n {2}status /);
      // Explicit F5 remains valid while the host preset uses ctrl-r.
      const toml = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(
        toml,
        `${readFileSync(toml, 'utf8')}\n[bind]\nctrl-r = "refresh"\nf5 = "refresh"\n`
      );
      const rebound = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(rebound.status).toBe(0);
      expect(rebound.body.sections).toEqual(one.body.sections);
    });
  });

  it('publishes opt-in percent and overflow metadata while fitting text and preserving full rows', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'worker');
      await squad(sandbox, ['init', 'product']);
      writeFileSync(
        path.join(sandbox.globalDir, 'squad.toml'),
        '[squad.product]\nlayout = "crew"\n'
      );
      await squad(sandbox, ['add', 'worker']);
      const task =
        'alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron';
      await squad(sandbox, ['set', 'worker', `task=${task}`]);
      const before = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(
        before.body.columns.every(
          (column: Record<string, unknown>) => !('overflow' in column) && !('max_lines' in column)
        )
      ).toBe(true);
      writeFileSync(
        path.join(sandbox.globalDir, 'squad.toml'),
        [
          '[squad.product]',
          'layout = "crew"',
          '[squad.product.rows]',
          'columns = [',
          '  { name = "member", width = "20%", overflow = "ellipsis" },',
          '  { name = "task", width = "40%", overflow = "wrap", max_lines = 2 },',
          ']',
          '',
        ].join('\n')
      );
      const configured = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(configured.status).toBe(0);
      expect(configured.body.columns[0]).toMatchObject({
        field: 'member',
        width: '20%',
        overflow: 'ellipsis',
      });
      expect(configured.body.columns[0]).not.toHaveProperty('max_lines');
      expect(configured.body.columns[1]).toMatchObject({
        field: 'task',
        width: '40%',
        overflow: 'wrap',
        max_lines: 2,
      });
      expect(configured.body.columns[1]).not.toHaveProperty('lines');
      expect(configured.body.sections).toEqual(before.body.sections);
      const text = await runCli(sandbox, ['sq', 'ls', '--squad', 'product']);
      expect(text.status).toBe(0);
      const data = text.stdout.split('\n').filter((line) => line.startsWith('  '));
      expect(data).toHaveLength(2);
      expect(data[0]).toContain('worker');
      expect(data[0]).toContain('alpha beta');
      expect(data[1]).toContain('…');
      expect(data[1]).not.toContain('worker');
      expect(configured.body.sections[0].rows[0].fields.task).toBe(task);
      const after = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(after.body).toEqual(configured.body);
    });
  });

  it('marks uncovered columns value-only while preserving their values and ignoring text sizing', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'worker');
      await squad(sandbox, ['init', 'checkout']);
      await squad(sandbox, ['add', 'worker']);
      await squad(sandbox, [
        'set',
        'worker',
        'task=alpha beta gamma delta epsilon zeta eta theta',
        'pr_state=OPEN',
      ]);
      const example = readFileSync(
        new URL(
          '../../../extensions/tmt-squad/rust/tmt-squad/src/rows/fixtures/uncovered-tracks.toml',
          import.meta.url
        ),
        'utf8'
      );
      const file = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(file, example);
      const result = await squad(sandbox, ['ls', '--squad', 'checkout']);
      expect(result.status).toBe(0);
      expect(
        result.body.columns.slice(0, 4).every((c: Record<string, unknown>) => !('valueOnly' in c))
      ).toBe(true);
      expect(
        result.body.columns.slice(4).map((c: Record<string, unknown>) => [c.field, c.valueOnly])
      ).toEqual([
        ['ctx', true],
        ['model', true],
      ]);
      expect(result.body.columns[4]).toMatchObject({
        width: 6,
        from: 'session.usage.tokens',
        format: 'tokens',
      });
      expect(result.body.lines[3]).toEqual([
        { field: null, span: 1 },
        { field: 'ctx', span: 1 },
        { field: 'model', span: 2 },
      ]);
      expect(result.body.sections[0].rows[0].fields.task).toBe(
        'alpha beta gamma delta epsilon zeta eta theta'
      );
      const first = await runCli(sandbox, ['sq', 'ls', '--squad', 'checkout']);
      writeFileSync(
        file,
        example.replace('width = 6', 'width = 200').replace('width = 14', 'width = 200')
      );
      const second = await runCli(sandbox, ['sq', 'ls', '--squad', 'checkout']);
      expect(second.status).toBe(0);
      expect(second.stdout).toBe(first.stdout);
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
  it('resolves state patterns into the same order and color tokens in JSON and text', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['Ben', 'exact', 'pattern', 'working', 'unknown']) {
        await identity(sandbox, name);
      }
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      const legacyConfig = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(
        legacyConfig,
        `${readFileSync(legacyConfig, 'utf8')}\n[squad.product]\nlayout = "crew"\n`
      );
      await squad(sandbox, ['add', 'exact', 'pattern', 'working', 'unknown']);
      await squad(sandbox, ['set', 'exact', 'state=blocked']);
      await squad(sandbox, ['set', 'pattern', 'state=BLOCKED-on-ci']);
      await squad(sandbox, ['set', 'unknown', 'state=unranked']);
      const squadToml = path.join(sandbox.globalDir, 'squad.toml');
      const base = readFileSync(squadToml, 'utf8');
      const settings = `
[squad.product.states]
blocked = { color = "red", sort = 7 }
[[squad.product.state_patterns]]
match = "blocked*"
color = "blocked"
sort = 0
ignore_case = true
[[squad.product.state_patterns]]
match = "blocked*"
color = "review"
sort = 9
`;
      writeFileSync(squadToml, base + settings);
      for (const sections of [
        '',
        '\n[[squad.product.section]]\ntitle = "All"\nsort = ["state"]\n',
      ]) {
        writeFileSync(squadToml, base + settings + sections);
        const listed = await squad(sandbox, ['ls', '--squad', 'product']);
        expect(listed.status).toBe(0);
        const rows = listed.body.sections[0].rows;
        expect(rows.map((row: { name: string }) => row.name)).toEqual([
          'pattern',
          'working',
          'exact',
          'unknown',
        ]);
        expect(rows.map((row: { colors?: { state?: string } }) => row.colors?.state)).toEqual([
          'blocked',
          'working',
          'red',
          undefined,
        ]);
        expect(rows[3]).not.toHaveProperty('colors');
        const text = await runCli(sandbox, ['sq', 'ls', '--squad', 'product']);
        expect(text.status).toBe(0);
        const order = ['pattern', 'working', 'exact', 'unknown'].map((name) =>
          text.stdout.indexOf(name)
        );
        expect(order.every((offset) => offset >= 0)).toBe(true);
        expect(order).toEqual([...order].sort((a, b) => a - b));
        expect(text.stdout).toContain('BLOCKED-on-ci');
      }
      writeFileSync(squadToml, base + settings.replace('color = "blocked"', 'color = "pink"'));
      const refused = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(refused.status).not.toBe(0);
      expect(JSON.stringify(refused.body)).toContain('squad.product.state_patterns[0].color');
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
      const status = await squad(sandbox, ['ls', '--squad', 'product']);
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
        (await squad(sandbox, ['ls', '--squad', 'product'])).body.sections[0].rows.map(
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
      writeExecutable(
        opener,
        `#!/bin/sh\nprintf '%s\\n' "$@" > '${opened}.tmp'\nmv '${opened}.tmp' '${opened}'\n`,
        0o755
      );
      writeExecutable(clipboard, `#!/bin/sh\ncat > '${copied}'\n`, 0o755);
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

  it('talks, annotates and answers as the user through core commands only', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const ids: Record<string, string> = {};
      for (const name of ['Ben', 'Sol', 'auth-fix', 'docs'])
        ids[name] = await identity(sandbox, name);
      await squad(sandbox, ['init', 'product', '--me', 'Ben']);
      const legacyConfig = path.join(sandbox.globalDir, 'squad.toml');
      writeFileSync(
        legacyConfig,
        `${readFileSync(legacyConfig, 'utf8')}\n[squad.product]\nlayout = "crew"\n`
      );
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

      // talk: the board's t is core's detached talk in the squad room, with
      // the text after `--` as the board sends it.
      const text = '-rf; $(touch pwned) "quoted"';
      const talk = await runCli(sandbox, [
        'talk',
        '--identity',
        'Ben',
        '--room',
        'squad-product',
        '--detach',
        '--json',
        '--',
        'auth-fix',
        text,
      ]);
      expect(talk.status).toBe(0);
      const talked = await prompt(JSON.parse(talk.stdout).requestId);
      expect(talked).toMatchObject({
        roomId,
        recipientId: ids['auth-fix'],
        sender: { identityId: ids.Ben },
        kind: 'request',
        final: { status: 'not_submitted' },
        prompt: { message: text },
      });
      const before = (await roomRequests()).length;
      const empty = await squad(sandbox, ['annotate', 'auth-fix', '']);
      expect(empty.body.error.code).toBe('SQUAD_ACTION_REFUSED');
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
        const rows = (await squad(sandbox, ['ls', '--squad', 'product'])).body.sections[0].rows;
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

      // Waiting on you comes from tmt inbox; the board's r is tmt answer.
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
      const waiting = async () =>
        (
          await squad(sandbox, ['ls', '--squad', 'product'])
        ).body.sections[0].rows[0].waitingOnYou.map(
          (item: { requestId: string }) => item.requestId
        );
      expect(await waiting(), 'oldest first').toEqual(asks);
      const attention = async () =>
        (await squad(sandbox, ['ls', '--squad', 'product'])).body.squad.attention;
      expect(await attention(), 'a request waiting on you counts').toEqual({
        state: 'waiting',
        waiting: 1,
        blocked: 0,
      });
      // Acknowledging a request (as live delivery does) does not stop it
      // waiting: it stays on the row until it has a final.
      const first = await incoming('Ben', asks[0]);
      const acked = await runCli(sandbox, [
        'x',
        'ack',
        asks[0],
        '--incoming',
        '--revision',
        String(first.revision),
        '--identity',
        'Ben',
        '--json',
      ]);
      expect(acked.status).toBe(0);
      expect(await waiting()).toEqual(asks);
      const board = async (request: string, body: string) =>
        runCli(sandbox, [
          'answer',
          '--identity',
          'Ben',
          '--request',
          request,
          '--json',
          '--',
          'auth-fix',
          body,
        ]);
      const chosen = await board(asks[0], '-postgres');
      expect(chosen.status, chosen.stdout).toBe(0);
      expect(JSON.parse(chosen.stdout)).toMatchObject({ requestId: asks[0], status: 'submitted' });
      const result = JSON.parse((await runCli(sandbox, ['result', asks[0], '--json'])).stdout);
      expect(result.response).toBe('-postgres');
      expect(await waiting()).toEqual([asks[1]]);
      expect((await board(asks[1], 'yes')).status).toBe(0);
      expect(await waiting()).toEqual([]);
      expect(await attention()).toEqual({ state: 'normal', waiting: 0, blocked: 0 });
      expect((await incoming('Ben', asks[1])).acknowledged, 'answering never acknowledges').toBe(
        false
      );

      expect(await notebook(), 'no notebook was created or written').toBe('NOTEBOOK_NOT_FOUND');
      const projection = async () =>
        (await squad(sandbox, ['ls', '--squad', 'product'])).body.squad;
      expect(await projection()).not.toHaveProperty('noteAnnotations');
      const note = await runCli(sandbox, [
        'talk',
        '--identity',
        'Ben',
        '--room',
        'squad-product',
        '--detach',
        '--json',
        '--',
        'Sol',
        '[product · notes L5 "- Keep context short."] Review this line.',
      ]);
      expect(note.status).toBe(0);
      const noteId = JSON.parse(note.stdout).requestId;
      expect((await projection()).noteAnnotations).toEqual([
        { requestId: noteId, line: 4, quote: '- Keep context short.' },
      ]);
      expect(
        (
          await runCli(sandbox, [
            'answer',
            '--identity',
            'Sol',
            '--request',
            noteId,
            '--json',
            '--',
            'Ben',
            'done',
          ])
        ).status
      ).toBe(0);
      expect(await projection()).not.toHaveProperty('noteAnnotations');
      expect(existsSync(path.join(sandbox.root, 'pwned'))).toBe(false);
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

describe('Squad cron clock', () => {
  it('keeps status read-only and admits exact paused manual sends through the explicit actor', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const user = await identity(sandbox, 'Ben');
      const lead = await identity(sandbox, 'Sol');
      const worker = await identity(sandbox, 'worker');
      expect((await squad(sandbox, ['init', 'product', '--me', 'Ben'])).status).toBe(0);
      expect((await squad(sandbox, ['lead', 'Sol'])).status).toBe(0);
      expect((await squad(sandbox, ['add', 'worker'])).status).toBe(0);
      const root = parseWholeStdout(
        await runCli(sandbox, ['api'], {
          stdin: JSON.stringify({ version: 1, operation: 'storage.root', input: {} }),
        })
      ).dataRoot as string;
      const directory = path.join(root, 'squad', 'cron');
      const absent = await squad(sandbox, ['cron', 'clock']);
      expect(absent).toMatchObject({ status: 0, body: { clock: { state: 'no clock' } } });
      expect(existsSync(directory)).toBe(false);
      const help = await runCli(sandbox, ['squad', 'cron', '--help']);
      expect(help.status).toBe(0);
      for (const command of ['send', 'run', 'tick', 'clock']) {
        expect(help.stdout).toMatch(new RegExp(`^  ${command}\\s`, 'm'));
      }
      const message = 'literal {time}\nmanual reminder';
      const added = await squad(sandbox, [
        'cron',
        'add',
        'product',
        'worker',
        '--every',
        '1h',
        '--paused',
        message,
      ]);
      expect(added.status, JSON.stringify(added.body)).toBe(0);
      const store = path.join(directory, 'jobs.json');
      const before = readFileSync(store);
      const denied = await squad(sandbox, [
        'cron',
        'send',
        'product',
        'c1',
        '--identity',
        'worker',
      ]);
      expect(denied).toMatchObject({
        status: 1,
        body: { error: { code: 'SQUAD_CRON_PERMISSION_DENIED' } },
      });
      const first = await squad(sandbox, ['cron', 'send', 'product', 'c1', '--identity', 'Sol']);
      const second = await squad(sandbox, ['cron', 'send', 'product', 'c1', '--identity', 'Ben']);
      expect(first.status, JSON.stringify(first.body)).toBe(0);
      expect(second.status, JSON.stringify(second.body)).toBe(0);
      expect(first.body.dispatch.operationId).not.toBe(second.body.dispatch.operationId);
      expect(readFileSync(store)).toEqual(before);
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          db
            .prepare(`SELECT recipient_identity_id AS recipient, originator_identity_id AS actor,
          room_id AS room, message_text AS message FROM request_attempts WHERE request_kind='request' ORDER BY rowid`)
            .all()
        ).toEqual([
          { recipient: worker, actor: lead, room: added.body.job.roomId, message },
          { recipient: worker, actor: user, room: added.body.job.roomId, message },
        ]);
      } finally {
        db.close();
      }
      const tick = await squad(sandbox, ['cron', 'tick']);
      expect(tick).toMatchObject({ status: 0, body: { accepted: 0, complete: true } });
      const lease = path.join(directory, 'clock.json');
      expect(existsSync(lease)).toBe(false);
      writeFileSync(lease, 'corrupt clock evidence');
      const unknown = await squad(sandbox, ['cron', 'clock']);
      expect(unknown).toMatchObject({ status: 1, body: { clock: { state: 'unknown' } } });
      expect(readFileSync(lease, 'utf8')).toBe('corrupt clock evidence');
    });
  }, 20_000);
});

describe('Squad cron management', () => {
  it('admits the user and lead, preserves exact jobs, and records announcements independently', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const user = await identity(sandbox, 'Ben');
      const lead = await identity(sandbox, 'Sol');
      const worker = await identity(sandbox, 'worker');
      const reviewer = await identity(sandbox, 'reviewer');
      expect((await squad(sandbox, ['init', 'product', '--me', 'Ben'])).status).toBe(0);
      expect((await squad(sandbox, ['lead', 'Sol'])).status).toBe(0);
      expect((await squad(sandbox, ['add', 'worker', 'reviewer'])).status).toBe(0);
      const announcements = () => {
        const db = new Database(sandbox.database, { readonly: true });
        try {
          return db
            .prepare(`SELECT recipient_identity_id AS recipient, originator_identity_id AS actor,
            message_text AS message, request_kind AS kind FROM request_attempts ORDER BY rowid`)
            .all() as { recipient: string; actor: string | null; message: string; kind: string }[];
        } finally {
          db.close();
        }
      };
      const message = 'literal {time}\n! reminder';
      const added = await squad(sandbox, [
        'cron',
        'add',
        'product',
        'worker',
        '--every',
        '30m',
        message,
      ]);
      expect(added.status, JSON.stringify(added.body)).toBe(0);
      expect(added.body).toMatchObject({
        changed: true,
        warnings: [],
        job: { id: 'c1', ownerId: worker, message, state: 'on', revision: 1 },
      });
      expect(announcements()).toMatchObject([
        { recipient: worker, actor: user, kind: 'announcement' },
      ]);
      const activeText = await runCli(sandbox, ['squad', 'cron', 'show', 'product', 'c1']);
      expect(activeText.status).toBe(0);
      expect(activeText.stdout).toContain('schedule');
      expect(activeText.stdout).not.toMatch(/^\s+pause\s/m);
      const root = parseWholeStdout(
        await runCli(sandbox, ['api'], {
          stdin: JSON.stringify({ version: 1, operation: 'storage.root', input: {} }),
        })
      ).dataRoot as string;
      const store = path.join(root, 'squad', 'cron', 'jobs.json');
      expect(JSON.parse(readFileSync(store, 'utf8')).jobs[0].message).toBe(message);
      expect(statSync(store).mode & 0o777).toBe(0o600);
      const config = path.join(sandbox.globalDir, 'squad.toml');
      const originalConfig = readFileSync(config, 'utf8');
      const denied = await squad(sandbox, [
        'cron',
        'pause',
        'product',
        'c1',
        '--identity',
        'worker',
      ]);
      expect(denied.status).toBe(1);
      expect(denied.body.error.code).toBe('SQUAD_CRON_PERMISSION_DENIED');
      expect(announcements()).toHaveLength(1);
      const paused = await squad(sandbox, ['cron', 'pause', 'product', 'c1', '--identity', 'Sol']);
      expect(paused.body.job).toMatchObject({ state: 'paused', pause: { by: lead }, revision: 2 });
      const pausedText = await runCli(sandbox, ['squad', 'cron', 'show', 'product', 'c1']);
      expect(pausedText.status).toBe(0);
      expect(pausedText.stdout).toMatch(/^\s+pause\s/m);
      expect(pausedText.stdout).toContain(lead);
      expect((await squad(sandbox, ['cron', 'resume', 'product', 'c1'])).body.job.state).toBe('on');
      expect((await squad(sandbox, ['cron', 'reassign', 'product', 'c1', 'reviewer'])).status).toBe(
        0
      );
      expect(announcements().slice(-2)).toMatchObject([
        { recipient: worker },
        { recipient: reviewer },
      ]);
      expect(announcements().at(-1)!.message).toContain('literal {time}\\n! reminder');
      expect(
        (await squad(sandbox, ['cron', 'show', 'product', 'c1'])).body.job.nextMs
      ).toHaveLength(3);
      expect(
        (await squad(sandbox, ['cron', 'edit', 'product', 'c1', '--message', ' \n'])).body.error
          .code
      ).toBe('SQUAD_CRON_MESSAGE_INVALID');
      expect((await squad(sandbox, ['cron', 'reassign', 'product', 'c1', 'Sol'])).status).toBe(0);
      const count = announcements().length;
      expect(
        (await squad(sandbox, ['cron', 'pause', 'product', 'c1', '--identity', 'Sol'])).status
      ).toBe(0);
      expect(
        (await squad(sandbox, ['cron', 'rm', 'product', 'c1', '--identity', 'Sol'])).status
      ).toBe(0);
      expect(announcements()).toHaveLength(count);
      expect(readFileSync(config, 'utf8')).toBe(originalConfig);
      expect(
        (
          await squad(sandbox, [
            'cron',
            'add',
            'product',
            'worker',
            '--at',
            '09:00',
            '--on',
            'weekdays',
            'check',
          ])
        ).body.job.id
      ).toBe('c2');
      writeFileSync(config, originalConfig + '\n[tabs]\nhide=["product"]\n');
      expect((await squad(sandbox, ['cron', 'ls'])).body.jobs).toHaveLength(1);
      for (const notice of announcements()) {
        expect(notice.kind).toBe('announcement');
        expect(notice.message.startsWith('▚ ⏱')).toBe(true);
        expect(notice.message).not.toContain('\n');
      }
    });
  }, 60_000);

  it('pauses retired ownership, acknowledges its hook and never follows a reused room name', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'Ben');
      const lead = await identity(sandbox, 'Sol');
      const worker = await identity(sandbox, 'worker');
      expect((await squad(sandbox, ['init', 'product', '--me', 'Ben'])).status).toBe(0);
      expect((await squad(sandbox, ['lead', 'Sol'])).status).toBe(0);
      expect((await squad(sandbox, ['add', 'worker'])).status).toBe(0);
      const added = await squad(sandbox, [
        'cron',
        'add',
        'product',
        'worker',
        '--every',
        '1h',
        'check',
      ]);
      const oldRoom = added.body.job.roomId;
      expect((await runCli(sandbox, ['rm', 'worker', '--force', '--json'])).status).toBe(0);
      const retired = await squad(sandbox, ['cron', 'show', 'product', 'c1']);
      expect(retired.status, JSON.stringify(retired.body)).toBe(0);
      expect(retired.body.job).toMatchObject({
        state: 'no owner',
        ownerId: null,
        revision: 2,
        nextMs: [],
      });
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          db
            .prepare(
              "SELECT state FROM identity_hooks WHERE consumer='squad-cron' AND identity_id=?"
            )
            .get(worker)
        ).toEqual({ state: 'delivered' });
        expect(
          db
            .prepare(
              'SELECT recipient_identity_id AS recipient, originator_identity_id AS actor, message_text AS message FROM request_attempts ORDER BY rowid DESC LIMIT 1'
            )
            .get()
        ).toMatchObject({
          recipient: lead,
          actor: null,
          message: expect.stringContaining('worker retired'),
        });
      } finally {
        db.close();
      }
      expect((await squad(sandbox, ['cron', 'resume', 'product', 'c1'])).body.error.code).toBe(
        'SQUAD_CRON_NO_OWNER'
      );
      expect((await runCli(sandbox, ['room', 'rm', oldRoom, '--json'])).status).toBe(0);
      expect((await squad(sandbox, ['init', 'product'])).status).toBe(0);
      expect((await squad(sandbox, ['cron', 'ls'])).body.jobs).toEqual([]);
      expect((await squad(sandbox, ['cron', 'show', 'product', 'c1'])).body.error.code).toBe(
        'SQUAD_CRON_NOT_FOUND'
      );
      expect((await squad(sandbox, ['add', 'Sol'])).status).toBe(0);
      const replacement = await squad(sandbox, [
        'cron',
        'add',
        'product',
        'Sol',
        '--every',
        '1h',
        'new room',
      ]);
      expect(replacement.body.job.id).toBe('c2');
      expect(replacement.body.job.roomId).not.toBe(oldRoom);
    });
  }, 60_000);
});
