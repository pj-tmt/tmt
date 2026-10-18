import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
  mkdirSync,
  readFileSync,
  writeFileSync,
  realpathSync,
  lstatSync,
  existsSync,
  readdirSync,
} from 'node:fs';
import { createHash } from 'node:crypto';
import { performance } from 'node:perf_hooks';
import { runInPrivateWorld } from './private-world.mjs';
import { runPackedCommand } from './packed-command.mjs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const clean = (text) => text.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '');

/** Libtest decorates should-panic names in --list, while nextest uses the actual name. */
export function libtestCases(text) {
  const names = clean(text)
    .split('\n')
    .flatMap((line) => {
      const match = /^(.*): test$/.exec(line);
      return match ? [match[1].replace(/ - should panic$/, '')] : [];
    });
  assert.equal(new Set(names).size, names.length, 'duplicate libtest case');
  return names.sort();
}

/** Compare an independently discovered Cargo binary closure and libtest cases. */
export function compareInventories(cargo, nextest) {
  const suites = Object.values(nextest['rust-suites']);
  assert.ok(suites.length > 0, 'empty nextest inventory');
  const binaries = new Map(cargo.map((suite) => [suite.executable, suite]));
  assert.equal(binaries.size, cargo.length, 'duplicate Cargo executable');
  assert.equal(suites.length, binaries.size, 'Cargo/nextest binary count differs');
  const cases = [];
  for (const suite of suites) {
    const original = binaries.get(suite['binary-path']);
    assert.ok(original, `nextest binary absent from Cargo: ${suite['binary-path']}`);
    binaries.delete(suite['binary-path']);
    const names = Object.keys(suite.testcases);
    assert.deepEqual(
      [...names].sort(),
      original.tests,
      `${suite['binary-id']}: case identities differ`
    );
    const ignored = names.filter((name) => suite.testcases[name].ignored);
    assert.deepEqual(
      [...ignored].sort(),
      original.ignored,
      `${suite['binary-id']}: ignored set differs`
    );
    for (const name of names) {
      const match = suite.testcases[name]['filter-match'];
      assert.ok(
        match.status === 'matches' ||
          (suite.testcases[name].ignored &&
            match.status === 'mismatch' &&
            match.reason === 'ignored'),
        `${suite['binary-id']}: ${name}: filtered test`
      );
      cases.push({ binary: suite['binary-id'], name, ignored: ignored.includes(name) });
    }
  }
  assert.equal(binaries.size, 0, 'Cargo binary omitted by nextest');
  return cases;
}

/** A green summary alone cannot hide a filtered, missing or repeated case. */
export function verifyExecution(cases, log) {
  const observed = clean(log)
    .split('\n')
    .flatMap((line) => {
      const match = /PASS\s+\[\s*[\d.]+s\]\s+\(\s*\d+\/\d+\)\s+(\S+)\s+(.+)$/.exec(line);
      return match ? [JSON.stringify([match[1], match[2]])] : [];
    });
  assert.equal(new Set(observed).size, observed.length, 'repeated nextest PASS');
  const expected = cases
    .filter((test) => !test.ignored)
    .map((test) => JSON.stringify([test.binary, test.name]));
  assert.deepEqual(observed.sort(), expected.sort(), 'nextest executed case identities differ');
  return { passed: observed.length, ignored: cases.length - observed.length, total: cases.length };
}

export const NEXTEST_VERSION = '0.9.148';
export const TOOLCHAIN = '1.97.0';
const key = (test) => JSON.stringify([test.binary, test.name]);
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');

function admittedCases(cases) {
  assert.ok(Array.isArray(cases) && cases.length > 0 && cases.length <= 100_000, 'case bound');
  for (const test of cases)
    assert.ok(
      typeof test.binary === 'string' &&
        test.binary.length > 0 &&
        test.binary.length < 1024 &&
        typeof test.name === 'string' &&
        test.name.length > 0 &&
        test.name.length < 8192 &&
        typeof test.ignored === 'boolean',
      'invalid case identity'
    );
  assert.equal(new Set(cases.map(key)).size, cases.length, 'duplicate case identity');
  assert.ok(
    cases.some((test) => !test.ignored),
    'no active cases'
  );
  return cases;
}

/** Preserve pinned nextest suite/test order after independently admitting every identity. */
export function partitionCases(cases, index, total) {
  admittedCases(cases);
  assert.ok(Number.isInteger(total) && total >= 2 && total <= 3, 'partition count');
  assert.ok(Number.isInteger(index) && index >= 1 && index <= total, 'partition index');
  return cases
    .filter((test) => !test.ignored)
    .filter((_, position) => position % total === index - 1);
}

