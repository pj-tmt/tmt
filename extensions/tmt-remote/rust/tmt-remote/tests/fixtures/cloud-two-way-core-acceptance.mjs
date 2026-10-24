// Explicit composed acceptance. The Rust fixture owns the two real Remote owners.
// Build ordinary Core/Herdr and runtime-caller-fixture separately, then feature Remote.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const rootUrl = new URL('../../../../../../', import.meta.url);
const root = fileURLToPath(rootUrl);
const support = new URL('typescript/test/support/', rootUrl);
const { withSandbox, runCli } = await import(new URL('cli-process.ts', support));
const { createArtifact } = await import(new URL('native-artifact.ts', support));
const { writeExecutable } = await import(new URL('executable-fixture.mjs', support));
const { default: Database } = await import(new URL('sqlite-oracle.ts', support));
const { runPackedCommand } = await import(
  new URL('typescript/scripts/packed-command.mjs', rootUrl)
);
process.chdir(path.join(root, 'typescript'));

for (const key of ['FIREBASE_AUTH_EMULATOR_HOST', 'FIRESTORE_EMULATOR_HOST']) {
  assert.match(process.env[key] ?? '', /^127\.0\.0\.1:[1-9][0-9]{0,4}$/);
  assert.ok(Number(process.env[key].split(':')[1]) <= 65535);
}
const remote = path.join(root, 'rust/target/debug/tmt-remote');
assert.ok(fs.statSync(remote).isFile());
const removed = [];
let summary;
await withSandbox(async (a) => {
  await withSandbox(async (b) => {
    async function world(sandbox, name) {
      removed.push(sandbox.root);
      assert.equal(sandbox.cli.executable, path.join(root, 'rust/target/debug/tmt'));
      assert.ok(
        Buffer.byteLength(path.join(sandbox.globalDir, 'remote/control.sock')) < 100,
        'Run this proof with a short private TMPDIR'
      );
      const json = async (args, options = {}) => {
        const answer = await runCli(sandbox, [...args, '--json'], options);
        assert.equal(answer.status, 0, answer.stderr);
        return JSON.parse(answer.stdout);
      };
      // The archive supplies Core's public protocol receipt. PATH selects the actual
      // built Remote, just as the maintained one-way public-command acceptance does.
      const artifact = await createArtifact(
        sandbox,
        '0.1.0-alpha.1',
        new Uint8Array(),
        'remote',
        {},
        undefined,
        JSON.stringify({ version: 1, messageProtocols: [1], uses: [] })
      );
      await json(
        [
          'extension',
          'install',
          'remote',
          '--yes',
          '--archive',
          artifact.archive,
          '--manifest',
          artifact.manifest,
          '--prefix',
          path.join(sandbox.home, '.local'),
          '--channel',
          'alpha',
        ],
        { deadlineMs: 20_000 }
      );
      await json(['identity', 'create', name]);
      const identity = await json(['identity', 'show', name]);
      const bin = path.join(sandbox.root, 'public-bin');
      fs.mkdirSync(bin);
      fs.symlinkSync(remote, path.join(bin, 'tmt-remote'));
      sandbox.env.PATH = [bin, sandbox.env.PATH].join(path.delimiter);
      const core = path.join(sandbox.root, 'actual-core');
      // Exact forwarding only: no synthetic discovery, dispatch, reply or result.
      // Bind every nested Core invocation to this world's private cwd and environment.
      writeExecutable(
        core,
        `#!${process.execPath}
import { spawn } from 'node:child_process';
const child = spawn(${JSON.stringify(sandbox.cli.executable)}, [...${JSON.stringify(sandbox.cli.args)}, ...process.argv.slice(2)], {
  cwd: ${JSON.stringify(sandbox.cwd)}, env: ${JSON.stringify(sandbox.env)}, stdio: 'inherit'
});
child.once('error', () => process.exit(1));
child.once('exit', (code, signal) => process.exit(signal === null ? (code ?? 1) : 1));
`
      );
      return { root: sandbox.root, data: sandbox.globalDir, core, name, id: identity.identity.id };
    }
    const config = path.join(a.root, 'private-two-way-proof.json');
    const report = path.join(a.root, 'private-two-way-result.json');
    fs.writeFileSync(
      config,
      JSON.stringify({ a: await world(a, 'public-a'), b: await world(b, 'public-b'), report }),
      { mode: 0o600 }
    );
    const output = runPackedCommand(
      '/bin/sh',
      [
        '-c',
        'exec cargo "$@" 2>&1',
        'public-two-way-proof',
        'test',
        '--offline',
        '--locked',
        '-p',
        'tmt-remote',
        '--features',
        'test-emulator',
        '--test',
        'cloud_public_two_way_emulator',
        'public_core_talk_and_exact_finals_both_directions',
        '--',
        '--exact',
        '--ignored',
        '--nocapture',
        '--test-threads=1',
      ],
      {
        cwd: path.join(root, 'rust'),
        env: {
          ...process.env,
          TMT_CLOUD_TWO_WAY_PROOF: config,
          CARGO_BUILD_JOBS: '2',
          CARGO_TARGET_DIR: path.join(root, 'rust/target'),
          TMT_AGENT: '1',
          BROWSER: '/usr/bin/true',
        },
        timeoutMs: 600_000,
        maxBuffer: 1024 * 1024,
      }
    );
    assert.match(output, /test result: ok/);
    const result = JSON.parse(fs.readFileSync(report, 'utf8'));
    assert.equal(result.completedDirections, 2);
    assert.equal(result.reverseDefaultRefused, true);
    assert.equal(result.ownersReleased, true);
    const receiverDb = new Database(a.database, { readonly: true });
    try {
      assert.equal(
        receiverDb
          .prepare('SELECT count(*) AS n FROM request_attempts WHERE instr(message_text, ?) > 0')
          .get(result.refusedMarker).n,
        0,
        'the refused reverse intent must stay refused after enabling the new scope and reopening'
      );
    } finally {
      receiverDb.close();
    }
    // Independent read-only SQL corroborates exactly one causal recipient effect,
    // rather than treating an empty pending inbox as an all-time dispatch count.
    for (const final of result.finals) {
      for (const [sandbox, requestId] of final.senderName === 'public-a'
        ? [
            [a, final.original],
            [b, final.adopted],
          ]
        : [
            [b, final.original],
            [a, final.adopted],
          ]) {
        const db = new Database(sandbox.database, { readonly: true });
        try {
          assert.equal(
            db
              .prepare(
                'SELECT count(*) AS n FROM request_attempts WHERE instr(message_text, ?) > 0'
              )
              .get(final.marker).n,
            1
          );
          const stored = db
            .prepare('SELECT body, body_bytes FROM request_responses WHERE request_id = ?')
            .get(requestId);
          assert.equal(stored.body_bytes, final.bodyBytes);
          assert.equal(
            createHash('sha256').update(Buffer.from(stored.body, 'utf8')).digest('hex'),
            final.sha256
          );
        } finally {
          db.close();
        }
      }
    }
    const { refusedMarker, ...safeResult } = result;
    assert.ok(refusedMarker);
    summary = {
      ...safeResult,
      finals: result.finals.map(({ bodyBytes, sha256, senderName, initialDelivery }) => ({
        direction: senderName === 'public-a' ? 'A-to-B' : 'B-to-A',
        bodyBytes,
        sha256,
        initialDelivery,
      })),
    };
  });
});
for (const directory of removed) assert.equal(fs.existsSync(directory), false);
assert.equal(removed.length, 2);
process.stdout.write(JSON.stringify({ ...summary, sandboxRootsAbsent: true }) + '\n');
