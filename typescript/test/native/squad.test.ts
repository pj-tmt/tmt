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
  const crewFields = ['member', 'state', 'task', 'pr_link', 'model', 'tok_1', 'tok_2', 'tok_3'];

  it('checks layout files offline without core, configuration or board startup', async () => {
    await withSandbox(async (sandbox) => {
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(path.join(sandbox.globalDir, 'squad.toml'), 'invalid [');
      sandbox.env.TMT_EXECUTABLE = 'relative-core-is-invalid';
      const offline = { ...sandbox, cli: { executable: squadExecutable, args: [] } };
      const file = path.join(sandbox.cwd, 'board.xml');
      writeFileSync(
        file,
        "<tmt-view version='1'><tmt-repeat each='$.rows' as='row'><tmt-cell bind='row.fields.task'/></tmt-repeat></tmt-view>"
      );
      const before = existsSync(sandbox.database);
      const valid = await runCli(offline, ['layout', 'validate', file, '--json']);
      expect(valid.status).toBe(0);
      expect(JSON.parse(valid.stdout)).toEqual({
        valid: true,
        file,
        version: 1,
        schema: 'squad-projected-v1',
      });
      const human = await runCli(offline, ['layout', 'validate', file]);
      expect(human.status).toBe(0);
      expect(human.stdout).toContain('Valid layout:');
      expect(human.stderr).toBe('');
      writeFileSync(
        file,
        "<tmt-view version='1'><tmt-repeat each='$.rows' as='row'><tmt-cell bind='row.fields.Bad'/></tmt-repeat></tmt-view>"
      );
      const invalid = await runCli(offline, ['layout', 'validate', file, '--json']);
      expect(invalid.status).toBe(1);
      expect(JSON.parse(invalid.stdout).error).toMatchObject({
        code: 'LAYOUT_INVALID',
        message: expect.stringContaining(`${file}:1:`),
      });
      writeFileSync(file, ' '.repeat(256 * 1024 + 1));
      expect((await runCli(offline, ['layout', 'validate', file, '--json'])).status).toBe(1);
      unlinkSync(file);
      expect(
        JSON.parse((await runCli(offline, ['layout', 'validate', file, '--json'])).stdout).error
          .code
      ).toBe('LAYOUT_IO');
      expect((await runCli(offline, ['layout', 'validate', '--json'])).status).toBe(2);
      expect(existsSync(sandbox.database)).toBe(before);
      expect(readFileSync(path.join(sandbox.globalDir, 'squad.toml'), 'utf8')).toBe('invalid [');
    });
  });

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

  it('ignores the obsolete HOME replies setting without rewriting authored TOML', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      await identity(sandbox, 'home-settings-owner');
      const config = path.join(sandbox.globalDir, 'squad.toml');
      const before = '# keep this comment\n[board]\nhome_replies=false\n';
      writeFileSync(config, before);
      const shown = await squad(sandbox, ['config', 'show', '--tab', 'all']);
      expect(shown.status).toBe(0);
      expect(
        shown.body.entries.some((entry: { key: string }) => entry.key === 'board.home_replies')
      ).toBe(false);
      expect(
        shown.body.notices.filter((notice: string) => notice.includes('board.home_replies'))
      ).toEqual(['board.home_replies is deprecated and ignored; e expands row details.']);
      const rejected = await squad(sandbox, ['config', 'set', 'board.home_replies', 'true']);
      expect(rejected.status).not.toBe(0);
      expect(rejected.body.error.message).toContain('read-only in this scope');
      expect(readFileSync(config, 'utf8')).toBe(before);
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

  it('validates board picks without writing config or changing ls tab resolution', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      for (const name of ['product', 'infra', 'quiet']) {
        expect((await squad(sandbox, ['init', name])).status).toBe(0);
      }
      const config = path.join(sandbox.globalDir, 'squad.toml');
      const source =
        '[tabs]\npin = ["tab:product"]\nhide = ["quiet"]\n[tabs.product]\nfilter = "squad = infra"\n[tabs.needs-me]\nfilter = "squad = product"\n';
      writeFileSync(config, source);
      const before = observe(sandbox);
      const listed = await squad(sandbox, ['ls', '--squad', 'product']);
      for (const names of ['all', 'product,infra', 'leads,@tab:needs-me', 'needs-me', 'quiet']) {
        const board = await squad(sandbox, ['board', '--tabs', names]);
        expect(board.status).toBe(0);
      }
      expect(
        (await squad(sandbox, ['board', '--squad', 'product', '--tabs', 'product'])).body
      ).toEqual(listed.body);
      for (const names of ['missing', '', 'product,', 'all,product']) {
        const invalid = await squad(sandbox, ['board', '--tabs', names]);
        expect(invalid.status).not.toBe(0);
        expect(invalid.body.error.code).toBe('USAGE_ERROR');
        for (const name of ['product', 'infra', '@tab:needs-me', 'all', 'leads']) {
          expect(invalid.body.error.message).toContain(name);
        }
      }
      const excluded = await squad(sandbox, [
        'board',
        '--squad',
        'product',
        '--tabs',
        '@tab:product',
      ]);
      expect(excluded.status).not.toBe(0);
      expect(excluded.body.error.code).toBe('USAGE_ERROR');
      expect((await squad(sandbox, ['ls', '--tab', '@tab:needs-me'])).status).not.toBe(0);
      expect((await squad(sandbox, ['ls', '--tab', 'needs-me'])).status).toBe(0);
      expect((await squad(sandbox, ['ls', '--tabs', 'product'])).status).not.toBe(0);
      expect(readFileSync(config, 'utf8')).toBe(source);
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
        expect(listed.body.columns.map((column: { field: string }) => column.field)).toEqual(
          crewFields
        );
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
        'tok_1',
        'tok_2',
        'tok_3',
      ]);
      expect(listed.body.columns[4].from).toBe('session.model');
      expect(listed.body.lines[1]).toEqual([
        { field: null, span: 1 },
        { field: null, span: 1 },
        { field: 'pending', span: 6, token: 'waiting' },
      ]);
      const rows = listed.body.sections[0].rows;
      expect(rows[0].fields).not.toHaveProperty('tok_1');
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
      expect(crew.body.columns.map((column: { field: string }) => column.field)).toEqual(
        crewFields
      );
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
        'members',
        'team',
        'focus',
        'notes',
        'detail',
        'wide',
      ]);
      expect((await runCli(sandbox, ['sq', 'view', 'ls'])).stdout).toContain('VIEWS 6');
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
      expect(
        (await squad(sandbox, ['view', 'ls', '--squad', 'product'])).body.effective
      ).toMatchObject({ view: 'members', source: 'default', layout: 'crew' });
      expect(
        (await squad(sandbox, ['config', 'set', 'board.view', 'team', '--squad', 'product'])).status
      ).toBe(0);
      expect(
        (await squad(sandbox, ['view', 'ls', '--squad', 'product'])).body.effective
      ).toMatchObject({ view: 'team', source: 'squad', layout: 'crew' });
      expect(await squad(sandbox, ['ls', '--squad', 'product'])).toEqual(before);
      expect(observe(sandbox).metadata).toEqual(metadata);
      expect((await squad(sandbox, ['view', 'rm', '--squad', 'product'])).status).toBe(0);
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

  it('has no conversation or annotate commands: they are core verbs or board-only', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const ids: Record<string, string> = {};
      for (const name of ['Ben', 'Sol', 'auth-fix']) ids[name] = await identity(sandbox, name);
      await squad(sandbox, ['init', 'product']);
      await squad(sandbox, ['lead', 'Sol']);
      await squad(sandbox, ['add', 'auth-fix']);
      await squad(sandbox, ['me', 'Ben']);
      const help = await runCli(sandbox, ['sq', '--help']);
      expect(help.status).toBe(0);

      // Talking and answering are core's (tmt talk, tmt answer); annotating is
      // the board's `a` key and has no command. None of them is a Squad
      // command, none is advertised, and none sends anything.
      for (const command of ['talk', 'reply', 'replies', 'annotate']) {
        const json = await squad(sandbox, [command, 'auth-fix', 'hello']);
        expect(json.status, command).toBe(2);
        const human = await runCli(sandbox, ['sq', command, 'auth-fix', 'hello']);
        expect(human.status, command).toBe(2);
        expect(human.stdout, command).toBe('');
        expect(human.stderr, command).toContain(`unrecognized subcommand '${command}'`);
        expect(help.stdout, command).not.toMatch(new RegExp(`^  ${command} `, 'm'));
      }
      const api = JSON.parse(
        (
          await runCli(sandbox, ['api'], {
            stdin: JSON.stringify({
              version: 1,
              operation: 'requests.list',
              input: { recipientId: ids['auth-fix'], limit: 50 },
            }),
          })
        ).stdout
      );
      expect(api.items).toHaveLength(0);
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
          .filter((key) => !['colors', 'focus'].includes(key))
          .sort()
      );
      expect(skill).toContain('- A row has the optional `colors` key');
      expect(skill).toContain('- A row has the optional `focus` object');
      expect(Object.keys(rows[0].focus).sort()).toEqual([
        'active',
        'focusUntilMs',
        'heldCount',
        'remainingMs',
      ]);
      expect(rows[0].focus).toEqual({
        active: false,
        focusUntilMs: 0,
        remainingMs: 0,
        heldCount: 0,
      });
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

  it('bare sq lists members with a board hint; explicit ls/status/board keep list bytes', async () => {
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
      for (const args of [
        ['sq', 'status'],
        ['sq', 'board'],
      ]) {
        const same = await runCli(sandbox, args);
        expect({ status: same.status, stdout: same.stdout }, args.join(' ')).toEqual({
          status: 0,
          stdout: ls.stdout,
        });
      }
      for (const args of [['sq'], ['squad']]) {
        const bare = await runCli(sandbox, args);
        expect(bare.status).toBe(0);
        expect(bare.stdout).toBe(`${ls.stdout}\ntmt sq board opens the board\n`);
      }
      // Both JSON shapes are pinned: --squad gives that squad's document;
      // without it, always {squads, you}, even with one squad.
      const json = await squad(sandbox, ['ls']);
      expect(Object.keys(json.body).sort()).toEqual(['squads', 'you']);
      expect(json.body.squads).toHaveLength(1);
      const one = await squad(sandbox, ['ls', '--squad', 'product']);
      expect(Object.keys(one.body).sort()).toEqual([
        'columns',
        'hidden_columns',
        'lines',
        'olderRequestsNotShown',
        'sections',
        'squad',
        'you',
      ]);
      expect(json.body.squads[0]).toEqual({ ...one.body, you: undefined });
      expect(one.body.hidden_columns).toEqual(['tok_1', 'tok_2', 'tok_3']);
      expect(one.body.columns.map((column: { field: string }) => column.field)).toEqual(crewFields);
      // The preset's grid: fixed widths, a growing task, and a link that
      // steps aside first on a narrow board; one line per row.
      expect(one.body.columns[2]).toMatchObject({ field: 'task', width: null, grow: 1, min: 20 });
      expect(one.body.columns[3]).toMatchObject({ field: 'pr_link', width: 12, priority: 6 });
      expect(one.body.lines).toEqual([crewFields.map((field) => ({ field, span: 1 }))]);
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
      // annotate: the board's `a` is a core talk in the squad room whose text is
      // tagged `[<squad> · <row>]`, to the lead by default or to the member.
      const annotate = async (to: string, row: string, note: string) => {
        const sent = await runCli(sandbox, [
          'talk',
          '--identity',
          'Ben',
          '--room',
          'squad-product',
          '--detach',
          '--json',
          '--',
          to,
          `[product · ${row}] ${note}`,
        ]);
        expect(sent.status).toBe(0);
        return { requestId: JSON.parse(sent.stdout).requestId as string };
      };
      const toLead = await annotate('Sol', 'auth-fix', 'split the job');
      expect(await prompt(toLead.requestId)).toMatchObject({
        recipientId: ids.Sol,
        roomId,
        prompt: { message: '[product · auth-fix] split the job' },
      });
      const toMember = await annotate('docs', 'docs', 'add examples');
      expect(await prompt(toMember.requestId)).toMatchObject({
        recipientId: ids.docs,
        prompt: { message: '[product · docs] add examples' },
      });
      const withdraw = async (requestId: string, who: string) => {
        const result = await runCli(sandbox, [
          'x',
          'withdraw',
          requestId,
          '--identity',
          who,
          '--reason',
          'obsolete',
          '--json',
        ]);
        expect(result.status, result.stdout).toBe(0);
        const history = await prompt(requestId);
        expect(history.final).toMatchObject({
          status: 'withdrawn',
          reason: 'obsolete',
          withdrawnAtMs: expect.any(Number),
        });
        expect(history.final).not.toHaveProperty('submittedAtMs');
        return history;
      };
      const obsolete = await annotate('Sol', 'auth-fix', 'obsolete annotation');
      const withdrawn = await withdraw(obsolete.requestId, 'Ben');
      expect(withdrawn.prompt.message).toBe('[product · auth-fix] obsolete annotation');
      expect((await roomRequests()).length).toBe(4);

      // The marker is derived from request state on every read.
      const marker = async () => {
        const rows = (await squad(sandbox, ['ls', '--squad', 'product'])).body.sections[0].rows;
        return Object.fromEntries(
          rows.map((row: { name: string; annotation: unknown }) => [row.name, row.annotation])
        );
      };
      expect(await marker()).toEqual({
        'auth-fix': { requestId: toLead.requestId, to: 'Sol', text: 'split the job' },
        docs: { requestId: toMember.requestId, to: 'docs', text: 'add examples' },
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
      const leadView = await incoming('Sol', toLead.requestId);
      const answered = await runCli(sandbox, [
        'reply',
        toLead.requestId,
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
      for (const question of ['approve the plan?', 'which database?', 'obsolete question']) {
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
      const obsoleteAsk = asks.pop()!;
      const withdrawnAsk = await withdraw(obsoleteAsk, 'auth-fix');
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
      const refused = await board(obsoleteAsk, 'cannot answer withdrawal');
      expect(refused.status, refused.stdout).toBe(5);
      expect(JSON.parse(refused.stdout).error.code).toBe('REQUEST_WITHDRAWN');
      expect(await prompt(obsoleteAsk)).toEqual(withdrawnAsk);
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

      expect(await prompt(obsolete.requestId)).toEqual(withdrawn);

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
      const sinceMs = Date.now() - 180_000;
      const evidence = JSON.stringify({
        version: 1,
        pane: '%41',
        pid: 123,
        sinceMs,
        expiresMs: sinceMs + 300_000,
      });
      writeFileSync(lease, evidence);
      const runningText = await runCli(sandbox, ['squad', 'cron', 'clock']);
      expect(runningText.status).toBe(0);
      expect(runningText.stdout).toContain('3m ago');
      const running = await squad(sandbox, ['cron', 'clock']);
      expect(running).toMatchObject({ status: 0, body: { clock: { state: 'running', sinceMs } } });
      expect(readFileSync(lease, 'utf8')).toBe(evidence);
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

describe('Squad checklist native commands', () => {
  const checklistId = '55555555-5555-4555-8555-555555555555';
  const itemId = '66666666-6666-4666-8666-666666666666';
  const otherId = '77777777-7777-4777-8777-777777777777';

  async function fixture(sandbox: Sandbox) {
    installSquad(sandbox);
    await identity(sandbox, 'Ben');
    const worker = await identity(sandbox, 'worker');
    expect((await squad(sandbox, ['init', 'product', '--me', 'Ben'])).status).toBe(0);
    expect((await squad(sandbox, ['add', 'worker'])).status).toBe(0);
    const room = parseWholeStdout(
      await runCli(sandbox, ['room', 'show', 'squad-product', '--json'])
    ).room as { id: string };
    const root = parseWholeStdout(
      await runCli(sandbox, ['api'], {
        stdin: JSON.stringify({ version: 1, operation: 'storage.root', input: {} }),
      })
    ).dataRoot as string;
    const directory = path.join(root, 'squad', 'checklist', room.id);
    const selected = ['--room', room.id, '--checklist', checklistId, '--item', itemId];
    return { worker, roomId: room.id, root, directory, selected };
  }

  // Observe metadata, dispatch/attention/request/hook state without invoking a product read.
  function unrelatedState(sandbox: Sandbox) {
    const db = new Database(sandbox.database, { readonly: true });
    try {
      const tables = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .all() as { name: string }[];
      return {
        tables: Object.fromEntries(
          tables
            .filter(({ name }) => /request|dispatch|notification|hook|identity_metadata/.test(name))
            .map(({ name }) => [
              name,
              db
                .prepare(`SELECT * FROM "${name}"`)
                .all()
                .sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b))),
            ])
        ),
        squadConfig: readFileSync(path.join(sandbox.globalDir, 'squad.toml')),
        coreConfig: existsSync(sandbox.globalConfig) ? readFileSync(sandbox.globalConfig) : null,
        localConfig: existsSync(sandbox.localConfig) ? readFileSync(sandbox.localConfig) : null,
      };
    } finally {
      db.close();
    }
  }

  it('binds real dispatch, alias, streams, revisions, full inventory, tombstones and inert content', async () => {
    await withSandbox(async (sandbox) => {
      const f = await fixture(sandbox);
      expect((await squad(sandbox, ['set', 'worker', 'state=busy', 'pending=review'])).status).toBe(
        0
      );
      const before = unrelatedState(sandbox);
      const cli = (words: string[]) => squad(sandbox, ['checklist', ...words]);
      const list = (filters: string[] = []) => cli(['ls', '--room', f.roomId, ...filters]);
      const mutation = (action: string, revision: number, options: string[] = []) =>
        cli([action, ...f.selected, '--expect-revision', String(revision), ...options]);
      const absent = await list();
      expect(absent).toMatchObject({
        status: 0,
        stderr: '',
        body: {
          action: 'list',
          totalCount: 0,
          matchedCount: 0,
          current: { checklistId: null, inventoryRevision: null, items: [], deletion: null },
        },
      });
      expect(existsSync(f.directory)).toBe(false);
      const body = `literal body\n\t$(touch ${path.join(sandbox.root, 'executed')}) !`;
      const created = await cli([
        'create',
        'Manually authored',
        ...f.selected,
        '--expect-inventory',
        'absent',
        '--body',
        body,
        '--reference',
        'https://example.invalid/inert',
      ]);
      expect(created).toMatchObject({
        status: 0,
        stderr: '',
        body: {
          action: 'create',
          itemId,
          itemRevision: 1,
          changed: true,
          current: {
            inventoryRevision: 1,
            items: [
              {
                id: itemId,
                revision: 1,
                title: 'Manually authored',
                body,
                reference: 'https://example.invalid/inert',
                assignee: null,
                completion: 'open',
                archived: false,
              },
            ],
          },
        },
      });
      const file = path.join(f.directory, 'items.json');
      const stored = () => JSON.parse(readFileSync(file, 'utf8'));
      expect(stored().items[0]).toMatchObject({
        id: itemId,
        revision: 1,
        title: 'Manually authored',
        body,
        reference: 'https://example.invalid/inert',
        assignee: null,
        completion: 'open',
        archived: false,
      });
      expect(statSync(file).mode & 0o777).toBe(0o600);
      expect(existsSync(path.join(sandbox.root, 'executed'))).toBe(false);
      const alias = await runCli(sandbox, [
        'sq',
        'checklist',
        'list',
        '--room',
        f.roomId,
        '--json',
      ]);
      expect(alias.status).toBe(0);
      expect(alias.stderr).toBe('');
      expect(parseWholeStdout(alias)).toEqual((await list()).body);
      // Seven actual content revisions provide the frozen revision7 -> revision8 example.
      for (let revision = 1; revision < 7; revision += 1) {
        expect(
          (await mutation('edit', revision, ['--title', `Draft ${revision}`])).body.itemRevision
        ).toBe(revision + 1);
      }
      const complete = await mutation('complete', 7);
      expect(complete).toMatchObject({
        status: 0,
        body: { itemRevision: 8, current: { inventoryRevision: 1 } },
      });
      const bytes = readFileSync(file);
      const conflict = await mutation('edit', 7, ['--title', 'Retained draft']);
      expect(conflict).toMatchObject({
        status: 1,
        stderr: '',
        body: {
          error: {
            code: 'CHECKLIST_CONFLICT',
            current: {
              items: [{ id: itemId, revision: 8, title: 'Draft 6', completion: 'complete' }],
            },
          },
        },
      });
      expect(readFileSync(file)).toEqual(bytes);
      const human = await runCli(sandbox, [
        'squad',
        'checklist',
        'complete',
        ...f.selected,
        '--expect-revision',
        '7',
      ]);
      expect(human.status).toBe(1);
      expect(human.stdout).toBe('');
      expect(human.stderr).toContain('error:');
      expect(human.stderr).toContain(`${itemId} revision 8`);
      expect((await mutation('complete', 8)).body).toMatchObject({
        changed: false,
        itemRevision: 8,
      });
      expect(readFileSync(file)).toEqual(bytes);
      const second = await cli([
        'create',
        'Independent item',
        '--room',
        f.roomId,
        '--checklist',
        checklistId,
        '--item',
        otherId,
        '--expect-inventory',
        '1',
      ]);
      expect(second.body).toMatchObject({ itemRevision: 1, current: { inventoryRevision: 2 } });
      expect((await mutation('archive', 8)).body.itemRevision).toBe(9);
      const filtered = await list(['--completion', 'complete']);
      expect(filtered.body).toMatchObject({ totalCount: 2, matchedCount: 0 });
      const reorder = (inventory: number, order: string[]) =>
        cli([
          'reorder',
          '--room',
          f.roomId,
          '--checklist',
          checklistId,
          '--expect-inventory',
          String(inventory),
          '--order',
          JSON.stringify(order),
        ]);
      expect((await reorder(1, [otherId, itemId])).body.error.code).toBe('CHECKLIST_CONFLICT');
      expect((await reorder(2, [otherId])).body.error.code).toBe('CHECKLIST_INPUT_INVALID');
      expect((await reorder(2, [otherId, itemId])).body.current).toMatchObject({
        inventoryRevision: 3,
        items: [
          { id: otherId, revision: 1 },
          { id: itemId, revision: 9, archived: true },
        ],
      });
      expect((await mutation('restore', 9)).body.current.items[1]).toMatchObject({
        id: itemId,
        revision: 10,
        completion: 'complete',
        archived: false,
      });
      expect((await mutation('assign', 10, ['--assignee', f.worker])).body.itemRevision).toBe(11);
      expect(
        (await runCli(sandbox, ['room', 'leave', f.roomId, '--identity', f.worker, '--json']))
          .status
      ).toBe(0);
      expect((await list()).body.current.items[1].assignee).toEqual({
        id: f.worker,
        label: 'worker',
        available: false,
      });
      expect((await mutation('unassign', 11)).body.itemRevision).toBe(12);
      const deletion = await mutation('delete', 12, [
        '--expect-inventory',
        '3',
        '--confirm-item',
        itemId,
        '--confirm-revision',
        '12',
      ]);
      expect(deletion).toMatchObject({
        status: 0,
        body: {
          itemRevision: 13,
          current: { inventoryRevision: 4, deletion: { itemId, deletionRevision: 13 } },
        },
      });
      expect(stored().deleted).toEqual([
        { roomId: f.roomId, checklistId, itemId, deletionRevision: 13 },
      ]);
      expect(stored().items).toHaveLength(1);
      expect(readFileSync(file, 'utf8')).not.toContain('Draft 6');
      expect(readFileSync(file, 'utf8')).not.toContain(body);
      const deleted = await cli(['show', ...f.selected]);
      expect(deleted).toMatchObject({
        status: 1,
        stderr: '',
        body: {
          error: {
            code: 'CHECKLIST_DELETED',
            current: { deletion: { itemId, deletionRevision: 13 } },
          },
        },
      });
      expect(
        (await cli(['create', 'Reuse denied', ...f.selected, '--expect-inventory', '4'])).body.error
          .code
      ).toBe('CHECKLIST_DELETED');
      const empty = await cli([
        'delete',
        '--room',
        f.roomId,
        '--checklist',
        checklistId,
        '--item',
        otherId,
        '--expect-revision',
        '1',
        '--expect-inventory',
        '4',
        '--confirm-item',
        otherId,
        '--confirm-revision',
        '1',
      ]);
      expect(empty.status).toBe(0);
      expect((await list()).body).toMatchObject({
        totalCount: 0,
        matchedCount: 0,
        current: { checklistId, inventoryRevision: 5, items: [] },
      });
      // Room departure was intentional; all checklist operations themselves remain inert.
      const after = unrelatedState(sandbox);
      expect(after).toEqual(before);
      expect(existsSync(path.join(f.directory, 'items.tmp'))).toBe(false);
    });
  }, 60_000);

  it('rejects operation inputs with exit1 and grammar inputs with exit2 without changing bytes', async () => {
    await withSandbox(async (sandbox) => {
      const f = await fixture(sandbox);
      const cli = (words: string[]) => squad(sandbox, ['checklist', ...words]);
      expect(
        (await cli(['create', 'Title', ...f.selected, '--expect-inventory', 'absent'])).status
      ).toBe(0);
      const file = path.join(f.directory, 'items.json');
      const bytes = readFileSync(file);
      const before = unrelatedState(sandbox);
      const selected = [...f.selected, '--expect-revision', '1'];
      const invalid = [
        ['ls', '--room', 'product'],
        ['show', '--room', f.roomId, '--checklist', 'named', '--item', itemId],
        ['complete', ...f.selected, '--expect-revision', '0'],
        ['complete', ...f.selected, '--expect-revision', '-1'],
        ['complete', ...f.selected, '--expect-revision', '18446744073709551616'],
        ['edit', ...selected, '--title', ' '],
        ['edit', ...selected, '--body', '\u001b'],
        ['edit', ...selected, '--reference', 'file:///tmp/secret'],
        ['edit', ...selected, '--reference', ''],
        ['assign', ...selected, '--assignee', 'worker'],
        [
          'delete',
          ...selected,
          '--expect-inventory',
          '1',
          '--confirm-item',
          otherId,
          '--confirm-revision',
          '1',
        ],
        [
          'delete',
          ...selected,
          '--expect-inventory',
          '1',
          '--confirm-item',
          itemId,
          '--confirm-revision',
          '2',
        ],
        [
          'reorder',
          '--room',
          f.roomId,
          '--checklist',
          checklistId,
          '--expect-inventory',
          '1',
          '--order',
          '[1]',
        ],
        [
          'reorder',
          '--room',
          f.roomId,
          '--checklist',
          checklistId,
          '--expect-inventory',
          '1',
          '--order',
          JSON.stringify([itemId, itemId]),
        ],
      ];
      for (const words of invalid) {
        const result = await cli(words);
        expect(result, JSON.stringify(words)).toMatchObject({
          status: 1,
          stderr: '',
          body: { error: { code: 'CHECKLIST_INPUT_INVALID' } },
        });
        expect(readFileSync(file)).toEqual(bytes);
      }
      const usage = [
        [],
        ['unknown'],
        ['ls'],
        ['ls', '--room', f.roomId, '--completion', 'closed'],
        ['ls', '--room', f.roomId, '--identity', f.worker],
        ['edit', ...selected],
        ['edit', ...selected, '--body', 'x', '--clear-body'],
        ['edit', ...selected, '--reference', 'https://example.org', '--clear-reference'],
        ['delete', ...selected, '--expect-inventory', '1', '--yes'],
      ];
      for (const words of usage) {
        expect(await cli(words), JSON.stringify(words)).toMatchObject({
          status: 2,
          stderr: '',
          body: { error: { code: 'USAGE_ERROR' } },
        });
        expect(readFileSync(file)).toEqual(bytes);
      }
      expect(
        (await cli(['show', '--room', f.roomId, '--checklist', checklistId, '--item', otherId]))
          .body.error.code
      ).toBe('CHECKLIST_NOT_FOUND');
      expect(unrelatedState(sandbox)).toEqual(before);
      expect(existsSync(path.join(f.directory, 'items.tmp'))).toBe(false);
      const help = await runCli(sandbox, ['sq', 'checklist', '--help']);
      expect(help.status).toBe(0);
      expect(help.stderr).toBe('');
      expect(help.stdout).toMatch(/^  ls\s/m);
      expect(help.stdout).not.toMatch(/^  list\s/m);
      expect(help.stdout).not.toContain('--identity');
      const offline = {
        ...sandbox,
        cli: { executable: squadExecutable, args: [] },
        env: { ...sandbox.env, TMT_EXECUTABLE: 'relative-invalid-core' },
      };
      for (const words of [
        ['checklist', '--help'],
        ['help', 'checklist', 'delete'],
        ['checklist', 'create', '--help'],
      ]) {
        const shown = await runCli(offline, words);
        expect(shown.status).toBe(0);
        expect(shown.stderr).toBe('');
        expect(shown.stdout).toContain('Usage: tmt squad checklist');
      }
      const completion = await runCli(offline, ['__complete', '--', 'checklist', '']);
      expect(completion.status).toBe(0);
      expect(completion.stderr).toBe('');
      expect(completion.stdout.split('\n')).toContain('ls');
      expect(completion.stdout.split('\n')).not.toContain('list');
      const invalidBeforeDiscovery = await runCli(offline, [
        'checklist',
        'ls',
        '--room',
        'name',
        '--json',
      ]);
      expect(invalidBeforeDiscovery.status).toBe(1);
      expect(parseWholeStdout(invalidBeforeDiscovery)).toMatchObject({
        error: { code: 'CHECKLIST_INPUT_INVALID' },
      });
    });
  }, 60_000);

  it('keeps two squads, renamed rooms, readonly orphans and corrupt reads distinct', async () => {
    await withSandbox(async (sandbox) => {
      const f = await fixture(sandbox);
      const cli = (words: string[]) => squad(sandbox, ['checklist', ...words]);
      expect(
        (await cli(['create', 'Retained', ...f.selected, '--expect-inventory', 'absent'])).status
      ).toBe(0);
      expect((await squad(sandbox, ['init', 'other'])).status).toBe(0);
      const otherRoom = (
        parseWholeStdout(await runCli(sandbox, ['room', 'show', 'squad-other', '--json'])).room as {
          id: string;
        }
      ).id;
      expect((await cli(['ls', '--room', otherRoom])).body.current).toMatchObject({
        checklistId: null,
        items: [],
      });
      expect(existsSync(path.join(f.root, 'squad', 'checklist', otherRoom))).toBe(false);
      const file = path.join(f.directory, 'items.json');
      const original = readFileSync(file);
      const corrupt = Buffer.from('{"version":999}\n');
      writeFileSync(file, corrupt);
      const failed = await cli(['ls', '--room', f.roomId]);
      expect(failed).toMatchObject({
        status: 1,
        stderr: '',
        body: { error: { code: 'CHECKLIST_STORAGE_ERROR' } },
      });
      expect(failed.body.error).not.toHaveProperty('current');
      expect(readFileSync(file)).toEqual(corrupt);
      writeFileSync(file, original);
      const lock = path.join(f.directory, 'items.lock');
      const lockBytes = readFileSync(lock);
      unlinkSync(lock);
      expect((await cli(['ls', '--room', f.roomId])).body.error.code).toBe(
        'CHECKLIST_STORAGE_ERROR'
      );
      expect(existsSync(lock)).toBe(false);
      expect(readFileSync(file)).toEqual(original);
      writeFileSync(lock, lockBytes, { mode: 0o600 });
      const beforeRename = parseWholeStdout(
        await runCli(sandbox, ['api'], {
          stdin: JSON.stringify({
            version: 1,
            operation: 'rooms.roster',
            input: { room: f.roomId },
          }),
        })
      );
      const renamed = await runCli(sandbox, ['api'], {
        stdin: JSON.stringify({
          version: 1,
          operation: 'rooms.write',
          identity: 'Ben',
          input: {
            roomId: f.roomId,
            room: {
              expectedRevision: (beforeRename.room as { revision: number }).revision,
              name: 'squad-renamed',
              memberIds: [f.worker],
            },
          },
        }),
      });
      expect(renamed.status, renamed.stdout).toBe(0);
      expect((await cli(['ls', '--room', f.roomId])).body.current.room).toMatchObject({
        id: f.roomId,
        name: 'renamed',
        available: true,
      });
      expect(readFileSync(file)).toEqual(original);
      expect((await runCli(sandbox, ['room', 'rm', f.roomId, '--json'])).status).toBe(0);
      const orphan = await cli(['ls', '--room', f.roomId]);
      expect(orphan).toMatchObject({
        status: 0,
        body: {
          current: {
            room: { id: f.roomId, name: null, available: false },
            checklistId,
            items: [{ id: itemId, title: 'Retained' }],
          },
        },
      });
      expect(
        (await cli(['complete', ...f.selected, '--expect-revision', '1'])).body.error.code
      ).toBe('CHECKLIST_ROOM_UNAVAILABLE');
      expect((await squad(sandbox, ['init', 'renamed'])).status).toBe(0);
      const successor = (
        parseWholeStdout(await runCli(sandbox, ['room', 'show', 'squad-renamed', '--json']))
          .room as { id: string }
      ).id;
      expect(successor).not.toBe(f.roomId);
      expect((await cli(['ls', '--room', successor])).body.current.checklistId).toBeNull();
      expect(readFileSync(file)).toEqual(original);
    });
  }, 60_000);
});

describe('Squad focus policies', () => {
  it('admits owner and lead, refuses other callers, preserves CAS and projects shared row policies', async () => {
    await withSandbox(async (sandbox) => {
      installSquad(sandbox);
      const owner = await identity(sandbox, 'Owner');
      const lead = await identity(sandbox, 'Lead');
      const worker = await identity(sandbox, 'worker');
      expect((await squad(sandbox, ['init', 'product', '--me', 'Owner'])).status).toBe(0);
      expect((await squad(sandbox, ['lead', 'Lead'])).status).toBe(0);
      expect(
        (await runCli(sandbox, ['room', 'join', 'squad-product', '--identity', 'worker', '--json']))
          .status
      ).toBe(0);
      const shim = path.join(sandbox.root, 'focus-core');
      const callerFile = path.join(sandbox.root, 'focus-caller.json');
      const callsFile = path.join(sandbox.root, 'focus-calls.jsonl');
      const modeFile = path.join(sandbox.root, 'focus-mode');
      // Only caller evidence and injected Core refusals are synthetic. All ordinary
      // API operations reach the sandbox native CLI and its real policy transaction.
      await writeExecutable(
        shim,
        `#!/usr/bin/env python3
import json,sys,subprocess,pathlib
root=pathlib.Path(__file__).parent
args=sys.argv[1:]
if args[0]=='whoami':
 value=json.loads((root/'focus-caller.json').read_text())
 print(json.dumps(value)); sys.exit(1 if 'error' in value else 0)
body=sys.stdin.buffer.read() if args[0]=='api' else None
if body:
 q=json.loads(body)
 if q['operation'].startswith('focus.policy.'):
  with open(root/'focus-calls.jsonl','a') as f: f.write(json.dumps(q)+chr(10))
  mode=(root/'focus-mode').read_text()
  if mode=='old' or (mode=='conflict' and q['operation']!='focus.policy.show'):
   code='API_INPUT_INVALID' if mode=='old' else 'FOCUS_REVISION_CONFLICT'
   print(json.dumps({'error':{'code':code,'message':'injected refusal'}})); sys.exit(1)
result=subprocess.run([${JSON.stringify(sandbox.cli.executable)}]+args,input=body)
sys.exit(result.returncode)
`,
        0o700
      );
      const asCaller = (id: string | null, name = 'caller') =>
        writeFileSync(
          callerFile,
          JSON.stringify(id ? { bound: true, id, name, lifetime: 'saved' } : { bound: false })
        );
      const focus = async (args: string[]) => {
        const result = await runCli(
          {
            ...sandbox,
            cli: { executable: squadExecutable, args: [] },
            env: { ...sandbox.env, TMT_EXECUTABLE: shim },
          },
          args.concat('--json')
        );
        return { status: result.status, stderr: result.stderr, body: JSON.parse(result.stdout) };
      };
      const calls = () =>
        existsSync(callsFile)
          ? readFileSync(callsFile, 'utf8')
              .trim()
              .split('\n')
              .filter(Boolean)
              .map((line) => JSON.parse(line))
          : [];
      writeFileSync(modeFile, 'normal');
      const forwarded = await runCli({ ...sandbox, cli: { executable: shim, args: [] } }, [
        'config',
        'show',
        '--json',
      ]);
      expect(forwarded.status, forwarded.stderr).toBe(0);
      expect(JSON.parse(forwarded.stdout)).toHaveProperty('resolved');
      asCaller(owner, 'Owner');
      const set = await focus(['focus', 'worker', '1h30m', '--squad', 'product']);
      expect(set).toMatchObject({
        status: 0,
        stderr: '',
        body: {
          identityId: worker,
          revision: 1,
          active: true,
          ownerIdentityId: owner,
          setterIdentityId: owner,
          heldCount: 0,
        },
      });
      expect(set.body.remainingMs).toBeGreaterThan(5_390_000);
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          db
            .prepare(
              'SELECT revision, owner_identity_id, setter_identity_id FROM focus_policies WHERE identity_id=?'
            )
            .get(worker)
        ).toEqual({ revision: 1, owner_identity_id: owner, setter_identity_id: owner });
      } finally {
        db.close();
      }
      expect((await focus(['focus', 'worker'])).body).toMatchObject({
        revision: 1,
        active: true,
        heldCount: 0,
      });
      asCaller(lead, 'Lead');
      const replaced = await focus(['focus', 'worker', '30m']);
      expect(replaced.body).toMatchObject({
        revision: 2,
        ownerIdentityId: owner,
        setterIdentityId: lead,
      });
      const request = calls().at(-1);
      expect(request).toEqual({
        version: 1,
        operation: 'focus.policy.set',
        input: {
          identityId: worker,
          ownerIdentityId: owner,
          setterIdentityId: lead,
          expectedRevision: 1,
          untilMs: replaced.body.focusUntilMs,
        },
      });
      asCaller(worker, 'worker');
      const before = calls().length;
      expect((await focus(['focus', 'worker', '30m'])).body.error.code).toBe(
        'SQUAD_FOCUS_PERMISSION_DENIED'
      );
      expect(calls().length).toBe(before);
      writeFileSync(
        callerFile,
        JSON.stringify({ error: { code: 'CALLER_IDENTITY_AMBIGUOUS', message: 'ambiguous' } })
      );
      expect((await focus(['focus', 'worker', 'off'])).body.error.code).toBe(
        'SQUAD_FOCUS_PERMISSION_DENIED'
      );
      const retired = await identity(sandbox, 'Retired');
      expect((await runCli(sandbox, ['rm', 'Retired', '--force', '--json'])).status).toBe(0);
      asCaller(retired, 'Retired');
      expect((await focus(['focus', 'worker', '30m'])).body.error.code).toBe(
        'SQUAD_FOCUS_PERMISSION_DENIED'
      );
      asCaller(owner, 'Owner');
      writeFileSync(modeFile, 'conflict');
      const conflict = await focus(['focus', 'worker', 'off']);
      expect(conflict.body.error.code).toBe('FOCUS_REVISION_CONFLICT');
      expect(conflict.body.error.message).toContain('Reload and retry');
      expect(
        calls()
          .slice(-2)
          .map((call) => call.operation)
      ).toEqual(['focus.policy.show', 'focus.policy.clear']);
      writeFileSync(modeFile, 'normal');
      const held = await runCli(sandbox, [
        'talk',
        'worker',
        'Review after focus',
        '--identity',
        'Lead',
        '--detach',
        '--json',
      ]);
      expect(held.status).toBe(0);
      expect(JSON.parse(held.stdout)).toMatchObject({ focus: true, notification: 'held' });
      const inventory = await runCli(sandbox, ['api'], {
        stdin: JSON.stringify({
          version: 1,
          operation: 'focus.checklist.read',
          input: { identityId: worker },
        }),
      });
      expect(inventory.status).toBe(0);
      expect(JSON.parse(inventory.stdout).total).toBe(1);
      // A second room and repeated source sections still share one read.
      expect((await squad(sandbox, ['init', 'other'])).status).toBe(0);
      expect(
        (await runCli(sandbox, ['room', 'join', 'squad-other', '--identity', 'worker', '--json']))
          .status
      ).toBe(0);
      const readStart = calls().length;
      const listed = await focus(['ls']);
      expect(listed).toMatchObject({ status: 0, stderr: '' });
      expect(calls().slice(readStart)).toHaveLength(1);
      expect(new Set(calls().at(-1).input.identities).size).toBe(
        calls().at(-1).input.identities.length
      );
      const active = listed.body.squads
        .flatMap((doc: { sections: { rows: { id: string; focus?: unknown }[] }[] }) =>
          doc.sections.flatMap((section) => section.rows)
        )
        .filter((row: { id: string }) => row.id === worker);
      expect(active).toHaveLength(2);
      expect(active[0].focus).toEqual(active[1].focus);
      expect(active[0].focus).toMatchObject({
        active: true,
        focusUntilMs: replaced.body.focusUntilMs,
        heldCount: 1,
      });
      const cleared = await focus(['focus', 'worker', 'off', '--squad', 'product']);
      expect(cleared.body).toMatchObject({
        revision: 3,
        active: false,
        focusUntilMs: 0,
        remainingMs: 0,
      });
      const lastSet = await focus(['focus', 'worker', '1s', '--squad', 'product']);
      expect(lastSet.body.revision).toBe(4);
      // Seed an already expired persisted window to prove native observation,
      // without a wall-clock sleep. Renderer expiry uses an injected clock below.
      const writable = new Database(sandbox.database);
      try {
        writable.prepare('UPDATE focus_policies SET until_ms=1 WHERE identity_id=?').run(worker);
      } finally {
        writable.close();
      }
      expect((await focus(['focus', 'worker', '--squad', 'product'])).body).toMatchObject({
        revision: 4,
        active: false,
        remainingMs: 0,
      });
      writeFileSync(modeFile, 'old');
      expect((await focus(['focus', 'worker', '30m', '--squad', 'product'])).body.error.code).toBe(
        'API_INPUT_INVALID'
      );
      const degraded = await focus(['ls', '--squad', 'product']);
      expect(degraded).toMatchObject({ status: 0, stderr: '' });
      expect(degraded.body.sections[0].rows[0].focus).toBeUndefined();
      for (const duration of ['0s', '-1m', '24h1s']) {
        expect(
          (await focus(['focus', 'worker', duration, '--squad', 'product'])).body.error.code
        ).toBe('SQUAD_FOCUS_DURATION_INVALID');
      }
      const help = await runCli(sandbox, ['sq', 'focus', '--help']);
      expect(help.status).toBe(0);
      expect(help.stdout).toContain('24h');
      expect(help.stdout).not.toContain('--every');
    });
  });
});