/** Prove the pinned nextest slice selects exactly the pre-registered identities. */
export function admitPartition(cases, inventory, index, total) {
  const all = new Map(admittedCases(cases).map((test) => [key(test), test]));
  const selected = [];
  for (const suite of Object.values(inventory['rust-suites'])) {
    for (const [name, test] of Object.entries(suite.testcases)) {
      const identity = { binary: suite['binary-id'], name };
      const original = all.get(key(identity));
      assert.ok(original, 'partition contains unknown case');
      all.delete(key(identity));
      assert.equal(test.ignored, original.ignored, 'partition ignored set differs');
      const match = test['filter-match'];
      if (match.status === 'matches') {
        assert.ok(!test.ignored, 'partition selected ignored case');
        selected.push(original);
      } else
        assert.ok(
          match.status === 'mismatch' &&
            (match.reason === 'partition' || (test.ignored && match.reason === 'ignored')),
          'unexpected partition filter'
        );
    }
  }
  assert.equal(all.size, 0, 'partition omitted inventory case');
  const expected = partitionCases(cases, index, total);
  assert.deepEqual(selected.map(key).sort(), expected.map(key).sort(), 'slice assignment differs');
  return expected;
}

/** Cargo builds non-test examples that nextest does not automatically archive. */
export function exampleExecutables(artifacts, targetDirectory) {
  assert.ok(path.isAbsolute(targetDirectory), 'target directory must be absolute');
  const files = artifacts
    .filter(
      (item) =>
        item.reason === 'compiler-artifact' &&
        !item.profile.test &&
        item.target.kind.includes('example') &&
        item.executable
    )
    .map((item) => path.relative(targetDirectory, item.executable));
  runtimeArchiveConfig(files);
  return files;
}

/** Explicit target-relative includes: missing runtime inputs must fail the producer. */
export function runtimeArchiveConfig(files) {
  assert.ok(Array.isArray(files) && files.length <= 1000, 'runtime executable bound');
  assert.equal(new Set(files).size, files.length, 'duplicate runtime executable');
  for (const file of files)
    assert.ok(
      typeof file === 'string' &&
        file.length < 4096 &&
        file === file.trim() &&
        /^[A-Za-z0-9_+.-]+(?:\/[A-Za-z0-9_+.-]+)+$/.test(file) &&
        file.split('/').every((part) => part !== '.' && part !== '..'),
      'runtime executable escape'
    );
  return (
    '[profile.default]\narchive.include = [\n' +
    files
      .map(
        (file) =>
          `  { path = ${JSON.stringify(file)}, relative-to = "target", on-missing = "error" },`
      )
      .join('\n') +
    '\n]\n'
  );
}

export function verifyRuntimeExecutables(files, targetDirectory) {
  runtimeArchiveConfig(files);
  if (files.length === 0) return;
  const root = realpathSync(targetDirectory);
  for (const file of files) {
    const full = path.join(root, file);
    const stat = lstatSync(full);
    assert.ok(
      stat.isFile() &&
        !stat.isSymbolicLink() &&
        (stat.mode & 0o111) !== 0 &&
        realpathSync(full).startsWith(root + path.sep),
      'runtime executable type or escape'
    );
  }
}

/** Admit bytes and producer identity before nextest can decode or execute anything. */
export function admitArchive(manifest, bytes, expectedIdentity) {
  assert.equal(manifest.schema, 2, 'manifest schema');
  assert.deepEqual(manifest.producerIdentity, expectedIdentity, 'consumer identity differs');
  assert.ok(bytes.length > 0 && bytes.length <= 1024 ** 3, 'archive byte bound');
  assert.equal(bytes.length, manifest.archive.bytes, 'archive size differs');
  assert.equal(sha256(bytes), manifest.archive.sha256, 'archive hash differs');
  partitionCases(manifest.cases, 1, manifest.partitions);
  runtimeArchiveConfig(manifest.runtimeFiles);
  return manifest;
}

/** Missing/skipped/failed/foreign consumers can never satisfy the caller's original gate. */
export function aggregatePartitions(manifest, results, states, selection) {
  assert.equal(manifest.schema, 2, 'manifest schema');
  assert.deepEqual(
    states,
    { producer: 'success', consumers: 'success' },
    'workspace worker failed or skipped'
  );
  const cases = admittedCases(manifest.cases);
  partitionCases(cases, 1, manifest.partitions);
  if (selection) admitSelection(manifest, selection);
  assert.equal(results.length, manifest.partitions, 'missing/extra partition result');
  const seen = new Set();
  const union = [];
  for (const result of results) {
    assert.equal(result.schema, 2, 'result schema');
    assert.deepEqual(
      result.producerIdentity,
      manifest.producerIdentity,
      'foreign partition identity'
    );
    const selected = selection?.partitions.find((row) => row.index === result.index);
    const attempt = selected?.attempt ?? manifest.producerIdentity.attempt;
    assert.deepEqual(
      result.executionIdentity,
      { ...manifest.producerIdentity, attempt },
      'foreign partition execution identity'
    );
    assert.equal(
      result.manifestSha256,
      sha256(JSON.stringify(manifest) + '\n'),
      'foreign manifest hash'
    );
    assert.deepEqual(result.archive, manifest.archive, 'foreign partition archive');
    assert.equal(result.partitions, manifest.partitions, 'result partition count');
    assert.ok(!seen.has(result.index), 'duplicate partition result');
    seen.add(result.index);
    assert.equal(result.exit, 0, 'partition execution failed');
    assert.equal(result.worldRemoved, true, 'partition world retained');
    const expected = partitionCases(cases, result.index, manifest.partitions);
    assert.deepEqual(result.cases, expected, 'partition result identities differ');
    verifyExecution(expected, result.log);
    union.push(...result.cases);
  }
  assert.equal(new Set(union.map(key)).size, union.length, 'overlapping partition cases');
  assert.deepEqual(
    union.map(key).sort(),
    cases
      .filter((test) => !test.ignored)
      .map(key)
      .sort(),
    'partition union differs'
  );
  return { passed: union.length, ignored: cases.length - union.length, total: cases.length };
}

