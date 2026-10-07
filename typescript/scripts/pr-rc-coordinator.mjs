// Unarmed trusted reuse-only coordinator. All remote effects and durable custody are injected.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readBoundedFile, selectNativeArtifact } from './native-artifact-policy.mjs';
import {
  CLI_SCHEMA_TARGETS,
  parseCompiledSchema,
  assertCompiledSource,
} from './native-application-schema.mjs';
import {
  planRCResources,
  rcGenerationKey,
  RC_RESOURCE_LIMITS as LIMIT,
} from './pr-rc-resources.mjs';
import { prepareRCCheckpoint, commitRCCheckpoint } from './pr-rc-journal.mjs';

const REPOSITORY = 'pj-tmt/tmt';
const MiB = 1024 * 1024;
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');
const positive = (n) => Number.isSafeInteger(n) && n > 0;
const digest = (s) => typeof s === 'string' && /^[a-f0-9]{64}$/.test(s);
const sha = (s) => typeof s === 'string' && /^[a-f0-9]{40}$/.test(s);
const reference = (s) => typeof s === 'string' && /^[A-Za-z0-9][A-Za-z0-9._/-]{0,255}$/.test(s);
const fields = (value, keys) => {
  assert(value && typeof value === 'object' && !Array.isArray(value), 'Evidence object required.');
  assert.deepEqual(
    Object.keys(value).sort(),
    [...keys].sort(),
    'Unknown or missing evidence fields.'
  );
};
const encode = (value) => new TextEncoder().encode(JSON.stringify(value));
// The same recovery port may encode raw bytes as base64; its finite envelope is charged as metadata.
const recoverySize = (state) =>
  Buffer.byteLength(
    JSON.stringify(state, (_key, value) =>
      value instanceof Uint8Array ? Buffer.from(value).toString('base64') : value
    )
  );

// Exact accepted encodings preserve duplicate-key sensitivity without another wire decoder.
function json(bytes, maximum = MiB) {
  assert(
    bytes instanceof Uint8Array && bytes.length > 0 && bytes.length <= maximum,
    'Evidence byte bound.'
  );
  const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  const value = JSON.parse(text);
  assert(
    [
      JSON.stringify(value),
      `${JSON.stringify(value)}\n`,
      `${JSON.stringify(value, null, 2)}\n`,
    ].includes(text),
    'Unsupported or duplicate-key evidence encoding.'
  );
  const visit = (item, depth) => {
    assert(depth <= 12, 'Evidence depth bound.');
    if (typeof item === 'number') assert(Number.isSafeInteger(item), 'Unsafe evidence integer.');
    if (typeof item === 'string') assert(item.isWellFormed(), 'Malformed evidence Unicode.');
    if (item && typeof item === 'object') Object.values(item).forEach((v) => visit(v, depth + 1));
  };
  visit(value, 0);
  return value;
}

function producerWire(identity) {
  const p = identity.producer;
  assert.equal(p.workflowPath, '.github/workflows/pr-rc.yml', 'Frozen producer path differs.');
  assert(identity.attempt <= 100, 'Frozen attempt bound.');
  return {
    workflow_id: p.workflowId,
    workflow_path: p.workflowPath,
    workflow_sha256: p.workflowSha256,
    tooling_sha: p.toolingCommit,
    run_id: identity.runId,
    run_attempt: identity.attempt,
  };
}

