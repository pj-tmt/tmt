// Unarmed candidate: every authority, transport and durable-recovery effect is injected.
import { createHash } from 'node:crypto';
import {
  planRCResources,
  rcGenerationKey,
  RC_RESOURCE_LIMITS as LIMIT,
} from './pr-rc-resources.mjs';

const REPOSITORY = 'pj-tmt/tmt';
const ENVIRONMENT = 'tmt-pr-rc-checkpoint';
const TASK = 'tmt-pr-rc-checkpoint';
const ROOT = `/repos/${REPOSITORY}/deployments`;
const SHA = /^[a-f0-9]{40}$/;
const DIGEST = /^[a-f0-9]{64}$/;
const integer = (n) => Number.isSafeInteger(n) && n > 0;
const requireEvidence = (condition, reason) => {
  if (!condition) throw new Error(reason);
};
const hash = (text) => createHash('sha256').update(text).digest('hex');
const fields = (value, keys) =>
  requireEvidence(
    value !== null &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value).length === keys.length &&
      keys.every((key) => Object.hasOwn(value, key)),
    'Incomplete or unknown checkpoint fields.'
  );

/** Finite JSON admission before JSON.parse: duplicate decoded keys cannot disappear. */
function parseFinite(bytes, maximum) {
  requireEvidence(bytes instanceof Uint8Array && bytes.byteLength <= maximum, 'JSON byte bound.');
  const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  let offset = 0;
  const whitespace = () => {
    while (/^[\t\n\r ]$/.test(text[offset] ?? '')) offset++;
  };
  const string = () => {
    requireEvidence(text[offset] === '"', 'Expected JSON string.');
    const start = offset++;
    while (offset < text.length) {
      const ch = text[offset++];
      if (ch === '\\') {
        offset++;
        continue;
      }
      if (ch === '"') {
        const value = JSON.parse(text.slice(start, offset));
        requireEvidence(value.isWellFormed(), 'Malformed Unicode.');
        return value;
      }
    }
    throw new Error('Unterminated JSON string.');
  };
  const value = (depth) => {
    requireEvidence(depth <= 16, 'JSON nesting bound.');
    whitespace();
    const ch = text[offset];
    if (ch === '"') {
      string();
      return;
    }
    if (ch === '{' || ch === '[') {
      offset++;
      const end = ch === '{' ? '}' : ']';
      const keys = new Set();
      whitespace();
      if (text[offset] === end) {
        offset++;
        return;
      }
      while (offset < text.length) {
        if (ch === '{') {
          whitespace();
          const key = string();
          requireEvidence(!keys.has(key), 'Duplicate JSON field.');
          keys.add(key);
          whitespace();
          requireEvidence(text[offset++] === ':', 'Expected JSON colon.');
        }
        value(depth + 1);
        whitespace();
        const separator = text[offset++];
        if (separator === end) return;
        requireEvidence(separator === ',', 'Expected JSON separator.');
      }
      throw new Error('Unterminated JSON container.');
    }
    const token = /^(?:true|false|null|-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?)/.exec(
      text.slice(offset)
    )?.[0];
    requireEvidence(token, 'Invalid JSON token.');
    if (!['true', 'false', 'null'].includes(token))
      requireEvidence(Number.isSafeInteger(Number(token)), 'Unsafe JSON number.');
    offset += token.length;
  };
  value(0);
  whitespace();
  requireEvidence(offset === text.length, 'Trailing JSON data.');
  return JSON.parse(text);
}