const attemptNumber = (value) => {
  assert.match(value ?? '', /^[1-9][0-9]{0,5}$/, 'invalid execution attempt');
  return Number(value);
};
const artifactId = (value) => {
  assert.match(value ?? '', /^[1-9][0-9]{0,15}$/, 'invalid artifact id');
  assert.ok(Number.isSafeInteger(Number(value)), 'unsafe artifact id');
  return value;
};
const digest = (value) => assert.match(value ?? '', /^[a-f0-9]{64}$/, 'invalid artifact digest');
const timestamp = (value) => {
  assert.ok(
    typeof value === 'string' && value.endsWith('Z') && Number.isFinite(Date.parse(value)),
    'invalid job timestamp'
  );
  return Date.parse(value);
};
export const resultArtifactName = (index, identity, attempt) =>
  `workspace-result-${index}-${identity.run}-${identity.attempt}-${attempt}`;

/** Outputs become a receipt only when the entire producer job and both uploads succeeded. */
export function admitProducerReceipt(receipt, expectedRun, producerResult) {
  assert.equal(producerResult, 'success', 'producer receipt requires successful job');
  assert.equal(receipt?.proof?.schema, 1, 'missing producer receipt');
  const { proof } = receipt;
  const a = attemptNumber(proof.producerIdentity?.attempt);
  const c = attemptNumber(expectedRun.attempt);
  assert.ok(a <= c && c - a <= 2, 'producer generation attempt bound; rerun all jobs');
  admitMachineIdentity(proof.producerIdentity, { ...expectedRun, attempt: String(a) });
  assert.ok(proof.partitions === 2 || proof.partitions === 3, 'receipt partition count');
  digest(proof.manifestSha256);
  digest(proof.archive?.sha256);
  assert.ok(
    Number.isSafeInteger(proof.archive.bytes) &&
      proof.archive.bytes > 0 &&
      proof.archive.bytes <= 1024 ** 3,
    'receipt archive byte bound'
  );
  for (const kind of ['closure', 'inventory']) {
    artifactId(receipt[kind]?.id);
    digest(receipt[kind]?.digest);
  }
  assert.notEqual(receipt.closure.id, receipt.inventory.id, 'duplicate producer artifact');
  return receipt;
}

/** Bind the downloaded manifest's exact bytes before admitting its archive or result slices. */
export function admitManifest(receipt, bytes) {
  assert.equal(sha256(bytes), receipt.proof.manifestSha256, 'producer manifest hash differs');
  const manifest = JSON.parse(Buffer.from(bytes).toString('utf8'));
  assert.equal(manifest.schema, 2, 'manifest schema');
  assert.deepEqual(
    manifest.producerIdentity,
    receipt.proof.producerIdentity,
    'producer identity differs'
  );
  assert.deepEqual(manifest.archive, receipt.proof.archive, 'producer archive differs');
  assert.equal(manifest.partitions, receipt.proof.partitions, 'producer partitions differ');
  partitionCases(manifest.cases, 1, manifest.partitions);
  runtimeArchiveConfig(manifest.runtimeFiles);
  return manifest;
}

/** A later consumer may change only its execution attempt, never its compiled environment. */
export function admitExecutionIdentity(producer, execution, collectorAttempt = execution.attempt) {
  const a = attemptNumber(producer.attempt);
  const e = attemptNumber(execution.attempt);
  const c = attemptNumber(collectorAttempt);
  assert.ok(a <= e && e <= c && c - a <= 2, 'execution attempt outside producer generation');
  assert.deepEqual(
    { ...execution, attempt: producer.attempt },
    producer,
    'consumer identity differs'
  );
  return execution;
}

function admitSelection(manifest, selection) {
  assert.equal(selection.schema, 1, 'selection schema');
  assert.deepEqual(
    selection.producerIdentity,
    manifest.producerIdentity,
    'foreign selection producer'
  );
  assert.equal(
    selection.manifestSha256,
    sha256(JSON.stringify(manifest) + '\n'),
    'foreign selection manifest'
  );
  const producer = manifest.producerIdentity;
  admitMachineIdentity(producer, { ...selection.collectorIdentity, attempt: producer.attempt });
  assert.equal(selection.partitions.length, manifest.partitions, 'incomplete partition selection');
  const seen = new Set();
  const artifacts = new Set();
  for (const row of selection.partitions) {
    partitionCases(manifest.cases, row.index, manifest.partitions);
    assert.ok(!seen.has(row.index), 'duplicate partition selection');
    seen.add(row.index);
    admitExecutionIdentity(
      producer,
      { ...producer, attempt: row.attempt },
      selection.collectorIdentity.attempt
    );
    if (producer.attempt !== selection.collectorIdentity.attempt) {
      assert.ok(Number.isSafeInteger(row.jobId) && row.jobId > 0, 'missing selected job');
      artifactId(row.artifact?.id);
      assert.ok(!artifacts.has(row.artifact.id), 'duplicate selected artifact');
      artifacts.add(row.artifact.id);
      assert.equal(
        row.artifact.name,
        resultArtifactName(row.index, producer, row.attempt),
        'foreign selected artifact'
      );
      assert.match(row.artifact.digest ?? '', /^sha256:[a-f0-9]{64}$/, 'selected artifact digest');
    } else
      assert.ok(
        row.attempt === producer.attempt && row.jobId === null && row.artifact === null,
        'unexpected single-attempt selection'
      );
  }
}

