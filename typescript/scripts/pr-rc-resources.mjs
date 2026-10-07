import { createHash } from 'node:crypto';

const MiB = 1024 ** 2;
// Local admission ceilings, including proposed journal costs; no service guarantee.
export const RC_RESOURCE_LIMITS = Object.freeze({
  channels: 8,
  incomplete: 1,
  generationBytes: 1024 * MiB,
  generationArtifacts: 64,
  aggregateBytes: 9 * 1024 * MiB,
  manifestBytes: MiB,
  diagnosticBytes: 16 * MiB,
  checkpointBytes: 128 * 1024,
  checkpoints: 3,
  terminalEntries: 32,
  channelRecords: 100,
  inventoryRecords: 1000,
  closeMs: 600_000,
  retentionMs: 3 * 24 * 60 * 60 * 1000,
  reconciliationRequests: 256,
  reconciliationBytes: 8 * MiB,
});

const SHA = /^[a-f0-9]{40}$/;
const DIGEST = /^[a-f0-9]{64}$/;
const TOKEN = /^[A-Za-z0-9][A-Za-z0-9._-]*(?:\/[A-Za-z0-9][A-Za-z0-9._-]*)*$/;
const fail = (condition, reason) => {
  if (!condition) throw new Error(reason);
};
const integer = (n, positive = false) => Number.isSafeInteger(n) && n >= (positive ? 1 : 0);
const reference = (s) => typeof s === 'string' && s.length <= 256 && TOKEN.test(s);
const sha = (s) => typeof s === 'string' && SHA.test(s);
const digest = (s) => typeof s === 'string' && DIGEST.test(s);
const fields = (value, names) => {
  fail(
    value !== null &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value).length === names.length &&
      names.every((key) => Object.hasOwn(value, key)),
    'Incomplete or unknown fields.'
  );
};
const unique = (values) => new Set(values).size === values.length;
const pathOrder = (a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
const sum = (values) =>
  values.reduce((total, n) => {
    fail(integer(n) && integer(total + n), 'Unsafe resource arithmetic.');
    return total + n;
  }, 0);

function producerTuple(producer) {
  fields(producer, ['workflowId', 'workflowPath', 'workflowSha256', 'toolingCommit']);
  fail(
    integer(producer.workflowId, true) &&
      typeof producer.workflowPath === 'string' &&
      /^\.github\/workflows\/[A-Za-z0-9][A-Za-z0-9._-]*\.ya?ml$/.test(producer.workflowPath) &&
      digest(producer.workflowSha256) &&
      sha(producer.toolingCommit),
    'Invalid producer identity.'
  );
  return [
    producer.workflowId,
    producer.workflowPath,
    producer.workflowSha256,
    producer.toolingCommit,
  ];
}

function epochTuple(epoch) {
  fields(epoch, ['labelId', 'eventId', 'enabledAtMs']);
  fail(
    Object.values(epoch).every((n) => integer(n, true)),
    'Invalid eligibility epoch.'
  );
  return [epoch.labelId, epoch.eventId, epoch.enabledAtMs];
}

/** Local journal identity, not a catalog decoder or approval/eligibility resolver. */
export function rcGenerationKey(identity) {
  fields(identity, [
    'repository',
    'pr',
    'sourceSha',
    'epoch',
    'producer',
    'runId',
    'attempt',
    'selection',
  ]);
  fail(
    identity.repository === 'pj-tmt/tmt' &&
      integer(identity.pr, true) &&
      identity.pr <= 2147483647 &&
      sha(identity.sourceSha) &&
      integer(identity.runId, true) &&
      integer(identity.attempt, true),
    'Invalid generation identity.'
  );
  fail(
    Array.isArray(identity.selection) &&
      identity.selection.length > 0 &&
      identity.selection.length <= 16,
    'Incomplete product/target selection.'
  );
  const selection = identity.selection.map((entry) => {
    fields(entry, ['product', 'target']);
    fail(reference(entry.product) && reference(entry.target), 'Invalid product/target identity.');
    return [entry.product, entry.target];
  });
  const keys = selection.map((entry) => JSON.stringify(entry));
  fail(
    unique(keys) && keys.every((key, i) => i === 0 || keys[i - 1] < key),
    'Duplicate or unsorted selection.'
  );
  return createHash('sha256')
    .update(
      JSON.stringify([
        identity.repository,
        identity.pr,
        identity.sourceSha,
        epochTuple(identity.epoch),
        producerTuple(identity.producer),
        identity.runId,
        identity.attempt,
        selection,
      ])
    )
    .digest('hex');
}

function generationCharge(generation) {
  fields(generation, ['identity', 'disposition', 'mutation', 'resources', 'reuse', 'observations']);
  const key = rcGenerationKey(generation.identity);
  fail(
    ['current', 'pending', 'retired'].includes(generation.disposition) &&
      ['none', 'unknown'].includes(generation.mutation),
    'Invalid generation disposition.'
  );
  fail(
    Array.isArray(generation.resources) &&
      generation.resources.length > 0 &&
      generation.resources.length <= RC_RESOURCE_LIMITS.generationArtifacts,
    'Generation artifact count exceeds bound.'
  );
  const resources = generation.resources;
  for (const resource of resources) {
    fields(resource, [
      'path',
      'kind',
      'selectionIndex',
      'transportBytes',
      'metadataBytes',
      'manifestBytes',
      'diagnosticBytes',
      'artifactId',
    ]);
    fail(
      reference(resource.path) &&
        !resource.path.split('/').some((part) => part === '..' || part === '.') &&
        ['payload', 'intermediate', 'catalog', 'diagnostic'].includes(resource.kind),
      'Invalid owned resource.'
    );
    fail(
      resource.kind === 'payload'
        ? integer(resource.selectionIndex) &&
            resource.selectionIndex < generation.identity.selection.length
        : resource.selectionIndex === null,
      'Unbound product/target output.'
    );
    fail(
      integer(resource.transportBytes, true) &&
        integer(resource.metadataBytes, true) &&
        integer(resource.manifestBytes) &&
        integer(resource.diagnosticBytes) &&
        (resource.artifactId === null || integer(resource.artifactId, true)),
      'Missing complete resource reservation.'
    );
    // Frozen transport maxima are reserved even when today's raw payload is small.
    fail(
      (resource.kind !== 'payload' || resource.transportBytes >= 69 * MiB) &&
        (resource.kind !== 'catalog' || resource.transportBytes >= 2 * MiB),
      'Incomplete transport worst case.'
    );
    fail(
      resource.transportBytes >= resource.manifestBytes + resource.diagnosticBytes,
      'Metadata content exceeds transport reservation.'
    );
  }
  fail(
    unique(resources.map((r) => r.path)) &&
      unique(resources.filter((r) => r.artifactId !== null).map((r) => r.artifactId)),
    'Duplicate output ownership.'
  );
  fail(
    resources.filter((r) => r.kind === 'catalog').length === 1,
    'One catalog reservation required.'
  );
  fail(
    sum(resources.map((r) => r.manifestBytes)) <= RC_RESOURCE_LIMITS.manifestBytes &&
      sum(resources.map((r) => r.diagnosticBytes)) <= RC_RESOURCE_LIMITS.diagnosticBytes,
    'Manifest or diagnostic bound exceeded.'
  );
  const bytes = sum(resources.flatMap((r) => [r.transportBytes, r.metadataBytes]));
  fail(bytes <= RC_RESOURCE_LIMITS.generationBytes, 'Generation byte bound exceeded.');
  const payloadIndices = resources.filter((r) => r.kind === 'payload').map((r) => r.selectionIndex);
  fail(
    unique(payloadIndices) && payloadIndices.length === generation.identity.selection.length,
    'Incomplete selected payload reservation.'
  );
  fail(
    Array.isArray(generation.reuse) && generation.reuse.length <= 64,
    'Incomplete reuse provenance.'
  );
  for (const upstream of generation.reuse) {
    fields(upstream, ['repository', 'runId', 'attempt', 'artifactId', 'sha256']);
    fail(
      upstream.repository === 'pj-tmt/tmt' &&
        integer(upstream.runId, true) &&
        integer(upstream.attempt, true) &&
        integer(upstream.artifactId, true) &&
        digest(upstream.sha256),
      'Invalid reuse provenance.'
    );
  }
  fail(unique(generation.reuse.map((r) => r.artifactId)), 'Duplicate reuse provenance.');
  fields(generation.observations, ['reservation', 'settlement', 'inventory', 'absence']);
  for (const [kind, observation] of Object.entries(generation.observations)) {
    fields(observation, ['generationKey', 'reference', 'state', 'artifactIds']);
    fail(
      observation.generationKey === key &&
        reference(observation.reference) &&
        ['confirmed', 'unknown'].includes(observation.state) &&
        Array.isArray(observation.artifactIds) &&
        observation.artifactIds.length <= 64 &&
        observation.artifactIds.every((id) => integer(id, true)) &&
        unique(observation.artifactIds),
      'Mismatched or incomplete observation.'
    );
    fail(
      ['inventory', 'absence'].includes(kind) || observation.artifactIds.length === 0,
      'Unexpected observation inventory.'
    );
  }
  const ids = resources
    .filter((r) => r.artifactId !== null)
    .map((r) => r.artifactId)
    .sort((a, b) => a - b);
  const sameIds = (observed) =>
    JSON.stringify([...observed].sort((a, b) => a - b)) === JSON.stringify(ids);
  for (const kind of ['inventory', 'absence']) {
    if (generation.observations[kind].state === 'confirmed')
      fail(sameIds(generation.observations[kind].artifactIds), 'Unmatched inventory or absence.');
  }
  return { key, bytes, artifacts: resources.length };
}

function currentChannel(generation, channels) {
  const identity = generation.identity;
  return channels.find(
    (channel) =>
      channel.pr === identity.pr &&
      channel.state === 'enabled' &&
      channel.sourceRepository === identity.repository &&
      channel.sourceSha === identity.sourceSha &&
      JSON.stringify(epochTuple(channel.epoch)) === JSON.stringify(epochTuple(identity.epoch))
  );
}

/** All outputs are conditional intents. No returned flag attests to a remote effect. */
export function planRCResources(snapshot, request) {
  let charges = null;
  try {
    fail(
      Buffer.byteLength(JSON.stringify({ snapshot, request })) <=
        RC_RESOURCE_LIMITS.checkpointBytes,
      'Checkpoint representation exceeds local bound.'
    );
    fields(snapshot, [
      'repository',
      'reference',
      'complete',
      'approvedProducer',
      'channels',
      'generations',
      'unknown',
      'journal',
    ]);
    fail(
      snapshot.repository === 'pj-tmt/tmt' &&
        reference(snapshot.reference) &&
        snapshot.complete === true,
      'Incomplete official inventory.'
    );
    const producer = JSON.stringify(producerTuple(snapshot.approvedProducer));
    fields(snapshot.journal, ['checkpointBytes', 'checkpoints', 'terminalEntries']);
    fail(
      integer(snapshot.journal.checkpointBytes, true) &&
        snapshot.journal.checkpointBytes <= RC_RESOURCE_LIMITS.checkpointBytes &&
        integer(snapshot.journal.checkpoints, true) &&
        snapshot.journal.checkpoints <= RC_RESOURCE_LIMITS.checkpoints &&
        integer(snapshot.journal.terminalEntries) &&
        snapshot.journal.terminalEntries <= RC_RESOURCE_LIMITS.terminalEntries,
      'Unbounded journal representation.'
    );
    fail(
      Array.isArray(snapshot.channels) &&
        snapshot.channels.length <= RC_RESOURCE_LIMITS.channelRecords &&
        Array.isArray(snapshot.generations) &&
        snapshot.generations.length <= 9 &&
        Array.isArray(snapshot.unknown) &&
        snapshot.unknown.length <= RC_RESOURCE_LIMITS.inventoryRecords,
      'Incomplete or oversized inventory.'
    );
    for (const channel of snapshot.channels) {
      fields(channel, ['pr', 'sourceRepository', 'sourceSha', 'epoch', 'state', 'reference']);
      fail(
        integer(channel.pr, true) &&
          channel.pr <= 2147483647 &&
          channel.sourceRepository === 'pj-tmt/tmt' &&
          sha(channel.sourceSha) &&
          ['enabled', 'disabled', 'closed', 'merged'].includes(channel.state) &&
          reference(channel.reference),
        'Invalid channel observation.'
      );
      epochTuple(channel.epoch);
    }
    fail(unique(snapshot.channels.map((c) => c.pr)), 'Forked channel observations.');
    const records = snapshot.generations.map(generationCharge);
    fail(unique(records.map((r) => r.key)), 'Duplicate generation identity.');
    for (const unknown of snapshot.unknown) {
      fields(unknown, ['reference', 'bytes', 'artifacts']);
      fail(
        reference(unknown.reference) &&
          integer(unknown.bytes, true) &&
          integer(unknown.artifacts, true),
        'Unknown resource has no conservative charge.'
      );
    }
    fail(unique(snapshot.unknown.map((r) => r.reference)), 'Duplicate unknown charge.');
    // Reserve all three maximum checkpoints even when the observed backend representation is smaller.
    const journalBytes = RC_RESOURCE_LIMITS.checkpointBytes * RC_RESOURCE_LIMITS.checkpoints;
    charges = {
      bytes: sum([
        journalBytes,
        ...records.map((r) => r.bytes),
        ...snapshot.unknown.map((r) => r.bytes),
      ]),
      artifacts: sum([
        ...records.map((r) => r.artifacts),
        ...snapshot.unknown.map((r) => r.artifacts),
      ]),
    };
    fail(
      charges.artifacts <= RC_RESOURCE_LIMITS.inventoryRecords,
      'Reconciliation inventory bound exceeded.'
    );
    const ownedIds = snapshot.generations.flatMap((g) =>
      g.resources.filter((r) => r.artifactId !== null).map((r) => r.artifactId)
    );
    const reuseIds = snapshot.generations.flatMap((g) => g.reuse.map((r) => r.artifactId));
    const outputKeys = snapshot.generations.flatMap((g) =>
      g.resources.map((r) => JSON.stringify([g.identity.runId, g.identity.attempt, r.path]))
    );
    fail(
      unique(ownedIds) && unique(outputKeys) && !reuseIds.some((id) => ownedIds.includes(id)),
      'Conflicting or adopted output ownership.'
    );
    const currents = snapshot.generations.filter((g) => g.disposition === 'current');
    fail(unique(currents.map((g) => g.identity.pr)), 'Multiple current generations for a PR.');
    const incomplete = snapshot.generations.filter(
      (g) =>
        g.disposition !== 'current' ||
        g.mutation === 'unknown' ||
        g.observations.absence.state === 'confirmed' ||
        ['reservation', 'settlement', 'inventory'].some(
          (kind) => g.observations[kind].state !== 'confirmed'
        ) ||
        g.resources.some((r) => r.artifactId === null) ||
        !currentChannel(g, snapshot.channels)
    );
    const capacity = () => {
      fail(
        snapshot.channels.filter((c) => c.state === 'enabled').length <=
          RC_RESOURCE_LIMITS.channels,
        'Open channel bound exceeded.'
      );
      fail(charges.bytes <= RC_RESOURCE_LIMITS.aggregateBytes, 'Aggregate byte bound exceeded.');
      fail(
        incomplete.length <= RC_RESOURCE_LIMITS.incomplete,
        'Incomplete generation bound exceeded.'
      );
    };
    capacity();
    const intents = [];
    const blocked = [];
    if (snapshot.unknown.length)
      blocked.push({ generationKey: null, obligations: ['unmatched-resources'] });
    fields(request, request?.kind === 'reserve' ? ['kind', 'generation'] : ['kind']);
    fail(['reserve', 'reconcile'].includes(request.kind), 'Unknown resource request.');
    if (request.kind === 'reserve') {
      fail(
        snapshot.unknown.length === 0 &&
          !snapshot.generations.some((g) => g.mutation === 'unknown'),
        'Uncertain resources remain charged.'
      );
      const candidate = request.generation;
      const reservation = generationCharge(candidate);
      fail(
        !records.some((r) => r.key === reservation.key),
        'Generation already reserved; reconcile by exact identity.'
      );
      fail(
        !candidate.resources.some((r) =>
          outputKeys.includes(
            JSON.stringify([candidate.identity.runId, candidate.identity.attempt, r.path])
          )
        ) && !candidate.reuse.some((r) => ownedIds.includes(r.artifactId)),
        'Conflicting proposed output ownership.'
      );
      fail(
        candidate.disposition === 'pending' &&
          candidate.mutation === 'none' &&
          currentChannel(candidate, snapshot.channels) &&
          JSON.stringify(producerTuple(candidate.identity.producer)) === producer,
        'Candidate identity is not current/approved.'
      );
      fail(
        candidate.resources.every((r) => r.artifactId === null) &&
          Object.values(candidate.observations).every(
            (o) => o.state === 'unknown' && o.artifactIds.length === 0
          ),
        'Fresh reservation contains effect claims.'
      );
      fail(
        incomplete.length === 0 && snapshot.generations.length < 9,
        'Replacement slot remains charged.'
      );
      fail(
        sum([charges.bytes, reservation.bytes]) <= RC_RESOURCE_LIMITS.aggregateBytes,
        'Aggregate reservation exceeds bound.'
      );
      intents.push({
        kind: 'reserve',
        generationKey: reservation.key,
        bytes: reservation.bytes,
        artifacts: reservation.artifacts,
        requires: [
          'exclusive-durable-journal-commit-and-readback',
          'complete-transport-and-metadata-bound-enforcement',
        ],
      });
    } else {
      for (let i = 0; i < snapshot.generations.length; i++) {
        const generation = snapshot.generations[i];
        const generationKey = records[i].key;
        const observation = generation.observations;
        const retiring =
          generation.disposition === 'retired' || !currentChannel(generation, snapshot.channels);
        if (retiring) {
          intents.push({
            kind: 'retire-discovery',
            generationKey,
            requires: ['exclusive-durable-retirement-commit'],
          });
          if (observation.settlement.state !== 'confirmed')
            intents.push({
              kind: 'cancel',
              generationKey,
              runId: generation.identity.runId,
              attempt: generation.identity.attempt,
              requires: ['discovery-retired', 'trusted-adapter-exact-RC-run-ownership'],
            });
          const obligations = ['settlement', 'inventory', 'absence'].filter(
            (kind) => observation[kind].state !== 'confirmed'
          );
          if (generation.mutation === 'unknown') obligations.push('mutation-outcome');
          if (snapshot.unknown.length) obligations.push('unmatched-resources');
          if (
            observation.settlement.state === 'confirmed' &&
            observation.inventory.state === 'confirmed' &&
            observation.absence.state !== 'confirmed' &&
            generation.mutation === 'none' &&
            snapshot.unknown.length === 0
          ) {
            const ordered = [...generation.resources].sort(
              (a, b) =>
                Number(b.kind === 'catalog') - Number(a.kind === 'catalog') || pathOrder(a, b)
            );
            for (const resource of ordered.filter((r) => r.artifactId !== null))
              intents.push({
                kind: 'delete',
                generationKey,
                artifactId: resource.artifactId,
                path: resource.path,
                requires: [
                  'discovery-retired',
                  'upload-settled',
                  'complete-exact-owned-inventory',
                  ...(resource.kind === 'catalog' ? [] : ['catalog-absence-confirmed']),
                  'trusted-adapter-revalidate-ownership',
                ],
              });
          }
          if (!obligations.length)
            intents.push({
              kind: 'release-reservation',
              generationKey,
              requires: [
                'trusted-adapter-revalidate-settlement-inventory-absence',
                'exclusive-durable-release-commit',
              ],
            });
          else blocked.push({ generationKey, obligations });
        } else if (generation.disposition === 'current' && incomplete.includes(generation)) {
          blocked.push({ generationKey, obligations: ['current-publication-unconfirmed'] });
        } else if (generation.disposition === 'pending') {
          const obligations = [];
          if (observation.reservation.state !== 'confirmed')
            obligations.push('durable-reservation');
          if (generation.mutation === 'unknown' || snapshot.unknown.length)
            obligations.push('uncertain-resources');
          if (JSON.stringify(producerTuple(generation.identity.producer)) !== producer)
            obligations.push('approved-producer');
          if (obligations.length) blocked.push({ generationKey, obligations });
          else {
            const ordered = [...generation.resources].sort(
              (a, b) =>
                Number(a.kind === 'catalog') - Number(b.kind === 'catalog') || pathOrder(a, b)
            );
            for (const resource of ordered.filter((r) => r.artifactId === null))
              intents.push({
                kind: 'upload',
                generationKey,
                path: resource.path,
                requires: [
                  'trusted-adapter-revalidate-durable-reservation-and-current-Core-eligibility',
                  'complete-transport-and-metadata-bound-enforcement',
                  ...(resource.kind === 'catalog'
                    ? ['all-payloads-independently-verified-and-settled']
                    : []),
                ],
              });
          }
        }
      }
    }
    // Charges do not decrease even when a conditional release intent is proposed.
    return {
      mode: 'unarmed',
      status: blocked.length ? 'blocked' : 'planned',
      charges,
      intents,
      blocked,
      evidence: [snapshot.reference],
    };
  } catch (error) {
    return {
      mode: 'unarmed',
      status: 'refused',
      reason: error instanceof Error ? error.message.slice(0, 256) : 'Invalid resource input.',
      charges,
      intents: [],
      blocked: [],
      evidence: reference(snapshot?.reference) ? [snapshot.reference] : [],
    };
  }
}
