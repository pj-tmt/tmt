import { writeExecutable } from '../support/executable-fixture.mjs';
import Database from 'better-sqlite3';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { parseWholeStdout, runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { seedResponse, type SeededResponse } from './response-fixture.js';

function sql<T>(sandbox: Sandbox, statement: string): T[] {
  const db = new Database(sandbox.database, { readonly: true });
  try {
    return db.prepare(statement).all() as T[];
  } finally {
    db.close();
  }
}
async function waitFor(check: () => boolean, label: string, timeout = 5000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (check()) return;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`Timed out: ${label}`);
}
function commands(sandbox: Sandbox) {
  const file = path.join(sandbox.root, 'host-commands.jsonl');
  return fs.existsSync(file)
    ? fs
        .readFileSync(file, 'utf8')
        .trim()
        .split('\n')
        .filter(Boolean)
        .map((line) => JSON.parse(line) as string[])
    : [];
}
function state(sandbox: Sandbox, typing: boolean) {
  fs.writeFileSync(path.join(sandbox.root, 'typing'), typing ? 'pending' : 'quiet');
}
function pendingKeyEvidence(sandbox: Sandbox) {
  const file = path.join(sandbox.root, 'key-evidence.jsonl');
  return fs.existsSync(file)
    ? fs
        .readFileSync(file, 'utf8')
        .trim()
        .split('\n')
        .filter(Boolean)
        .map((line) => JSON.parse(line) as { pending: boolean; activity: number })
        .filter((evidence) => evidence.pending && evidence.activity > 0).length
    : 0;
}
async function fixture(sandbox: Sandbox, window: number, coldHost = false) {
  const created = await runCli(sandbox, ['identity', 'create', 'sender', '--json']);
  expect(created.status).toBe(0);
  const identity = (parseWholeStdout(created).identity as { id: string }).id;
  const binding = '95400000-0000-4000-8000-000000000001';
  const server = '95400000-0000-4000-8000-000000000002';
  const socket = path.join(sandbox.root, 'mock.sock');
  const now = new Date().toISOString();
  const db = new Database(sandbox.database);
  try {
    db.prepare(
      "INSERT INTO bindings (id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at) VALUES (?,?,'tmux','%1',?,?,1234,'start',5678,?,?)"
    ).run(binding, identity, server, socket, now, now);
  } finally {
    db.close();
  }
  fs.writeFileSync(
    sandbox.globalConfig,
    JSON.stringify({ notifications: { replyBatchWindowMs: window, typingQuietMs: 2000 } })
  );
  state(sandbox, false);
  const directory = path.join(sandbox.root, 'host');
  fs.mkdirSync(directory);
  const marker = {
    version: 1,
    globalIdentity: {
      name: 'sender',
      canonicalName: 'sender',
      identityId: identity,
      bindingId: binding,
      serverId: server,
      panePid: 5678,
    },
  };
  // This executable only supplies protocol evidence. The real CLI owns queue,
  // scheduling, policy and transport sequencing; no user's tmux is reachable.
  const row = [
    server,
    socket,
    '1234',
    'start',
    '%1',
    'test:0.0',
    sandbox.cwd,
    'bash',
    '5678',
    '1',
    JSON.stringify(marker),
  ].join('__TMT_FIELD_4f1c__');
  fs.writeFileSync(path.join(sandbox.root, 'endpoint'), row);
  const helper = fileURLToPath(new URL('./fixtures/reply-notice-host.cjs', import.meta.url));
  const host = path.join(directory, 'tmux');
  // Fault injection models a cold executable that cannot enter the helper
  // within a one-second product probe. Readiness must await its actual close.
  const startup = coldHost
    ? `const fs = require('node:fs'); if (!fs.existsSync(${JSON.stringify(path.join(sandbox.root, 'host-ready'))})) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1500);\n`
    : '';
  writeExecutable(
    host,
    `#!${process.execPath}\n${startup}require(${JSON.stringify(helper)});\n`,
    0o755
  );
  sandbox.env.PATH = `${directory}${path.delimiter}${sandbox.env.PATH ?? ''}`;
  sandbox.env.TMT_954_ROOT = sandbox.root;
  // Keep first-exec assessment outside unchanged product deadlines. The shared
  // runner waits for child close and confirms process-group cleanup.
  const ready = await runCli(
    { ...sandbox, cli: { executable: host, args: [] } },
    ['__tmt_fixture_ready'],
    { deadlineMs: 30_000 }
  );
  expect(ready.status, ready.stderr).toBe(0);
  expect(ready.signal).toBeNull();
  expect(ready.stdout).toBe('');
  expect(fs.readFileSync(path.join(sandbox.root, 'host-ready'), 'utf8')).toBe('ready');
  expect(commands(sandbox)).toEqual([]);
  const seed = (id: string) => {
    const item = seedResponse(sandbox.database, id);
    const db = new Database(sandbox.database);
    try {
      db.prepare(
        "UPDATE request_attempts SET originator_kind='explicit', originator_identity_id=?, recipient_identity_id=? WHERE request_id=?"
      ).run(identity, identity, id);
      db.prepare(
        'INSERT INTO request_notifications (request_id,deadline_ms,timeout_ms) VALUES (?,?,1000)'
      ).run(id, Date.now() + 1000);
    } finally {
      db.close();
    }
    return item;
  };
  return { seed };
}
async function reply(sandbox: Sandbox, item: SeededResponse) {
  const result = await runCli(sandbox, [
    'reply',
    item.requestId,
    '--receipt',
    item.compactReceipt,
    '--message',
    `answer ${item.requestId}`,
    '--json',
  ]);
  expect(result.status, result.stderr).toBe(0);
  return parseWholeStdout(result);
}
function batches(sandbox: Sandbox) {
  return sql<{ id: string; due_ms: number; worker_pid: number; sending: number; members: number }>(
    sandbox,
    'SELECT b.*, (SELECT count(*) FROM reply_notices n WHERE n.batch_id=b.id) AS members FROM reply_notice_batches b'
  );
}
async function reaped(pid: number) {
  await waitFor(() => {
    try {
      execFileSync('ps', ['-p', String(pid), '-o', 'pid='], { stdio: 'pipe' });
      return false;
    } catch (error) {
      if (error instanceof Error && 'status' in error && error.status === 1) return true;
      throw error;
    }
  }, 'worker reaped');
}
async function cleanup(sandbox: Sandbox) {
  if (!fs.existsSync(sandbox.database)) return;
  for (const worker of batches(sandbox)) {
    if (!worker.worker_pid) continue;
    let argv: string;
    try {
      argv = execFileSync('ps', ['-p', String(worker.worker_pid), '-o', 'args='], {
        encoding: 'utf8',
        stdio: 'pipe',
      });
    } catch (error) {
      if (error instanceof Error && 'status' in error && error.status === 1) continue;
      throw error;
    }
    if (argv.includes(`__reply-notice-worker ${worker.id} `)) {
      try {
        process.kill(-worker.worker_pid, 'SIGKILL');
      } catch (error) {
        if (!(error instanceof Error && 'code' in error && error.code === 'ESRCH')) throw error;
      }
      await reaped(worker.worker_pid);
    }
  }
}

