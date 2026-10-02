import { writeExecutable } from '../support/executable-fixture.mjs';
import Database from 'better-sqlite3';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import { randomUUID } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { installTmuxTrace, type TmuxTrace } from './tmux-trace.js';
import { waitForFileContent } from './wait-for-file.js';

// Real product CLI/router, real isolated tmux, deterministic native protocol peer.
// This proves routing and durable request behavior, not provider/model continuity.
const localMock = fileURLToPath(
  new URL('../../../rust/target/debug/examples/codex-channel-fixture', import.meta.url)
);
const mock = fs.existsSync('/opt/tmt-tests/codex-channel-fixture')
  ? '/opt/tmt-tests/codex-channel-fixture'
  : localMock;
const quote = (text: string) => `'${text.replaceAll("'", "'\\''")}'`;
interface Session {
  pane: string;
  log: string;
  status: string;
}
interface Event {
  event: string;
  content?: string;
  line?: string;
  pid?: number;
  [key: string]: unknown;
}
interface RecordFile {
  bindingId: string;
  generation: string;
  launchOwner: { pid: number; start: string };
  foreground: { state: string; process?: { pid: number; start: string } };
  ready: { server: { pid: number; start: string }; port: number; thread: string };
}
function start(
  f: E2EFixture,
  name: string,
  channel: boolean | 'default' = 'default',
  extra: Record<string, string> = {},
  existingPane?: string,
  resume = false
): Session {
  const pane = existingPane ?? f.createShellPane(`codex-${name}`).pane;
  const run = randomUUID();
  const log = path.join(f.root, `${name}-${run}.log`);
  const status = `${log}.status`;
  const executable = path.join(f.wrapperDir, 'codex');
  if (!fs.existsSync(executable)) {
    if (extra.MOCK_HOOK_MODEL) {
      // This scenario needs real Codex-named app-server ancestry for its hooks.
      writeExecutable(executable, fs.readFileSync(mock));
    } else {
      writeExecutable(executable, `#!/bin/sh\nexec ${quote(mock)} "$@"\n`);
    }
  }
  const home = path.join(f.root, `home-${name}-${run}`);
  fs.mkdirSync(home);
  const env = {
    HOME: home,
    CODEX_HOME: home,
    MOCK_CHANNEL_LOG: log,
    MOCK_AUTOREPLY: '1',
    MOCK_PEER: JSON.stringify(f.executables.peer),
    ...extra,
  };
  const command = [
    'env',
    ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
    f.executables.cli.executable,
    ...f.executables.cli.args,
    resume ? 'resume' : 'run',
    ...(channel === true ? ['--channel'] : channel === false ? ['--no-channel'] : []),
    ...(resume ? [name] : ['-s', name, executable]),
  ]
    .map(quote)
    .join(' ');
  f.tmux(['send-keys', '-t', pane, '-l', `${command}; printf '%s' "$?" > ${quote(status)}`]);
  f.tmux(['send-keys', '-t', pane, 'Enter']);
  return { pane, log, status };
}
function events(s: Session, name: string): Event[] {
  return fs.existsSync(s.log)
    ? fs
        .readFileSync(s.log, 'utf8')
        .split('\n')
        .filter(Boolean)
        .map((line) => JSON.parse(line) as Event)
        .filter((e) => e.event === name)
    : [];
}
async function ready(f: E2EFixture, s: Session) {
  await f.waitFor(
    () => events(s, 'started').length === 1,
    30000,
    'mock foreground started after bounded channel startup'
  );
  const deadline = Date.now() + 15000;
  for (;;) {
    const result = await f.runJsonCli<{ sessionState: string }>(['whoami'], { pane: s.pane });
    if (result.json?.sessionState === 'running') break;
    if (Date.now() >= deadline)
      throw new Error(`Foreground not admitted: ${result.stdout}${result.stderr}`);
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}
async function quit(s: Session) {
  fs.writeFileSync(`${s.log}.quit`, '');
  expect(await waitForFileContent(s.status, { description: 'launcher reaped foreground' })).toBe(
    '0'
  );
}
function writes(trace: TmuxTrace, pane: string) {
  const target = new RegExp(`${pane}(?![0-9])`);
  return trace
    .invocations()
    .filter(
      (line) => /^(send-keys|paste-buffer|load-buffer|set-buffer)\t/.test(line) && target.test(line)
    );
}
function records(f: E2EFixture) {
  const directory = path.join(f.globalDir, 'channels', 'codex');
  return fs.existsSync(directory)
    ? fs
        .readdirSync(directory)
        .filter((file) => file.endsWith('.json'))
        .map((file) => ({
          file: path.join(directory, file),
          record: JSON.parse(fs.readFileSync(path.join(directory, file), 'utf8')) as RecordFile,
        }))
    : [];
}
function sql<T>(f: E2EFixture, run: (db: Database.Database) => T): T {
  const db = new Database(path.join(f.globalDir, 'tmux-team.db'));
  try {
    return run(db);
  } finally {
    db.close();
  }
}
const talk = (f: E2EFixture, target: string, message: string, pane?: string) =>
  f.runJsonCli<Record<string, unknown>>(
    ['talk', target, message, '--detach'],
    pane ? { pane } : {}
  );

function gone(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return false;
  } catch {
    return true;
  }
}