/** Only the recovery collector constructs this bounded GET-only metadata port. */
export function createRecoveryReader(repository, execute = runPackedCommand, clock = Date.now) {
  assert.match(repository ?? '', /^[\w.-]+\/[\w.-]+$/);
  let requests = 0;
  const deadline = clock() + 90_000;
  return (resource) => {
    assert.match(
      resource,
      /^actions\/(?:runs\/[1-9][0-9]*(?:\/attempts\/[1-9][0-9]*(?:\/jobs\?per_page=100&page=[12])?|\/artifacts\?name=workspace-result-[0-9-]+&per_page=2)|artifacts\/[1-9][0-9]*)$/,
      'unexpected recovery metadata path'
    );
    assert.ok(requests < 20, 'recovery metadata request bound');
    assert.ok(clock() < deadline, 'recovery metadata deadline');
    requests++;
    return JSON.parse(
      execute('gh', ['api', '--method', 'GET', `repos/${repository}/${resource}`], {
        timeoutMs: Math.min(10_000, deadline - clock()),
      })
    );
  };
}

/** Select actual executions, not the synthetic job IDs GitHub assigns retained successes. */
export function planPartitionRecovery(receipt, collectorIdentity, states, read, workflowHead) {
  admitProducerReceipt(receipt, collectorIdentity, states.producer);
  assert.equal(states.consumers, 'success', 'workspace worker failed or skipped');
  const producer = receipt.proof.producerIdentity;
  const a = attemptNumber(producer.attempt);
  const c = attemptNumber(collectorIdentity.attempt);
  const plan = {
    schema: 1,
    producerIdentity: producer,
    collectorIdentity,
    manifestSha256: receipt.proof.manifestSha256,
    partitions: Array.from({ length: receipt.proof.partitions }, (_, i) => ({
      index: i + 1,
      attempt: String(c),
      jobId: null,
      artifact: null,
    })),
  };
  if (a === c) return plan; // No extra REST or artifact discovery on the ordinary path.
  assert.match(workflowHead ?? '', /^[a-f0-9]{40}$/, 'missing workflow head');
  let prefix;
  let previousStart = -1;
  const actual = new Map();
  const fingerprint = (job) => ({
    name: job.name,
    started: job.started_at,
    completed: job.completed_at,
    status: job.status,
    conclusion: job.conclusion,
    steps: job.steps,
  });
  let producerJob;
  for (let e = a; e <= c; e++) {
    const context = read(`actions/runs/${producer.run}/attempts/${e}`);
    assert.equal(String(context.id), producer.run, 'foreign attempt run');
    assert.equal(context.run_attempt, e, 'foreign attempt number');
    assert.equal(context.head_sha, workflowHead, 'foreign attempt source');
    const start = timestamp(context.run_started_at);
    assert.ok(start > previousStart, 'attempts not chronological');
    previousStart = start;
    const jobs = [];
    let count;
    for (let page = 1; page <= 2; page++) {
      const data = read(
        `actions/runs/${producer.run}/attempts/${e}/jobs?per_page=100&page=${page}`
      );
      assert.ok(
        Number.isSafeInteger(data.total_count) &&
          data.total_count > 0 &&
          data.total_count <= 200 &&
          Array.isArray(data.jobs),
        'job page bound'
      );
      if (count === undefined) count = data.total_count;
      assert.equal(data.total_count, count, 'changing job count');
      assert.equal(data.jobs.length, Math.min(100, count - jobs.length), 'incomplete job page');
      jobs.push(...data.jobs);
      if (jobs.length === count) break;
    }
    const ids = new Set();
    const labels = new Set();
    for (const job of jobs) {
      assert.ok(
        Number.isSafeInteger(job.id) && job.id > 0 && !ids.has(job.id),
        'duplicate/invalid job id'
      );
      ids.add(job.id);
      assert.equal(String(job.run_id), producer.run, 'foreign job run');
      assert.equal(job.run_attempt, e, 'foreign job attempt');
      assert.equal(job.head_sha, workflowHead, 'foreign job source');
      assert.ok(typeof job.name === 'string', 'missing job label');
      const label =
        /^(?:(.*) \/ )?(Compile workspace test closure|Execute partition ([1-3]))$/.exec(job.name);
      if (!label) {
        assert.ok(
          !job.name.includes('Execute partition') &&
            !job.name.includes('Compile workspace test closure'),
          'ambiguous workspace job label'
        );
        continue;
      }
      if (prefix === undefined) prefix = label[1] ?? '';
      assert.equal(label[1] ?? '', prefix, 'ambiguous workspace job prefix');
      assert.ok(!labels.has(label[2]), 'duplicate workspace job label');
      labels.add(label[2]);
      assert.ok(Array.isArray(job.steps) && job.steps.length > 0, 'missing job steps');
      const started = timestamp(job.started_at);
      const prior = actual.get(job.name);
      if (started < start) {
        assert.ok(
          job.status === 'completed' &&
            job.conclusion === 'success' &&
            timestamp(job.completed_at) <= start &&
            prior,
          'unproved retained job'
        );
        assert.deepEqual(
          fingerprint(job),
          fingerprint(prior.job),
          'retained success fingerprint differs'
        );
        continue;
      }
      if (label[2] === 'Compile workspace test closure') {
        assert.equal(e, a, 'producer generation changed; rerun all jobs');
        assert.ok(
          job.status === 'completed' && job.conclusion === 'success',
          'producer job not successful'
        );
        producerJob = job;
      } else assert.ok(Number(label[3]) <= receipt.proof.partitions, 'unexpected partition label');
      actual.set(job.name, { job, attempt: String(e) });
    }
  }
  assert.ok(producerJob, 'missing producer execution');
  const artifacts = new Set();
  const admitArtifact = (row, name, job, expected) => {
    artifactId(String(row?.id));
    assert.ok(!artifacts.has(row.id), 'duplicate artifact id');
    artifacts.add(row.id);
    assert.equal(row.name, name, 'foreign artifact name');
    assert.equal(String(row.workflow_run?.id), producer.run, 'foreign artifact run');
    assert.equal(row.workflow_run?.head_sha, workflowHead, 'foreign artifact source');
    assert.equal(row.expired, false, 'artifact expired; rerun all jobs');
    assert.ok(
      Number.isSafeInteger(row.size_in_bytes) &&
        row.size_in_bytes > 0 &&
        row.size_in_bytes <= 2 * 1024 ** 3,
      'artifact byte bound'
    );
    assert.match(row.digest ?? '', /^sha256:[a-f0-9]{64}$/, 'artifact digest missing');
    assert.ok(
      timestamp(row.created_at) >= timestamp(job.started_at) &&
        timestamp(row.created_at) <= timestamp(job.completed_at),
      'artifact outside selected execution'
    );
    if (expected) {
      assert.equal(String(row.id), expected.id, 'producer artifact id differs');
      assert.equal(row.digest, `sha256:${expected.digest}`, 'producer artifact digest differs');
    }
    return { id: String(row.id), name: row.name, digest: row.digest };
  };
  for (const kind of ['closure', 'inventory'])
    admitArtifact(
      read(`actions/artifacts/${receipt[kind].id}`),
      `workspace-${kind === 'closure' ? 'closure' : 'producer'}-${producer.run}-${producer.attempt}`,
      producerJob,
      receipt[kind]
    );
  plan.partitions = plan.partitions.map(({ index }) => {
    const label = `${prefix ? prefix + ' / ' : ''}Execute partition ${index}`;
    const selected = actual.get(label);
    assert.ok(selected, 'missing partition execution');
    assert.ok(
      selected.job.status === 'completed' && selected.job.conclusion === 'success',
      'latest partition execution failed or incomplete'
    );
    const name = resultArtifactName(index, producer, selected.attempt);
    const data = read(`actions/runs/${producer.run}/artifacts?name=${name}&per_page=2`);
    assert.ok(
      data.total_count === 1 && Array.isArray(data.artifacts) && data.artifacts.length === 1,
      'missing/duplicate partition artifact'
    );
    return {
      index,
      attempt: selected.attempt,
      jobId: selected.job.id,
      artifact: admitArtifact(data.artifacts[0], name, selected.job),
    };
  });
  return plan;
}

