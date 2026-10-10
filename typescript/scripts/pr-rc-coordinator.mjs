// Trusted main PR-candidate producer; Core owns installer admission and schema semantics.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { readBoundedFile, selectNativeArtifact } from './native-artifact-policy.mjs';
import {
  CLI_SCHEMA_TARGETS,
  parseCompiledSchema,
  assertCompiledSource,
  readSchemaEvidence,
} from './native-application-schema.mjs';

export const RC_REPOSITORY = 'pj-tmt/tmt';
export const RC_TARGETS = CLI_SCHEMA_TARGETS;
export const RC_TTL_MS = 3 * 24 * 60 * 60 * 1000;
const MiB = 1024 * 1024;
const sha = (value) => typeof value === 'string' && /^[a-f0-9]{40}$/.test(value);
const digest = (value) => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const positive = (value) => Number.isSafeInteger(value) && value > 0;
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');
const descriptor = (name, bytes) => ({ name, sha256: hash(bytes), bytes: bytes.length });
const json = (file, limit) => JSON.parse(readBoundedFile(file, limit).toString('utf8'));
const fields = (value, names) => {
  assert(value && typeof value === 'object' && !Array.isArray(value), 'Evidence object required.');
  assert.deepEqual(
    Object.keys(value).sort(),
    [...names].sort(),
    'Unknown or missing evidence fields.'
  );
};

/** Captured after the existing matching-host verification and final source recheck. */
export function capturePRRCVerification({
  root,
  directory,
  target,
  snapshot,
  runId,
  attempt,
  toolingSha,
}) {
  assert(
    RC_TARGETS.includes(target) &&
      positive(runId) &&
      positive(attempt) &&
      attempt <= 100 &&
      sha(toolingSha),
    'Invalid preparation identity.'
  );
  const source = json(snapshot, 4 * MiB);
  const manifest = readBoundedFile(path.join(directory, 'dist-manifest.json'), 4 * MiB);
  const name = `tmt-cli-${target}.tar.gz`;
  const archive = readBoundedFile(path.join(directory, name), 64 * MiB);
  const metadata = selectNativeArtifact(
    path.join(directory, 'dist-manifest.json'),
    path.join(directory, name),
    target,
    'cli',
    { release: true }
  );
  const schema = parseCompiledSchema(
    `${JSON.stringify(JSON.parse(manifest).tmt_application_schema)}\n`
  );
  assertCompiledSource(schema, source, root);
  const evidence = readSchemaEvidence(path.join(directory, `${target}-application-schema.json`));
  assert.equal(evidence.target, target, 'Matching-host evidence target differs.');
  assert.deepEqual(evidence.record, schema, 'Matching-host schema differs.');
  assert.equal(evidence.archive_sha256, hash(archive), 'Verified archive changed.');
  assert.equal(metadata.sha256, hash(archive), 'Manifest archive digest differs.');
  assert.equal(metadata.version, source.version, 'Verified version differs.');
  assert.equal(
    evidence.output_sha256,
    hash(`${JSON.stringify(schema)}\n`),
    'Compiled output digest differs.'
  );
  assert(digest(evidence.binary_sha256), 'Missing verified binary digest.');
  const notices = readBoundedFile(path.join(directory, `${target}-notices.txt`), 16 * MiB);
  assert(notices.length > 0, 'Verified notices missing.');
  return {
    schema_version: 1,
    product: 'cli',
    target,
    source_sha: source.cut,
    version: source.version,
    prepare_run_id: runId,
    prepare_run_attempt: attempt,
    prepare_tooling_sha: toolingSha,
    application_schema: schema,
    dist_manifest: descriptor('dist-manifest.json', manifest),
    archive: descriptor(name, archive),
    binary_sha256: evidence.binary_sha256,
    output_sha256: evidence.output_sha256,
    notices: descriptor(`${target}-notices.txt`, notices),
    source_snapshot_sha256: evidence.source_snapshot_sha256,
  };
}

