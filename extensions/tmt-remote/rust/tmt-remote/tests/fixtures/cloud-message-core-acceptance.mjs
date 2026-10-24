// Explicit public-command proof. The Rust scenario owns the real serving owner and transport.
// Build this checkout's CLI, Remote, Herdr and runtime-caller-fixture first; use Node 26.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const config = JSON.parse(fs.readFileSync(0, 'utf8'));
const rootUrl = new URL('../../../../../../', import.meta.url);
const root = fileURLToPath(rootUrl);
const support = new URL('typescript/test/support/', rootUrl);
const { withSandbox, runCli } = await import(new URL('cli-process.ts', support));
const { createArtifact } = await import(new URL('native-artifact.ts', support));
const { writeExecutable } = await import(new URL('executable-fixture.mjs', support));
process.chdir(path.join(root, 'typescript'));
assert.equal(config.remoteExecutable, path.join(root, 'rust/target/debug/tmt-remote'));
await withSandbox(async (sandbox) => {
  assert.equal(sandbox.cli.executable, path.join(root, 'rust/target/debug/tmt'));
  const json = async (args, options = {}) => {
    const result = await runCli(sandbox, [...args, '--json'], options);
    assert.equal(result.status, 0, result.stderr);
    return JSON.parse(result.stdout);
  };
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
  await json(['identity', 'create', 'public-cloud-origin']);
  const bin = path.join(sandbox.root, 'proof-bin');
  fs.mkdirSync(bin);
  // Only discovery is fixture-owned. The exact Core-prepared bytes go to the actual Remote CLI.
  // The capture is private evidence for same-ID recovery, never stdout, logs or a cloud field.
  writeExecutable(
    path.join(bin, 'tmt-remote'),
    `#!${process.execPath}
import fs from 'node:fs';
import { spawn } from 'node:child_process';
const bytes = fs.readFileSync(0);
fs.writeFileSync(${JSON.stringify(config.capture)}, bytes, {mode: 0o600});
const child = spawn(${JSON.stringify(config.remoteExecutable)}, process.argv.slice(2), {
  env: {...process.env, TMT_EXECUTABLE: ${JSON.stringify(config.discovery)}, TMT_AGENT: '1', BROWSER: '/usr/bin/true'},
  stdio: ['pipe', 'inherit', 'inherit']
});
child.stdin.end(bytes);
child.once('error', () => process.exit(1));
child.once('exit', (code, signal) => process.exit(signal === null ? (code ?? 1) : 1));
`
  );
  sandbox.env.PATH = [bin, sandbox.env.PATH].join(path.delimiter);
  const result = await runCli(
    sandbox,
    [
      'talk',
      config.target,
      'Public one-way cloud proof',
      '--expect-name',
      'target',
      '--remote',
      config.peer,
      '--identity',
      'public-cloud-origin',
      '--json',
    ],
    { deadlineMs: 70_000 }
  );
  const answer = JSON.parse(result.stdout);
  if (config.state === 'sent' || config.state === 'held') {
    assert.equal(result.status, 0, result.stderr);
    assert.equal(answer.status, config.state === 'held' ? 'queued' : 'sent');
  } else {
    assert.notEqual(result.status, 0);
    assert.equal(
      answer.error.code,
      config.state === 'unconfirmed' ? 'REMOTE_UNCONFIRMED' : 'REMOTE_REFUSED'
    );
    if (config.state === 'unconfirmed') assert.equal(answer.deliveryState, 'uncertain');
  }
  const request = JSON.parse(fs.readFileSync(config.capture, 'utf8'));
  assert.equal(request.reply.requestId, `req_${request.requestId}`);
  assert.equal(request.remote, config.peer);
  assert.equal(request.target.id, config.target);
  const observed = await runCli(sandbox, ['api'], {
    stdin: JSON.stringify({
      version: 1,
      operation: 'requests.show',
      input: { requestId: request.reply.requestId },
    }),
  });
  assert.equal(observed.status, 0, observed.stderr);
  const state = JSON.parse(observed.stdout);
  assert.equal(
    state.delivery,
    config.state === 'unconfirmed'
      ? 'uncertain'
      : config.state === 'held'
        ? 'queued'
        : config.state === 'refused'
          ? 'definitely_failed'
          : 'sent'
  );
  if (config.state === 'unconfirmed') {
    assert.equal(state.messageState.state, 'unconfirmed');
    assert.equal(state.messageState.canSendNow, false);
  }
  process.stdout.write(
    JSON.stringify({
      requestId: request.reply.requestId,
      status: answer.status ?? null,
      code: answer.error?.code ?? null,
      deliveryState: answer.deliveryState ?? null,
      messageState: state.messageState,
    }) + '\n'
  );
});