function readJSONBytes(file) {
  const stat = lstatSync(file);
  assert.ok(
    stat.isFile() && !stat.isSymbolicLink() && stat.size <= 64 * 1024 ** 2,
    'JSON file bound/type'
  );
  return readFileSync(file);
}
function readJSON(file) {
  return JSON.parse(readJSONBytes(file).toString('utf8'));
}
function writeJSON(file, data) {
  writeFileSync(file, JSON.stringify(data) + '\n');
}

function runIdentity() {
  const source = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
  assert.equal(source.status, 0, 'source identity unavailable');
  assert.equal(source.stdout.trim(), process.env.GITHUB_SHA, 'checkout differs from event');
  assert.match(process.env.GITHUB_RUN_ID ?? '', /^[1-9][0-9]{0,15}$/);
  assert.match(process.env.GITHUB_RUN_ATTEMPT ?? '', /^[1-9][0-9]{0,5}$/);
  assert.equal(process.platform, 'linux', 'consumer platform');
  assert.equal(process.arch, 'x64', 'consumer architecture');
  assert.equal(process.env.CARGO_PROFILE_DEV_DEBUG, '0', 'dev debug environment');
  assert.equal(process.env.CARGO_INCREMENTAL, '0', 'incremental environment');
  return {
    source: source.stdout.trim(),
    run: process.env.GITHUB_RUN_ID,
    attempt: process.env.GITHUB_RUN_ATTEMPT,
    toolchain: TOOLCHAIN,
    nextest: NEXTEST_VERSION,
    platform: 'linux-x64',
    debug: '0',
    incremental: '0',
    workspace: realpathSync('.'),
  };
}

