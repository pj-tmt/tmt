// Manual N=1 experiment. Reports never replace the required workspace worker.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { readCargoWorkspace, cargoWorkspaceCommand } from './cargo-workspace.mjs';
import { runPackedCommand } from './packed-command.mjs';
import {
  LIMITS,
  TOOLCHAIN,
  ROLES,
  cargoArgs,
  canonical,
  digest,
  exact,
  relativeFile,
  validateFiles,
  cargoOutput,
  admitInventory,
  enumeration,
  parseExecution,
  cargoCoverage,
  assignment,
  aggregate,
} from './workspace-proof.mjs';

const SYSTEM_TOOLS = [
  'tar',
  'ldd',
  'readelf',
  'ss',
  'git',
  'bash',
  'sh',
  'cc',
  'ld',
  'as',
  'rm',
].map((tool) => `/usr/bin/${tool}`);

export function hashFile(file) {
  const fd = fs.openSync(file, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  try {
    const stat = fs.fstatSync(fd);
    assert(
      stat.isFile() && stat.size <= LIMITS.fileBytes && !(stat.mode & 0o7000),
      'Bounded regular closure file without special permissions required'
    );
    const hash = createHash('sha256'),
      buffer = Buffer.alloc(1024 * 1024);
    let size = 0,
      count;
    while ((count = fs.readSync(fd, buffer, 0, buffer.length, null))) {
      size += count;
      assert(size <= LIMITS.fileBytes, 'File grew beyond bound');
      hash.update(buffer.subarray(0, count));
    }
    assert.equal(size, stat.size, 'File changed during hashing');
    return { size, mode: stat.mode & 0o777, sha256: hash.digest('hex') };
  } finally {
    fs.closeSync(fd);
  }
}
function largeHash(file, limit) {
  const fd = fs.openSync(file, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  try {
    const stat = fs.fstatSync(fd);
    assert(stat.isFile() && stat.size <= limit, 'Archive byte bound');
    const hash = createHash('sha256'),
      buffer = Buffer.alloc(1024 * 1024);
    let size = 0,
      count;
    while ((count = fs.readSync(fd, buffer, 0, buffer.length, null))) {
      size += count;
      assert(size <= limit, 'Archive grew');
      hash.update(buffer.subarray(0, count));
    }
    assert.equal(size, stat.size, 'Archive changed');
    return hash.digest('hex');
  } finally {
    fs.closeSync(fd);
  }
}
export function treeFiles(root) {
  const files = [];
  function visit(dir) {
    for (const entry of fs
      .readdirSync(dir, { withFileTypes: true })
      .sort((a, b) => a.name.localeCompare(b.name))) {
      const file = path.join(dir, entry.name);
      assert(!entry.isSymbolicLink(), `Closure symlink is unsupported: ${file}`);
      if (entry.isDirectory()) visit(file);
      else {
        assert(entry.isFile(), 'Special closure file');
        files.push({ path: relativeFile(path.relative(root, file)), ...hashFile(file) });
        assert(files.length <= LIMITS.files, 'Closure file count');
      }
    }
  }
  visit(root);
  validateFiles(files);
  return files;
}
export function verifyTree(root, expected) {
  exact(treeFiles(root), expected, 'Missing/extra/changed closure input');
}
export async function packClosure(root, file, files) {
  validateFiles(files);
  verifyTree(root, files);
  const tar = await import('tar');
  // Copying to the owned staging tree makes hard links independent regular files.
  await tar.c(
    { cwd: root, file, portable: true, noMtime: true, gzip: false, strict: true },
    files.map((entry) => entry.path)
  );
  return largeHash(file, LIMITS.archiveBytes);
}
export async function unpackClosure(file, expectedHash, files, destination) {
  validateFiles(files);
  assert(!fs.existsSync(destination), 'Extraction requires absent owned destination');
  exact(largeHash(file, LIMITS.archiveBytes), expectedHash, 'Archive hash mismatch');
  const tar = await import('tar'),
    expected = new Map(files.map((entry) => [entry.path, entry])),
    seen = new Set();
  tar.t({
    file,
    sync: true,
    strict: true,
    onReadEntry(entry) {
      relativeFile(entry.path);
      assert(!seen.has(entry.path), 'Duplicate archive entry');
      seen.add(entry.path);
      const wanted = expected.get(entry.path);
      assert(wanted && entry.type === 'File', 'Unexpected/non-regular archive entry');
      exact(
        [entry.size, entry.mode & 0o7777],
        [wanted.size, wanted.mode],
        'Archive entry size/mode mismatch'
      );
    },
  });
  exact([...seen].sort(), [...expected.keys()].sort(), 'Missing archive entry');
  fs.mkdirSync(destination);
  try {
    tar.x({ file, cwd: destination, sync: true, strict: true, preserveOwner: false });
    verifyTree(destination, files);
  } catch (error) {
    fs.rmSync(destination, { recursive: true, force: true });
    throw error;
  }
}

// Only the group created by this spawn is a signalling authority. Snapshot PIDs
// are detection evidence, never an ownership list. Close waits for inherited pipes.
export async function captureCommand(
  executable,
  args,
  {
    cwd,
    environment,
    input,
    outFile,
    errFile,
    executionMs,
    settlementMs = LIMITS.settlementMs,
    outputBytes = LIMITS.outputBytes,
    evidenceBytes = LIMITS.evidenceBytes,
    launch = spawn,
    signal = process.kill.bind(process),
  }
) {
  assert(executionMs > 0 && settlementMs > 0, 'No command settlement budget');
  const began = performance.now();
  const out = fs.openSync(outFile, 'wx'),
    err = fs.openSync(errFile, 'wx');
  let child,
    bytes = 0,
    reason = null,
    signalError = null;
  let exitObserved = false,
    closeObserved = false,
    streamsComplete = false,
    outputComplete = true;
  let code = null,
    termination = null;
  try {
    child = launch(executable, args, {
      cwd,
      env: environment,
      detached: true,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    await new Promise((resolve) => {
      let settled = false,
        settlementTimer;
      const finish = () => {
        if (settled) return;
        settled = true;
        clearTimeout(executionTimer);
        clearTimeout(settlementTimer);
        streamsComplete = closeObserved && child.stdout.readableEnded && child.stderr.readableEnded;
        // Closing our pipe handles does not establish that an escaped child died.
        child.stdin.destroy();
        child.stdout.destroy();
        child.stderr.destroy();
        child.unref();
        resolve();
      };
      const boundSettlement = () => {
        if (settled) return;
        settlementTimer ??= setTimeout(() => {
          reason ??= 'unconfirmed exit/pipe settlement';
          finish();
        }, settlementMs);
      };
      const stop = (why) => {
        if (settled) return;
        reason ??= why;
        if (child.pid) {
          try {
            signal(-child.pid, 'SIGKILL');
          } catch (error) {
            if (error.code !== 'ESRCH') signalError ??= error.message;
          }
        }
        boundSettlement();
      };
      const executionTimer = setTimeout(() => stop('deadline'), executionMs);
      function capture(fd, chunk) {
        if (settled) return;
        const available = Math.max(0, Math.min(outputBytes, evidenceBytes) - bytes);
        const kept = chunk.subarray(0, available);
        try {
          if (kept.length) fs.writeSync(fd, kept);
        } catch (error) {
          outputComplete = false;
          stop(`stream capture: ${error.message}`);
          return;
        }
        bytes += kept.length;
        if (chunk.length > available) {
          outputComplete = false;
          stop('output bound');
        }
      }
      child.stdout.on('data', (chunk) => capture(out, chunk));
      child.stderr.on('data', (chunk) => capture(err, chunk));
      for (const stream of [child.stdin, child.stdout, child.stderr])
        stream.on('error', (error) => {
          if (error.code !== 'EPIPE') stop(error.message);
        });
      child.once('error', (error) => stop(`spawn: ${error.message}`));
      child.once('exit', (status, receivedSignal) => {
        exitObserved = true;
        code = status;
        termination = receivedSignal;
        // Even a successful exit cannot wait indefinitely for inherited pipes.
        boundSettlement();
      });
      child.once('close', (status, receivedSignal) => {
        closeObserved = true;
        code = status;
        termination = receivedSignal;
        finish();
      });
      child.stdin.end(input);
    });
  } catch (error) {
    reason ??= `spawn/capture: ${error.message}`;
  } finally {
    fs.closeSync(out);
    fs.closeSync(err);
  }
  let groupAbsent = !child?.pid;
  if (child?.pid) {
    try {
      signal(-child.pid, 0);
    } catch (error) {
      if (error.code === 'ESRCH') groupAbsent = true;
      else signalError ??= error.message;
    }
    if (!groupAbsent) {
      reason ??= 'surviving/unconfirmed process group';
      try {
        signal(-child.pid, 'SIGKILL');
      } catch (error) {
        if (error.code !== 'ESRCH') signalError ??= error.message;
      }
    }
  }
  const settled = exitObserved && closeObserved && streamsComplete;
  const complete = settled && outputComplete;
  const cleanup = settled && groupAbsent && !signalError;
  if (!complete) reason ??= 'unconfirmed exit/pipe settlement';
  return {
    pid: child?.pid ?? null,
    processGroup: child?.pid ? -child.pid : null,
    status: code,
    signal: termination,
    reason,
    signalError,
    exitObserved,
    closeObserved,
    streamsComplete,
    outputComplete,
    complete,
    groupAbsent,
    cleanup,
    capturedBytes: bytes,
    elapsedMs: performance.now() - began,
    stdout: hashFile(outFile),
    stderr: hashFile(errFile),
  };
}

export function admitCleanup(before, after, listenersBefore, listenersAfter) {
  assert(Array.isArray(before) && Array.isArray(after), 'Missing process observations');
  for (const snapshot of [before, after]) {
    assert(
      snapshot.every((entry) => /^\d+$/.test(entry.pid) && /^\d+$/.test(entry.start)),
      'Unconfirmed process identity'
    );
    assert(
      new Set(snapshot.map((entry) => entry.pid)).size === snapshot.length,
      'Duplicate process observation'
    );
  }
  assert(
    after.every((entry) =>
      before.some((old) => old.pid === entry.pid && old.start === entry.start)
    ),
    'New surviving process after harness execution'
  );
  exact(listenersAfter, listenersBefore, 'Leaked/changed listener');
  return true;
}

async function main(role, directory) {
  assert(ROLES.includes(role) || role === 'aggregate', 'Unknown proof role');
  assert(process.platform === 'linux' && process.arch === 'x64', 'Proof requires Linux x86_64');
  const root = fs.realpathSync(path.join(path.dirname(fileURLToPath(import.meta.url)), '../..'));
  const rustRoot = path.join(root, 'rust'),
    output = path.resolve(directory);
  assert(
    path.isAbsolute(output) && !output.startsWith(`${root}/`),
    'Evidence must be outside source/target'
  );
  fs.mkdirSync(output, { recursive: true });
  const started = performance.now(),
    deadline = started + LIMITS.commandMs,
    workDeadline = deadline - LIMITS.cleanupMs,
    cleanupDeadline = deadline - LIMITS.settlementMs;
  const ownedBase = '/home/runner/work/_temp/tmt-workspace-proof';
  const cargoHome = `${ownedBase}/cargo`,
    target = path.join(rustRoot, 'target'),
    home = `${ownedBase}/home`,
    temporary = `${ownedBase}/tmp`;
  const setupEnv = {
    PATH: process.env.PATH,
    HOME: process.env.HOME,
    RUSTUP_HOME: process.env.RUSTUP_HOME ?? '/home/runner/.rustup',
    RUSTUP_TOOLCHAIN: TOOLCHAIN,
  };
  const toolchain = runPackedCommand('rustup', ['run', TOOLCHAIN, 'rustc', '--print', 'sysroot'], {
    cwd: rustRoot,
    env: setupEnv,
  }).trim();
  const cargo = path.join(toolchain, 'bin/cargo');
  const env = {
    PATH: `${toolchain}/bin:${path.dirname(process.execPath)}:/usr/local/bin:/usr/bin:/bin`,
    HOME: home,
    TMPDIR: temporary,
    LANG: 'C.UTF-8',
    TZ: 'UTC',
    RUSTUP_HOME: process.env.RUSTUP_HOME ?? '/home/runner/.rustup',
    RUSTUP_TOOLCHAIN: TOOLCHAIN,
    CARGO_HOME: cargoHome,
    CARGO_TARGET_DIR: target,
    CARGO_BUILD_JOBS: '2',
    CARGO_PROFILE_DEV_DEBUG: '0',
    CARGO_INCREMENTAL: '0',
    CARGO_TERM_COLOR: 'never',
    CARGO_NET_OFFLINE: 'true',
    CARGO: cargo,
    LD_LIBRARY_PATH: [
      path.join(target, 'debug/deps'),
      path.join(target, 'debug'),
      path.join(toolchain, 'lib'),
      path.join(toolchain, 'lib/rustlib/x86_64-unknown-linux-gnu/lib'),
    ].join(':'),
  };
  assert(
    /^\d+$/.test(process.env.GITHUB_RUN_ID ?? '') && process.env.GITHUB_RUN_ATTEMPT === '1',
    'Only one original workflow attempt is admitted'
  );
  assert(/^[a-f0-9]{40}$/.test(process.env.PROOF_HEAD ?? ''), 'Exact proof source SHA required');
  for (const dir of [target, cargoHome, home, temporary])
    assert(!fs.existsSync(dir), 'Job-owned roots must be fresh; never discard an existing cache');
  for (const dir of [home, temporary]) {
    assert(!fs.existsSync(dir), 'Owned runtime home/temp must be fresh');
    fs.mkdirSync(dir, { recursive: true });
  }
  const records = [];
  let sequence = 0,
    evidenceBytes = 0;
  async function command(
    label,
    executable,
    args,
    {
      cwd = rustRoot,
      input,
      network = false,
      expectedStatus = 0,
      extraEnv = {},
      timeoutMs = LIMITS.commandMs,
      cleanupPhase = false,
    } = {}
  ) {
    assert(
      performance.now() < (cleanupPhase ? cleanupDeadline : workDeadline) - LIMITS.settlementMs &&
        sequence < LIMITS.commands,
      'Proof phase deadline/command count'
    );
    const id = `${String(sequence++).padStart(3, '0')}-${label}`,
      outFile = path.join(output, `${id}.stdout`),
      errFile = path.join(output, `${id}.stderr`);
    const environment = { ...env, ...extraEnv, ...(network ? { CARGO_NET_OFFLINE: 'false' } : {}) };
    const result = await captureCommand(executable, args, {
      cwd,
      environment,
      input,
      outFile,
      errFile,
      executionMs: Math.max(
        1,
        Math.min(
          timeoutMs,
          (cleanupPhase ? cleanupDeadline : workDeadline) - performance.now() - LIMITS.settlementMs
        )
      ),
      evidenceBytes: LIMITS.evidenceBytes - evidenceBytes,
    });
    evidenceBytes += result.capturedBytes;
    const record = {
      id,
      executable,
      args,
      cwd,
      environment,
      input:
        input === undefined ? null : { bytes: Buffer.byteLength(input), sha256: digest(input) },
      network,
      ...result,
    };
    records.push(record);
    fs.writeFileSync(path.join(output, `${id}.process.json`), JSON.stringify(record, null, 2));
    assert(
      !result.reason &&
        result.complete &&
        result.cleanup &&
        result.signal === null &&
        result.status === expectedStatus,
      `Proof command failed: ${id}; original streams/process retained`
    );
    return { stdout: fs.readFileSync(outFile, 'utf8'), stderr: fs.readFileSync(errFile, 'utf8') };
  }
  const run = async (label, args, options) =>
    command(label, cargo, args[0]?.startsWith('+') ? args.slice(1) : args, options);
  const json = (name, value) =>
    fs.writeFileSync(path.join(output, name), JSON.stringify(value, null, 2) + '\n');
  const artifactIds = JSON.parse(process.env.PROOF_ARTIFACT_IDS ?? '{}');
  let report = { role, artifactIds, complete: false, cleanupVerified: false };
  let processesBefore, listenersBefore;
  try {
    processesBefore = processSnapshot();
    json('processes-before.json', processesBefore);
    listenersBefore = (await command('listeners-before', '/usr/bin/ss', ['-H', '-ltnup'])).stdout;
    const source = (
      await command('source', 'git', ['rev-parse', 'HEAD'], { cwd: root })
    ).stdout.trim();
    exact(source, process.env.PROOF_HEAD, 'Source head differs from launch plan');
    exact(
      (
        await command('status', 'git', ['status', '--porcelain', '--untracked-files=no'], {
          cwd: root,
        })
      ).stdout,
      '',
      'Dirty source'
    );
    const tracked = (await command('tracked', 'git', ['ls-files', '-z'], { cwd: root })).stdout
      .split('\0')
      .filter(Boolean)
      .sort();
    const sourceFiles = tracked.map((file) => ({
      path: relativeFile(file),
      ...hashFile(path.join(root, file)),
    }));
    validateFiles(sourceFiles);
    const versions = {};
    for (const tool of ['cargo', 'rustc', 'rustdoc'])
      versions[tool] = {
        path: path.join(toolchain, 'bin', tool),
        ...hashFile(path.join(toolchain, 'bin', tool)),
        version: (await command(`${tool}-version`, path.join(toolchain, 'bin', tool), ['-Vv']))
          .stdout,
      };
    assert(versions.rustc.version.includes(`rustc ${TOOLCHAIN} `), 'Pinned Rust version mismatch');
    const identity = {
      systemTools: Object.fromEntries(
        SYSTEM_TOOLS.map((file) => [file, hashFile(fs.realpathSync(file))])
      ),
      tooling: {
        node: process.version,
        executable: fs.realpathSync(process.execPath),
        nodeHash: hashFile(fs.realpathSync(process.execPath)).sha256,
        packageManager: runPackedCommand('pnpm', ['--version'], {
          cwd: root,
          env: setupEnv,
        }).trim(),
      },
      source,
      tree: (
        await command('tree', 'git', ['rev-parse', 'HEAD^{tree}'], { cwd: root })
      ).stdout.trim(),
      sourceHash: digest(canonical(sourceFiles)),
      run: process.env.GITHUB_RUN_ID,
      attempt: process.env.GITHUB_RUN_ATTEMPT,
      platform: {
        os: process.platform,
        arch: process.arch,
        image: process.env.ImageOS,
        imageVersion: process.env.ImageVersion,
        parallelism: os.availableParallelism(),
      },
      versions,
      env,
      root,
      target,
      cargoHome,
      sysrootHash: digest(canonical(toolFiles(toolchain))),
    };
    assert(
      identity.platform.image && identity.platform.imageVersion,
      'Runner image identity missing'
    );
    exact(identity.tooling.node, 'v22.23.2', 'Pinned Node mismatch');
    exact(identity.tooling.packageManager, '10.33.0', 'Pinned pnpm mismatch');
    report.identity = identity;
    json('source-files.json', sourceFiles);
    if (role === 'aggregate') {
      const inputs = ROLES.map((name) =>
        JSON.parse(fs.readFileSync(path.join(output, name, `${name}.json`), 'utf8'))
      );
      const results = JSON.parse(process.env.PROOF_RESULTS ?? '{}');
      exact(
        Object.keys(artifactIds).sort(),
        [...ROLES, 'closure'].sort(),
        'Missing/extra artifact identity'
      );
      assert(
        Object.values(artifactIds).every((id) => /^\d+$/.test(id)) &&
          new Set(Object.values(artifactIds)).size === 5,
        'Missing/duplicate artifact ID'
      );
      exact(
        inputs.find((input) => input.role === 'consumer').artifactIds,
        { producer: artifactIds.producer, closure: artifactIds.closure },
        'Consumer downloaded another producer/closure'
      );
      exact(
        inputs.find((input) => input.role === 'doctest').artifactIds,
        { producer: artifactIds.producer },
        'Doctest downloaded another producer'
      );
      for (const input of inputs)
        exact(input.identity, identity, 'Aggregate source/run/environment mismatch');
      for (const input of inputs) verifyReportFiles(path.join(output, input.role), input);
      report = { role, artifactIds, identity, ...aggregate(inputs, results) };
      return;
    }
    const expectedArtifacts =
      role === 'consumer' ? ['producer', 'closure'] : role === 'doctest' ? ['producer'] : [];
    exact(
      Object.keys(artifactIds).sort(),
      expectedArtifacts.sort(),
      'Missing/extra consumed artifact IDs'
    );
    assert(
      Object.values(artifactIds).every((id) => /^\d+$/.test(id)),
      'Missing consumed artifact ID'
    );
    // Setup/acquisition belongs to callers, never to offline metadata or test execution.
    const free = fs.statfsSync('/home/runner/work');
    const freeRequired = (role === 'producer' ? 12 : 6) * 1024 ** 3;
    assert(
      Number(free.bavail) * Number(free.bsize) >= freeRequired,
      `Insufficient owned proof disk: requires ${freeRequired} bytes free (consumer archive already downloaded)`
    );
    let bundle;
    if (role === 'consumer') {
      bundle = JSON.parse(fs.readFileSync(path.join(output, 'producer', 'bundle.json'), 'utf8'));
      exact(bundle.identity, identity, 'Restored environment/source/toolchain mismatch');
      assert(
        !fs.existsSync(target) && !fs.existsSync(cargoHome),
        'Consumer may not overlay a target/cache'
      );
      const extracted = path.join(output, 'restored');
      await unpackClosure(
        path.join(output, 'producer', 'closure.tar'),
        bundle.archiveHash,
        bundle.files,
        extracted
      );
      for (const [name, destination] of [
        ['target', target],
        ['cargo', cargoHome],
      ]) {
        fs.mkdirSync(path.dirname(destination), { recursive: true });
        fs.renameSync(path.join(extracted, name), destination);
      }
    } else {
      assert(
        !fs.existsSync(target) && !fs.existsSync(cargoHome),
        'Producer/baseline/doctest target/cache must be cold'
      );
      fs.mkdirSync(cargoHome, { recursive: true });
      await run(
        'fetch',
        [
          `+${TOOLCHAIN}`,
          'fetch',
          '--locked',
          '--manifest-path',
          path.join(rustRoot, 'Cargo.toml'),
        ],
        { network: true }
      );
    }
    const metadataCommand = cargoWorkspaceCommand(root);
    const metadata = await command('metadata', cargo, metadataCommand.args, {
      cwd: metadataCommand.options.cwd,
      timeoutMs: metadataCommand.options.timeoutMs,
    });
    assert(!metadata.stderr, 'Offline metadata diagnostics');
    const workspace = readCargoWorkspace(root, { runner: () => metadata.stdout });
    fs.writeFileSync(path.join(output, 'metadata.raw.json'), metadata.stdout);
    for (const pkg of workspace.packages)
      for (const target of pkg.targets)
        assert(target.src_path.startsWith(`${root}/`), 'Cargo target source escapes checkout');
    let inventory;
    async function listings(inv) {
      for (const harness of inv.harnesses) {
        assert(
          harness.executable.startsWith(`${target}/debug/`),
          'Executable escapes owned target'
        );
        const binary = hashFile(harness.executable);
        assert(binary.mode & 0o111, 'Harness lacks executable permission');
        harness.sha256 = binary.sha256;
        const ordinary = await command(
          'list',
          harness.executable,
          ['--list', '--format=pretty', '--color=never'],
          { cwd: harness.cwd }
        );
        const ignored = await command(
          'ignored',
          harness.executable,
          ['--list', '--ignored', '--format=pretty', '--color=never'],
          { cwd: harness.cwd }
        );
        assert(!ordinary.stderr && !ignored.stderr, 'Harness list diagnostics');
        harness.list = enumeration(ordinary.stdout, ignored.stdout);
      }
      assert(
        inv.harnesses
          .filter((harness) => harness.package === 'tmt-remote')
          .some((harness) => harness.list.names.length),
        'Workspace Remote listing is empty'
      );
      assert(
        inv.harnesses.find(
          (harness) => harness.package === 'tmt-cli' && harness.target.name === 'architecture'
        ).list.names.length,
        'Architecture listing is empty'
      );
      return inv;
    }
    const loadManifests = async () => {
      const manifests = {};
      const rootManifest = JSON.parse(
        (
          await command(
            'workspace-manifest',
            path.join(target, 'debug/release-version'),
            ['parse'],
            { input: fs.readFileSync(path.join(rustRoot, 'Cargo.toml')) }
          )
        ).stdout
      );
      for (const pkg of workspace.packages) {
        const parsed = await command(
          'manifest',
          path.join(target, 'debug/release-version'),
          ['parse'],
          { input: fs.readFileSync(pkg.manifestPath) }
        );
        assert(!parsed.stderr, 'TOML helper diagnostics');
        manifests[pkg.name] = JSON.parse(parsed.stdout);
        pkg.runtimeMetadata = Object.fromEntries(
          Object.entries(manifests[pkg.name].package).map(([key, value]) => [
            key,
            value?.workspace === true ? rootManifest.workspace.package[key] : value,
          ])
        );
      }
      return manifests;
    };
    if (role === 'producer' || role === 'baseline') {
      // Build first only in the proposed producer; baseline preserves test-before-build execution.
      if (role === 'producer') await run('build', cargoArgs('build'));
      else {
        const remote = await run('baseline-remote', [
          `+${TOOLCHAIN}`,
          'test',
          '--locked',
          '-p',
          'tmt-remote',
          '--',
          '--list',
        ]);
        assert(/: test$/m.test(remote.stdout), 'Original Remote listing is empty');
      }
      const compile = await run(
        role === 'producer' ? 'compile' : 'baseline-list',
        cargoArgs('test', [
          ...(role === 'producer' ? ['--no-run'] : []),
          '--message-format=json',
          ...(role === 'baseline' ? ['--', '--list'] : []),
        ])
      );
      const records = cargoOutput(compile.stdout, { mixed: role === 'baseline' }).records;
      if (role === 'baseline') {
        // Preparing only the existing private helper cannot overwrite an acceptance executable.
        await run('helper', [
          `+${TOOLCHAIN}`,
          'build',
          '--locked',
          '-p',
          'tmt-release-tool',
          '--bin',
          'release-version',
        ]);
      }
      inventory = await listings({
        ...admitInventory(records, workspace.packages, await loadManifests()),
        rustRoot,
        runtimeMetadata: Object.fromEntries(
          workspace.packages.map((pkg) => [pkg.name, pkg.runtimeMetadata])
        ),
        loaderPaths: records
          .filter((record) => record.reason === 'build-script-executed')
          .flatMap((record) => record.linked_paths ?? [])
          .map((value) => value.replace(/^[a-z-]+=/, ''))
          .sort(),
      });
      if (role === 'baseline') {
        const normal = cargoCoverage(compile.stdout, compile.stderr, inventory, 'list');
        const ignored = await run(
          'baseline-ignored',
          cargoArgs('test', ['--message-format=json', '--', '--list', '--ignored'])
        );
        const ignoredCoverage = cargoCoverage(
          ignored.stdout,
          ignored.stderr,
          inventory,
          'list',
          true
        );
        setDocLists(inventory, normal.docs, ignoredCoverage.docs);
        exact(
          normal.ordinary,
          inventory.harnesses
            .map((harness) => ({ id: harness.id, values: harness.list.names }))
            .sort((a, b) => a.id.localeCompare(b.id)),
          'Default binary list mismatch'
        );
        exact(
          ignoredCoverage.ordinary,
          inventory.harnesses
            .map((harness) => ({ id: harness.id, values: harness.list.ignored }))
            .sort((a, b) => a.id.localeCompare(b.id)),
          'Default ignored list mismatch'
        );
        const execution = await run(
          'baseline-execution',
          cargoArgs('test', ['--message-format=json'])
        );
        const coverage = cargoCoverage(execution.stdout, execution.stderr, inventory, 'execution');
        for (const harness of inventory.harnesses)
          exact(
            hashFile(harness.executable).sha256,
            harness.sha256,
            'Baseline binary changed before execution'
          );
        exact(
          coverage.ordinary.map((item) => item.id),
          inventory.harnesses.map((harness) => harness.id).sort(),
          'Default execution missing binary'
        );
        report.execution = coverage.ordinary;
        report.docs = {
          normal: normal.docs,
          ignored: ignoredCoverage.docs,
          execution: coverage.docs,
        };
        report.libraryFeatures = coverage.libraryFeatures;
        await run('baseline-build', cargoArgs('build'));
      }
    } else {
      const producer = JSON.parse(
        fs.readFileSync(path.join(output, 'producer', 'producer.json'), 'utf8')
      );
      exact(producer.identity, identity, 'Producer provenance mismatch');
      inventory = JSON.parse(
        fs.readFileSync(path.join(output, 'producer', 'inventory.json'), 'utf8')
      );
      exact(producer.inventoryHash, digest(canonical(inventory)), 'Inventory hash mismatch');
    }
    if (role === 'producer') {
      const roots = { cargo: cargoHome, target };
      const bookkeeping = new Set([
        '.global-cache',
        '.global-cache-journal',
        '.package-cache',
        '.package-cache-mutate',
      ]);
      const files = Object.entries(roots).flatMap(([name, directory]) =>
        treeFiles(directory)
          .filter((file) => name !== 'cargo' || !bookkeeping.has(file.path))
          .map((file) => ({ ...file, path: `${name}/${file.path}` }))
      );
      validateFiles(files);
      const linkage = await runtimeLibraries(target, toolchain, command, inventory.loaderPaths);
      // GNU tar dereferences hard links only after the regular-file inventory has
      // rejected all symlinks. No staging copy and no directory/glob discovery.
      const transforms = Object.entries(roots).map(([name, directory]) => {
        assert(/^\/[A-Za-z0-9_./-]+$/.test(directory), 'Unsupported archive root spelling');
        const escaped = directory.slice(1).replaceAll('.', '\\.');
        return `s|^${escaped}/|${name}/|`;
      });
      const archive = path.join(output, 'closure.tar');
      const sourcePaths = files.map((file) => {
        const [name, ...parts] = file.path.split('/');
        return path.join(roots[name], ...parts).slice(1);
      });
      await command(
        'archive',
        '/usr/bin/tar',
        [
          '--create',
          '--format=posix',
          '--hard-dereference',
          '--no-recursion',
          '--file',
          archive,
          '--directory',
          '/',
          ...transforms.flatMap((value) => ['--transform', value]),
          '--null',
          '--files-from',
          '-',
        ],
        { input: sourcePaths.join('\0') + '\0' }
      );
      const archiveHash = largeHash(archive, LIMITS.archiveBytes);
      for (const [name, directory] of Object.entries(roots))
        exact(
          treeFiles(directory).filter((file) => name !== 'cargo' || !bookkeeping.has(file.path)),
          files
            .filter((file) => file.path.startsWith(`${name}/`))
            .map((file) => ({ ...file, path: file.path.slice(name.length + 1) })),
          'Payload changed during archive creation'
        );
      json('inventory.json', inventory);
      json('bundle.json', { identity, files, bytes: validateFiles(files), archiveHash, linkage });
      report.bundleHash = digest(canonical({ archiveHash, files, linkage }));
      report.inventoryHash = digest(canonical(inventory));
      report.assignment = assignment(inventory.harnesses);
    }
    if (role === 'consumer') {
      exact(
        await runtimeLibraries(target, toolchain, command, inventory.loaderPaths),
        bundle.linkage,
        'Runtime dependency mismatch'
      );
      const child = inventory.support
        .find((item) => item.package === 'tmt-cli' && item.target.kind.includes('bin'))
        .units.find((unit) => unit.executable && !JSON.parse(unit.key).at(-1).test).executable;
      const library = bundle.linkage.find(
        (item) => item.resolved.startsWith('/usr/lib/') || item.resolved.startsWith('/lib/')
      ).resolved;
      report.controls = inputControls(path.join(temporary, 'controls'), {
        child,
        library,
        cargo,
        source: path.join(root, '.github/components.json'),
      });
      json('controls.json', report.controls);
      const execution = [];
      for (const harness of inventory.harnesses) {
        exact(hashFile(harness.executable).sha256, harness.sha256, 'Executable hash mismatch');
        const normal = await command(
          'consumer-list',
          harness.executable,
          ['--list', '--format=pretty', '--color=never'],
          { cwd: harness.cwd }
        );
        const ignored = await command(
          'consumer-ignored',
          harness.executable,
          ['--list', '--ignored', '--format=pretty', '--color=never'],
          { cwd: harness.cwd }
        );
        assert(!normal.stderr && !ignored.stderr, 'Restored list diagnostics');
        exact(
          enumeration(normal.stdout, ignored.stdout),
          harness.list,
          'Restored list/disposition mismatch'
        );
        // Cargo's target/deps/sysroot loader paths are explicit; env_clear children remain unchanged.
        const result = await command(
          'execute',
          harness.executable,
          ['--format=pretty', '--color=never'],
          {
            cwd: harness.cwd,
            extraEnv: cargoPackageEnv(
              workspace.packages.find((pkg) => pkg.name === harness.package),
              inventory,
              toolchain,
              target
            ),
          }
        );
        // Cargo does not turn successful-test stderr diagnostics into a failure.
        // Both streams remain hashed originals; stdout coverage and actual status are strict.
        execution.push({ id: harness.id, values: parseExecution(result.stdout, harness.list) });
      }
      // Tests must not change a frozen executable, fixture, or dependency payload.
      for (const [name, destination] of [
        ['target', target],
        ['cargo', cargoHome],
      ])
        exact(
          treeFiles(destination).filter(
            (file) =>
              name !== 'cargo' ||
              ![
                '.global-cache',
                '.global-cache-journal',
                '.package-cache',
                '.package-cache-mutate',
              ].includes(file.path)
          ),
          bundle.files
            .filter((file) => file.path.startsWith(`${name}/`))
            .map((file) => ({ ...file, path: file.path.slice(name.length + 1) })),
          'Frozen runtime input changed'
        );
      exact(
        tracked.map((file) => ({ path: file, ...hashFile(path.join(root, file)) })),
        sourceFiles,
        'Source fixture changed'
      );
      report.bundleHash = digest(
        canonical({ archiveHash: bundle.archiveHash, files: bundle.files, linkage: bundle.linkage })
      );
      report.execution = execution.sort((a, b) => a.id.localeCompare(b.id));
      report.inventoryHash = digest(canonical(inventory));
      report.assignment = assignment(inventory.harnesses);
      report.closureVerified = true;
    }
    if (role === 'doctest') {
      // Separate graph, cold target: only the same workspace --doc selection, never feature repair.
      const normal = await run(
        'doc-list',
        cargoArgs('test', ['--doc', '--message-format=json', '--', '--list'])
      );
      const ignored = await run(
        'doc-ignored',
        cargoArgs('test', ['--doc', '--message-format=json', '--', '--list', '--ignored'])
      );
      const normalCoverage = cargoCoverage(normal.stdout, normal.stderr, inventory, 'list');
      const ignoredCoverage = cargoCoverage(
        ignored.stdout,
        ignored.stderr,
        inventory,
        'list',
        true
      );
      exact(normalCoverage.ordinary, [], 'Doctest worker executed an ordinary binary');
      setDocLists(inventory, normalCoverage.docs, ignoredCoverage.docs);
      const execution = await run(
        'doc-execution',
        cargoArgs('test', ['--doc', '--message-format=json'])
      );
      const coverage = cargoCoverage(execution.stdout, execution.stderr, inventory, 'execution');
      exact(coverage.ordinary, [], 'Doctest worker executed an ordinary binary');
      report.docs = {
        normal: normalCoverage.docs,
        ignored: ignoredCoverage.docs,
        execution: coverage.docs,
      };
      report.libraryFeatures = coverage.libraryFeatures;
    }
    if (inventory) report.inventory = inventoryComparison(inventory);
    report.complete = true;
  } catch (error) {
    report.complete = false;
    report.failure = error.message;
    throw error;
  } finally {
    report.elapsedMs = performance.now() - started;
    await finalizeRole({
      report,
      records,
      processesBefore,
      listenersBefore,
      roots: [target, cargoHome, home, temporary, path.join(output, 'restored')],
      deadline,
      command,
      json,
    });
    report.elapsedMs = performance.now() - started;
    json(`${role}.json`, report);
    if (!report.complete) process.exitCode = 1;
  }
}
// One cleanup boundary for every role, including a failed command or admission.
// Observations only detect; no PID from a snapshot is used for signalling.
export async function finalizeRole({
  report,
  records,
  processesBefore,
  listenersBefore,
  roots,
  deadline,
  command,
  json,
  snapshot = processSnapshot,
  now = () => performance.now(),
}) {
  report.complete = false;
  report.cleanupVerified = false;
  report.commands = records;
  report.runtimeRootsRemoved = false;
  json(`${report.role}.json`, report);
  try {
    const processesAfter = snapshot();
    json('processes-after.json', processesAfter);
    const listenersAfter = (
      await command('listeners-after', '/usr/bin/ss', ['-H', '-ltnup'], {
        cleanupPhase: true,
        timeoutMs: 10_000,
      })
    ).stdout;
    report.cleanupVerified =
      admitCleanup(processesBefore, processesAfter, listenersBefore, listenersAfter) &&
      records.length > 0 &&
      records.every((record) => record.cleanup === true);
    assert(report.cleanupVerified, 'Command/role cleanup unconfirmed');
    report.processObservations = {
      before: digest(canonical(processesBefore)),
      after: digest(canonical(processesAfter)),
    };
    // The command caller bounds this removal; no unbounded synchronous tree walk.
    await command('remove-owned-roots', '/usr/bin/rm', ['-rf', '--', ...roots], {
      cwd: path.dirname(fileURLToPath(import.meta.url)),
      cleanupPhase: true,
      timeoutMs: 30_000,
    });
    assert(
      roots.every((dir) => !fs.existsSync(dir)),
      'Owned root cleanup incomplete'
    );
    report.runtimeRootsRemoved = true;
    assert(now() < deadline, 'Report finalization deadline');
    report.complete = !report.failure;
  } catch (error) {
    report.cleanupFailure = error.message;
    report.complete = false;
  } finally {
    report.commands = records;
    json(`${report.role}.json`, report);
  }
}

/** Sensitivity of admission using copies of the actual immutable runtime inputs.
 * This does not run failing harnesses or claim per-input causal necessity. */
export function inputControls(directory, inputs) {
  assert(!fs.existsSync(directory), 'Controls require absent owned directory');
  assert(
    Object.values(inputs).reduce((bytes, file) => bytes + hashFile(file).size, 0) <=
      LIMITS.controlBytes,
    'Control-copy byte bound'
  );
  fs.mkdirSync(directory);
  try {
    return Object.entries(inputs).map(([kind, original]) => {
      const root = path.join(directory, kind);
      fs.mkdirSync(root);
      const file = path.join(root, 'required');
      const restore = () => {
        fs.copyFileSync(original, file, fs.constants.COPYFILE_EXCL);
        fs.chmodSync(file, hashFile(original).mode);
      };
      restore();
      const expected = [{ path: 'required', ...hashFile(file) }];
      verifyTree(root, expected);
      const refuses = () => {
        let failure;
        try {
          verifyTree(root, expected);
        } catch (error) {
          failure = error;
        }
        assert(
          failure instanceof assert.AssertionError &&
            /Closure file count|Missing\/extra\/changed closure input/.test(failure.message),
          'Control did not fail for missing/hash-mismatched input'
        );
      };
      fs.rmSync(file);
      refuses();
      restore();
      verifyTree(root, expected);
      const fd = fs.openSync(file, 'r+');
      try {
        const byte = Buffer.alloc(1);
        const count = fs.readSync(fd, byte, 0, 1, 0);
        byte[0] = count ? byte[0] ^ 1 : 1;
        fs.writeSync(fd, byte, 0, 1, 0);
      } finally {
        fs.closeSync(fd);
      }
      refuses();
      fs.rmSync(file);
      restore();
      verifyTree(root, expected);
      return {
        kind,
        original,
        inputHash: expected[0].sha256,
        positive: true,
        missing: true,
        changedHash: true,
        restored: true,
      };
    });
  } finally {
    fs.rmSync(directory, { recursive: true });
  }
}
export function verifyReportFiles(directory, report) {
  const additional =
    report.role === 'producer'
      ? ['inventory.json', 'bundle.json']
      : report.role === 'consumer'
        ? ['controls.json']
        : [];
  const expected = [
    `${report.role}.json`,
    'source-files.json',
    'metadata.raw.json',
    'processes-before.json',
    'processes-after.json',
    ...additional,
  ];
  assert(
    new Set(report.commands.map((command) => command.id)).size === report.commands.length,
    'Duplicate command identity'
  );
  for (const phase of ['before', 'after']) {
    const observations = JSON.parse(
      fs.readFileSync(path.join(directory, `processes-${phase}.json`), 'utf8')
    );
    exact(
      digest(canonical(observations)),
      report.processObservations?.[phase],
      'Process observation hash mismatch'
    );
  }
  for (const record of report.commands) {
    assert(/^\d{3,4}-[a-z-]+$/.test(record.id), 'Unsafe command identity');
    for (const stream of ['stdout', 'stderr']) {
      const name = `${record.id}.${stream}`;
      expected.push(name);
      exact(hashFile(path.join(directory, name)), record[stream], 'Original stream hash mismatch');
    }
    const name = `${record.id}.process.json`;
    expected.push(name);
    exact(
      JSON.parse(fs.readFileSync(path.join(directory, name), 'utf8')),
      record,
      'Original process record mismatch'
    );
  }
  exact(fs.readdirSync(directory).sort(), expected.sort(), 'Missing/extra report artifact file');
}
function inventoryComparison(inventory) {
  return {
    harnesses: inventory.harnesses.map(({ id, package: name, target, features, list, sha256 }) => ({
      id,
      package: name,
      target,
      features,
      list,
      sha256,
    })),
    support: inventory.support,
    docs: inventory.docs,
    runtimeMetadata: inventory.runtimeMetadata,
    loaderPaths: inventory.loaderPaths,
    libraryFeatures: inventory.libraryFeatures,
  };
}
function setDocLists(inventory, normal, ignored) {
  inventory.docLists = {};
  exact(
    normal.map((item) => item.package),
    ignored.map((item) => item.package),
    'Missing ignored doctest target'
  );
  for (const item of normal) {
    const subset = ignored.find((entry) => entry.package === item.package).values;
    assert(
      subset.every((name) => item.values.includes(name)),
      'Ignored doctest subset mismatch'
    );
    for (const name of item.values) {
      assert(!Object.hasOwn(inventory.docLists, name), 'Duplicate doctest identity');
      inventory.docLists[name] = { ignored: subset.includes(name) };
    }
  }
}
function cargoPackageEnv(pkg, inventory, sysroot, target) {
  const metadata = inventory.runtimeMetadata[pkg.name];
  assert(metadata, 'Missing Cargo package runtime metadata');
  const binaries = inventory.support
    .filter((item) => item.package === pkg.name && item.target.kind.includes('bin'))
    .flatMap((item) =>
      item.units
        .filter((unit) => unit.executable && !JSON.parse(unit.key).at(-1).test)
        .map((unit) => [`CARGO_BIN_EXE_${item.target.name}`, unit.executable])
    );
  const version = /^(\d+)\.(\d+)\.(\d+)(?:-([^+]+))?(?:\+.*)?$/.exec(pkg.version);
  assert(version, 'Unsupported Cargo version');
  const fields = Object.fromEntries(
    [
      'authors',
      'description',
      'homepage',
      'license',
      'license-file',
      'readme',
      'repository',
      'rust-version',
    ].map((key) => [
      `CARGO_PKG_${key.replaceAll('-', '_').toUpperCase()}`,
      Array.isArray(metadata[key]) ? metadata[key].join(':') : (metadata[key] ?? ''),
    ])
  );
  const loaderPaths = [
    ...inventory.loaderPaths,
    path.join(target, 'debug/deps'),
    path.join(target, 'debug'),
    path.join(sysroot, 'lib'),
    path.join(sysroot, 'lib/rustlib/x86_64-unknown-linux-gnu/lib'),
  ];
  for (const file of loaderPaths)
    assert(
      path.isAbsolute(file) && (file.startsWith(`${target}/`) || file.startsWith(`${sysroot}/`)),
      'Unbounded Cargo loader path'
    );
  return {
    CARGO_MANIFEST_DIR: path.dirname(pkg.manifestPath),
    CARGO_MANIFEST_PATH: pkg.manifestPath,
    CARGO_PKG_NAME: pkg.name,
    CARGO_PKG_VERSION: pkg.version,
    CARGO_PKG_VERSION_MAJOR: version[1],
    CARGO_PKG_VERSION_MINOR: version[2],
    CARGO_PKG_VERSION_PATCH: version[3],
    CARGO_PKG_VERSION_PRE: version[4] ?? '',
    ...fields,
    ...Object.fromEntries(binaries),
    LD_LIBRARY_PATH: [...new Set(loaderPaths)].join(':'),
  };
}
function processSnapshot() {
  return fs
    .readdirSync('/proc')
    .filter((name) => /^\d+$/.test(name))
    .flatMap((pid) => {
      try {
        const stat = fs.readFileSync(`/proc/${pid}/stat`, 'utf8');
        const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
        return [{ pid, start: fields[19] }];
      } catch (error) {
        if (error.code === 'ENOENT' || error.code === 'ESRCH') return [];
        throw error;
      }
    })
    .sort((a, b) => Number(a.pid) - Number(b.pid));
}
function toolFiles(root) {
  const files = [];
  function visit(directory) {
    for (const entry of fs
      .readdirSync(directory, { withFileTypes: true })
      .sort((a, b) => a.name.localeCompare(b.name))) {
      const file = path.join(directory, entry.name);
      if (entry.isDirectory()) visit(file);
      else {
        const resolved = fs.realpathSync(file);
        assert(resolved.startsWith(`${root}/`), 'Toolchain link escapes sysroot');
        files.push({
          path: path.relative(root, file),
          resolved: path.relative(root, resolved),
          ...hashFile(resolved),
        });
      }
    }
  }
  visit(root);
  validateFiles(files);
  return files;
}
async function runtimeLibraries(target, sysroot, command, loaderPaths) {
  const libraries = new Map();
  // All target ELF executables/shared objects, including ordinary CARGO_BIN_EXE children.
  const executables = [
    ...treeFiles(target).map((item) => path.join(target, item.path)),
    ...['cargo', 'rustc', 'rustdoc'].map((tool) => path.join(sysroot, 'bin', tool)),
    fs.realpathSync(process.execPath),
    ...SYSTEM_TOOLS.map((file) => fs.realpathSync(file)),
  ];
  for (const file of [...new Set(executables)]) {
    const fd = fs.openSync(file, 'r'),
      header = Buffer.alloc(20);
    let count;
    try {
      count = fs.readSync(fd, header, 0, 20, 0);
    } finally {
      fs.closeSync(fd);
    }
    if (count < 20 || header.subarray(0, 4).toString('hex') !== '7f454c46') continue;
    assert(header[5] === 1, 'Unsupported ELF endian');
    if (![2, 3].includes(header.readUInt16LE(16))) continue;
    const readelf = await command('elf', '/usr/bin/readelf', [
      '--wide',
      '--program-headers',
      '--dynamic',
      file,
    ]);
    if (!readelf.stdout.includes('NEEDED') && !readelf.stdout.includes('INTERP')) continue;
    const result = await command('linkage', '/usr/bin/ldd', [file], {
      extraEnv: {
        LD_LIBRARY_PATH: [
          ...loaderPaths,
          path.join(target, 'debug/deps'),
          path.join(target, 'debug'),
          path.join(sysroot, 'lib'),
          path.join(sysroot, 'lib/rustlib/x86_64-unknown-linux-gnu/lib'),
        ].join(':'),
      },
    });
    assert(!result.stderr && !result.stdout.includes('not found'), 'Missing GNU dependency');
    for (const match of result.stdout.matchAll(/(?:=>\s+|^\s*)(\/[^\s]+)\s+\(/gm)) {
      const resolved = fs.realpathSync(match[1]);
      assert(
        resolved.startsWith('/usr/') ||
          resolved.startsWith('/lib') ||
          resolved.startsWith(`${target}/`) ||
          resolved.startsWith(`${sysroot}/`),
        'Unbounded GNU dependency root'
      );
      libraries.set(match[1], { path: match[1], resolved, ...hashFile(resolved) });
    }
  }
  assert(libraries.size > 0, 'GNU loader closure empty');
  return [...libraries.values()].sort((a, b) => a.path.localeCompare(b.path));
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [role, output, ...extra] = process.argv.slice(2);
  assert(
    role && output && !extra.length,
    'Usage: run-workspace-proof.mjs <baseline|producer|consumer|doctest|aggregate> <owned-output>'
  );
  await main(role, output);
}
