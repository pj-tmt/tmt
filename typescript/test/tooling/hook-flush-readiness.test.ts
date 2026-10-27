import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import Database from 'better-sqlite3';
import { expect, it } from 'vite-plus/test';

it('observes flusher completion even when WAL notifications are missed', async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hook-flush-readiness-'));
  const database = new Database(path.join(root, 'hook-writes.db'));
  database.pragma('journal_mode = WAL');
  database.pragma('user_version = 3');
  database.exec(
    'CREATE TABLE hook_writes(id INTEGER PRIMARY KEY); INSERT INTO hook_writes VALUES(1)'
  );
  const log = path.join(root, 'hook-flush-11111111-1111-4111-8111-111111111111.log');
  fs.writeFileSync(log, 'flusher_pid=1\n');
  const helper = fileURLToPath(new URL('../support/wait-hook-flush.mjs', import.meta.url));
  const bootstrap = path.join(root, 'bootstrap.mjs');
  // Simulate coalesced/missed WAL events while retaining real directory events.
  // The database is real: the worker log is removed only after its commit.
  fs.writeFileSync(
    bootstrap,
    `import fs from 'node:fs';
import { EventEmitter } from 'node:events';
import { pathToFileURL } from 'node:url';
const watch = fs.watch;
fs.watch = (file, callback) => {
  if (String(file).endsWith('-wal')) return Object.assign(new EventEmitter(), { close() {} });
  return watch(file, (event, name) => {
    if (name && name.toString().startsWith('hook-flush-')) callback(event, name);
  });
};
await import(pathToFileURL(${JSON.stringify(helper)}));
`
  );
  const child = spawn(process.execPath, [bootstrap, root, '3'], {
    stdio: ['ignore', 'ignore', 'pipe', 'ipc'],
  });
  let stderr = '';
  child.stderr!.on('data', (part) => (stderr += part));
  const exited = new Promise<number | null>((resolve, reject) => {
    child.once('error', reject);
    child.once('exit', resolve);
  });
  try {
    await new Promise<void>((resolve, reject) => {
      child.once('message', (message) => (message === 'watching' ? resolve() : reject(message)));
      child.once('exit', (code) => reject(new Error(`waiter exited before readiness: ${code}`)));
      child.once('error', reject);
    });
    database.exec('DELETE FROM hook_writes');
    fs.unlinkSync(log);
    const code = await exited;
    expect(code, stderr).toBe(0);
    expect(database.prepare('SELECT count(*) AS n FROM hook_writes').get()).toEqual({ n: 0 });
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill();
    await exited.catch(() => undefined);
    database.close();
    fs.rmSync(root, { recursive: true, force: true });
  }
});
