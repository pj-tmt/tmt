// Event-driven readiness for the private Docker fixture's asynchronous Stop.
// The queue acknowledgement follows the capture and optional follow-up.
import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';

const [directory, schema] = process.argv.slice(2);
const queue = path.join(directory, 'hook-writes.db');
if (fs.existsSync(queue)) {
  const db = new Database(queue, { readonly: true, fileMustExist: true });
  if (db.pragma('user_version', { simple: true }) !== Number(schema)) {
    db.close(); // Approved direct fallback.
  } else {
    await new Promise((resolve, reject) => {
      let done = false;
      let walWatcher;
      const wal = `${queue}-wal`;
      const finish = (error) => {
        if (done) return;
        done = true;
        clearTimeout(deadline);
        watcher.close();
        walWatcher?.close();
        db.close();
        if (error) reject(error);
        else resolve();
      };
      const inspect = () => {
        if (done) return;
        try {
          if (db.prepare('SELECT count(*) AS n FROM hook_writes').get().n === 0) finish();
        } catch (error) {
          finish(error);
        }
      };
      const watchWal = () => {
        if (done) return;
        walWatcher?.close();
        walWatcher = undefined;
        try {
          walWatcher = fs.watch(wal, inspect);
          walWatcher.on('error', finish);
        } catch (error) {
          if (error.code !== 'ENOENT') finish(error);
        }
      };
      // Register before the first read so an acknowledgment cannot be missed.
      const watcher = fs.watch(directory, (_event, filename) => {
        if (!filename || filename.toString() === 'hook-writes.db-wal') {
          watchWal();
          inspect();
        } else if (filename.toString().startsWith('hook-flush-')) {
          // A successful worker removes its own log only after acknowledging
          // the committed queue entry, even when a WAL notification coalesces.
          inspect();
        }
      });
      watcher.on('error', finish);
      const deadline = setTimeout(
        () => finish(new Error('queued Stop was not acknowledged')),
        5000
      );
      watchWal();
      inspect();
      if (!done && process.send) process.send('watching');
    });
  }
}