describe('Codex native channel product routing', { concurrent: false }, () => {
  it('first opt-in channel hook remembers its session and model without replacing foreground admission', async () => {
    await withE2EFixture(async (f) => {
      const worker = start(f, 'HookModel', true, { MOCK_HOOK_MODEL: 'fixture-hook-model' });
      await ready(f, worker);
      const [{ record }] = records(f);
      const foreground = record.foreground.process!;
      const trace = installTmuxTrace(f);
      expect((await talk(f, 'HookModel', 'observe first hook model')).code).toBe(0);
      await f.waitFor(() => events(worker, 'hook').length === 1, 10000, 'real first enrolled hook');
      const result = events(worker, 'hook')[0];
      expect(result.ok).toBe(true);
      expect(result.stderr).toBe('');
      expect(JSON.parse(result.stdout as string).hookSpecificOutput.additionalContext).toContain(
        'HookModel'
      );
      const saved = sql(f, (db) =>
        db
          .prepare(
            `SELECT b.runtime_pid, b.runtime_start_identity, b.observed_provider_session_id, p.provider_session_id, p.driver_state FROM bindings b JOIN identity_session_preferences p ON b.identity_id=p.identity_id JOIN identities i ON i.id=b.identity_id WHERE i.name='HookModel'`
          )
          .get()
      ) as {
        runtime_pid: number;
        runtime_start_identity: string;
        observed_provider_session_id: string;
        provider_session_id: string;
        driver_state: string;
      };
      expect(saved.runtime_pid).toBe(foreground.pid);
      expect(saved.runtime_start_identity).toBe(foreground.start);
      expect(saved.runtime_pid).not.toBe(record.ready.server.pid);
      expect(saved.observed_provider_session_id).toBe(record.ready.thread);
      expect(saved.provider_session_id).toBe(record.ready.thread);
      expect(JSON.parse(saved.driver_state).model).toBe('fixture-hook-model');
      expect(records(f)[0].record.foreground.process).toEqual(foreground);
      expect(events(worker, 'queue')).toHaveLength(1);
      expect(events(worker, 'paste')).toEqual([]);
      expect(writes(trace, worker.pane)).toEqual([]);
      await quit(worker);
      await f.waitFor(
        () => gone(record.ready.server.pid) && records(f).length === 0,
        5000,
        'owned app-server and enrollment gone after first hook'
      );
    });
  });

  it('default enrolls on a qualified build and cleans the owned endpoint on Ctrl-C', async () => {
    await withE2EFixture(async (f) => {
      const worker = start(f, 'Default');
      await ready(f, worker);
      const [{ file, record }] = records(f);
      const trace = installTmuxTrace(f);
      expect((await talk(f, 'Default', 'default native')).code).toBe(0);
      expect(events(worker, 'queue')).toHaveLength(1);
      expect(events(worker, 'paste')).toEqual([]);
      expect(writes(trace, worker.pane)).toEqual([]);
      f.tmux(['send-keys', '-t', worker.pane, 'C-c']);
      expect(
        await waitForFileContent(worker.status, { description: 'Ctrl-C reaped foreground' })
      ).toBe('130');
      await f.waitFor(
        () =>
          gone(record.ready.server.pid) &&
          !fs.existsSync(file) &&
          !fs.existsSync(path.join(path.dirname(file), record.generation)),
        10000,
        'Ctrl-C endpoint, capability and record cleanup'
      );
    });
  }, 60000);

  for (const [reason, extra] of [
    ['unqualified build', { MOCK_VERSION: '0.160.1' }],
    ['qualification advisory', { MOCK_VERSION: '0.159.4' }],
    ['missing app-server', { MOCK_SERVER_FAILURE: '1' }],
  ] as Array<[string, Record<string, string>]>) {
    it(`${reason} defaults to plain launch with one reason notice; strict mode fails`, async () => {
      await withE2EFixture(async (f) => {
        const worker = start(f, 'Fallback', 'default', extra);
        await ready(f, worker);
        const output = f.capture(200, worker.pane);
        expect(output.match(/tmt: Fallback uses paste delivery:/g)).toHaveLength(1);
        expect(output.replaceAll(/\s+/g, ' ')).toMatch(
          /outside the supported channel range|has not been qualified|enrollment could not be recorded/
        );
        expect(records(f)).toEqual([]);
        expect(events(worker, 'thread-start')).toEqual([]);
        const trace = installTmuxTrace(f);
        expect((await talk(f, 'Fallback', 'plain control')).code).toBe(0);
        await f.waitFor(
          () => events(worker, 'paste').some((e) => e.line?.includes('plain control')),
          10000,
          'plain foreground processed paste'
        );
        expect(writes(trace, worker.pane).length).toBeGreaterThan(0);
        await quit(worker);
        const strict = start(f, 'Strict', true, extra);
        expect(
          await waitForFileContent(strict.status, {
            description: 'strict launch refused after bounded startup',
            timeoutMs: 30000,
          })
        ).toBe('1');
        expect(events(strict, 'started')).toEqual([]);
        expect(f.capture(200, strict.pane)).not.toContain('uses paste delivery:');
        expect(records(f)).toEqual([]);
      });
    }, 60000);
  }

  for (const [mode, failure] of [
    ['default', ''],
    [false, ''],
    [true, ''],
    ['default', 'refused'],
    ['default', 'mismatch'],
    [true, 'refused'],
    [true, 'mismatch'],
  ] as Array<[boolean | 'default', string]>) {
    it(`exact resume mode=${mode} ${failure || 'reattaches the remembered thread'} without substitution`, async () => {
      await withE2EFixture(async (f) => {
        const first = start(f, 'Resume');
        await ready(f, first);
        const original = records(f)[0].record.ready.thread;
        await quit(first);
        // This protocol peer has no provider lifecycle hooks. Seed only the
        // historical hook-owned preference; real CLI selects the exact session.
        sql(f, (db) =>
          db
            .prepare(
              `UPDATE identity_session_preferences SET remembered_harness='codex', runtime_mode='embedded', provider_session_id=? WHERE identity_id=(SELECT id FROM identities WHERE name='Resume')`
            )
            .run(original)
        );
        const resumed = start(
          f,
          'Resume',
          mode,
          failure ? { MOCK_RESUME_FAILURE: failure } : {},
          first.pane,
          true
        );
        if (mode === true && failure) {
          expect(
            await waitForFileContent(resumed.status, { description: 'strict exact-resume refusal' })
          ).toBe('1');
          expect(events(resumed, 'started')).toEqual([]);
          expect(events(resumed, 'thread-start')).toEqual([]);
          expect(events(resumed, 'thread-resume')).toHaveLength(1);
          expect(records(f)).toEqual([]);
          expect(f.capture(200, resumed.pane)).not.toContain('uses paste delivery:');
          return;
        }
        await ready(f, resumed);
        expect(events(resumed, 'thread-start')).toEqual([]);
        expect(events(resumed, 'thread-resume')).toHaveLength(mode === false ? 0 : 1);
        if (mode !== false) expect(events(resumed, 'thread-resume')[0].thread).toBe(original);
        const trace = installTmuxTrace(f);
        expect((await talk(f, 'Resume', 'resumed message')).code).toBe(0);
        if (failure || mode === false) {
          expect(records(f)).toEqual([]);
          expect(events(resumed, 'started')[0].args).toEqual(['resume', original, '--no-daemon']);
          expect(
            f.capture(200, resumed.pane).match(/tmt: Resume uses paste delivery:/g) ?? []
          ).toHaveLength(mode === false ? 0 : 1);
          await f.waitFor(
            () => events(resumed, 'paste').some((e) => e.line?.includes('resumed message')),
            10000,
            'original plain resume received paste'
          );
          expect(writes(trace, resumed.pane).length).toBeGreaterThan(0);
        } else {
          expect(records(f)[0].record.ready.thread).toBe(original);
          expect(events(resumed, 'attached')).toHaveLength(1);
          expect(events(resumed, 'queue')).toHaveLength(1);
          expect(events(resumed, 'paste')).toEqual([]);
          expect(writes(trace, resumed.pane)).toEqual([]);
        }
        await quit(resumed);
        await f.waitFor(() => records(f).length === 0, 5000, 'resumed endpoint enrollment retired');
      });
    }, 60000);
  }

  it('default channel hooks preserve the foreground and remembered model through exact resume', async () => {
    await withE2EFixture(async (f) => {
      const preference = () =>
        sql(f, (db) =>
          db
            .prepare(
              `SELECT b.runtime_pid, b.runtime_start_identity, b.observed_provider_session_id, p.provider_session_id, p.driver_state FROM bindings b JOIN identity_session_preferences p ON b.identity_id=p.identity_id JOIN identities i ON i.id=b.identity_id WHERE i.name='HookModel'`
            )
            .get()
        ) as {
          runtime_pid: number;
          runtime_start_identity: string;
          observed_provider_session_id: string;
          provider_session_id: string;
          driver_state: string;
        };
      const first = start(f, 'HookModel', 'default', { MOCK_HOOK_MODEL: 'fixture-first-model' });
      await ready(f, first);
      const original = records(f)[0].record.ready.thread;
      const trace = installTmuxTrace(f);
      const exerciseHook = async (s: Session, model: string) => {
        const [{ record }] = records(f);
        const foreground = record.foreground.process!;
        trace.clear();
        expect((await talk(f, 'HookModel', 'observe hook model')).code).toBe(0);
        await f.waitFor(
          () => events(s, 'hook').length === 1,
          10000,
          'real enrolled lifecycle hook'
        );
        const result = events(s, 'hook')[0];
        expect(result.ok).toBe(true);
        expect(result.stderr).toBe('');
        expect(JSON.parse(result.stdout as string).hookSpecificOutput.additionalContext).toContain(
          'HookModel'
        );
        const saved = preference();
        expect(saved.runtime_pid).toBe(foreground.pid);
        expect(saved.runtime_start_identity).toBe(foreground.start);
        expect(saved.runtime_pid).not.toBe(record.ready.server.pid);
        expect(saved.observed_provider_session_id).toBe(original);
        expect(saved.provider_session_id).toBe(original);
        expect(JSON.parse(saved.driver_state).model).toBe(model);
        expect(records(f)[0].record.foreground.process).toEqual(foreground);
        expect(events(s, 'queue')).toHaveLength(1);
        expect(events(s, 'paste')).toEqual([]);
        expect(writes(trace, s.pane)).toEqual([]);
        await quit(s);
        await f.waitFor(
          () => gone(record.ready.server.pid) && records(f).length === 0,
          5000,
          'hook scenario owned app-server and enrollment gone'
        );
      };
      await exerciseHook(first, 'fixture-first-model');
      const resumed = start(
        f,
        'HookModel',
        'default',
        { MOCK_HOOK_MODEL: 'fixture-resumed-model' },
        first.pane,
        true
      );
      await ready(f, resumed);
      expect(events(resumed, 'thread-start')).toEqual([]);
      expect(events(resumed, 'thread-resume')).toHaveLength(1);
      expect(events(resumed, 'thread-resume')[0].params).toMatchObject({
        threadId: original,
        model: 'fixture-first-model',
      });
      expect(records(f)[0].record.ready.thread).toBe(original);
      const argv = events(resumed, 'started')[0].args as string[];
      expect(argv[0]).toBe('resume');
      expect(argv).toContain('--remote');
      expect(argv.slice(argv.indexOf('-m'), argv.indexOf('-m') + 2)).toEqual([
        '-m',
        'fixture-first-model',
      ]);
      expect(argv.at(-1)).toBe(original);
      await exerciseHook(resumed, 'fixture-resumed-model');
    });
  }, 60000);

  it('queue receipt is delivery only; durable reply completes and name/raw sends never paste', async () => {
    await withE2EFixture(async (f) => {
      const worker = start(f, 'Worker', true, { MOCK_AUTOREPLY: '0' });
      await ready(f, worker);
      const trace = installTmuxTrace(f);
      for (const target of ['Worker', worker.pane]) {
        const result = await talk(f, target, `native to ${target}`);
        expect(result.code, result.stdout + result.stderr).toBe(0);
        expect(result.json).toMatchObject({ status: 'sent' });
        // Public sent means receipt acceptance, never provider processing or reply.
        expect(result.json).not.toHaveProperty('response');

        const id = result.json!.requestId as string;
        expect(
          sql(f, (db) =>
            db.prepare('SELECT body FROM request_responses WHERE request_id = ?').get(id)
          )
        ).toBeUndefined();
        await f.waitFor(
          () =>
            events(worker, 'channel').some((event) =>
              event.content?.includes(`native to ${target}`)
            ),
          10000,
          'accepted input published by the mock foreground'
        );
        const content = events(worker, 'channel').find((e) =>
          e.content?.includes(`native to ${target}`)
        )!.content!;
        const receipt = /tmt reply (\S+) --receipt (\S+) --message <text>/.exec(content)!;
        const response = await f.runJsonCli(
          ['reply', receipt[1], '--receipt', receipt[2], '--message', 'durable-only'],
          { pane: worker.pane }
        );
        expect(response.code, response.stdout + response.stderr).toBe(0);
        expect(
          sql(f, (db) =>
            db.prepare('SELECT body FROM request_responses WHERE request_id = ?').get(id)
          )
        ).toMatchObject({ body: 'durable-only' });
      }
      expect(events(worker, 'queue')).toHaveLength(2);
      expect(events(worker, 'paste')).toEqual([]);
      expect(writes(trace, worker.pane)).toEqual([]);
      await quit(worker);
      await f.waitFor(() => records(f).length === 0, 5000, 'exact record retired');
    });
  }, 60000);

  for (const [mode, code] of [
    ['internal', 'DELIVERY_UNCERTAIN'],
    ['lost', 'DELIVERY_UNCERTAIN'],
    ['archived', 'DELIVERY_PREPARATION_FAILED'],
  ] as const) {
    it(`${mode} receipt is terminal with one queue frame and no fallback`, async () => {
      await withE2EFixture(async (f) => {
        const worker = start(f, 'Worker', true, { MOCK_RECEIPT: mode, MOCK_AUTOREPLY: '0' });
        await ready(f, worker);
        const trace = installTmuxTrace(f);
        const result = await talk(f, 'Worker', 'one attempt');
        expect(result.code, result.stdout + result.stderr).toBe(1);
        expect(result.json).toMatchObject({ error: { code } });
        expect(events(worker, 'queue')).toHaveLength(1);
        expect(events(worker, 'channel')).toEqual([]);
        expect(events(worker, 'paste')).toEqual([]);
        expect(writes(trace, worker.pane)).toEqual([]);
        await quit(worker);
      });
    }, 60000);
  }

  it('reply notification uses native route while a never-enrolled originator proves paste control', async () => {
    await withE2EFixture(async (f) => {
      const worker = start(f, 'Worker');
      const boss = start(f, 'Boss', true, { MOCK_AUTOREPLY: '0' });
      const plain = start(f, 'Plain', false, { MOCK_AUTOREPLY: '0' });
      for (const s of [worker, boss, plain]) await ready(f, s);
      const trace = installTmuxTrace(f);
      const native = await talk(f, 'Worker', 'native originator', boss.pane);
      const baseline = await talk(f, 'Worker', 'plain originator', plain.pane);
      expect(native.code, native.stdout + native.stderr).toBe(0);
      expect(baseline.code, baseline.stdout + baseline.stderr).toBe(0);
      await f.waitFor(
        () => events(boss, 'channel').some((e) => e.content?.includes('reply from Worker')),
        15000,
        'native reply hint'
      );
      await f.waitFor(
        () => events(plain, 'paste').some((e) => e.line?.includes('reply from Worker')),
        15000,
        'baseline reply hint'
      );
      // Receiving the hint precedes the sender's settlement write. Wait on the
      // independent durable oracle, never trigger notification service reads.
      await f.waitFor(
        () =>
          [native, baseline].every(
            (result) =>
              sql(
                f,
                (db) =>
                  (
                    db
                      .prepare('SELECT reply_state FROM request_notifications WHERE request_id = ?')
                      .get(result.json!.requestId) as { reply_state: string }
                  ).reply_state
              ) === 'sent'
          ),
        10000,
        'notification attempts settled'
      );
      for (const result of [native, baseline]) {
        expect(
          sql(f, (db) =>
            db
              .prepare('SELECT body FROM request_responses WHERE request_id = ?')
              .get(result.json!.requestId)
          )
        ).toMatchObject({ body: 'channel-ok' });
        expect(
          sql(f, (db) =>
            db
              .prepare('SELECT reply_state FROM request_notifications WHERE request_id = ?')
              .get(result.json!.requestId)
          )
        ).toMatchObject({ reply_state: 'sent' });
      }
      expect(writes(trace, boss.pane)).toEqual([]);
      expect(writes(trace, worker.pane)).toEqual([]);
      expect(writes(trace, plain.pane).length).toBeGreaterThan(0);
      for (const s of [worker, boss, plain]) await quit(s);
    });
  }, 90000);

  it('binding retirement cannot permit paste to the surviving opted-in foreground after launcher kill', async () => {
    await withE2EFixture(async (f) => {
      const worker = start(f, 'Worker', true, { MOCK_AUTOREPLY: '0' });
      await ready(f, worker);
      const [{ file, record }] = records(f);
      process.kill(record.launchOwner.pid, 'SIGKILL');
      await f.waitFor(
        () => !fs.existsSync(path.join(path.dirname(file), record.generation)),
        10000,
        'owned endpoint generation cleaned'
      );
      expect(fs.existsSync(file)).toBe(true);
      process.kill(record.foreground.process!.pid, 0);
      f.tmux(['set-option', '-p', '-u', '-t', worker.pane, '@tmux-team.agent']);
      await f.runJsonCli(['ls'], { pane: worker.pane });
      expect(
        sql(f, (db) => db.prepare('SELECT id FROM bindings WHERE id = ?').get(record.bindingId))
      ).toBeUndefined();
      const trace = installTmuxTrace(f);
      const result = await talk(f, worker.pane, 'must not paste to orphan');
      expect(result.code, result.stdout + result.stderr).toBe(1);
      expect(writes(trace, worker.pane)).toEqual([]);
      expect(events(worker, 'paste')).toEqual([]);
      // End only this fixture foreground by its own control, never a later PID.
      fs.writeFileSync(`${worker.log}.quit`, '');
      await f.waitFor(
        () => {
          try {
            process.kill(record.foreground.process!.pid, 0);
            return false;
          } catch {
            return true;
          }
        },
        10000,
        'orphan foreground ended'
      );
      const control = await f.runJsonCli([
        'talk',
        worker.pane,
        '# baseline after foreground ended',
        '--detach',
        '--no-preamble',
      ]);
      expect(control.code, control.stdout + control.stderr).toBe(0);
      expect(writes(trace, worker.pane).length).toBeGreaterThan(0);
    });
  }, 60000);
  it('pre-admission enrollment wins route selection even without a preferred harness', async () => {
    await withE2EFixture(async (f) => {
      const worker = start(f, 'Worker', true, { MOCK_AUTOREPLY: '0' });
      await ready(f, worker);
      const [{ record }] = records(f);
      sql(f, (db) =>
        db
          .prepare(
            `UPDATE identity_session_preferences SET preferred_harness=NULL,
        remembered_harness=NULL, runtime_mode=NULL, provider_session_id=NULL`
          )
          .run()
      );
      const trace = installTmuxTrace(f);
      // With no preferred harness, enrollment alone selects Codex and reaches
      // its actual queue transport through the shared enrolled_harness path.
      const routed = await talk(f, 'Worker', 'enrollment selects native');
      expect(routed.code, routed.stdout + routed.stderr).toBe(0);
      expect(events(worker, 'queue')).toHaveLength(1);
      sql(f, (db) => {
        expect(
          db
            .prepare(
              `UPDATE bindings SET runtime_state='unknown', last_transition=NULL,
          runtime_pid=NULL, runtime_start_identity=NULL, observed_provider_session_id=NULL,
          launch_owner_pid=NULL, launch_owner_start_identity=NULL WHERE id=?`
            )
            .run(record.bindingId).changes
        ).toBe(1);
        db.prepare(
          `UPDATE identity_session_preferences SET preferred_harness=NULL,
          remembered_harness=NULL, runtime_mode=NULL, provider_session_id=NULL`
        ).run();
      });
      for (const target of ['Worker', worker.pane]) {
        const result = await talk(f, target, 'pending launch');
        expect(result.code, result.stdout + result.stderr).toBe(1);
        expect(result.json).toMatchObject({ error: { code: 'DELIVERY_PREPARATION_FAILED' } });
      }
      expect(events(worker, 'queue')).toHaveLength(1);
      expect(writes(trace, worker.pane)).toEqual([]);
      await quit(worker);
    });
  }, 60000);

  it('Unknown stays scoped and unpruned after server cleanup; same-binding same-pane explicit relaunch replaces it', async () => {
    await withE2EFixture(async (f) => {
      const old = start(f, 'Worker', true, { MOCK_AUTOREPLY: '0' });
      await ready(f, old);
      const [{ file, record }] = records(f);
      // Reconstruct precisely the pre-publication window from an attributed real
      // launch. Server history remains: it never proves a foreground ended.
      fs.writeFileSync(file, JSON.stringify({ ...record, foreground: { state: 'unknown' } }));
      process.kill(record.launchOwner.pid, 'SIGKILL');
      fs.writeFileSync(`${old.log}.quit`, '');
      await f.waitFor(
        () =>
          fs.existsSync(old.status) &&
          !fs.existsSync(path.join(path.dirname(file), record.generation)),
        10000,
        'old launcher and endpoint cleaned'
      );
      await f.waitFor(() => gone(record.foreground.process!.pid), 10000, 'old foreground ended');
      const before = fs.readFileSync(file, 'utf8');
      const fallback = start(f, 'Worker', 'default', { MOCK_VERSION: '0.160.1' }, old.pane);
      expect(
        await waitForFileContent(fallback.status, {
          description: 'retained enrollment blocks fallback launch',
        })
      ).toBe('1');
      expect(events(fallback, 'started')).toEqual([]);
      expect(fs.readFileSync(file, 'utf8')).toBe(before);
      expect(f.capture(200, fallback.pane)).not.toContain('uses paste delivery:');
      const plain = start(f, 'Bystander', false, { MOCK_AUTOREPLY: '0' });
      await ready(f, plain);
      const trace = installTmuxTrace(f);
      const blocked = await talk(f, old.pane, 'unknown must not paste');
      // A saved offline identity may queue its durable inbox without touching
      // this terminal. The identity-less pane guard is exercised below separately.
      expect(blocked.code, blocked.stdout + blocked.stderr).toBe(0);
      expect(blocked.json).toMatchObject({ status: 'queued', offline: true });
      expect(writes(trace, old.pane)).toEqual([]);
      const control = await talk(f, 'Bystander', 'unrelated pane control');
      expect(control.code, control.stdout + control.stderr).toBe(0);
      await f.waitFor(
        () => events(plain, 'paste').some((e) => e.line?.includes('unrelated pane control')),
        10000,
        'bystander baseline'
      );
      expect(writes(trace, plain.pane).length).toBeGreaterThan(0);
      // Enrollment elsewhere cannot prune Unknown, even with every recorded process gone.
      const elsewhere = start(f, 'Elsewhere');
      await ready(f, elsewhere);
      expect(fs.existsSync(file)).toBe(true);
      const next = start(f, 'Worker', true, { MOCK_AUTOREPLY: '0' }, old.pane);
      await ready(f, next);
      const replacement = records(f).find((row) => row.record.bindingId === record.bindingId)!;
      expect(replacement.record.generation).not.toBe(record.generation);
      const sent = await talk(f, 'Worker', 'new explicit channel');
      expect(sent.code, sent.stdout + sent.stderr).toBe(0);
      await f.waitFor(
        () =>
          events(next, 'channel').some((event) => event.content?.includes('new explicit channel')),
        10000,
        'replacement foreground received its accepted input'
      );
      expect(events(next, 'channel')).toHaveLength(1);
      for (const session of [plain, elsewhere, next]) await quit(session);
    });
  }, 90000);

  it('unattributed records warn by name at both paste sites without blocking an unrelated pane', async () => {
    await withE2EFixture(async (f) => {
      const plain = start(f, 'Plain', false, { MOCK_AUTOREPLY: '0' });
      await ready(f, plain);
      const directory = path.join(f.globalDir, 'channels', 'codex');
      fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
      const file = path.join(directory, `${randomUUID()}.json`);
      fs.writeFileSync(file, '{ invalid', { mode: 0o600 });
      const trace = installTmuxTrace(f);
      const named = await talk(f, 'Plain', 'named baseline');
      expect(named.code, named.stdout + named.stderr).toBe(0);
      expect(named.stderr).toContain(`Skipped channel record ${file}`);
      await f.waitFor(
        () => events(plain, 'paste').some((e) => e.line?.includes('named baseline')),
        10000,
        'named paste control'
      );
      await quit(plain);
      f.tmux(['set-option', '-p', '-u', '-t', plain.pane, '@tmux-team.agent']);
      await f.runJsonCli(['ls'], { pane: plain.pane });
      const raw = await f.runJsonCli([
        'talk',
        plain.pane,
        '# raw baseline',
        '--detach',
        '--no-preamble',
      ]);
      expect(raw.code, raw.stdout + raw.stderr).toBe(0);
      expect(raw.stderr).toContain(`Skipped channel record ${file}`);
      expect(writes(trace, plain.pane).length).toBeGreaterThan(0);
      expect(fs.readFileSync(file, 'utf8')).toBe('{ invalid');
    });
  }, 60000);

  it('an unavailable native originator never gets pasted a reply; duplicate durable reply does not redeliver the hint', async () => {
    await withE2EFixture(async (f) => {
      const worker = start(f, 'Worker', true, { MOCK_AUTOREPLY: '0' });
      const boss = start(f, 'Boss', true, { MOCK_AUTOREPLY: '0' });
      await ready(f, worker);
      await ready(f, boss);
      const trace = installTmuxTrace(f);
      const result = await talk(f, 'Worker', 'reply later', boss.pane);
      expect(result.code, result.stdout + result.stderr).toBe(0);
      await f.waitFor(
        () => events(worker, 'channel').some((event) => event.content?.includes('reply later')),
        10000,
        'deferred reply input published after native receipt'
      );
      const content = events(worker, 'channel').find((event) =>
        event.content?.includes('reply later')
      )!.content!;
      const receipt = /tmt reply (\S+) --receipt (\S+) --message <text>/.exec(content)!;
      const bossRecord = records(f).find(
        (row) => row.record.foreground.process?.pid === events(boss, 'started')[0].pid
      )!;
      // Signal a fixture-owned still-live child, then prove its endpoint is gone.
      process.kill(bossRecord.record.ready.server.pid, 'SIGKILL');
      await f.waitFor(
        () => {
          if (gone(bossRecord.record.ready.server.pid)) return true;
          return execFileSync(
            'ps',
            ['-o', 'stat=', '-p', String(bossRecord.record.ready.server.pid)],
            { encoding: 'utf8' }
          )
            .trim()
            .startsWith('Z');
        },
        10000,
        'originator endpoint gone'
      );
      const args = ['reply', receipt[1], '--receipt', receipt[2], '--message', 'late durable'];
      for (const attempt of [1, 2]) {
        const reply = await f.runJsonCli(args, { pane: worker.pane });
        expect(reply.code, `reply ${attempt}: ${reply.stdout}${reply.stderr}`).toBe(0);
        expect(
          sql(f, (db) =>
            db
              .prepare('SELECT reply_state FROM request_notifications WHERE request_id=?')
              .get(receipt[1])
          )
        ).toMatchObject({ reply_state: 'unavailable' });
      }
      expect(
        sql(f, (db) =>
          db.prepare('SELECT body FROM request_responses WHERE request_id=?').get(receipt[1])
        )
      ).toMatchObject({ body: 'late durable' });
      expect(events(boss, 'queue')).toEqual([]);
      expect(events(boss, 'paste')).toEqual([]);
      expect(writes(trace, boss.pane)).toEqual([]);
      await quit(worker);
      await quit(boss);
    });
  }, 60000);
  it('an Unknown attributed record protects the unbound raw pane after all recorded processes ended', async () => {
    await withE2EFixture(async (f) => {
      const old = start(f, 'Window', true, { MOCK_AUTOREPLY: '0' });
      await ready(f, old);
      const [{ file, record }] = records(f);
      fs.writeFileSync(file, JSON.stringify({ ...record, foreground: { state: 'unknown' } }));
      process.kill(record.launchOwner.pid, 'SIGKILL');
      fs.writeFileSync(`${old.log}.quit`, '');
      await f.waitFor(
        () =>
          fs.existsSync(old.status) &&
          gone(record.foreground.process!.pid) &&
          !fs.existsSync(path.join(path.dirname(file), record.generation)),
        10000,
        'owned processes ended'
      );
      f.tmux(['set-option', '-p', '-u', '-t', old.pane, '@tmux-team.agent']);
      await f.runJsonCli(['ls'], { pane: old.pane });
      expect(
        sql(f, (db) => db.prepare('SELECT id FROM bindings WHERE id=?').get(record.bindingId))
      ).toBeUndefined();
      const trace = installTmuxTrace(f);
      const result = await talk(f, old.pane, 'no input to unknown pane');
      expect(result.code, result.stdout + result.stderr).toBe(1);
      expect(result.stdout).toContain(file);
      expect(writes(trace, old.pane)).toEqual([]);
      expect(fs.existsSync(file)).toBe(true);
    });
  }, 60000);
});