function eligibility(observation, identity, nowMs) {
  fields(observation, ['pull', 'timeline', 'run', 'workflow']);
  fields(observation.workflow, ['path', 'tooling_sha', 'body']);
  assert(
    observation.workflow.path === identity.producer.workflowPath &&
      observation.workflow.tooling_sha === identity.producer.toolingCommit &&
      typeof observation.workflow.body === 'string' &&
      Buffer.byteLength(observation.workflow.body) <= MiB &&
      hash(observation.workflow.body) === identity.producer.workflowSha256,
    'Actual producer workflow blob differs from reviewed tuple.'
  );
  const pull = observation.pull;
  assert(
    pull.number === identity.pr &&
      pull.state === 'open' &&
      pull.base?.repo?.full_name === REPOSITORY &&
      pull.head?.repo?.full_name === REPOSITORY &&
      pull.head.sha === identity.sourceSha,
    'PR is forked, closed or no longer current.'
  );
  assert(Array.isArray(pull.labels), 'Missing current labels.');
  const labels = pull.labels.filter((l) => l.name === 'rc-build');
  assert(
    labels.length === 1 && labels[0].id === identity.epoch.labelId,
    'Label is disabled or recreated.'
  );
  assert(
    Array.isArray(observation.timeline) && observation.timeline.length <= 100,
    'Complete one-page timeline required.'
  );
  const events = observation.timeline.filter(
    (e) => ['labeled', 'unlabeled'].includes(e.event) && e.label?.name === 'rc-build'
  );
  assert(
    events.length > 0 && new Set(events.map((e) => e.id)).size === events.length,
    'Unknown or duplicate enable events.'
  );
  for (const event of events)
    assert(
      positive(event.id) &&
        event.label.id === identity.epoch.labelId &&
        /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/.test(event.created_at) &&
        positive(Date.parse(event.created_at)) &&
        new Date(event.created_at).toISOString() === event.created_at.replace('Z', '.000Z'),
      'Invalid label event.'
    );
  const latestMs = Math.max(...events.map((e) => Date.parse(e.created_at)));
  const latest = events.filter((e) => Date.parse(e.created_at) === latestMs);
  assert(
    latest.length === 1 &&
      latest[0].event === 'labeled' &&
      latest[0].id === identity.epoch.eventId &&
      latestMs === identity.epoch.enabledAtMs &&
      latestMs <= nowMs,
    'Enable epoch changed or ambiguous.'
  );
  const run = observation.run;
  assert(
    run.repository?.full_name === REPOSITORY &&
      run.id === identity.runId &&
      run.run_attempt === identity.attempt &&
      run.workflow_id === identity.producer.workflowId &&
      run.path === identity.producer.workflowPath &&
      run.head_sha === identity.producer.toolingCommit &&
      run.head_branch === 'main' &&
      run.event === 'workflow_dispatch' &&
      run.status === 'in_progress',
    'Current trusted writer run/attempt unavailable.'
  );
}

function closure(files) {
  assert(
    Array.isArray(files) && files.length > 0 && files.length <= 128,
    'Missing reviewed tooling/workflow/action/verifier/dependency closure.'
  );
  let previous = '';
  for (const file of files) {
    fields(file, ['path', 'sha256']);
    assert(
      typeof file.path === 'string' &&
        file.path.length <= 256 &&
        file.path > previous &&
        file.path
          .split('/')
          .every((p) => /^[A-Za-z0-9_.-]+$/.test(p) && !['.', '..'].includes(p)) &&
        digest(file.sha256),
      'Invalid closure inventory.'
    );
    previous = file.path;
  }
}

