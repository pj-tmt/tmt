// Office browser specs live outside this package and must not resolve root-hoisted
// test dependencies; they reach the tooling-owned SQLite oracle through this module.
import { existsSync } from 'node:fs';
import path from 'node:path';
import BetterSqlite3 from 'better-sqlite3';

/**
 * Read-only opens of core's `tmux-team.db` see Office data where it lives: once an
 * install has `office/office.db`, that file is opened and core is attached as `core`,
 * so unqualified Office tables resolve to Office storage and core tables resolve to
 * core. Installs still on the shared core file, and writable opens, are unchanged.
 */
export default class SqliteOracle extends BetterSqlite3 {
  constructor(file: string, options?: BetterSqlite3.Options) {
    const office = path.join(path.dirname(file), 'office', 'office.db');
    const split =
      options?.readonly === true && path.basename(file) === 'tmux-team.db' && existsSync(office);
    super(split ? office : file, options);
    if (split) this.exec(`ATTACH DATABASE '${file.replaceAll("'", "''")}' AS core`);
  }
}
