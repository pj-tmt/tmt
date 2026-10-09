import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { bootstrapTools } from './native-bootstrap-proof.mjs';
import { runPackedCommand } from './packed-command.mjs';
import { assertMacOsArchitecture } from './native-runtime-proof.mjs';
import { checkMigratedState, snapshotState, writePriorState } from './migrated-state.mjs';

export const HANDOFF_PROBE = ['__native-install', '--handoff-version', '1', '--probe', '--json'];

/** Candidate source owns historical applicability; malformed/new protocols never become legacy. */
export function hasInstallerHandoff(sourceRoot) {
  const directory = new URL('rust/crates/tmt-adapters/src/native_install/', sourceRoot);
  assert(fs.statSync(directory).isDirectory(), 'Candidate installer source is required');
  const marker = new URL('handoff.rs', directory);
  if (!fs.existsSync(marker)) return false;
  assert.match(
    fs.readFileSync(marker, 'utf8'),
    /pub const VERSION: u32 = 1;/,
    'Candidate source must declare installer handoff protocol 1'
  );
  return true;
}

/** Exact protocol response: malformed output and unrelated failures never identify a legacy source. */
export function installerProtocol(executable, options, execute = runPackedCommand) {
  try {
    const output = execute(executable, HANDOFF_PROBE, options);
    assert(
      output === '{"protocol":1}' || output === '{"protocol":1}\n',
      'Installer must answer exactly {"protocol":1}'
    );
    return 'handoff';
  } catch (error) {
    const cause = error.cause;
    if (cause?.status === 1 && cause.stderr === '') {
      const failure = JSON.parse(cause.stdout);
      if (
        failure.error?.code === 'USAGE_ERROR' &&
        typeof failure.error.message === 'string' &&
        failure.error.message.startsWith("error: unexpected argument '--handoff-version' found\n")
      )
        return 'legacy';
    }
    throw error;
  }
}

export function requireInstallerProtocol(executable, options, execute = runPackedCommand) {
  assert.equal(
    installerProtocol(executable, options, execute),
    'handoff',
    'Candidate must support installer handoff protocol 1 before publication'
  );
}

/** Only the historical inventory rejection satisfies the recovery control. */
export function requireLegacyInventoryError(result) {
  assert.equal(result.error?.code, 'NATIVE_INSTALL_FAILED');
  assert.equal(result.error.message, 'Unexpected native archive asset inventory.');
}