function canonical(value, maximum = LIMIT.reconciliationBytes) {
  const chunks = [];
  let byteCount = 0;
  const append = (text) => {
    byteCount += Buffer.byteLength(text);
    requireEvidence(byteCount <= maximum, 'JSON serialization byte bound.');
    chunks.push(text);
  };
  const write = (item, depth) => {
    requireEvidence(depth <= 16, 'JSON nesting bound.');
    if (item === null || typeof item === 'boolean') {
      append(JSON.stringify(item));
      return;
    }
    if (typeof item === 'number') {
      requireEvidence(Number.isSafeInteger(item), 'Unsafe JSON number.');
      append(JSON.stringify(item));
      return;
    }
    if (typeof item === 'string') {
      requireEvidence(
        item.length <= maximum && item.isWellFormed(),
        'Malformed or oversized Unicode.'
      );
      append(JSON.stringify(item));
      return;
    }
    requireEvidence(typeof item === 'object' && item !== null, 'Non-JSON checkpoint value.');
    if (Array.isArray(item)) {
      requireEvidence(
        item.length <= maximum && Object.keys(item).length === item.length,
        'Sparse or oversized JSON array.'
      );
      append('[');
      for (let index = 0; index < item.length; index++) {
        requireEvidence(Object.hasOwn(item, index), 'Sparse JSON array.');
        if (index) append(',');
        write(item[index], depth + 1);
      }
      append(']');
      return;
    }
    requireEvidence(
      Object.getPrototypeOf(item) === Object.prototype &&
        Reflect.ownKeys(item).length === Object.keys(item).length,
      'Non-plain checkpoint object.'
    );
    append('{');
    const keys = Object.keys(item).sort();
    keys.forEach((key, index) => {
      requireEvidence(
        Object.hasOwn(Object.getOwnPropertyDescriptor(item, key), 'value'),
        'JSON accessor refused.'
      );
      if (index) append(',');
      write(key, depth + 1);
      append(':');
      write(item[key], depth + 1);
    });
    append('}');
  };
  write(value, 0);
  return chunks.join('');
}

function writerIdentity(writer) {
  fields(writer, ['repository', 'ownerId', 'producer', 'runId', 'attempt']);
  requireEvidence(
    writer.repository === REPOSITORY &&
      integer(writer.ownerId) &&
      integer(writer.runId) &&
      integer(writer.attempt),
    'Invalid checkpoint writer.'
  );
  fields(writer.producer, ['workflowId', 'workflowPath', 'workflowSha256', 'toolingCommit']);
  requireEvidence(
    integer(writer.producer.workflowId) &&
      /^\.github\/workflows\/[A-Za-z0-9][A-Za-z0-9._-]*\.ya?ml$/.test(
        writer.producer.workflowPath
      ) &&
      DIGEST.test(writer.producer.workflowSha256) &&
      SHA.test(writer.producer.toolingCommit),
    'Invalid tooling identity.'
  );
}

function retainReservations(previous, next) {
  for (const field of ['generations', 'unknown'])
    for (const entry of previous.snapshot[field])
      requireEvidence(
        next.snapshot[field].some((candidate) => canonical(candidate) === canonical(entry)),
        'Previous unresolved reservation changed or dropped.'
      );
  for (const entry of previous.terminal)
    requireEvidence(
      next.terminal.some((candidate) => canonical(candidate) === canonical(entry)),
      'Terminal record dropped without qualified compaction.'
    );
  const additions = next.snapshot.generations.filter(
    (entry) =>
      !previous.snapshot.generations.some(
        (old) => rcGenerationKey(old.identity) === rcGenerationKey(entry.identity)
      )
  );
  requireEvidence(additions.length <= 1, 'Multiple reservation additions.');
  if (additions.length) {
    const generation = additions[0];
    const input = structuredClone(next.snapshot);
    input.generations = input.generations.filter(
      (entry) => rcGenerationKey(entry.identity) !== rcGenerationKey(generation.identity)
    );
    const plan = planRCResources(input, { kind: 'reserve', generation });
    requireEvidence(
      plan.status === 'planned' &&
        generation.identity.runId === next.writer.runId &&
        generation.identity.attempt === next.writer.attempt &&
        generation.identity.sourceSha === next.sourceSha,
      'Successor reservation disagrees with original policy/writer.'
    );
  }
}