/** The preparation owner must export authenticated final verification facts; successful jobs are insufficient. */
function preparedBundle(input, approval, proof, identity) {
  assert(
    proof,
    'Missing native-release-prepare.yml authenticated source/closure/four-host final-verification export.'
  );
  const required = [
    'repository',
    'run_id',
    'run_attempt',
    'api_head_sha',
    'source_sha',
    'tooling_sha',
    'workflow_id',
    'workflow_path',
    'workflow_sha256',
    'closure',
    'run',
    'source_snapshot',
    'artifacts',
    'targets',
  ];
  assert(
    required.every((key) => Object.hasOwn(proof, key)),
    `Missing native-release-prepare.yml authenticated export fields: ${required.filter((key) => !Object.hasOwn(proof, key)).join(', ')}`
  );
  fields(proof, required);
  const approved = approval.preparation;
  fields(approved, ['workflow_id', 'workflow_path', 'workflow_sha256', 'tooling_sha', 'closure']);
  closure(approved.closure);
  assert(
    approved.closure.some(
      (f) => f.path === approved.workflow_path && f.sha256 === approved.workflow_sha256
    ),
    'Preparation workflow missing from reviewed closure.'
  );
  assert(
    positive(approved.workflow_id) &&
      /^\.github\/workflows\/[A-Za-z0-9._-]+\.yml$/.test(approved.workflow_path) &&
      digest(approved.workflow_sha256) &&
      sha(approved.tooling_sha),
    'Unknown preparation authority.'
  );
  assert(
    proof.repository === REPOSITORY &&
      positive(proof.run_id) &&
      positive(proof.run_attempt) &&
      proof.run_attempt <= 100 &&
      sha(proof.api_head_sha) &&
      proof.source_sha === identity.sourceSha,
    'Actual prepared source differs from PR; API synthetic head is not source authority.'
  );
  for (const key of ['workflow_id', 'workflow_path', 'workflow_sha256', 'tooling_sha', 'closure'])
    assert.deepEqual(
      proof[key],
      approved[key],
      'Preparation tooling closure differs from externally reviewed authority.'
    );
  const run = proof.run;
  assert(
    run.repository?.full_name === REPOSITORY &&
      run.id === proof.run_id &&
      run.run_attempt === proof.run_attempt &&
      run.head_sha === proof.api_head_sha &&
      run.workflow_id === proof.workflow_id &&
      run.path === proof.workflow_path &&
      run.status === 'completed' &&
      run.conclusion === 'success',
    'Preparation exact current run/attempt unavailable.'
  );
  const source = proof.source_snapshot;
  assert(
    source.schema === 1 &&
      source.product === 'cli' &&
      source.cut === identity.sourceSha &&
      typeof source.version === 'string' &&
      source.version.length <= 128 &&
      source.version.length > 0,
    'Unknown source/version snapshot.'
  );
  const manifestBytes = readBoundedFile(input.manifestPath, Math.floor(LIMIT.manifestBytes / 4));
  const manifest = json(manifestBytes, 4 * MiB);
  const schema = parseCompiledSchema(`${JSON.stringify(manifest.tmt_application_schema)}\n`);
  assertCompiledSource(schema, source, input.sourceRoot);
  assert(
    Array.isArray(input.archives) &&
      input.archives.length === 4 &&
      input.archives.every((a, i) => a.target === CLI_SCHEMA_TARGETS[i]),
    'Exactly four sorted unique CLI inputs required.'
  );
  assert(
    Array.isArray(proof.targets) &&
      proof.targets.length === 4 &&
      proof.targets.every((a, i) => a.target === CLI_SCHEMA_TARGETS[i]),
    'Missing or duplicate matching-host final verification.'
  );
  assert(
    Array.isArray(proof.artifacts) &&
      proof.artifacts.length > 0 &&
      proof.artifacts.length <= 64 &&
      new Set(proof.artifacts.map((a) => a.id)).size === proof.artifacts.length,
    'Incomplete authenticated preparation artifact inventory.'
  );
  for (const artifact of proof.artifacts) {
    fields(artifact, ['id', 'name', 'run_id', 'run_attempt', 'zip_sha256', 'zip_bytes']);
    assert(
      positive(artifact.id) &&
        reference(artifact.name) &&
        artifact.run_id === proof.run_id &&
        artifact.run_attempt === proof.run_attempt &&
        digest(artifact.zip_sha256) &&
        positive(artifact.zip_bytes) &&
        artifact.zip_bytes <= LIMIT.generationBytes,
      'Wrong preparation artifact identity/digest.'
    );
  }
  const payloads = input.archives.map((archive, i) => {
    fields(archive, ['target', 'path']);
    const target = archive.target;
    const name = `tmt-cli-${target}.tar.gz`;
    const metadata = selectNativeArtifact(input.manifestPath, archive.path, target, 'cli', {
      release: true,
    });
    assert(
      metadata.name === name && metadata.version === source.version,
      'Archive/version mismatch.'
    );
    const bytes = readBoundedFile(archive.path, 64 * MiB);
    const p = proof.targets[i];
    fields(p, [
      'target',
      'host_target',
      'artifact_id',
      'archive_name',
      'archive_sha256',
      'archive_bytes',
      'manifest_sha256',
      'manifest_bytes',
      'binary_sha256',
      'output_sha256',
      'application_schema',
      'notices_sha256',
      'notices_bytes',
      'inventory_sha256',
      'verification_reference',
    ]);
    assert(
      p.host_target === target &&
        proof.artifacts.some((a) => a.id === p.artifact_id) &&
        p.archive_name === name &&
        p.archive_sha256 === hash(bytes) &&
        p.archive_sha256 === metadata.sha256 &&
        p.archive_bytes === bytes.length &&
        p.manifest_sha256 === hash(manifestBytes) &&
        p.manifest_bytes === manifestBytes.length,
      'Missing matching-host archive/manifest provenance.'
    );
    assert.deepEqual(p.application_schema, schema, 'Final verified schema mismatch.');
    assert(
      digest(p.binary_sha256) &&
        p.output_sha256 === hash(`${JSON.stringify(schema)}\n`) &&
        digest(p.notices_sha256) &&
        positive(p.notices_bytes) &&
        p.notices_bytes <= 16 * MiB &&
        digest(p.inventory_sha256) &&
        reference(p.verification_reference),
      'Missing binary/schema/notices/inventory proof.'
    );
    return {
      target,
      version: metadata.version,
      application_schema: schema,
      files: [
        { name: 'dist-manifest.json', bytes: new Uint8Array(manifestBytes) },
        { name, bytes: new Uint8Array(bytes) },
      ],
      verification: {
        prepare_run_id: proof.run_id,
        prepare_run_attempt: proof.run_attempt,
        prepare_tooling_sha: proof.tooling_sha,
        source_sha: proof.source_sha,
        complete: true,
      },
    };
  });
  return {
    payloads,
    reuse: proof.artifacts.map((a) => ({
      repository: REPOSITORY,
      runId: proof.run_id,
      attempt: proof.run_attempt,
      artifactId: a.id,
      sha256: a.zip_sha256,
    })),
  };
}