/** API observations, not PR-controlled files, establish current opt-in and trusted run ownership. */
export function validatePRRCEligibility(
  { pull, timeline, run },
  { pr, head, runId, attempt, head12 },
  nowMs
) {
  assert(
    positive(pr) && sha(head) && head12 === head.slice(0, 12) && positive(nowMs),
    'Invalid requested PR/head.'
  );
  assert(
    pull.number === pr &&
      pull.state === 'open' &&
      pull.head?.sha === head &&
      pull.head?.repo?.full_name === RC_REPOSITORY &&
      pull.base?.repo?.full_name === RC_REPOSITORY,
    'PR is forked, closed or no longer current.'
  );
  const labels = pull.labels?.filter((label) => label.name === 'rc-build');
  assert(labels?.length === 1 && positive(labels[0].id), 'PR lacks the current rc-build label.');
  assert(Array.isArray(timeline) && timeline.length < 100, 'PR timeline is incomplete.');
  const events = timeline.filter(
    (event) => ['labeled', 'unlabeled'].includes(event.event) && event.label?.name === 'rc-build'
  );
  assert(
    events.length > 0 && new Set(events.map((event) => event.id)).size === events.length,
    'Missing or duplicate opt-in events.'
  );
  for (const event of events)
    assert(
      positive(event.id) &&
        event.label.id === labels[0].id &&
        /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/.test(event.created_at) &&
        Number.isSafeInteger(Date.parse(event.created_at)),
      'Invalid opt-in event.'
    );
  const latestTime = Math.max(...events.map((event) => Date.parse(event.created_at)));
  const latest = events.filter((event) => Date.parse(event.created_at) === latestTime);
  assert(
    latest.length === 1 && latest[0].event === 'labeled' && latestTime <= nowMs,
    'Opt-in epoch is disabled or ambiguous.'
  );
  const reopened = timeline.filter((event) => event.event === 'reopened');
  assert(
    reopened.every(
      (event) =>
        Number.isSafeInteger(Date.parse(event.created_at)) &&
        Date.parse(event.created_at) < latestTime
    ),
    'Reopened PR requires a fresh rc-build opt-in.'
  );
  assert(
    run.id === runId &&
      run.run_attempt === attempt &&
      attempt <= 100 &&
      run.repository?.full_name === RC_REPOSITORY &&
      positive(run.workflow_id) &&
      run.path === '.github/workflows/pr-rc.yml' &&
      run.head_branch === 'main' &&
      run.event === 'workflow_dispatch' &&
      run.status === 'in_progress' &&
      sha(run.head_sha) &&
      run.display_title === `pr-rc #${pr} ${head12}`,
    'Untrusted PR producer run.'
  );
  return {
    label: 'rc-build',
    label_id: labels[0].id,
    enabled_event_id: latest[0].id,
    enabled_at_ms: latestTime,
  };
}

/** Admit only four final matching-host reports and the exact unchanged bundle. */
export function stagePRRCPayloads({
  directory,
  reports,
  output,
  head,
  runId,
  attempt,
  toolingSha,
}) {
  assert(
    sha(head) && positive(runId) && positive(attempt) && sha(toolingSha),
    'Invalid preparation binding.'
  );
  assert(
    Array.isArray(reports) &&
      reports.length === 4 &&
      reports.every((report, i) => report.target === RC_TARGETS[i]),
    'Four sorted target reports required.'
  );
  const manifestBytes = readBoundedFile(path.join(directory, 'dist-manifest.json'), 4 * MiB);
  const schema = parseCompiledSchema(
    `${JSON.stringify(JSON.parse(manifestBytes).tmt_application_schema)}\n`
  );
  assert.equal(schema.source_sha, head, 'Bundle schema source differs from PR.');
  let version;
  return reports.map((report) => {
    fields(report, [
      'schema_version',
      'product',
      'target',
      'source_sha',
      'version',
      'prepare_run_id',
      'prepare_run_attempt',
      'prepare_tooling_sha',
      'application_schema',
      'dist_manifest',
      'archive',
      'binary_sha256',
      'output_sha256',
      'notices',
      'source_snapshot_sha256',
    ]);
    assert(
      report.schema_version === 1 &&
        report.product === 'cli' &&
        report.source_sha === head &&
        report.prepare_run_id === runId &&
        report.prepare_run_attempt === attempt &&
        report.prepare_tooling_sha === toolingSha,
      'Preparation report identity differs.'
    );
    assert.deepEqual(report.application_schema, schema, 'Final target schemas disagree.');
    const name = `tmt-cli-${report.target}.tar.gz`;
    const archive = readBoundedFile(path.join(directory, name), 64 * MiB);
    const metadata = selectNativeArtifact(
      path.join(directory, 'dist-manifest.json'),
      path.join(directory, name),
      report.target,
      'cli',
      { release: true }
    );
    assert.equal(metadata.version, report.version, 'Preparation version differs.');
    if (version === undefined) version = report.version;
    assert.equal(report.version, version, 'Final target versions disagree.');
    assert.equal(metadata.sha256, hash(archive), 'Archive bytes differ from manifest.');
    assert.deepEqual(
      report.dist_manifest,
      descriptor('dist-manifest.json', manifestBytes),
      'Verified manifest changed.'
    );
    assert.deepEqual(report.archive, descriptor(name, archive), 'Verified archive changed.');
    assert(
      digest(report.binary_sha256) &&
        digest(report.source_snapshot_sha256) &&
        report.output_sha256 === hash(`${JSON.stringify(schema)}\n`),
      'Missing binary/schema/source evidence.'
    );
    const notices = readBoundedFile(path.join(directory, `${report.target}-notices.txt`), 16 * MiB);
    assert.deepEqual(
      report.notices,
      descriptor(`${report.target}-notices.txt`, notices),
      'Verified notices changed.'
    );
    // Artifact uploads use compression-level:0; two bounded root files leave >1 MiB ZIP overhead.
    assert(
      manifestBytes.length + archive.length <= 68 * MiB,
      'Payload member bytes exceed ZIP allowance.'
    );
    const destination = path.join(output, report.target);
    fs.mkdirSync(destination, { recursive: true });
    fs.writeFileSync(path.join(destination, 'dist-manifest.json'), manifestBytes);
    fs.writeFileSync(path.join(destination, name), archive);
    return {
      product: 'cli',
      target: report.target,
      version,
      application_schema: schema,
      dist_manifest: report.dist_manifest,
      archive: report.archive,
      verification: {
        prepare_run_id: runId,
        prepare_run_attempt: attempt,
        prepare_tooling_sha: toolingSha,
        source_sha: head,
        complete: true,
      },
    };
  });
}

