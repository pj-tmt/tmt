import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const DATABASE = 'tmt.db';
const FORMER_DATABASE = 'tmux-team.db';
const SIDECARS = ['-wal', '-shm', '-journal'];

function present(file) {
  try {
    return fs.lstatSync(file);
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw error;
  }
}

function ownedDirectory(directory) {
  assert(fs.lstatSync(directory).isDirectory(), 'Database state must be an owned directory.');
  return fs.realpathSync(directory);
}

function noSidecars(directory, name) {
  for (const suffix of SIDECARS)
    assert(
      !present(path.join(directory, name + suffix)),
      `Unexpected database sidecar: ${name + suffix}`
    );
}

/** Observe the selected existing store without opening SQLite or changing its files. */
export function readCoreDatabase(directory) {
  const root = ownedDirectory(directory);
  const current = path.join(root, DATABASE);
  const former = path.join(root, FORMER_DATABASE);
  const currentStat = present(current);
  const formerStat = present(former);
  if (!currentStat) noSidecars(root, DATABASE);
  const selected = currentStat ? current : former;
  const selectedStat = currentStat ?? formerStat;
  assert(selectedStat?.isFile(), 'Selected Core database must be an existing regular file.');
  if (currentStat) {
    if (formerStat?.isSymbolicLink())
      assert.equal(
        fs.readlinkSync(former),
        DATABASE,
        'Former database link must name tmt.db relatively.'
      );
    else if (formerStat)
      assert(
        formerStat.isFile() &&
          formerStat.dev === currentStat.dev &&
          formerStat.ino === currentStat.ino,
        'Database names identify separate or invalid stores.'
      );
    noSidecars(root, FORMER_DATABASE);
  } else {
    assert(!present(`${former}-journal`), 'Unexpected database sidecar: tmux-team.db-journal');
  }
  // Do not follow a replacement link between metadata observation and bounded header read.
  const descriptor = fs.openSync(selected, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  try {
    assert(fs.fstatSync(descriptor).isFile(), 'Selected Core database must be a regular file.');
    const header = Buffer.alloc(16);
    assert(
      fs.readSync(descriptor, header, 0, header.length, 0) === header.length &&
        header.toString() === 'SQLite format 3\0',
      'Selected Core database has an invalid SQLite header.'
    );
  } finally {
    fs.closeSync(descriptor);
  }
  return selected;
}

/** Absence checks must not miss a new-name store or a dangling database link. */
export function assertNoCoreDatabase(directory, message) {
  if (!present(directory)) return;
  const root = ownedDirectory(directory);
  for (const name of [DATABASE, FORMER_DATABASE]) {
    assert(!present(path.join(root, name)), message);
    noSidecars(root, name);
  }
}