/** The native metadata tests use both runtime CARGO and a producer-compiled Cargo path. */
export function consumerEnvironment(cargo, base = process.env) {
  assert.ok(
    typeof cargo === 'string' && path.isAbsolute(cargo),
    'Cargo executable must be absolute'
  );
  return { ...base, CARGO: cargo };
}

/** Collectors validate run provenance; only actual execution machines supply compiler evidence. */
export function admitMachineIdentity(identity, expectedRun) {
  const { cargo, cargoVersion, rustcVersion, ...run } = identity;
  assert.deepEqual(run, expectedRun, 'run identity differs');
  consumerEnvironment(cargo, {});
  assert.ok(
    typeof cargoVersion === 'string' && cargoVersion.startsWith(`cargo ${TOOLCHAIN} `),
    'Cargo version differs'
  );
  assert.ok(
    typeof rustcVersion === 'string' && rustcVersion.startsWith(`rustc ${TOOLCHAIN} `),
    'rustc version differs'
  );
  return identity;
}

function executionIdentity(context = runIdentity()) {
  const read = (program, args) => {
    const result = spawnSync(program, args, { encoding: 'utf8', timeout: 30_000, maxBuffer: 4096 });
    if (result.error) throw result.error;
    assert.equal(result.status, 0, `${program} ${args.join(' ')} identity failed`);
    return result.stdout.trim();
  };
  const cargo = realpathSync(read('rustup', ['which', 'cargo']));
  return admitMachineIdentity(
    {
      ...context,
      cargo,
      cargoVersion: read(cargo, ['--version']),
      rustcVersion: read(path.join(path.dirname(cargo), 'rustc'), ['--version']),
    },
    context
  );
}

/** Every effect is bounded and its timing/result survives a failing phase. */
async function command(directory, phase, program, args, isolated = false, base = process.env) {
  const started = new Date().toISOString();
  const clock = performance.now();
  let result;
  const diagnostics = [];
  const spawn = (executable, argv, options = {}) => {
    result = spawnSync(executable, argv, {
      env: options.env ?? base,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 ** 2,
      timeout: 18 * 60_000,
    });
    return result;
  };
  const world = isolated
    ? runInPrivateWorld({
        argv: [program, ...args],
        base,
        spawn,
        log: (line) => diagnostics.push(line),
      })
    : undefined;
  if (!world) spawn(program, args);
  else result.status = world.status;
  const worldRemoved = world ? !existsSync(world.world.root) : undefined;
  const seconds = (performance.now() - clock) / 1000;
  writeFileSync(
    path.join(directory, `${phase}.log`),
    `${result.stdout ?? ''}${result.stderr ?? ''}${diagnostics.join('\n')}`
  );
  const file = path.join(directory, 'phases.json');
  const phases = existsSync(file) ? readJSON(file) : [];
  phases.push({
    phase,
    started,
    completed: new Date().toISOString(),
    seconds,
    exit: result.status,
    signal: result.signal,
    world: world?.world.root,
    worldRemoved,
    leaks: world?.leaks,
  });
  writeJSON(file, phases);
  const stdout = result.stdout ?? '';
  const stderr = (result.stderr ?? '') + diagnostics.join('\n');
  if (result.error || result.status !== 0 || (world && !worldRemoved)) {
    // An uncaught failure can exit before pipe writes drain; artifacts are already durable.
    await Promise.all(
      [
        [process.stdout, stdout],
        [process.stderr, stderr],
      ].map(
        ([stream, text]) =>
          new Promise((resolve, reject) => {
            stream.write(text, (error) => (error ? reject(error) : resolve()));
          })
      )
    );
  } else {
    process.stdout.write(stdout);
    process.stderr.write(stderr);
  }
  if (world) assert.equal(worldRemoved, true, 'private world retained');
  console.log(`#3284 phase=${phase} seconds=${seconds.toFixed(3)} exit=${result.status}`);
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `${program} ${args.join(' ')} failed`);
  return result.stdout;
}