export function prRCPayloadName(pr, target, runId, attempt) {
  assert(
    positive(pr) && RC_TARGETS.includes(target) && positive(runId) && positive(attempt),
    'Invalid payload identity.'
  );
  return `tmt-pr-rc-payload-v2-pr${pr}-cli-${target}-${runId}-a${attempt}`;
}
export function prRCCatalogName(pr) {
  assert(positive(pr), 'Invalid PR catalog identity.');
  return `tmt-pr-rc-catalog-v2-pr${pr}`;
}

/** Each returned artifact is read back and byte-checked before catalog exposure. */
export async function buildPRRCCatalog(
  { identity, candidates, producer, publishedAtMs },
  { observe, payload }
) {
  const before = validatePRRCEligibility(await observe(), identity, publishedAtMs);
  assert(
    candidates.length === 4 &&
      candidates.every((candidate, i) => candidate.target === RC_TARGETS[i]),
    'Four candidate targets required.'
  );
  assert(
    producer.workflow_id > 0 &&
      producer.workflow_path === '.github/workflows/pr-rc.yml' &&
      digest(producer.workflow_sha256) &&
      sha(producer.tooling_sha) &&
      producer.run_id === identity.runId &&
      producer.run_attempt === identity.attempt,
    'Invalid producer diagnostics.'
  );
  const complete = [];
  for (const candidate of candidates) {
    const name = prRCPayloadName(identity.pr, candidate.target, identity.runId, identity.attempt);
    const received = await payload(name);
    assert(
      positive(received.id) &&
        received.name === name &&
        digest(received.zip_sha256) &&
        positive(received.zip_bytes) &&
        received.zip_bytes <= 69 * MiB,
      'Payload metadata unavailable or oversized.'
    );
    assert.deepEqual(
      received.members,
      [candidate.dist_manifest, candidate.archive],
      'Read-back payload member bytes differ.'
    );
    assert(
      !complete.some((previous) => previous.payload_artifact.id === received.id),
      'Duplicate payload artifact ID.'
    );
    const { members: _members, ...artifact } = received;
    complete.push({ ...candidate, payload_artifact: artifact });
  }
  const after = validatePRRCEligibility(await observe(), identity, publishedAtMs);
  assert.deepEqual(after, before, 'PR opt-in epoch changed before catalog.');
  const catalog = {
    schema_version: 2,
    kind: 'tmt-pr-rc-catalog',
    repository: RC_REPOSITORY,
    pr: identity.pr,
    head_sha: identity.head,
    producer,
    eligibility: after,
    published_at_ms: publishedAtMs,
    expires_at_ms: publishedAtMs + RC_TTL_MS,
    candidates: complete,
  };
  const bytes = Buffer.from(`${JSON.stringify(catalog)}\n`);
  assert(bytes.length <= MiB, 'Catalog exceeds 1 MiB.');
  return bytes;
}