describe('native reply notice process scheduling', () => {
  it('prepares a cold host before timed probes without emitting protocol commands', async () => {
    await withSandbox(async (sandbox) => {
      const f = await fixture(sandbox, 1500, true);
      try {
        expect((await reply(sandbox, f.seed('cold-host'))).notification).toBe('queued');
        const first = batches(sandbox)[0];
        await waitFor(() => batches(sandbox).length === 0, 'prepared cold host delivery');
        await reaped(first.worker_pid);
        expect(fs.readdirSync(path.join(sandbox.globalDir, 'reply-notice-workers'))).toEqual([]);
        expect(sql(sandbox, 'SELECT reply_state FROM request_notifications')).toEqual([
          { reply_state: 'sent' },
        ]);
        expect(commands(sandbox).filter((args) => args.includes('paste-buffer'))).toHaveLength(1);
        expect(
          parseWholeStdout(await runCli(sandbox, ['result', 'cold-host', '--json']))
        ).toMatchObject({ response: 'answer cold-host' });
      } finally {
        await cleanup(sandbox);
      }
    });
  });
  it('persists three independent replies in one fixed batch and delivers once', async () => {
    await withSandbox(async (sandbox) => {
      const f = await fixture(sandbox, 1500);
      state(sandbox, true);
      try {
        const items = ['notice-one', 'notice-two', 'notice-three'].map(f.seed);
        expect((await reply(sandbox, items[0])).notification).toBe('queued');
        const first = batches(sandbox)[0];
        // The elapsed window and pending keys hold membership open. Later accepts
        // need not win a wall-clock race against the 1500 ms window.
        await waitFor(() => pendingKeyEvidence(sandbox) >= 1, 'pending batch key evidence');
        for (const item of items.slice(1)) await reply(sandbox, item);
        expect(batches(sandbox)).toMatchObject([
          {
            id: first.id,
            due_ms: first.due_ms,
            worker_pid: first.worker_pid,
            members: 3,
            sending: 0,
          },
        ]);
        expect(
          sql(sandbox, 'SELECT request_id, attempted FROM reply_notices ORDER BY request_id')
        ).toEqual(
          items
            .map((item) => ({ request_id: item.requestId, attempted: 0 }))
            .sort((a, b) => a.request_id.localeCompare(b.request_id))
        );
        state(sandbox, false);
        await waitFor(() => batches(sandbox).length === 0, 'batch settled');
        await reaped(first.worker_pid);
        const workerLogs = path.join(sandbox.globalDir, 'reply-notice-workers');
        const evidence = JSON.stringify({
          notifications: sql(sandbox, 'SELECT reply_state FROM request_notifications'),
          commands: commands(sandbox),
          diagnostics: fs.existsSync(workerLogs)
            ? fs
                .readdirSync(workerLogs)
                .map((name) => fs.readFileSync(path.join(workerLogs, name), 'utf8'))
            : [],
        });
        expect(
          commands(sandbox).filter((args) => args.includes('paste-buffer')),
          evidence
        ).toHaveLength(1);
        expect(sql(sandbox, 'SELECT reply_state FROM request_notifications')).toEqual([
          { reply_state: 'sent' },
          { reply_state: 'sent' },
          { reply_state: 'sent' },
        ]);
        const rendered = commands(sandbox)
          .filter((args) => args.includes('set-buffer'))
          .flat()
          .join('\n');
        for (const item of items) {
          expect(rendered).toContain(
            `reply from sender to ${item.requestId}: tmt result ${item.requestId}`
          );
          expect(
            parseWholeStdout(await runCli(sandbox, ['result', item.requestId, '--json']))
          ).toMatchObject({ response: `answer ${item.requestId}` });
        }
      } finally {
        await cleanup(sandbox);
      }
    });
  });
  it('defers on key evidence until quiet and does not reset the batch deadline', async () => {
    await withSandbox(async (sandbox) => {
      const f = await fixture(sandbox, 100);
      state(sandbox, true);
      try {
        await reply(sandbox, f.seed('typing-one'));
        const first = batches(sandbox)[0];
        await waitFor(() => pendingKeyEvidence(sandbox) >= 3, 'multiple typing rechecks');
        expect(batches(sandbox)).toMatchObject([
          {
            id: first.id,
            due_ms: first.due_ms,
            worker_pid: first.worker_pid,
            members: 1,
            sending: 0,
          },
        ]);
        expect(sql(sandbox, 'SELECT request_id, attempted FROM reply_notices')).toEqual([
          { request_id: 'typing-one', attempted: 0 },
        ]);
        expect(commands(sandbox).filter((args) => args.includes('paste-buffer'))).toEqual([]);
        await reply(sandbox, f.seed('typing-two'));
        expect(batches(sandbox)[0]).toMatchObject({
          id: first.id,
          due_ms: first.due_ms,
          members: 2,
        });
        state(sandbox, false);
        await waitFor(() => batches(sandbox).length === 0, 'quiet delivery');
        await reaped(first.worker_pid);
        expect(commands(sandbox).filter((args) => args.includes('paste-buffer'))).toHaveLength(1);
        expect(sql(sandbox, 'SELECT reply_state FROM request_notifications')).toEqual([
          { reply_state: 'sent' },
          { reply_state: 'sent' },
        ]);
        for (const id of ['typing-one', 'typing-two']) {
          expect(parseWholeStdout(await runCli(sandbox, ['result', id, '--json']))).toMatchObject({
            response: `answer ${id}`,
          });
        }
      } finally {
        await cleanup(sandbox);
      }
    });
  });
  it('zero produces separate deliveries', async () => {
    await withSandbox(async (sandbox) => {
      const f = await fixture(sandbox, 0);
      try {
        for (const id of ['disabled-one', 'disabled-two']) await reply(sandbox, f.seed(id));
        await waitFor(() => batches(sandbox).length === 0, 'disabled grouping settled');
        expect(commands(sandbox).filter((args) => args.includes('paste-buffer'))).toHaveLength(2);
      } finally {
        await cleanup(sandbox);
      }
    });
  });
  it('serializes concurrent separate notices after typing deferral', async () => {
    await withSandbox(async (sandbox) => {
      const f = await fixture(sandbox, 0);
      state(sandbox, true);
      try {
        const items = ['separate-one', 'separate-two'].map(f.seed);
        const accepted = await Promise.all(items.map((item) => reply(sandbox, item)));
        expect(accepted.map((result) => result.notification)).toEqual(['queued', 'queued']);
        const pending = batches(sandbox);
        expect(pending).toHaveLength(2);
        expect(pending.every((batch) => batch.members === 1)).toBe(true);
        expect(commands(sandbox).filter((args) => args.includes('paste-buffer'))).toEqual([]);
        state(sandbox, false);
        await waitFor(() => batches(sandbox).length === 0, 'serialized separate notices');
        for (const batch of pending) await reaped(batch.worker_pid);
        const transport = commands(sandbox)
          .filter((args) => args.includes('paste-buffer') || args.includes('send-keys'))
          .map((args) => (args.includes('paste-buffer') ? 'paste' : 'enter'));
        expect(transport).toEqual(['paste', 'enter', 'paste', 'enter']);
        expect(sql(sandbox, 'SELECT reply_state FROM request_notifications')).toEqual([
          { reply_state: 'sent' },
          { reply_state: 'sent' },
        ]);
      } finally {
        await cleanup(sandbox);
      }
    });
  });

  it('a later reply resumes a proven-dead worker before any input was attempted', async () => {
    await withSandbox(async (sandbox) => {
      const f = await fixture(sandbox, 100);
      state(sandbox, true);
      try {
        await reply(sandbox, f.seed('before-crash'));
        const first = batches(sandbox)[0];
        await waitFor(
          () => commands(sandbox).some((args) => args.includes('list-clients')),
          'typing worker started'
        );
        await cleanup(sandbox);
        expect(batches(sandbox)).toMatchObject([{ id: first.id, members: 1, sending: 0 }]);
        expect(commands(sandbox).filter((args) => args.includes('paste-buffer'))).toEqual([]);
        expect((await reply(sandbox, f.seed('after-crash'))).notification).toBe('queued');
        const resumed = batches(sandbox)[0];
        expect(resumed).toMatchObject({ id: first.id, due_ms: first.due_ms, members: 2 });
        expect(resumed.worker_pid).not.toBe(first.worker_pid);
        state(sandbox, false);
        await waitFor(() => batches(sandbox).length === 0, 'untouched notices recovered');
        await reaped(resumed.worker_pid);
        expect(commands(sandbox).filter((args) => args.includes('paste-buffer'))).toHaveLength(1);
        expect(sql(sandbox, 'SELECT reply_state FROM request_notifications')).toEqual([
          { reply_state: 'sent' },
          { reply_state: 'sent' },
        ]);
        for (const id of ['before-crash', 'after-crash']) {
          expect(parseWholeStdout(await runCli(sandbox, ['result', id, '--json']))).toMatchObject({
            response: `answer ${id}`,
          });
        }
      } finally {
        await cleanup(sandbox);
      }
    });
  });

  it('retains the storage fault diagnostic and never pastes when the input claim fails', async () => {
    await withSandbox(async (sandbox) => {
      const f = await fixture(sandbox, 100);
      state(sandbox, true);
      try {
        await reply(sandbox, f.seed('failed-claim'));
        const first = batches(sandbox)[0];
        await waitFor(
          () => commands(sandbox).some((args) => args.includes('list-clients')),
          'typing worker ready'
        );
        const db = new Database(sandbox.database);
        try {
          db.exec(
            "CREATE TRIGGER reject_notice_attempt BEFORE UPDATE OF attempted ON reply_notices WHEN NEW.attempted=1 BEGIN SELECT RAISE(ABORT, '954 rejected notice claim'); END"
          );
        } finally {
          db.close();
        }
        state(sandbox, false);
        await reaped(first.worker_pid);
        expect(commands(sandbox).filter((args) => args.includes('paste-buffer'))).toEqual([]);
        expect(sql(sandbox, 'SELECT reply_state FROM request_notifications')).toEqual([
          { reply_state: 'claimed' },
        ]);
        expect(sql(sandbox, 'SELECT attempted FROM reply_notices')).toEqual([{ attempted: 0 }]);
        const directory = path.join(sandbox.globalDir, 'reply-notice-workers');
        const diagnostic = fs
          .readdirSync(directory)
          .map((name) => fs.readFileSync(path.join(directory, name), 'utf8'))
          .join('\n');
        expect(diagnostic).toContain('Claim joined reply notice input failed');
        expect(
          parseWholeStdout(await runCli(sandbox, ['result', 'failed-claim', '--json']))
        ).toMatchObject({ response: 'answer failed-claim' });
      } finally {
        await cleanup(sandbox);
      }
    });
  });

  it.each(['unrelated', 'notification'])(
    'keeps accepted replies intact with invalid %s configuration',
    async (kind) => {
      await withSandbox(async (sandbox) => {
        const f = await fixture(sandbox, 100);
        const item = f.seed('config-notice');
        fs.writeFileSync(sandbox.localConfig, '{');
        fs.writeFileSync(
          sandbox.globalConfig,
          JSON.stringify({
            defaults: { timeout: 'invalid' },
            notifications: {
              replyBatchWindowMs: kind === 'notification' ? 'invalid' : 100,
              typingQuietMs: 0,
            },
          })
        );
        try {
          const accepted = await reply(sandbox, item);
          expect(accepted.notification).toBe(kind === 'notification' ? 'unavailable' : 'queued');
          await waitFor(() => batches(sandbox).length === 0, 'advisory configuration outcome');
          expect(sql(sandbox, 'SELECT reply_state FROM request_notifications')).toEqual([
            { reply_state: kind === 'notification' ? 'unavailable' : 'sent' },
          ]);
          expect(
            parseWholeStdout(await runCli(sandbox, ['result', item.requestId, '--json']))
          ).toMatchObject({ response: `answer ${item.requestId}` });
        } finally {
          await cleanup(sandbox);
        }
      });
    }
  );

  it('rechecks channel enrollment before delivering a pending batch', async () => {
    await withSandbox(async (sandbox) => {
      const f = await fixture(sandbox, 1500);
      const item = f.seed('channel-transition');
      try {
        expect((await reply(sandbox, item)).notification).toBe('queued');
        const first = batches(sandbox)[0];
        const directory = path.join(sandbox.globalDir, 'channels');
        fs.mkdirSync(directory, { recursive: true });
        fs.writeFileSync(path.join(directory, '95400000-0000-4000-8000-000000000001.json'), '{');
        await waitFor(() => batches(sandbox).length === 0, 'enrolled batch refused safely');
        await reaped(first.worker_pid);
        expect(commands(sandbox).filter((args) => args.includes('paste-buffer'))).toEqual([]);
        expect(sql(sandbox, 'SELECT reply_state FROM request_notifications')).toEqual([
          { reply_state: 'unavailable' },
        ]);
        expect(
          parseWholeStdout(await runCli(sandbox, ['result', item.requestId, '--json']))
        ).toMatchObject({ response: `answer ${item.requestId}` });
      } finally {
        await cleanup(sandbox);
      }
    });
  });

  it.each(['same', 'replaced'])(
    'a prior binding on the %s pane incarnation follows pane enrollment evidence',
    async (incarnation) => {
      await withSandbox(async (sandbox) => {
        const f = await fixture(sandbox, 100);
        const directory = path.join(sandbox.globalDir, 'channels');
        fs.mkdirSync(directory, { recursive: true });
        const binding = '95400000-0000-4000-8000-000000000004';
        const ownerStart = execFileSync('ps', ['-p', String(process.pid), '-o', 'lstart='], {
          encoding: 'utf8',
        }).trim();
        fs.writeFileSync(
          path.join(directory, `${binding}.json`),
          JSON.stringify({
            version: 1,
            bindingId: binding,
            generation: '95400000-0000-4000-8000-000000000003',
            launchOwner: { pid: process.pid, start: ownerStart },
            pane: {
              host: 'tmux',
              serverId: '95400000-0000-4000-8000-000000000002',
              socketPath: path.join(sandbox.root, 'mock.sock'),
              serverPid: 1234,
              serverStartTime: 'start',
              paneId: '%1',
              panePid: incarnation === 'same' ? 5678 : 5679,
            },
            claude: null,
          })
        );
        try {
          const accepted = await reply(sandbox, f.seed('prior-binding-notice'));
          expect(accepted.notification).toBe(incarnation === 'same' ? 'unavailable' : 'queued');
          await waitFor(() => batches(sandbox).length === 0, 'pane evidence outcome');
          expect(commands(sandbox).filter((args) => args.includes('paste-buffer'))).toHaveLength(
            incarnation === 'same' ? 0 : 1
          );
          expect(sql(sandbox, 'SELECT reply_state FROM request_notifications')).toEqual([
            { reply_state: incarnation === 'same' ? 'unavailable' : 'sent' },
          ]);
        } finally {
          await cleanup(sandbox);
        }
      });
    }
  );

  it.each(['valid', 'corrupt'])(
    '%s channel enrollment bypasses batching and never pastes',
    async (kind) => {
      await withSandbox(async (sandbox) => {
        const f = await fixture(sandbox, 5000);
        // A driver's binding-addressed corrupt enrollment is sticky unknown;
        // ordinary delivery must refuse it rather than queue or baseline paste.
        const directory = path.join(sandbox.globalDir, 'channels');
        fs.mkdirSync(directory, { recursive: true });
        const binding = '95400000-0000-4000-8000-000000000001';
        const ownerStart = execFileSync('ps', ['-p', String(process.pid), '-o', 'lstart='], {
          encoding: 'utf8',
        }).trim();
        const record = {
          version: 1,
          bindingId: binding,
          generation: '95400000-0000-4000-8000-000000000003',
          launchOwner: { pid: process.pid, start: ownerStart },
          claude: null,
        };
        fs.writeFileSync(
          path.join(directory, `${binding}.json`),
          kind === 'corrupt' ? '{' : JSON.stringify(record)
        );
        try {
          const accepted = await reply(sandbox, f.seed('channel-notice'));
          expect(accepted.notification).toBe('unavailable');
          expect(batches(sandbox)).toEqual([]);
          expect(commands(sandbox).filter((args) => args.includes('paste-buffer'))).toEqual([]);
        } finally {
          await cleanup(sandbox);
        }
      });
    }
  );
});
