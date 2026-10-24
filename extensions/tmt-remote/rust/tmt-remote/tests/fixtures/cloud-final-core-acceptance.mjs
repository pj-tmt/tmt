// Explicit native acceptance, not a normal package test or an emulator runner.
// Build tmt-cli, tmt-remote and runtime-caller-fixture first; run with Node 26.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const rootUrl = new URL('../../../../../../', import.meta.url);
const root = fileURLToPath(rootUrl);
const support = new URL('typescript/test/support/', rootUrl);
const { withSandbox, runCli } = await import(new URL('cli-process.ts', support));
const { createArtifact } = await import(new URL('native-artifact.ts', support));
const { writeExecutable } = await import(new URL('executable-fixture.mjs', support));
const { runPackedCommand } = await import(
  new URL('typescript/scripts/packed-command.mjs', rootUrl)
);
process.chdir(path.join(root, 'typescript'));

for (const response of ['', ' \r\nBOM:\uFEFF NUL:\0 Unicode: π\n ']) {
  let usedRoot;
  await withSandbox(async (sandbox) => {
    usedRoot = sandbox.root;
    const json = async (args, options) => {
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
    const prefix = path.join(sandbox.home, '.local');
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
        prefix,
        '--channel',
        'alpha',
      ],
      { deadlineMs: 20_000 }
    );
    await json(['identity', 'create', 'core-final-origin']);

    // Capture the exact public Core request at its extension-process boundary.
    // This fixture does not claim to exercise the original carrier publication.
    const bin = path.join(sandbox.root, 'capture-bin');
    fs.mkdirSync(bin);
    const captured = path.join(sandbox.root, 'captured-request.json');
    writeExecutable(
      path.join(bin, 'tmt-remote'),
      `#!${process.execPath}
import fs from 'node:fs';
const bytes = fs.readFileSync(0);
fs.writeFileSync(${JSON.stringify(captured)}, bytes, { mode: 0o600 });
const request = JSON.parse(bytes);
process.stdout.write(JSON.stringify({ version: 1, state: 'sent', remote: request.remote, target: request.target }));
`
    );
    sandbox.env.PATH = [bin, path.join(prefix, 'bin'), sandbox.env.PATH].join(path.delimiter);
    const sent = await json([
      'talk',
      'target',
      'Core final receipt acceptance',
      '--remote',
      'paired',
      '--identity',
      'core-final-origin',
    ]);
    const request = JSON.parse(fs.readFileSync(captured, 'utf8'));
    assert.equal(request.reply.requestId, sent.requestId);
    assert.equal(request.requestId, sent.requestId.slice(4));

    const calls = path.join(sandbox.root, 'core-command-names');
    const wrapper = path.join(sandbox.root, 'actual-core');
    // Exact forwarding only. No receipt, argv or body is logged. The existing
    // invocation owner bounds and reaps this wrapper and its non-detached child.
    // Core runs in that owner's private cwd, including test-isolated Core builds.
    writeExecutable(
      wrapper,
      `#!${process.execPath}
import fs from 'node:fs';
import { spawn } from 'node:child_process';
fs.appendFileSync(${JSON.stringify(calls)}, process.argv[2] + '\\n', { mode: 0o600 });
const child = spawn(${JSON.stringify(sandbox.cli.executable)}, [...${JSON.stringify(sandbox.cli.args)}, ...process.argv.slice(2)], { cwd: ${JSON.stringify(sandbox.cwd)}, env: ${JSON.stringify(sandbox.env)}, stdio: 'inherit' });
child.once('error', () => process.exit(1));
child.once('exit', (code, signal) => process.exit(signal === null ? (code ?? 1) : 1));
`
    );
    const proof = path.join(sandbox.root, 'private-final-proof.json');
    fs.writeFileSync(proof, JSON.stringify({ request, response, core: wrapper, calls }), {
      mode: 0o600,
    });
    const output = runPackedCommand(
      '/bin/sh',
      [
        '-c',
        'exec cargo "$@" 2>&1',
        'core-final-acceptance',
        'test',
        '--offline',
        '--locked',
        '-p',
        'tmt-remote',
        '--lib',
        'cloud_inbox::finals::tests::pushed_final_submits_a_real_core_request_receipt_once',
        '--',
        '--exact',
        '--ignored',
        '--nocapture',
        '--test-threads=1',
      ],
      {
        cwd: path.join(root, 'rust'),
        env: { ...process.env, CARGO_BUILD_JOBS: '2', TMT_REMOTE_FINAL_CORE_PROOF: proof },
        timeoutMs: 180_000,
      }
    );
    assert.match(output, /running 1 test/);
    assert.match(output, /1 passed; 0 failed; 0 ignored/);
    assert.equal((await json(['result', sent.requestId])).response, response);

    // Core remains the immutable final owner after Remote's deduplicated ingress.
    await json(['reply', sent.requestId, '--receipt', request.reply.receipt, '--stdin'], {
      stdin: response,
    });
    const conflict = await runCli(
      sandbox,
      ['reply', sent.requestId, '--receipt', request.reply.receipt, '--stdin', '--json'],
      { stdin: 'conflicting final' }
    );
    assert.equal(conflict.status, 5);
    assert.equal(JSON.parse(conflict.stdout).error.code, 'RESPONSE_CONFLICT');
    assert.equal((await json(['result', sent.requestId])).response, response);
  });
  assert.equal(
    fs.existsSync(usedRoot),
    false,
    'the existing sandbox owner removed its private root'
  );
  console.log(
    JSON.stringify({
      bodyBytes: Buffer.byteLength(response),
      finalIngress: 'passed',
      coreResult: 'exact',
      duplicateAfterReopen: 'one reply call',
      conflict: 'refused',
      cleanup: 'absent',
    })
  );
}