function validateCheckpoint(checkpoint) {
  fields(checkpoint, [
    'schema',
    'writer',
    'revision',
    'sourceSha',
    'predecessor',
    'snapshot',
    'snapshotDigest',
    'terminal',
  ]);
  writerIdentity(checkpoint.writer);
  requireEvidence(
    checkpoint.schema === 1 && integer(checkpoint.revision) && SHA.test(checkpoint.sourceSha),
    'Invalid checkpoint revision.'
  );
  if (checkpoint.predecessor !== null) {
    fields(checkpoint.predecessor, ['id', 'digest']);
    requireEvidence(
      integer(checkpoint.predecessor.id) && DIGEST.test(checkpoint.predecessor.digest),
      'Invalid checkpoint predecessor.'
    );
  }
  requireEvidence(
    checkpoint.snapshotDigest === hash(canonical(checkpoint.snapshot)),
    'Snapshot digest mismatch.'
  );
  requireEvidence(
    canonical(checkpoint.writer.producer) === canonical(checkpoint.snapshot.approvedProducer),
    'Checkpoint producer mismatch.'
  );
  requireEvidence(
    Array.isArray(checkpoint.terminal) &&
      checkpoint.terminal.length <= LIMIT.terminalEntries &&
      checkpoint.snapshot.journal.terminalEntries === checkpoint.terminal.length,
    'Terminal ring bound/mismatch.'
  );
  for (const entry of checkpoint.terminal) {
    fields(entry, ['generationKey', 'reference']);
    requireEvidence(
      DIGEST.test(entry.generationKey) &&
        typeof entry.reference === 'string' &&
        /^[A-Za-z0-9][A-Za-z0-9._/-]{0,255}$/.test(entry.reference),
      'Invalid terminal record.'
    );
  }
  requireEvidence(
    new Set(checkpoint.terminal.map((entry) => entry.generationKey)).size ===
      checkpoint.terminal.length,
    'Duplicate terminal record.'
  );
  const plan = planRCResources(checkpoint.snapshot, { kind: 'reconcile' });
  requireEvidence(plan.status !== 'refused', plan.reason ?? 'Invalid checkpoint snapshot.');
  return plan;
}

export function encodeRCCheckpoint(checkpoint) {
  validateCheckpoint(checkpoint);
  const bytes = new TextEncoder().encode(canonical(checkpoint, LIMIT.checkpointBytes));
  requireEvidence(bytes.byteLength <= LIMIT.checkpointBytes, 'Checkpoint byte bound.');
  return bytes;
}
export function decodeRCCheckpoint(bytes) {
  const checkpoint = parseFinite(bytes, LIMIT.checkpointBytes);
  validateCheckpoint(checkpoint);
  return checkpoint;
}

/** Snapshot accounting remains with the delivered planner; no upload intent is executed. */
export function prepareRCCheckpoint({ writer, sourceSha, previous, snapshot, request, terminal }) {
  writerIdentity(writer);
  const original = structuredClone(snapshot);
  const plan = planRCResources(original, request);
  requireEvidence(plan.status !== 'refused', plan.reason ?? 'Invalid resource request.');
  requireEvidence(
    canonical(writer.producer) === canonical(original.approvedProducer),
    'Checkpoint producer mismatch.'
  );
  if (request.kind === 'reserve') {
    requireEvidence(
      plan.status === 'planned' && plan.intents.length === 1 && plan.intents[0].kind === 'reserve',
      'Reservation is blocked.'
    );
    requireEvidence(
      request.generation.identity.runId === writer.runId &&
        request.generation.identity.attempt === writer.attempt &&
        request.generation.identity.sourceSha === sourceSha,
      'Reservation writer run/attempt/source mismatch.'
    );
    rcGenerationKey(request.generation.identity);
    original.generations.push(structuredClone(request.generation));
  }
  const checkpoint = {
    schema: 1,
    writer: structuredClone(writer),
    sourceSha,
    revision: previous === null ? 1 : decodeRCCheckpoint(previous.bytes).revision + 1,
    predecessor: previous === null ? null : { id: previous.id, digest: hash(previous.bytes) },
    snapshot: original,
    snapshotDigest: hash(canonical(original)),
    terminal: structuredClone(terminal),
  };
  if (previous !== null) retainReservations(decodeRCCheckpoint(previous.bytes), checkpoint);
  const bytes = encodeRCCheckpoint(checkpoint);
  return { bytes, digest: hash(bytes), charges: validateCheckpoint(checkpoint).charges };
}