async function inventory(directory, total) {
  const run = (phase, program, args) => command(directory, phase, program, args);
  const build = await run('compile-inventory', 'cargo', [
    'test',
    '--locked',
    '--workspace',
    '--no-run',
    '--message-format=json',
  ]);
  const artifacts = build
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line));
  const runtimeFiles = exampleExecutables(artifacts, path.resolve('target'));
  verifyRuntimeExecutables(runtimeFiles, path.resolve('target'));
  writeJSON(path.join(directory, 'runtime-files.json'), runtimeFiles);
  const executables = artifacts
    .filter((item) => item.reason === 'compiler-artifact' && item.profile.test && item.executable)
    .map((item) => item.executable);
  assert.ok(executables.length > 0 && executables.length <= 1000, 'Cargo executable bound');
  const cargo = [];
  for (const [index, executable] of executables.entries()) {
    cargo.push({
      executable,
      tests: libtestCases(
        await run(`libtest-${index}`, executable, ['--list', '--format', 'terse'])
      ),
      ignored: libtestCases(
        await run(`libtest-ignored-${index}`, executable, [
          '--ignored',
          '--list',
          '--format',
          'terse',
        ])
      ),
    });
  }
  writeJSON(path.join(directory, 'cargo-inventory.json'), cargo);
  const nextest = JSON.parse(
    await run('nextest-inventory', 'cargo-nextest', [
      'nextest',
      'list',
      '--locked',
      '--workspace',
      '--message-format',
      'json',
    ])
  );
  writeJSON(path.join(directory, 'nextest-inventory.json'), nextest);
  const cases = compareInventories(cargo, nextest);
  admittedCases(cases);
  writeJSON(path.join(directory, 'cases.json'), cases);
  for (let index = 1; index <= total; index++) {
    const selected = JSON.parse(
      await run(`partition-list-${index}`, 'cargo-nextest', [
        'nextest',
        'list',
        '--locked',
        '--workspace',
        '--partition',
        `slice:${index}/${total}`,
        '--message-format',
        'json',
      ])
    );
    admitPartition(cases, selected, index, total);
  }
  // The archive carries these integration executables as well as test binaries.
  const binaries = Object.values(nextest['rust-build-meta']['non-test-binaries']).flat();
  for (const binary of binaries) {
    assert.equal(binary.kind, 'bin-exe', 'unknown non-test binary kind');
    const file = path.resolve('target', binary.path);
    assert.ok(file.startsWith(path.resolve('target') + path.sep), 'non-test binary escape');
    assert.ok(lstatSync(file).isFile(), 'missing runtime executable');
  }
  console.log(`Cargo/nextest inventory equal: ${cases.length} cases in ${cargo.length} binaries`);
}

async function archive(directory, total) {
  const runtimeFiles = readJSON(path.join(directory, 'runtime-files.json'));
  verifyRuntimeExecutables(runtimeFiles, path.resolve('target'));
  const config = path.join(directory, 'archive-config.toml');
  writeFileSync(config, runtimeArchiveConfig(runtimeFiles));
  const archive = path.join(directory, 'workspace.tar.zst');
  await command(directory, 'archive', 'cargo-nextest', [
    'nextest',
    '--config-file',
    config,
    'archive',
    '--locked',
    '--workspace',
    '--archive-file',
    archive,
  ]);
  const stat = lstatSync(archive);
  assert.ok(stat.isFile() && stat.size > 0 && stat.size <= 1024 ** 3, 'archive byte bound');
  const cases = admittedCases(readJSON(path.join(directory, 'cases.json')));
  partitionCases(cases, 1, total);
  const manifest = {
    schema: 2,
    producerIdentity: executionIdentity(),
    partitions: total,
    cases,
    runtimeFiles,
    archive: { bytes: stat.size, sha256: sha256(readFileSync(archive)) },
  };
  const file = path.join(directory, 'manifest.json');
  writeJSON(file, manifest);
  assert.ok(process.env.GITHUB_OUTPUT, 'producer output port missing');
  writeFileSync(
    process.env.GITHUB_OUTPUT,
    `proof=${JSON.stringify({
      schema: 1,
      producerIdentity: manifest.producerIdentity,
      partitions: total,
      manifestSha256: sha256(readFileSync(file)),
      archive: manifest.archive,
    })}\nattempt=${manifest.producerIdentity.attempt}\n`,
    { flag: 'a' }
  );
}