/** Only bounded official-repository Actions/PR reads; never token or branch mutation. */
export function createPRRCAPI(execute = execFileSync) {
  const read = (route, maximum = 2 * MiB, binary = false) =>
    execute(
      'gh',
      [
        'api',
        `repos/${RC_REPOSITORY}/${route}`,
        '-H',
        `Accept: ${binary ? 'application/octet-stream' : 'application/vnd.github+json'}`,
      ],
      { timeout: 120_000, maxBuffer: maximum }
    );
  const metadata = (route) => JSON.parse(read(route).toString('utf8'));
  const inventory = (runId) => {
    const result = metadata(`actions/runs/${runId}/artifacts?per_page=100`);
    assert(
      Array.isArray(result.artifacts) &&
        result.total_count === result.artifacts.length &&
        result.total_count < 100,
      'Artifact inventory is incomplete.'
    );
    return result.artifacts;
  };
  const members = (artifact, maximum) => {
    assert(
      positive(artifact.id) &&
        positive(artifact.size_in_bytes) &&
        artifact.size_in_bytes <= maximum &&
        artifact.expired === false &&
        /^sha256:[a-f0-9]{64}$/.test(artifact.digest),
      'Actions artifact identity/digest unavailable.'
    );
    const zip = read(`actions/artifacts/${artifact.id}/zip`, maximum, true);
    assert.equal(zip.length, artifact.size_in_bytes, 'Actions ZIP size differs.');
    assert.equal(`sha256:${hash(zip)}`, artifact.digest, 'Actions ZIP digest differs.');
    // A bounded regular-root ZIP is data, never an executable or a filesystem extraction.
    const program = `import sys,io,zipfile,json,base64
z=zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read()))
a=z.infolist()
assert 1<=len(a)<=2 and len({i.filename for i in a})==len(a)
assert sum(i.file_size for i in a)<=int(sys.argv[1])
for i in a:
 assert i.filename and '/' not in i.filename and '\\\\' not in i.filename and i.filename not in ('.','..')
 assert i.compress_type in (0,8) and not i.flag_bits&1 and (i.external_attr>>16)&0o170000 in (0,0o100000)
print(json.dumps([{'name':i.filename,'data':base64.b64encode(z.read(i)).decode()} for i in a]))`;
    const decoded = JSON.parse(
      execute('python3', ['-c', program, String(maximum)], {
        input: zip,
        timeout: 120_000,
        maxBuffer: 96 * MiB,
      }).toString('utf8')
    );
    return decoded.map((member) => ({
      name: member.name,
      bytes: Buffer.from(member.data, 'base64'),
    }));
  };
  return { metadata, inventory, members };
}

function runtimeIdentity(env) {
  assert.equal(env.GITHUB_REPOSITORY, RC_REPOSITORY, 'Official repository required.');
  assert.equal(env.GITHUB_REF, 'refs/heads/main', 'PR RC producer requires refs/heads/main.');
  const identity = {
    pr: Number(env.REQUESTED_PR),
    head: env.REQUESTED_HEAD,
    head12: env.REQUESTED_HEAD12,
    runId: Number(env.GITHUB_RUN_ID),
    attempt: Number(env.GITHUB_RUN_ATTEMPT),
  };
  assert(
    positive(identity.pr) &&
      sha(identity.head) &&
      identity.head12 === identity.head.slice(0, 12) &&
      positive(identity.runId) &&
      positive(identity.attempt),
    'Invalid requested identity.'
  );
  return identity;
}