function recordedChainCharges(recorded, checkpoint) {
  requireEvidence(
    Array.isArray(recorded) &&
      recorded.length < LIMIT.checkpoints &&
      new Set(recorded.map((entry) => entry.id)).size === recorded.length,
    'Recorded checkpoint count/IDs.'
  );
  for (let index = 0; index < recorded.length; index++) {
    const record = recorded[index];
    requireEvidence(
      integer(record.id) && record.digest === hash(record.bytes),
      'Recorded checkpoint digest/ID mismatch.'
    );
    const decoded = decodeRCCheckpoint(record.bytes);
    requireEvidence(
      decoded.writer.repository === checkpoint.writer.repository &&
        decoded.writer.ownerId === checkpoint.writer.ownerId &&
        canonical(decoded.writer.producer) === canonical(checkpoint.writer.producer),
      'Recorded checkpoint writer/tooling mismatch.'
    );
    if (index > 0) {
      const prior = recorded[index - 1];
      retainReservations(decodeRCCheckpoint(prior.bytes), decoded);
      requireEvidence(
        decoded.predecessor?.id === prior.id &&
          decoded.predecessor?.digest === prior.digest &&
          decoded.revision === decodeRCCheckpoint(prior.bytes).revision + 1,
        'Recorded checkpoint chain/fork mismatch.'
      );
    }
  }
  const predecessor = recorded.at(-1);
  if (predecessor) retainReservations(decodeRCCheckpoint(predecessor.bytes), checkpoint);
  requireEvidence(
    predecessor
      ? checkpoint.predecessor?.id === predecessor.id &&
          checkpoint.predecessor.digest === predecessor.digest &&
          checkpoint.revision === decodeRCCheckpoint(predecessor.bytes).revision + 1
      : checkpoint.predecessor === null && checkpoint.revision === 1,
    'Expected predecessor/revision mismatch.'
  );
  return validateCheckpoint(checkpoint).charges;
}

