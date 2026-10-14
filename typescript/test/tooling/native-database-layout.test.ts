import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it, vi } from 'vite-plus/test';
import { assertNoCoreDatabase, readCoreDatabase } from '../../scripts/native-database-layout.mjs';

function withState(run: (root: string, state: string) => void) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-database-reader-')));
  const state = path.join(root, 'owned state');
  fs.mkdirSync(state);
  try {
    run(root, state);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

function store(state: string, name: string) {
  const file = path.join(state, name);
  // Header evidence only; the observer must not open or validate this with SQLite.
  fs.writeFileSync(file, 'SQLite format 3\0fixture payload');
  return file;
}

describe('read-only Core database-name evidence', () => {
  it.each(['tmux-team.db', 'tmt.db'])('reads the existing %s without changing bytes', (name) => {
    withState((_root, state) => {
      const file = store(state, name);
      const before = fs.readFileSync(file);
      const open = vi.spyOn(fs, 'openSync');
      try {
        expect(readCoreDatabase(state)).toBe(file);
        expect(open).toHaveBeenCalledWith(file, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
      } finally {
        open.mockRestore();
      }
      expect(fs.readFileSync(file)).toEqual(before);
      expect(fs.readdirSync(state)).toEqual([name]);
    });
  });

  it('selects the current store with only the agreed former relative link', () => {
    withState((_root, state) => {
      const current = store(state, 'tmt.db');
      fs.symlinkSync('tmt.db', path.join(state, 'tmux-team.db'));
      expect(readCoreDatabase(state)).toBe(current);
      expect(fs.readlinkSync(path.join(state, 'tmux-team.db'))).toBe('tmt.db');
    });
  });

  it('observes the interrupted same-inode names without completing the rename', () => {
    withState((_root, state) => {
      const current = store(state, 'tmt.db');
      const former = path.join(state, 'tmux-team.db');
      fs.linkSync(current, former);
      expect(readCoreDatabase(state)).toBe(current);
      expect(fs.lstatSync(former).isFile()).toBe(true);
    });
  });

  it.each(['tmt.db', 'tmux-team.db'])(
    'refuses a dangling %s, including in absence checks',
    (name) => {
      withState((_root, state) => {
        fs.symlinkSync('missing.db', path.join(state, name));
        expect(() => readCoreDatabase(state)).toThrow('existing regular file');
        expect(() => assertNoCoreDatabase(state, 'must not open SQLite')).toThrow(
          'must not open SQLite'
        );
      });
    }
  );

  it.each(['tmt.db', 'tmux-team.db'])('never opens the foreign %s link target', (name) => {
    withState((root, state) => {
      const foreign = store(root, 'outside.db');
      if (name === 'tmux-team.db') store(state, 'tmt.db');
      fs.symlinkSync(foreign, path.join(state, name));
      const open = vi.spyOn(fs, 'openSync');
      try {
        expect(() => readCoreDatabase(state)).toThrow();
        expect(open).not.toHaveBeenCalled();
      } finally {
        open.mockRestore();
      }
      expect(fs.readFileSync(foreign).toString()).toContain('fixture payload');
    });
  });

  it('refuses a linked state directory before opening any database', () => {
    withState((root, state) => {
      store(state, 'tmt.db');
      const alias = path.join(root, 'alias');
      fs.symlinkSync(state, alias);
      const open = vi.spyOn(fs, 'openSync');
      try {
        expect(() => readCoreDatabase(alias)).toThrow('owned directory');
        expect(() => assertNoCoreDatabase(alias, 'must not open SQLite')).toThrow(
          'owned directory'
        );
        expect(open).not.toHaveBeenCalled();
      } finally {
        open.mockRestore();
      }
    });
  });

  it('refuses corrupt selected new bytes instead of falling back to the former store', () => {
    withState((_root, state) => {
      store(state, 'tmux-team.db');
      fs.writeFileSync(path.join(state, 'tmt.db'), 'corrupt');
      expect(() => readCoreDatabase(state)).toThrow();
      fs.unlinkSync(path.join(state, 'tmux-team.db'));
      fs.symlinkSync('tmt.db', path.join(state, 'tmux-team.db'));
      expect(() => readCoreDatabase(state)).toThrow('invalid SQLite header');
    });
  });

  it('refuses distinct stores and a directory selected as the new store', () => {
    withState((_root, state) => {
      store(state, 'tmux-team.db');
      const current = store(state, 'tmt.db');
      expect(() => readCoreDatabase(state)).toThrow('separate or invalid');
      fs.unlinkSync(current);
      fs.mkdirSync(current);
      expect(() => readCoreDatabase(state)).toThrow('existing regular file');
    });
  });

  it.each(['-wal', '-shm'])(
    'allows released former-name %s only for the former selected store',
    (suffix) => {
      withState((_root, state) => {
        const former = store(state, 'tmux-team.db');
        const sidecar = path.join(state, `tmux-team.db${suffix}`);
        fs.writeFileSync(sidecar, 'released sidecar');
        expect(readCoreDatabase(state)).toBe(former);
        store(state, 'tmt.db');
        fs.unlinkSync(former);
        fs.symlinkSync('tmt.db', former);
        expect(() => readCoreDatabase(state)).toThrow('Unexpected database sidecar');
        fs.unlinkSync(sidecar);
        fs.writeFileSync(path.join(state, `tmt.db${suffix}`), 'current sidecar');
        expect(readCoreDatabase(state)).toBe(path.join(state, 'tmt.db'));
      });
    }
  );

  it.each(['-wal', '-shm', '-journal'])('refuses stray new-name %s without tmt.db', (suffix) => {
    withState((_root, state) => {
      store(state, 'tmux-team.db');
      fs.writeFileSync(path.join(state, `tmt.db${suffix}`), 'stray');
      expect(() => readCoreDatabase(state)).toThrow('Unexpected database sidecar');
    });
  });

  it('does not ignore a former transaction journal', () => {
    withState((_root, state) => {
      store(state, 'tmux-team.db');
      fs.writeFileSync(path.join(state, 'tmux-team.db-journal'), 'transaction');
      expect(() => readCoreDatabase(state)).toThrow('Unexpected database sidecar');
    });
  });

  it('requires both names absent, without creating a directory or a database', () => {
    withState((root, state) => {
      const missing = path.join(root, 'missing');
      expect(() => assertNoCoreDatabase(missing, 'must not open SQLite')).not.toThrow();
      expect(fs.existsSync(missing)).toBe(false);
      expect(() => assertNoCoreDatabase(state, 'must not open SQLite')).not.toThrow();
      for (const name of ['tmux-team.db', 'tmt.db']) {
        const file = store(state, name);
        expect(() => assertNoCoreDatabase(state, 'must not open SQLite')).toThrow(
          'must not open SQLite'
        );
        fs.unlinkSync(file);
      }
      expect(fs.readdirSync(state)).toEqual([]);
    });
  });
});