/** Workflow commands use the same owners exercised by the inert controls. */
export async function runPRRCCommand(command, env = process.env, api = createPRRCAPI()) {
  if (command === 'report') {
    const report = capturePRRCVerification({
      root: env.SOURCE_ROOT,
      directory: env.BUNDLE_DIRECTORY,
      target: env.TARGET,
      snapshot: env.SOURCE_SNAPSHOT,
      runId: Number(env.GITHUB_RUN_ID),
      attempt: Number(env.GITHUB_RUN_ATTEMPT),
      toolingSha: env.GITHUB_SHA,
    });
    fs.writeFileSync(env.REPORT_FILE, `${JSON.stringify(report)}\n`);
    return;
  }
  const identity = runtimeIdentity(env);
  const observe = () => ({
    pull: api.metadata(`pulls/${identity.pr}`),
    timeline: api.metadata(`issues/${identity.pr}/timeline?per_page=100`),
    run: api.metadata(`actions/runs/${identity.runId}/attempts/${identity.attempt}`),
  });
  const epoch = validatePRRCEligibility(observe(), identity, Date.now());
  if (command === 'guard') {
    fs.appendFileSync(env.GITHUB_OUTPUT, `head=${identity.head}\n`);
    return;
  }
  const selected = (name, maximum) => {
    const found = api.inventory(identity.runId).filter((artifact) => artifact.name === name);
    assert.equal(found.length, 1, 'Missing or duplicate Actions artifact.');
    assert.equal(found[0].workflow_run.id, identity.runId, 'Artifact belongs to another run.');
    return { artifact: found[0], members: api.members(found[0], maximum) };
  };
  const reports = RC_TARGETS.map((target) => {
    const received = selected(`native-pr-rc-verify-cli-${target}-main`, MiB);
    assert.deepEqual(
      received.members.map((member) => member.name),
      ['report.json'],
      'Unexpected verification report members.'
    );
    return JSON.parse(received.members[0].bytes.toString('utf8'));
  });
  const candidates = stagePRRCPayloads({
    directory: env.BUNDLE_DIRECTORY,
    reports,
    output: path.join(env.RUNNER_TEMP, 'pr-rc-payloads'),
    head: identity.head,
    runId: identity.runId,
    attempt: identity.attempt,
    toolingSha: env.GITHUB_SHA,
  });
  const payload = (name) => {
    const received = selected(name, 69 * MiB);
    const members = received.members
      .map((member) => descriptor(member.name, member.bytes))
      .sort((a, b) => a.name.localeCompare(b.name));
    return {
      id: received.artifact.id,
      name,
      zip_sha256: received.artifact.digest.slice(7),
      zip_bytes: received.artifact.size_in_bytes,
      members,
    };
  };
  if (command === 'stage') {
    assert(RC_TARGETS.includes(env.TARGET), 'Unknown payload target.');
    fs.appendFileSync(
      env.GITHUB_OUTPUT,
      `name=${prRCPayloadName(identity.pr, env.TARGET, identity.runId, identity.attempt)}\n`
    );
  } else if (command === 'payload') {
    const received = payload(
      prRCPayloadName(identity.pr, env.TARGET, identity.runId, identity.attempt)
    );
    assert.equal(
      received.id,
      Number(env.RETURNED_ARTIFACT_ID),
      'Returned payload ID differs from authenticated readback.'
    );
    const candidate = candidates.find((item) => item.target === env.TARGET);
    assert.deepEqual(
      received.members,
      [candidate.dist_manifest, candidate.archive],
      'Returned payload bytes differ.'
    );
  } else if (command === 'catalog') {
    const run = observe().run;
    const bytes = await buildPRRCCatalog(
      {
        identity,
        candidates,
        publishedAtMs: Date.now(),
        producer: {
          workflow_id: run.workflow_id,
          workflow_path: '.github/workflows/pr-rc.yml',
          workflow_sha256: hash(readBoundedFile('.github/workflows/pr-rc.yml', MiB)),
          tooling_sha: env.GITHUB_SHA,
          run_id: identity.runId,
          run_attempt: identity.attempt,
        },
      },
      { observe, payload }
    );
    fs.writeFileSync(path.join(env.RUNNER_TEMP, 'catalog.json'), bytes);
  } else if (command === 'finish') {
    const received = selected(prRCCatalogName(identity.pr), 2 * MiB);
    assert.equal(
      received.artifact.id,
      Number(env.RETURNED_ARTIFACT_ID),
      'Returned catalog ID differs.'
    );
    assert.deepEqual(
      received.members.map((member) => member.name),
      ['catalog.json'],
      'Unexpected catalog members.'
    );
    assert.deepEqual(
      received.members[0].bytes,
      readBoundedFile(path.join(env.RUNNER_TEMP, 'catalog.json'), MiB),
      'Catalog readback bytes differ.'
    );
  } else throw new Error('Unknown PR RC command.');
  assert.deepEqual(
    validatePRRCEligibility(observe(), identity, Date.now()),
    epoch,
    'PR opt-in changed during publication.'
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  await runPRRCCommand(process.argv[2]);
