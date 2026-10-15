import Database from 'better-sqlite3';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { hookResult, waitForLifecycle } from '../e2e/mock-claude-lifecycle.mjs';

const warning = 'tmt: lifecycle context unavailable; continuing without context.\n';
const roots: string[] = [];
const databases: Database.Database[] = [];
afterEach(() => {
  vi.useRealTimers();
  for (const database of databases.splice(0)) database.close();
  for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

function ledger() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'claude-lifecycle-'));
  roots.push(root);
  const file = path.join(root, 'state.db');
  const database = new Database(file);
  databases.push(database);
  database.exec(`
    CREATE TABLE bindings(identity_id TEXT, runtime_state TEXT, runtime_pid INTEGER,
      runtime_start_identity TEXT, observed_provider_session_id TEXT);
    CREATE TABLE identity_session_preferences(identity_id TEXT, remembered_harness TEXT,
      provider_session_id TEXT, driver_state TEXT);
    INSERT INTO bindings VALUES('identity','running',42,'birth',NULL);
    INSERT INTO identity_session_preferences VALUES('identity',NULL,NULL,NULL);
  `);
  const commitStart = () =>
    database.exec(`
    UPDATE bindings SET observed_provider_session_id='session';
    UPDATE identity_session_preferences SET remembered_harness='claude',
      provider_session_id='session',driver_state='{"model":"model-a"}';
  `);
  const commitPrompt = () =>
    database.prepare('UPDATE identity_session_preferences SET driver_state=?').run(
      JSON.stringify({
        activity: { session: 'session', pid: 42, start: 'birth', state: 'working' },
      })
    );
  return { file, database, commitStart, commitPrompt };
}

describe('Claude launch lifecycle readiness', () => {
  it('keeps the exact zero-exit warning diagnostic after an actually committed start', async () => {
    const state = ledger();
    state.commitStart();
    expect(hookResult(null, '', warning)).toEqual({ stdout: '', diagnostic: warning });
    await waitForLifecycle(state.file, 42, 'session', 'started', Date.now() + 100);
  });

  it('refuses readiness without a committed session instead of invoking a prompt', async () => {
    vi.useFakeTimers();
    const state = ledger();
    const ready = waitForLifecycle(state.file, 42, 'session', 'started', Date.now() + 100);
    const assertion = expect(ready).rejects.toThrow('started was not committed');
    await Promise.all([assertion, vi.advanceTimersByTimeAsync(100)]);
  });

  it('keeps nonzero exits, other stderr and malformed hook output as hard failures', () => {
    const failure = new Error('hook exited 1');
    expect(() => hookResult(failure, '', warning)).toThrow(failure);
    for (const stderr of ['other warning\n', ` ${warning}`, warning + warning])
      expect(() => hookResult(null, '', stderr)).toThrow();
    expect(() => hookResult(null, '{', '')).toThrow();
    expect(() => hookResult(null, '{', warning)).toThrow();
    expect(hookResult(null, '{"ok":true}', '')).toEqual({ stdout: '{"ok":true}' });
  });

  it('does not publish prompt readiness until matching Working activity commits', async () => {
    vi.useFakeTimers();
    const state = ledger();
    state.commitStart();
    let recorded = false;
    const ready = waitForLifecycle(state.file, 42, 'session', 'working', Date.now() + 100).then(
      () => {
        recorded = true;
      }
    );
    try {
      await vi.advanceTimersByTimeAsync(25);
      expect(recorded, 'the prompt completion is still held').toBe(false);
    } finally {
      state.commitPrompt();
      await vi.advanceTimersByTimeAsync(25);
      await ready;
    }
    expect(recorded).toBe(true);
  });
});
