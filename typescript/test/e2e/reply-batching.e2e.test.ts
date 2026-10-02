import Database from 'better-sqlite3';
import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { expectJsonResult } from './cli-assertions.js';

function query<T>(fixture: E2EFixture, sql: string): T[] {
  const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
  try {
    return db.prepare(sql).all() as T[];
  } finally {
    db.close();
  }
}
function batches(fixture: E2EFixture) {
  return query<{
    id: string;
    due_ms: number;
    window_ms: number;
    worker_pid: number;
    sending: number;
    members: number;
  }>(
    fixture,
    'SELECT b.*, (SELECT count(*) FROM reply_notices n WHERE n.batch_id=b.id) AS members FROM reply_notice_batches b ORDER BY due_ms'
  );
}
function states(fixture: E2EFixture) {
  return query<{ reply_state: string }>(fixture, 'SELECT reply_state FROM request_notifications');
}
function settings(fixture: E2EFixture, window: number, quiet = 2000) {
  fs.writeFileSync(
    path.join(fixture.globalDir, 'config.json'),
    JSON.stringify({ notifications: { replyBatchWindowMs: window, typingQuietMs: quiet } })
  );
}
async function request(fixture: E2EFixture, recipient: string) {
  const sent = expectJsonResult(
    await fixture.runJsonCli([
      'talk',
      recipient,
      'batch question',
      '--identity',
      'sender',
      '--detach',
      '--no-preamble',
    ])
  );
  const id = String(sent.requestId);
  const detail = expectJsonResult<{ exchange: { reply: { receipt: string } } }>(
    await fixture.runJsonCli(['x', 'show', id, '--incoming', '--identity', recipient])
  );
  return { id, receipt: detail.exchange.reply.receipt };
}
async function reply(fixture: E2EFixture, item: { id: string; receipt: string }) {
  return expectJsonResult(
    await fixture.runJsonCli(
      ['reply', item.id, '--receipt', item.receipt, '--message', `answer ${item.id}`],
      { transportTrace: true }
    )
  );
}
function transportTrace(fixture: E2EFixture) {
  return fs.existsSync(fixture.transportTracePath)
    ? fs.readFileSync(fixture.transportTracePath, 'utf8').split('\n').filter(Boolean)
    : [];
}
function submissions(fixture: E2EFixture) {
  return transportTrace(fixture).filter((line) => line.startsWith('submit.after.0|'));
}
async function gone(fixture: E2EFixture, pid: number) {
  await fixture.waitFor(
    () => {
      try {
        execFileSync('ps', ['-p', String(pid), '-o', 'pid='], { stdio: 'pipe' });
        return false;
      } catch (error) {
        if (error instanceof Error && 'status' in error && error.status === 1) return true;
        throw error;
      }
    },
    5000,
    'reply worker exited and was reaped'
  );
}
async function sender(fixture: E2EFixture) {
  expectJsonResult(await fixture.runJsonCli(['name', 'receiver']));
  const pane = await fixture.createMockPane('sender');
  expectJsonResult(await fixture.runJsonCli(['name', 'sender'], { pane: pane.pane }));
  return pane;
}
async function terminal(fixture: E2EFixture, pane: string) {
  fixture.tmux(['select-window', '-t', pane]);
  const helper = fileURLToPath(new URL('./reply-notice-pty.py', import.meta.url));
  const child = spawn('python3', [helper, fixture.socketPath, 'e2e'], {
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  let output = '';
  let errors = '';
  child.stdout.on('data', (data) => {
    output += String(data);
  });
  child.stderr.on('data', (data) => {
    errors += String(data);
  });
  try {
    await fixture.waitFor(
      () => output.includes('\n') || child.exitCode !== null || child.signalCode !== null,
      3000,
      'owned PTY client started'
    );
    expect(child.exitCode, errors).toBeNull();
    const pid = JSON.parse(output.split('\n')[0]).pid as number;
    await fixture.waitFor(
      () =>
        fixture
          .tmux(['list-clients', '-F', '#{client_pid}|#{pane_id}'])
          .trim()
          .split('\n')
          .includes(`${pid}|${pane}`),
      3000,
      'PTY client viewing target pane'
    );
    let keys = 0;
    const acknowledgedKeys = () =>
      output
        .split('\n')
        .slice(0, -1)
        .filter((line) => line && JSON.parse(line).key === true).length;
    return {
      async key() {
        const expected = ++keys;
        child.stdin.write('"key"\n');
        await fixture.waitFor(
          () =>
            acknowledgedKeys() >= expected || child.exitCode !== null || child.signalCode !== null,
          3000,
          'PTY helper wrote the real key byte'
        );
        expect(acknowledgedKeys(), errors).toBeGreaterThanOrEqual(expected);
      },
      async close() {
        const closed = new Promise<void>((resolve, reject) => {
          const timer = setTimeout(() => {
            child.kill('SIGKILL');
            reject(new Error('PTY helper did not reap its client'));
          }, 4000);
          child.once('close', (code) => {
            clearTimeout(timer);
            if (code === 0) resolve();
            else reject(new Error(errors));
          });
        });
        child.stdin.end('"close"\n');
        await closed;
        expect(
          fixture.tmux(['list-clients', '-F', '#{client_pid}']).trim().split('\n')
        ).not.toContain(String(pid));
      },
    };
  } catch (error) {
    child.stdin.end();
    if (child.exitCode === null && child.signalCode === null) {
      await new Promise<void>((resolve, reject) => {
        const terminate = setTimeout(() => child.kill('SIGTERM'), 1000);
        const kill = setTimeout(() => child.kill('SIGKILL'), 6000);
        const deadline = setTimeout(
          () => reject(new Error('PTY helper cleanup did not close')),
          7000
        );
        child.once('close', () => {
          clearTimeout(terminate);
          clearTimeout(kill);
          clearTimeout(deadline);
          resolve();
        });
      });
    }
    throw error;
  }
}

describe('reply notice batching and real key debounce', { concurrent: false }, () => {
  it('uses one fixed default window and one paste for three senders across reply processes', async () => {
    await withE2EFixture(
      async (fixture) => {
        await sender(fixture);
        const requests = [await request(fixture, 'receiver')];
        for (const name of ['second', 'third']) {
          const pane = await fixture.createMockPane(name);
          expectJsonResult(await fixture.runJsonCli(['name', name], { pane: pane.pane }));
          requests.push(await request(fixture, name));
        }
        expect((await reply(fixture, requests[0])).notification).toBe('queued');
        const first = batches(fixture)[0];
        expect(first.window_ms).toBe(5000);
        for (const item of requests.slice(1))
          expect((await reply(fixture, item)).notification).toBe('queued');
        expect(batches(fixture)).toMatchObject([
          { id: first.id, due_ms: first.due_ms, members: 3 },
        ]);
        expect(submissions(fixture)).toHaveLength(0);
        expect(states(fixture).map((row) => row.reply_state)).toEqual([
          'claimed',
          'claimed',
          'claimed',
        ]);
        for (const item of requests)
          expect(expectJsonResult(await fixture.runJsonCli(['result', item.id]))).toMatchObject({
            response: `answer ${item.id}`,
          });
        await fixture.waitFor(
          () => states(fixture).every((row) => row.reply_state === 'sent'),
          10000,
          'batch delivered after fixed window with Unknown input'
        );
        expect(Date.now()).toBeGreaterThanOrEqual(first.due_ms);
        expect(submissions(fixture)).toHaveLength(1);
        for (const [index, item] of requests.entries()) {
          const name = ['receiver', 'second', 'third'][index];
          await fixture.waitForEvent(
            (event) =>
              event.event === 'input' &&
              event.line === `[tmt] reply from ${name} to ${item.id}: tmt result ${item.id}`
          );
        }
        expect(batches(fixture)).toEqual([]);
        await gone(fixture, first.worker_pid);
        expect(fs.readdirSync(path.join(fixture.globalDir, 'reply-notice-workers'))).toEqual([]);
      },
      { mode: 'input-log' }
    );
  });

  it('zero disables grouping and delivers each notice separately', async () => {
    await withE2EFixture(
      async (fixture) => {
        await sender(fixture);
        settings(fixture, 0, 0);
        const items = [await request(fixture, 'receiver'), await request(fixture, 'receiver')];
        for (const item of items) await reply(fixture, item);
        await fixture.waitFor(
          () => states(fixture).every((row) => row.reply_state === 'sent'),
          5000,
          'individual notices'
        );
        expect(submissions(fixture)).toHaveLength(2);
        expect(batches(fixture)).toEqual([]);
      },
      { mode: 'input-log' }
    );
  });

  it('serializes two concurrent zero-window notices after real typing deferral', async () => {
    await withE2EFixture(
      async (fixture) => {
        const pane = await sender(fixture);
        settings(fixture, 0, 2000);
        const items = [await request(fixture, 'receiver'), await request(fixture, 'receiver')];
        const client = await terminal(fixture, pane.pane);
        try {
          await client.key();
          const accepted = await Promise.all(items.map((item) => reply(fixture, item)));
          expect(accepted.map((result) => result.notification)).toEqual(['queued', 'queued']);
          const queued = batches(fixture);
          expect(queued).toHaveLength(2);
          expect(queued.every((batch) => batch.members === 1 && batch.window_ms === 0)).toBe(true);
          expect(submissions(fixture)).toHaveLength(0);
          await client.key();
          await fixture.waitFor(
            () => states(fixture).every((row) => row.reply_state === 'sent'),
            8000,
            'separate serialized notices after quiet'
          );
          const operations = transportTrace(fixture).filter(
            (line) => line.startsWith('paste-buffer.before|') || line.startsWith('submit.after.0|')
          );
          expect(operations.map((line) => line.split('|')[0])).toEqual([
            'paste-buffer.before',
            'submit.after.0',
            'paste-buffer.before',
            'submit.after.0',
          ]);
          for (const item of items) {
            await fixture.waitForEvent(
              (event) =>
                event.event === 'input' &&
                event.pid === pane.pid &&
                event.line === `[tmt] reply from receiver to ${item.id}: tmt result ${item.id}`
            );
          }
          expect(batches(fixture)).toEqual([]);
          for (const batch of queued) await gone(fixture, batch.worker_pid);
        } finally {
          await client.close();
        }
      },
      { mode: 'input-log' }
    );
  });

  it('an uncertain send settles once and an identical reply never replays it', async () => {
    await withE2EFixture(
      async (fixture) => {
        await sender(fixture);
        settings(fixture, 100, 0);
        const item = await request(fixture, 'receiver');
        const args = ['reply', item.id, '--receipt', item.receipt, '--message', 'uncertain hint'];
        const accepted = expectJsonResult(
          await fixture.runJsonCli(args, {
            transportTrace: true,
            transportFault: { stage: 'submit' },
          })
        );
        expect(accepted.notification).toBe('queued');
        const first = batches(fixture)[0];
        await fixture.waitFor(
          () => states(fixture)[0].reply_state === 'uncertain',
          5000,
          'uncertain notice settled'
        );
        await gone(fixture, first.worker_pid);
        expect(submissions(fixture)).toHaveLength(1);
        const retried = expectJsonResult(await fixture.runJsonCli(args, { transportTrace: true }));
        expect(retried).not.toHaveProperty('notification');
        expect(submissions(fixture)).toHaveLength(1);
        expect(batches(fixture)).toEqual([]);
        expect(expectJsonResult(await fixture.runJsonCli(['result', item.id]))).toMatchObject({
          response: 'uncertain hint',
        });
      },
      { mode: 'input-log' }
    );
  });

  it('a replacement binding does not receive a previously queued notice', async () => {
    await withE2EFixture(
      async (fixture) => {
        const pane = await sender(fixture);
        settings(fixture, 1500, 0);
        const item = await request(fixture, 'receiver');
        await reply(fixture, item);
        const first = batches(fixture)[0];
        expectJsonResult(await fixture.runJsonCli(['unbind'], { pane: pane.pane }));
        expectJsonResult(await fixture.runJsonCli(['name', 'replacement'], { pane: pane.pane }));
        await fixture.waitFor(
          () => states(fixture)[0].reply_state === 'unavailable',
          5000,
          'binding-fenced notice settled'
        );
        await gone(fixture, first.worker_pid);
        expect(submissions(fixture)).toHaveLength(0);
        expect(expectJsonResult(await fixture.runJsonCli(['result', item.id]))).toMatchObject({
          response: `answer ${item.id}`,
        });
      },
      { mode: 'input-log' }
    );
  });

  it('each actual terminal key resets quiet while new replies leave the window fixed', async () => {
    await withE2EFixture(
      async (fixture) => {
        const pane = await sender(fixture);
        settings(fixture, 100, 2000);
        const items = [await request(fixture, 'receiver'), await request(fixture, 'receiver')];
        const client = await terminal(fixture, pane.pane);
        try {
          await client.key();
          await reply(fixture, items[0]);
          const first = batches(fixture)[0];
          for (let i = 0; i < 5; i++) {
            await client.key();
            // This spans real key clock ticks to prove debounce, not readiness.
            await new Promise((resolve) => setTimeout(resolve, 600));
            expect(submissions(fixture)).toHaveLength(0);
          }
          await reply(fixture, items[1]);
          expect(batches(fixture)[0]).toMatchObject({
            id: first.id,
            due_ms: first.due_ms,
            members: 2,
          });
          const finalKey = Date.now();
          await client.key();
          await fixture.waitFor(
            () => transportTrace(fixture).some((line) => line.startsWith('paste-buffer.before|')),
            5000,
            'first paste only after the final key quiet period'
          );
          expect(Date.now()).toBeGreaterThanOrEqual(finalKey + 2000);
          await fixture.waitFor(
            () => states(fixture).every((row) => row.reply_state === 'sent'),
            5000,
            'quiet typing debounce'
          );
          expect(submissions(fixture)).toHaveLength(1);
          for (const item of items)
            await fixture.waitForEvent(
              (event) =>
                event.event === 'input' &&
                event.pid === pane.pid &&
                event.line === `[tmt] reply from receiver to ${item.id}: tmt result ${item.id}`
            );
          await gone(fixture, first.worker_pid);
        } finally {
          await client.close();
        }
      },
      { mode: 'input-log' }
    );
  });

  it('delivers at the 30 s bound despite continuous attached-client typing', async () => {
    await withE2EFixture(
      async (fixture) => {
        const pane = await sender(fixture);
        settings(fixture, 100, 30000);
        const item = await request(fixture, 'receiver');
        const client = await terminal(fixture, pane.pane);
        try {
          await client.key();
          await reply(fixture, item);
          const first = batches(fixture)[0];
          const began = Date.now();
          while (
            states(fixture)[0].reply_state === 'claimed' &&
            batches(fixture)[0]?.sending !== 1 &&
            Date.now() - began < 34000
          ) {
            await client.key();
            await new Promise((resolve) => setTimeout(resolve, 500));
          }
          expect(Date.now()).toBeGreaterThanOrEqual(first.due_ms + 30000);
          await fixture.waitFor(
            () => states(fixture)[0].reply_state === 'sent',
            5000,
            'bounded forced delivery'
          );
          expect(submissions(fixture)).toHaveLength(1);
          await fixture.waitForEvent(
            (event) =>
              event.event === 'input' &&
              event.pid === pane.pid &&
              event.line === `[tmt] reply from receiver to ${item.id}: tmt result ${item.id}`
          );
          await gone(fixture, first.worker_pid);
        } finally {
          await client.close();
        }
      },
      { mode: 'input-log' }
    );
  }, 45000);
});