function descriptor(file) {
  assert(
    /^[A-Za-z0-9][A-Za-z0-9._-]{0,159}$/.test(file.name) &&
      file.bytes instanceof Uint8Array &&
      file.bytes.length > 0,
    'Root regular member required.'
  );
  return { name: file.name, sha256: hash(file.bytes), bytes: file.bytes.length };
}

function artifactMetadata(value, entry, identity, maximum, reusedIds) {
  fields(value, [
    'id',
    'repository',
    'run_id',
    'run_attempt',
    'name',
    'zip_sha256',
    'zip_bytes',
    'members',
  ]);
  assert(
    positive(value.id) &&
      !reusedIds.includes(value.id) &&
      value.repository === REPOSITORY &&
      value.run_id === identity.runId &&
      value.run_attempt === identity.attempt &&
      value.name === entry.name &&
      digest(value.zip_sha256) &&
      positive(value.zip_bytes) &&
      value.zip_bytes <= maximum,
    'Artifact identity/ZIP bounds mismatch or ordinary artifact adopted.'
  );
  assert.deepEqual(value.members, entry.members, 'Uploaded fixed regular members differ.');
}

/** No default transport, CLI or workflow. Ports are trusted, externally qualified effect owners. */
export async function coordinatePRRC(input, ports) {
  input = structuredClone(input);
  let recovery = null;
  let charges = planRCResources(input.snapshot, { kind: 'reconcile' }).charges;
  let checkpointResult = null;
  let requests = 0;
  let responseBytes = 0;
  const startedAtMs = input.startedAtMs;
  const deadline = startedAtMs + LIMIT.closeMs;
  const observed = [];
  try {
    const old = await ports.checkpoint.recovery.load();
    if (old !== null) {
      recovery = old;
      charges = null;
      checkpointResult = await commitRCCheckpoint({
        candidate: old.candidate,
        recorded: old.recorded,
        bootstrap: null,
        ports: ports.checkpoint,
        startedAtMs,
      });
      return {
        mode: 'unarmed',
        status: 'frozen',
        reason: 'Existing checkpoint recovery requires qualified disposition; no upload replay.',
        charges: checkpointResult.charges,
        recovery: old,
        checkpoint: checkpointResult,
      };
    }
    const identity = input.identity;
    const key = rcGenerationKey(identity);
    assert.deepEqual(
      identity.selection,
      CLI_SCHEMA_TARGETS.map((target) => ({ product: 'cli', target })),
      'CLI-only complete four-target selection required.'
    );
    producerWire(identity);
    assert(
      positive(startedAtMs) &&
        Number.isSafeInteger(deadline) &&
        ports.checkpoint.now() >= startedAtMs &&
        ports.checkpoint.now() < deadline,
      'Original publication deadline exceeded.'
    );
    assert(
      positive(input.publishedAtMs) &&
        input.publishedAtMs >= identity.epoch.enabledAtMs &&
        positive(input.expiresAtMs) &&
        input.expiresAtMs > input.publishedAtMs &&
        input.expiresAtMs - input.publishedAtMs <= LIMIT.retentionMs,
      'Publication lifetime bound.'
    );
    const bounded = async (operation, maximum = LIMIT.reconciliationBytes) => {
      const timeoutMs = Math.min(10_000, deadline - ports.checkpoint.now());
      assert(
        timeoutMs > 0 &&
          requests < LIMIT.reconciliationRequests &&
          responseBytes < LIMIT.reconciliationBytes,
        'Publication control-plane budget exceeded.'
      );
      requests++;
      const controller = new AbortController();
      let timer;
      try {
        return await Promise.race([
          operation({ timeoutMs, maximum, signal: controller.signal }),
          new Promise((_, reject) => {
            timer = setTimeout(() => {
              controller.abort();
              reject(new Error('Publication operation timed out; settlement unknown.'));
            }, timeoutMs);
          }),
        ]);
      } finally {
        clearTimeout(timer);
      }
    };
    const capture = async (kind, artifactId = null) => {
      const response = structuredClone(
        await bounded((limits) =>
          ports.observe({ kind, identity: structuredClone(identity), artifactId, ...limits })
        )
      );
      // Raw evidence is retained before interpretation, including rejected observations.
      observed.push({ kind, artifactId, response });
      assert(response.body instanceof Uint8Array, 'Missing raw authenticated observation.');
      responseBytes += response.body.length;
      assert(
        response.authenticated?.repository === REPOSITORY &&
          response.authenticated?.ownerId === input.writer.ownerId &&
          response.status === 200 &&
          response.nextPage === null &&
          reference(response.reference) &&
          positive(response.observedAtMs) &&
          positive(response.elapsedMs + 1) &&
          response.elapsedMs <= 10_000 &&
          responseBytes <= LIMIT.reconciliationBytes &&
          ports.checkpoint.now() < deadline,
        'Unknown/partial/denied or unbounded observation.'
      );
      return { value: json(response.body), response };
    };
    const approval = structuredClone(await bounded((limits) => ports.approval.capture(limits)));
    responseBytes += encode(approval).length;
    assert(responseBytes <= LIMIT.reconciliationBytes, 'Approval evidence byte bound.');
    fields(approval, ['reference', 'producer', 'preparation']);
    assert(reference(approval.reference), 'Missing external immutable approval reference.');
    assert.deepEqual(
      approval.producer,
      identity.producer,
      'Unknown externally reviewed producer/tooling.'
    );
    assert.deepEqual(
      input.writer.producer,
      approval.producer,
      'Checkpoint writer differs from approval.'
    );
    const first = await capture('eligibility');
    eligibility(first.value, identity, first.response.observedAtMs);
    const preparation = await capture('preparation');
    const bundle = preparedBundle(input, approval, preparation.value, identity);
    const payloadNames = CLI_SCHEMA_TARGETS.map(
      (target) =>
        `tmt-pr-rc-payload-v2-pr${identity.pr}-cli-${target}-${identity.runId}-a${identity.attempt}`
    );
    const catalogName = `tmt-pr-rc-catalog-v2-pr${identity.pr}`;
    const generation = {
      identity,
      disposition: 'pending',
      mutation: 'none',
      resources: [
        ...payloadNames.map((path, i) => ({
          path,
          kind: 'payload',
          selectionIndex: i,
          transportBytes: 69 * MiB,
          metadataBytes: 4 * MiB,
          manifestBytes: bundle.payloads[i].files[0].bytes.length,
          diagnosticBytes: 0,
          artifactId: null,
        })),
        {
          path: catalogName,
          kind: 'catalog',
          selectionIndex: null,
          transportBytes: 2 * MiB,
          metadataBytes: 4 * MiB,
          manifestBytes: 0,
          diagnosticBytes: 0,
          artifactId: null,
        },
      ],
      reuse: bundle.reuse,
      observations: Object.fromEntries(
        ['reservation', 'settlement', 'inventory', 'absence'].map((kind) => [
          kind,
          { generationKey: key, reference: `${kind}/${key}`, state: 'unknown', artifactIds: [] },
        ])
      ),
    };
    const snapshot = structuredClone(input.snapshot);
    const channel = snapshot.channels.find((c) => c.pr === identity.pr);
    assert(
      channel &&
        channel.state === 'enabled' &&
        channel.sourceSha === identity.sourceSha &&
        channel.sourceRepository === REPOSITORY,
      'Snapshot does not match observed channel.'
    );
    assert.deepEqual(
      channel.epoch,
      identity.epoch,
      'Snapshot epoch differs from fresh observation.'
    );
    const candidate = prepareRCCheckpoint({
      writer: input.writer,
      sourceSha: identity.sourceSha,
      previous: input.recorded.at(-1) ?? null,
      snapshot,
      request: { kind: 'reserve', generation },
      terminal: input.terminal,
    });
    checkpointResult = await commitRCCheckpoint({
      candidate,
      recorded: input.recorded,
      bootstrap: null,
      ports: {
        ...ports.checkpoint,
        transport: async (request) => {
          responseBytes += request.body?.byteLength ?? 0;
          const response = await bounded(() => ports.checkpoint.transport(request));
          assert(response.body instanceof Uint8Array, 'Missing raw checkpoint response.');
          responseBytes += response.body.byteLength;
          assert(
            responseBytes <= LIMIT.reconciliationBytes,
            'Shared checkpoint/publication byte budget exceeded.'
          );
          return response;
        },
      },
      startedAtMs,
    });
    charges = checkpointResult.charges;
    assert(
      checkpointResult.status === 'readback-confirmed' &&
        checkpointResult.successor &&
        checkpointResult.recovery?.phase === 'complete',
      checkpointResult.reason ?? 'Worst-case reservation readback unavailable.'
    );
    recovery = structuredClone(checkpointResult.recovery);
    recovery.publication = {
      generationKey: key,
      reservationId: checkpointResult.successor.id,
      reservationDigest: candidate.digest,
      approvalReference: approval.reference,
      phase: 'reserved',
      observations: observed,
      uploads: [],
      catalog: null,
      unknown: false,
    };
    const save = async () => {
      assert(
        recoverySize(recovery) <= LIMIT.diagnosticBytes,
        'Recovery envelope exceeds reserved metadata bound; retain original state.'
      );
      await ports.checkpoint.recovery.save(structuredClone(recovery));
    };
    await save();
    const latest = async () => {
      const observation = await capture('eligibility');
      recovery.publication.observations = structuredClone(observed);
      await save();
      eligibility(observation.value, identity, observation.response.observedAtMs);
      return observation;
    };
    const upload = async (name, files, kind) => {
      const maximum = kind === 'catalog' ? 2 * MiB : 69 * MiB;
      const members = files.map(descriptor);
      assert(
        files.length === (kind === 'catalog' ? 1 : 2) &&
          members[0].name === (kind === 'catalog' ? 'catalog.json' : 'dist-manifest.json') &&
          new Set(members.map((m) => m.name)).size === members.length,
        'Fixed regular-member payload required.'
      );
      const entry = {
        name,
        kind,
        members,
        phase: 'intent',
        id: null,
        rawReturnedId: null,
        response: null,
        readback: null,
        finalization: null,
      };
      recovery.publication.uploads.push(entry);
      recovery.publication.phase = 'uploading';
      recovery.publication.unknown = true;
      await save();
      const response = structuredClone(
        await bounded((limits) =>
          ports.upload({
            name,
            files: structuredClone(files),
            identity: structuredClone(identity),
            maxZipBytes: maximum,
            ...limits,
          })
        )
      );
      entry.response = response;
      // Actual returned ID and raw response must survive before the next asynchronous call.
      entry.id = positive(response.id) ? response.id : null;
      try {
        const rawId = json(response.body).id;
        entry.rawReturnedId = positive(rawId) ? rawId : null;
      } catch {
        /* Preserve unsupported raw bytes; missing identity never permits publication. */
      }
      entry.phase = 'returned';
      await save();
      assert(
        response.body instanceof Uint8Array &&
          response.authenticated?.repository === REPOSITORY &&
          response.authenticated?.ownerId === input.writer.ownerId,
        'Upload response unavailable or unauthenticated.'
      );
      responseBytes += response.body.length;
      assert(
        response.status === 201 &&
          responseBytes <= LIMIT.reconciliationBytes &&
          response.elapsedMs >= 0 &&
          response.elapsedMs <= 10_000 &&
          ports.checkpoint.now() < deadline,
        'Upload response/time bound.'
      );
      const returned = json(response.body);
      assert(
        returned.id === entry.id &&
          !recovery.publication.uploads.slice(0, -1).some((e) => e.id === entry.id),
        'Returned ID missing or duplicate.'
      );
      artifactMetadata(
        returned,
        entry,
        identity,
        maximum,
        bundle.reuse.map((r) => r.artifactId)
      );
      const readback = await capture('artifact', entry.id);
      entry.readback = readback.response;
      entry.phase = 'readback';
      await save();
      artifactMetadata(
        readback.value,
        entry,
        identity,
        maximum,
        bundle.reuse.map((r) => r.artifactId)
      );
      assert.deepEqual(
        readback.value,
        returned,
        'Authenticated ZIP readback differs from returned receipt.'
      );
      const final = await capture('finalization', entry.id);
      entry.finalization = final.response;
      await save();
      fields(final.value, [
        'repository',
        'run_id',
        'run_attempt',
        'artifact_id',
        'zip_sha256',
        'zip_bytes',
        'settlement_reference',
        'writer_ended_at_ms',
        'inventory_reference',
      ]);
      assert(
        final.value.repository === REPOSITORY &&
          final.value.run_id === identity.runId &&
          final.value.run_attempt === identity.attempt &&
          final.value.artifact_id === entry.id &&
          final.value.zip_sha256 === returned.zip_sha256 &&
          final.value.zip_bytes === returned.zip_bytes &&
          reference(final.value.settlement_reference) &&
          reference(final.value.inventory_reference) &&
          positive(final.value.writer_ended_at_ms) &&
          final.value.writer_ended_at_ms <= final.response.observedAtMs,
        'Missing bounded exact-upload finalization/quiescence evidence.'
      );
      entry.phase = 'finalized';
      recovery.publication.unknown = false;
      await save();
      return { id: entry.id, name, zip_sha256: returned.zip_sha256, zip_bytes: returned.zip_bytes };
    };
    const candidates = [];
    for (let i = 0; i < bundle.payloads.length; i++) {
      await latest();
      const payload = bundle.payloads[i];
      const artifact = await upload(payloadNames[i], payload.files, 'payload');
      candidates.push({
        product: 'cli',
        target: payload.target,
        version: payload.version,
        application_schema: payload.application_schema,
        payload_artifact: artifact,
        dist_manifest: descriptor(payload.files[0]),
        archive: descriptor(payload.files[1]),
        verification: payload.verification,
      });
    }
    const fresh = await latest();
    assert(
      fresh.response.observedAtMs >= input.publishedAtMs &&
        fresh.response.observedAtMs < input.expiresAtMs,
      'Catalog publication time is stale or expired.'
    );
    const catalog = {
      schema_version: 2,
      kind: 'tmt-pr-rc-catalog',
      repository: REPOSITORY,
      pr: identity.pr,
      head_sha: identity.sourceSha,
      producer: producerWire(identity),
      eligibility: {
        label: 'rc-build',
        label_id: identity.epoch.labelId,
        enabled_event_id: identity.epoch.eventId,
        enabled_at_ms: identity.epoch.enabledAtMs,
      },
      published_at_ms: input.publishedAtMs,
      expires_at_ms: input.expiresAtMs,
      candidates,
    };
    const catalogBytes = encode(catalog);
    assert(catalogBytes.length <= MiB, 'Catalog raw bound.');
    recovery.publication.catalog = catalog;
    await save();
    await upload(catalogName, [{ name: 'catalog.json', bytes: catalogBytes }], 'catalog');
    const finalEligibility = await latest();
    assert(
      finalEligibility.response.observedAtMs < input.expiresAtMs,
      'Catalog expired during final readback.'
    );
    recovery.publication.phase = 'readback-confirmed';
    recovery.publication.observations = structuredClone(observed);
    await save();
    return {
      mode: 'unarmed',
      status: 'readback-confirmed',
      charges,
      recovery,
      checkpoint: checkpointResult,
    };
  } catch (error) {
    let reason = error instanceof Error ? error.message : 'Unknown publication evidence.';
    if (recovery?.publication) {
      recovery.publication.phase = 'frozen';
      recovery.publication.unknown = true;
      recovery.publication.observations = structuredClone(observed);
      try {
        if (recoverySize(recovery) > LIMIT.diagnosticBytes) throw new Error('Recovery byte bound');
        await ports.checkpoint.recovery.save(structuredClone(recovery));
      } catch {
        reason +=
          ' Durable recovery unconfirmed; retain the full returned recovery and all charges.';
      }
    }
    return {
      mode: 'unarmed',
      status: recovery || checkpointResult?.status === 'frozen' ? 'frozen' : 'refused',
      reason,
      charges,
      recovery: recovery ?? checkpointResult?.recovery ?? null,
      checkpoint: checkpointResult,
      observations: observed,
    };
  }
}
