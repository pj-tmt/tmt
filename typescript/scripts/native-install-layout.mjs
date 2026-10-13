import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

export const CLI_NAMESPACE = 'lib/tmt';
export const LEGACY_CLI_NAMESPACE = 'lib/tmux-team';

/** Observe the managed command's active release, never a directory-precedence fallback. */
export function readCliLayout(installationPrefix, inspectReceiptText = (text) => text) {
  const prefix = fs.realpathSync(installationPrefix);
  const bin = path.join(prefix, 'bin');
  assert.equal(fs.realpathSync(bin), bin, 'Active CLI link escapes owned prefix.');
  const command = path.join(bin, 'tmt');
  assert(fs.lstatSync(command).isSymbolicLink(), 'Active CLI command must be a managed link.');
  const target = path.resolve(bin, fs.readlinkSync(command));
  assert(target.startsWith(`${prefix}${path.sep}`), 'Active CLI link escapes owned prefix.');
  const namespace = [CLI_NAMESPACE, LEGACY_CLI_NAMESPACE].find(
    (name) => target === path.join(prefix, name, 'current/tmt')
  );
  assert(namespace, 'Active CLI link names an unknown namespace.');
  const root = path.join(prefix, namespace);
  assert.equal(fs.realpathSync(root), root, 'Active CLI namespace escapes owned prefix.');
  const current = path.join(root, 'current');
  assert(fs.lstatSync(current).isSymbolicLink(), 'Active CLI current must be a release link.');
  const relative = fs.readlinkSync(current);
  const match = /^releases\/([^/]+)$/.exec(relative);
  assert(match && match[1] !== '.' && match[1] !== '..', 'Active receipt escapes owned prefix.');
  const release = path.join(root, relative);
  assert.equal(fs.realpathSync(current), release, 'Active receipt escapes owned prefix.');
  const executable = path.join(release, 'tmt');
  assert.equal(fs.realpathSync(command), executable, 'Active CLI executable/current mismatch.');
  assert(fs.lstatSync(executable).isFile(), 'Active CLI executable must be a regular file.');
  const receiptFile = path.join(release, 'receipt.json');
  assert(fs.lstatSync(receiptFile).isFile(), 'Active CLI receipt must be a regular file.');
  const receipt = JSON.parse(inspectReceiptText(fs.readFileSync(receiptFile, 'utf8')));
  assert(
    receipt !== null &&
      typeof receipt === 'object' &&
      !Array.isArray(receipt) &&
      receipt.schema_version === 1 &&
      receipt.prefix === prefix &&
      receipt.release_id === match[1] &&
      typeof receipt.version === 'string' &&
      typeof receipt.channel === 'string',
    'Active CLI receipt does not match the selected release.'
  );
  return { prefix, namespace, root, current, release, executable, receipt };
}