/** Real source executable and receipt, then real candidate bootstrap; no public upgrade or delegation claim. */
export function proveSourceBootstrap({
  root,
  source,
  current,
  previous,
  bootstrap,
  archive,
  manifest,
  previousArchive,
  previousManifest,
  expectedMigrations,
  report = console.log,
}) {
  const started = performance.now();
  fs.mkdirSync(root);
  const prefix = path.join(root, 'source prefix with spaces');
  const state = path.join(root, 'source state');
  // Harness only: remove when the prior release is post-rename.
  const env = {
    HOME: root,
    TMT_HOME: state,
    TMUX_TEAM_HOME: state,
    TMPDIR: root,
    PATH: '',
    LANG: 'C',
  };
  const options = { cwd: root, env };
  const installed = path.join(prefix, 'bin/tmt');
  const pointer = path.join(prefix, 'lib/tmux-team/current');
  const sourceInstall = (inputArchive, inputManifest, expectedStatus = 0) =>
    JSON.parse(
      runPackedCommand(
        source,
        [
          '__native-install',
          '--archive',
          path.resolve(inputArchive),
          '--manifest',
          path.resolve(inputManifest),
          '--prefix',
          prefix,
          '--channel',
          'alpha',
          '--json',
        ],
        { ...options, expectedStatus }
      )
    );
  assert.equal(sourceInstall(previousArchive, previousManifest).changed, true);
  assertMacOsArchitecture(installed, previous.target, options);
  assert.equal(runPackedCommand(installed, ['--version'], options).trim(), previous.version);
  const protocol = installerProtocol(installed, options);
  const run = (args) => runPackedCommand(installed, args, options);
  const ids = writePriorState(run);
  const database = path.join(state, 'tmux-team.db');
  const databaseBytes = fs.readFileSync(database);
  const before = snapshotState(database, ids);
  assert.deepEqual(before.missing, []);
  const priorPointer = fs.readlinkSync(pointer);
  const priorReceipt = fs.readFileSync(path.join(pointer, 'receipt.json'));
  const priorBytes = fs.readFileSync(installed);
  const installationInventory = () =>
    fs
      .readdirSync(prefix, { recursive: true })
      .sort()
      .map((name) => {
        const file = path.join(prefix, name);
        const stat = fs.lstatSync(file);
        if (stat.isSymbolicLink()) return [name, 'symlink', fs.readlinkSync(file)];
        if (stat.isDirectory()) return [name, 'directory'];
        assert(stat.isFile(), 'Installation contains an unexpected file type');
        return [
          name,
          'file',
          stat.mode,
          createHash('sha256').update(fs.readFileSync(file)).digest('hex'),
        ];
      });
  const priorInventory = installationInventory();
  // Offline installation keeps the source's inventory policy even after #1454.
  // A protocol-capable source's self-upgrade delegation is proved by adapter acceptance.
  const inventoryChanged = (current.companions ?? []).some(
    (name) => !(previous.companions ?? []).includes(name)
  );
  if (inventoryChanged) {
    requireLegacyInventoryError(sourceInstall(archive, manifest, 1));
    assert.equal(fs.readlinkSync(pointer), priorPointer);
    assert(
      fs.readFileSync(installed).equals(priorBytes),
      'Rejected installation changed old bytes'
    );
    assert(fs.readFileSync(path.join(pointer, 'receipt.json')).equals(priorReceipt));
    assert.deepEqual(
      installationInventory(),
      priorInventory,
      'Rejected installation left partial files or changed permissions'
    );
    report(
      `Source offline inventory rejection verified: ${previous.version}; ${protocol} source (not self-upgrade).`
    );
  } else {
    assert.equal(sourceInstall(archive, manifest).changed, true);
    report(
      `Source offline installation verified: ${previous.version}; ${protocol} source (not delegation).`
    );
  }
  assert(
    fs.readFileSync(database).equals(databaseBytes),
    'Source installer changed application state'
  );
  const tools = bootstrapTools(root);
  runPackedCommand(
    '/bin/sh',
    [path.resolve(bootstrap), '--prefix', prefix, '--no-skill', '--no-setup'],
    {
      ...options,
      timeoutMs: 30_000,
      env: {
        ...env,
        PATH: tools,
        TMT_FIXTURE_VERSION: current.version,
        TMT_FIXTURE_NAME: current.name,
        TMT_FIXTURE_MANIFEST: path.resolve(manifest),
        TMT_FIXTURE_ARCHIVE: path.resolve(archive),
      },
    }
  );
  assert.equal(run(['--version']).trim(), current.version);
  assertMacOsArchitecture(installed, current.target, options);
  assert(fs.readFileSync(database).equals(databaseBytes), 'Bootstrap changed application state');
  const oldExecutable = path.join(prefix, 'lib/tmux-team', priorPointer, 'tmt');
  assert(fs.readFileSync(oldExecutable).equals(priorBytes), 'Bootstrap lost old executable');
  run(['identity', 'ls', '--json']);
  assert.deepEqual(
    checkMigratedState({
      before,
      after: snapshotState(database, ids),
      expected: expectedMigrations,
    }),
    []
  );
  assert(
    !fs.readdirSync(root).some((name) => name.startsWith('tmt-bootstrap.')),
    'Bootstrap staging leaked'
  );
  report(
    `Source/bootstrap proof: ${previous.version} -> ${current.version} (${current.target}), ${Math.ceil((performance.now() - started) / 1000)} seconds; curl acquisition injected.`
  );
}