async function consume(directory, index) {
  const run = runIdentity();
  const machine = executionIdentity(run);
  const receipt = admitProducerReceipt(
    JSON.parse(process.env.PRODUCER_RECEIPT || 'null'),
    run,
    process.env.PRODUCER_RESULT
  );
  const manifest = admitManifest(receipt, readJSONBytes(path.join(directory, 'manifest.json')));
  admitExecutionIdentity(manifest.producerIdentity, machine);
  const expected = partitionCases(manifest.cases, index, manifest.partitions);
  const archive = path.join(directory, 'workspace.tar.zst');
  const stat = lstatSync(archive);
  assert.ok(
    stat.isFile() && !stat.isSymbolicLink() && stat.size <= 1024 ** 3,
    'archive byte bound/type'
  );
  admitArchive(manifest, readFileSync(archive), manifest.producerIdentity);
  assert.ok(!existsSync('target'), 'consumer target must start absent');
  // nextest owns decoding; extraction into this existing workspace preserves compiled absolute paths.
  const inventory = JSON.parse(
    await command(directory, 'downloaded-extract-list', 'cargo-nextest', [
      'nextest',
      'list',
      '--archive-file',
      archive,
      '--extract-to',
      realpathSync('.'),
      '--message-format',
      'json',
      '--partition',
      `slice:${index}/${manifest.partitions}`,
    ])
  );
  assert.equal(
    inventory['rust-build-meta']['target-directory'],
    path.resolve('target'),
    'extracted target differs'
  );
  admitPartition(manifest.cases, inventory, index, manifest.partitions);
  for (const suite of Object.values(inventory['rust-suites']))
    assert.ok(lstatSync(suite['binary-path']).isFile(), 'missing extracted test executable');
  for (const binary of Object.values(inventory['rust-build-meta']['non-test-binaries']).flat())
    assert.ok(
      lstatSync(path.join('target', binary.path)).isFile(),
      'missing extracted runtime executable'
    );
  verifyRuntimeExecutables(manifest.runtimeFiles, path.resolve('target'));
  const metadata = [
    '--binaries-metadata',
    path.resolve('target/nextest/binaries-metadata.json'),
    '--cargo-metadata',
    path.resolve('target/nextest/cargo-metadata.json'),
  ];
  await command(
    directory,
    'nextest-execution',
    'cargo-nextest',
    [
      'nextest',
      'run',
      ...metadata,
      '--no-fail-fast',
      '--test-threads',
      '4',
      '--partition',
      `slice:${index}/${manifest.partitions}`,
    ],
    true,
    consumerEnvironment(machine.cargo)
  );
  const phases = readJSON(path.join(directory, 'phases.json'));
  const worldRemoved = phases.at(-1).worldRemoved;
  assert.equal(worldRemoved, true, 'partition world retained');
  const log = readFileSync(path.join(directory, 'nextest-execution.log'), 'utf8');
  verifyExecution(expected, log);
  writeJSON(path.join(directory, 'result.json'), {
    schema: 2,
    producerIdentity: manifest.producerIdentity,
    executionIdentity: machine,
    manifestSha256: receipt.proof.manifestSha256,
    archive: manifest.archive,
    index,
    partitions: manifest.partitions,
    exit: 0,
    worldRemoved,
    cases: expected,
    log,
  });
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [mode, location, value] = process.argv.slice(2);
  assert.ok(
    location,
    'usage: workspace-nextest.mjs inventory|archive|consume|aggregate <directory> [partition]'
  );
  const directory = path.resolve(location);
  mkdirSync(directory, { recursive: true });
  if (mode === 'inventory') await inventory(directory, Number(value));
  else if (mode === 'archive') await archive(directory, Number(value));
  else if (mode === 'consume') await consume(directory, Number(value));
  else if (mode === 'phase') {
    assert.match(value ?? '', /^[a-z][a-z0-9-]{0,63}$/);
    const [program, ...args] = process.argv.slice(5);
    assert.ok(program, 'phase program missing');
    await command(directory, value, program, args, true);
  } else if (mode === 'plan') {
    const collector = runIdentity();
    const states = {
      producer: process.env.PRODUCER_RESULT,
      consumers: process.env.CONSUMERS_RESULT,
    };
    const receipt = JSON.parse(process.env.PRODUCER_RECEIPT || 'null');
    const read =
      collector.attempt === receipt?.proof?.producerIdentity?.attempt
        ? () => assert.fail('single-attempt REST is forbidden')
        : createRecoveryReader(process.env.GITHUB_REPOSITORY);
    const selection = planPartitionRecovery(
      receipt,
      collector,
      states,
      read,
      process.env.WORKFLOW_HEAD_SHA
    );
    writeJSON(path.join(directory, 'selection.json'), selection);
    assert.ok(process.env.GITHUB_OUTPUT, 'collector output port missing');
    writeFileSync(
      process.env.GITHUB_OUTPUT,
      `artifact-ids=${selection.partitions
        .map((row) => row.artifact?.id)
        .filter(Boolean)
        .join(',')}\n`,
      { flag: 'a' }
    );
  } else if (mode === 'aggregate') {
    const collector = runIdentity();
    const receipt = admitProducerReceipt(
      JSON.parse(process.env.PRODUCER_RECEIPT || 'null'),
      collector,
      process.env.PRODUCER_RESULT
    );
    const manifest = admitManifest(receipt, readJSONBytes(path.join(directory, 'manifest.json')));
    const selection = readJSON(path.join(directory, 'selection.json'));
    assert.deepEqual(selection.collectorIdentity, collector, 'foreign collector identity');
    admitSelection(manifest, selection);
    const states = {
      producer: process.env.PRODUCER_RESULT,
      consumers: process.env.CONSUMERS_RESULT,
    };
    const names = selection.partitions.map((row) =>
      resultArtifactName(row.index, manifest.producerIdentity, row.attempt)
    );
    assert.deepEqual(
      readdirSync(directory)
        .filter((name) => name.startsWith('workspace-result-'))
        .sort(),
      [...names].sort(),
      'missing/extra partition artifact'
    );
    const results = names.map((name) => readJSON(path.join(directory, name, 'result.json')));
    const summary = aggregatePartitions(manifest, results, states, selection);
    writeJSON(path.join(directory, 'summary.json'), summary);
    writeJSON(path.join(directory, 'selection.json'), {
      ...selection,
      archive: manifest.archive,
      partitions: selection.partitions.map((row, index) => ({
        ...row,
        resultSha256: sha256(readJSONBytes(path.join(directory, names[index], 'result.json'))),
      })),
    });
    console.log(JSON.stringify(summary));
  } else assert.fail('unknown workspace-nextest mode');
}