/** Injected candidate ports only. Custody capture is external evidence, never CAS. */
export async function commitRCCheckpoint({ candidate, recorded, bootstrap, ports, startedAtMs }) {
  // Capture caller-owned bytes before any asynchronous port can mutate aliases.
  candidate = structuredClone(candidate);
  recorded = structuredClone(recorded);
  bootstrap = structuredClone(bootstrap);
  let recovery = null;
  let checkpoint;
  let charges = null;
  const evidence = [];
  let requests = 0;
  let transferredBytes = 0;
  const deadline = startedAtMs + LIMIT.closeMs;
  try {
    // A prior unresolved operation freezes this lane even if the remote inventory is empty.
    const retained = await ports.recovery.load();
    if (retained !== null) {
      recovery = retained;
      checkpoint = decodeRCCheckpoint(retained.candidate.bytes);
      charges = recordedChainCharges(retained.recorded, checkpoint);
      return {
        mode: 'unarmed',
        status: 'frozen',
        reason: 'Unresolved checkpoint operation requires qualified recovery.',
        recovery,
        recorded: recovery.recorded,
        charges,
        evidence,
        requests,
        transferredBytes,
      };
    }
    checkpoint = decodeRCCheckpoint(candidate.bytes);
    charges = recordedChainCharges(recorded, checkpoint);
    requireEvidence(candidate.digest === hash(candidate.bytes), 'Candidate digest mismatch.');
    requireEvidence(
      canonical(candidate.charges) === canonical(charges),
      'Candidate accounting mismatch.'
    );
    const captured = structuredClone(await ports.custody.capture());
    fields(captured, ['writer', 'reference', 'concurrencyDomain', 'cancelInProgress']);
    requireEvidence(
      canonical(captured.writer) === canonical(checkpoint.writer) &&
        typeof captured.reference === 'string' &&
        captured.reference.length > 0 &&
        captured.reference.length <= 256 &&
        captured.concurrencyDomain === 'tmt-pr-rc-writers' &&
        captured.cancelInProgress === false,
      'Missing exclusive trusted-writer custody.'
    );
    requireEvidence(
      Number.isSafeInteger(startedAtMs) &&
        Number.isSafeInteger(deadline) &&
        startedAtMs >= 0 &&
        ports.now() >= startedAtMs &&
        ports.now() < deadline,
      'Original checkpoint deadline exceeded.'
    );
    const call = async (method, path, body = null, expected = 200) => {
      const timeoutMs = Math.min(10_000, deadline - ports.now());
      const requestBody = body === null ? null : new TextEncoder().encode(canonical(body));
      requireEvidence(
        timeoutMs > 0 &&
          requests < LIMIT.reconciliationRequests &&
          transferredBytes + (requestBody?.byteLength ?? 0) <= LIMIT.reconciliationBytes,
        'Checkpoint control-plane budget exceeded.'
      );
      requests++;
      transferredBytes += requestBody?.byteLength ?? 0;
      const controller = new AbortController();
      let timer;
      let response;
      try {
        response = structuredClone(
          await Promise.race([
            ports.transport({
              method,
              path,
              body: requestBody,
              timeoutMs,
              maxResponseBytes: LIMIT.reconciliationBytes - transferredBytes,
              signal: controller.signal,
            }),
            new Promise((_, reject) => {
              timer = setTimeout(() => {
                controller.abort();
                reject(new Error('Checkpoint transport deadline exceeded.'));
              }, timeoutMs);
            }),
          ])
        );
      } finally {
        clearTimeout(timer);
      }
      evidence.push({ method, path, response });
      requireEvidence(response.body instanceof Uint8Array, 'Missing raw response evidence.');
      transferredBytes += response.body.byteLength;
      requireEvidence(
        transferredBytes <= LIMIT.reconciliationBytes &&
          ports.now() < deadline &&
          Number.isSafeInteger(response.elapsedMs) &&
          response.elapsedMs >= 0 &&
          response.elapsedMs <= timeoutMs,
        'Checkpoint response byte/time budget exceeded.'
      );
      requireEvidence(
        response.authenticated?.repository === REPOSITORY &&
          response.authenticated?.ownerId === checkpoint.writer.ownerId,
        'Unauthenticated checkpoint response.'
      );
      requireEvidence(
        response.status === expected,
        `Unexpected checkpoint HTTP ${response.status}.`
      );
      return response;
    };
    const list = async (path, maximum) => {
      const rows = [];
      for (let page = 1; page <= Math.ceil(maximum / 100); page++) {
        const response = await call('GET', `${path}?per_page=100&page=${page}`);
        const batch = parseFinite(response.body, LIMIT.reconciliationBytes);
        requireEvidence(
          Array.isArray(batch) && batch.length <= 100,
          'Invalid checkpoint inventory page.'
        );
        rows.push(...batch);
        requireEvidence(
          rows.length <= maximum &&
            rows.every((row) => integer(row.id)) &&
            new Set(rows.map((row) => row.id)).size === rows.length,
          'Duplicate/oversized checkpoint inventory IDs.'
        );
        if (batch.length < 100) {
          requireEvidence(response.nextPage === null, 'Partial checkpoint pagination.');
          return rows;
        }
        requireEvidence(response.nextPage === page + 1, 'Incomplete checkpoint pagination.');
      }
      throw new Error('Incomplete bounded checkpoint inventory.');
    };
    const matches = (row, record) => {
      requireEvidence(
        row.id === record.id &&
          row.repository_url === `https://api.github.com/repos/${REPOSITORY}` &&
          row.creator?.id === checkpoint.writer.ownerId &&
          row.environment === ENVIRONMENT &&
          row.task === TASK &&
          row.transient_environment === false &&
          row.production_environment === false,
        'Checkpoint Deployment ownership mismatch.'
      );
      requireEvidence(
        typeof row.payload === 'string',
        'Unsupported checkpoint payload representation.'
      );
      const bytes = new TextEncoder().encode(row.payload);
      const decoded = decodeRCCheckpoint(bytes);
      requireEvidence(
        row.sha === decoded.sourceSha &&
          hash(bytes) === record.digest &&
          canonical(decoded) === canonical(decodeRCCheckpoint(record.bytes)),
        'Checkpoint immutable readback mismatch.'
      );
      return decoded;
    };
    const inventory = async (expected) => {
      const rows = await list(ROOT, LIMIT.inventoryRecords);
      const owned = rows.filter((row) => row.environment === ENVIRONMENT || row.task === TASK);
      requireEvidence(
        owned.length === expected.length && owned.length <= LIMIT.checkpoints,
        'Missing checkpoint, fork or unexpected checkpoint inventory.'
      );
      for (const record of expected) {
        const row = owned.find((entry) => entry.id === record.id);
        requireEvidence(row, 'Expected checkpoint missing.');
        matches(row, record);
      }
      return rows;
    };
    const predecessor = recorded.at(-1);
    if (!predecessor) {
      fields(bootstrap, ['writer', 'admissionReference', 'emptySnapshotDigest']);
      requireEvidence(
        canonical(bootstrap.writer) === canonical(checkpoint.writer) &&
          typeof bootstrap.admissionReference === 'string' &&
          bootstrap.admissionReference.length > 0 &&
          bootstrap.admissionReference.length <= 256 &&
          checkpoint.snapshot.generations.length === 0 &&
          checkpoint.snapshot.unknown.length === 0 &&
          bootstrap.emptySnapshotDigest === checkpoint.snapshotDigest,
        'Missing explicit empty bootstrap admission.'
      );
    }
    await inventory(recorded);
    for (const record of recorded)
      matches(
        parseFinite((await call('GET', `${ROOT}/${record.id}`)).body, LIMIT.reconciliationBytes),
        record
      );
    recovery = {
      candidate: structuredClone(candidate),
      recorded: structuredClone(recorded),
      custody: captured,
      phase: 'before-create',
      createdId: null,
      statusId: null,
      pruningId: null,
      evidence: [],
    };
    const save = async (phase) => {
      recovery.phase = phase;
      recovery.evidence = structuredClone(evidence);
      await ports.recovery.save(structuredClone(recovery));
    };
    // A persisted intent precedes every non-idempotent mutation. Recovery never replays it.
    await save('before-create');
    const created = await call(
      'POST',
      ROOT,
      {
        ref: checkpoint.sourceSha,
        task: TASK,
        auto_merge: false,
        required_contexts: [],
        payload: new TextDecoder().decode(candidate.bytes),
        environment: ENVIRONMENT,
        description: 'Unarmed PR RC checkpoint candidate',
        transient_environment: false,
        production_environment: false,
      },
      201
    );
    const row = parseFinite(created.body, LIMIT.reconciliationBytes);
    requireEvidence(integer(row.id), 'Create response lacks actual returned ID.');
    recovery.createdId = row.id;
    await save('after-returned-id');
    const successor = { ...candidate, charges: structuredClone(charges), id: row.id };
    matches(row, successor);
    matches(
      parseFinite((await call('GET', `${ROOT}/${row.id}`)).body, LIMIT.reconciliationBytes),
      successor
    );
    await inventory([...recorded, successor]);
    await save('after-readback');
    const remaining = [...recorded, successor];
    for (const old of recorded) {
      recovery.pruningId = old.id;
      recovery.statusId = null;
      await save('before-inactive-status');
      const statusResponse = await call(
        'POST',
        `${ROOT}/${old.id}/statuses`,
        { state: 'inactive', auto_inactive: false, environment: ENVIRONMENT },
        201
      );
      const status = parseFinite(statusResponse.body, LIMIT.reconciliationBytes);
      requireEvidence(integer(status.id), 'Inactive status lacks returned ID.');
      recovery.statusId = status.id;
      await save('after-returned-status-id');
      const statuses = await list(`${ROOT}/${old.id}/statuses`, LIMIT.inventoryRecords);
      const verified = statuses.find((entry) => entry.id === status.id);
      requireEvidence(
        statuses[0]?.id === status.id &&
          verified?.state === 'inactive' &&
          status.state === 'inactive' &&
          verified.creator?.id === checkpoint.writer.ownerId &&
          status.creator?.id === checkpoint.writer.ownerId &&
          verified.deployment_url === `https://api.github.com${ROOT}/${old.id}` &&
          status.deployment_url === verified.deployment_url &&
          verified.environment === ENVIRONMENT &&
          status.environment === ENVIRONMENT,
        'Inactive status readback mismatch.'
      );
      await save('after-inactive-status');
      await inventory(remaining);
      matches(
        parseFinite((await call('GET', `${ROOT}/${old.id}`)).body, LIMIT.reconciliationBytes),
        old
      );
      await save('before-delete');
      await call('DELETE', `${ROOT}/${old.id}`, null, 204);
      await save('after-delete');
      await call('GET', `${ROOT}/${old.id}`, null, 404);
      remaining.splice(
        remaining.findIndex((record) => record.id === old.id),
        1
      );
      await inventory(remaining);
    }
    await save('complete');
    // Retain completed exact-ID evidence until an independently qualified handoff.
    return {
      mode: 'unarmed',
      status: 'readback-confirmed',
      recorded,
      successor,
      charges,
      evidence,
      requests,
      transferredBytes,
      recovery,
    };
  } catch (error) {
    let reason = error instanceof Error ? error.message : 'Unknown checkpoint failure.';
    if (recovery !== null) {
      recovery.evidence = structuredClone(evidence.length ? evidence : recovery.evidence);
      // This writes recovery evidence only; it never retries a remote mutation.
      try {
        await ports.recovery.save(structuredClone(recovery));
      } catch {
        reason += ' Durable recovery write unconfirmed; retain the complete returned state.';
      }
    }
    return {
      mode: 'unarmed',
      status: recovery ? 'frozen' : 'refused',
      reason,
      recovery,
      recorded: recovery?.recorded ?? recorded,
      charges,
      evidence,
      requests,
      transferredBytes,
    };
  }
}
