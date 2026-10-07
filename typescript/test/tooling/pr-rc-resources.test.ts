import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import {
  planRCResources,
  rcGenerationKey,
  type RCGeneration,
  type RCIdentity,
  type RCResource,
  type RCSnapshot,
} from '../../scripts/pr-rc-resources.mjs';

// Synthetic planning data only: these IDs do not register producer trust.
const MiB = 1024 ** 2;
const GiB = 1024 * MiB;
const journalBytes = 3 * 128 * 1024;
const producer = {
  workflowId: 10,
  workflowPath: '.github/workflows/synthetic-rc.yml',
  workflowSha256: 'b'.repeat(64),
  toolingCommit: 'c'.repeat(40),
};
const epoch = { labelId: 10, eventId: 20, enabledAtMs: 1000 };
const identity = (pr = 1, runId = pr): RCIdentity => ({
  repository: 'pj-tmt/tmt',
  pr,
  sourceSha: 'a'.repeat(40),
  epoch: { ...epoch },
  producer: { ...producer },
  runId,
  attempt: 1,
  selection: [{ product: 'cli', target: 'aarch64-apple-darwin' }],
});
const resource = (path: string, kind: RCResource['kind'], transportBytes: number): RCResource => ({
  path,
  kind,
  selectionIndex: kind === 'payload' ? 0 : null,
  transportBytes,
  metadataBytes: 1,
  manifestBytes: kind === 'catalog' ? 1024 : 0,
  diagnosticBytes: 0,
  artifactId: null,
});
function generation(pr = 1, runId = pr): RCGeneration {
  const id = identity(pr, runId);
  const key = rcGenerationKey(id);
  const observation = (reference: string) => ({
    generationKey: key,
    reference,
    state: 'unknown' as const,
    artifactIds: [] as number[],
  });
  return {
    identity: id,
    disposition: 'pending',
    mutation: 'none',
    resources: [
      resource('catalog.zip', 'catalog', 2 * MiB),
      resource('payload.zip', 'payload', 69 * MiB),
    ],
    reuse: [
      {
        repository: 'pj-tmt/tmt',
        runId: 999,
        attempt: 2,
        artifactId: 9000,
        sha256: 'd'.repeat(64),
      },
    ],
    observations: {
      reservation: observation('journal/1'),
      settlement: observation('settlement/1'),
      inventory: observation('inventory/1'),
      absence: observation('absence/1'),
    },
  };
}
function published(pr = 1): RCGeneration {
  const g = generation(pr);
  g.disposition = 'current';
  g.resources.forEach((r, i) => {
    r.artifactId = pr * 100 + i;
  });
  for (const kind of ['reservation', 'settlement', 'inventory'] as const)
    g.observations[kind].state = 'confirmed';
  g.observations.inventory.artifactIds = g.resources.map((r) => r.artifactId!);
  return g;
}
const snapshot = (generations: RCGeneration[] = [], count = 1): RCSnapshot => ({
  repository: 'pj-tmt/tmt',
  reference: 'snapshot/1',
  complete: true,
  approvedProducer: { ...producer },
  channels: Array.from({ length: count }, (_, i) => ({
    pr: i + 1,
    sourceRepository: 'pj-tmt/tmt',
    sourceSha: 'a'.repeat(40),
    epoch: { ...epoch },
    state: 'enabled',
    reference: `channel/${i + 1}`,
  })),
  generations,
  unknown: [],
  journal: { checkpointBytes: 128 * 1024, checkpoints: 3, terminalEntries: 32 },
});
const reconcile = (s: RCSnapshot) => planRCResources(s, { kind: 'reconcile' });
const reserve = (s = snapshot(), g = generation()) =>
  planRCResources(s, { kind: 'reserve', generation: g });
const refuse = (result: ReturnType<typeof reconcile>, reason: string) => {
  expect(result.status).toBe('refused');
  expect(result.reason).toContain(reason);
  expect(result.intents).toEqual([]);
};
function atBytes(g: RCGeneration, bytes: number) {
  const catalog = resource('catalog.zip', 'catalog', 2 * MiB);
  const payload = resource('payload.zip', 'payload', 69 * MiB);
  const remainder = resource('remaining.zip', 'intermediate', bytes - 71 * MiB - 3);
  g.resources = [catalog, payload, remainder];
  if (g.disposition === 'current') {
    g.resources.forEach((r, i) => {
      r.artifactId = g.identity.pr * 100 + i;
    });
    g.observations.inventory.artifactIds = g.resources.map((r) => r.artifactId!);
  }
  return g;
}

describe('complete generation reservations', () => {
  it('reserves transport, metadata and checkpoint overlap before any upload intent', () => {
    const result = reserve();
    expect(result.status).toBe('planned');
    expect(result.mode).toBe('unarmed');
    expect(result.charges).toEqual({ bytes: journalBytes, artifacts: 0 });
    expect(result.intents).toEqual([
      {
        kind: 'reserve',
        generationKey: rcGenerationKey(identity()),
        bytes: 71 * MiB + 2,
        artifacts: 2,
        requires: [
          'exclusive-durable-journal-commit-and-readback',
          'complete-transport-and-metadata-bound-enforcement',
        ],
      },
    ]);
  });
  it('does not mutate input on success or refusal', () => {
    const s = snapshot();
    const g = generation();
    const original = JSON.stringify({ s, g });
    reserve(s, g);
    expect(JSON.stringify({ s, g })).toBe(original);
    s.complete = false;
    const refused = JSON.stringify({ s, g });
    reserve(s, g);
    expect(JSON.stringify({ s, g })).toBe(refused);
  });
  it.each([GiB, GiB + 1])('enforces the exact generation byte boundary %i', (bytes) => {
    const result = reserve(snapshot(), atBytes(generation(), bytes));
    if (bytes === GiB) expect(result.intents[0].bytes).toBe(GiB);
    else refuse(result, 'Generation byte');
  });
  it.each([64, 65])('enforces artifact count %i without dropping records', (count) => {
    const g = generation();
    g.resources = [
      resource('catalog.zip', 'catalog', 2 * MiB),
      resource('payload.zip', 'payload', 69 * MiB),
      ...Array.from({ length: count - 2 }, (_, i) =>
        resource(`intermediate-${i}.zip`, 'intermediate', 1)
      ),
    ];
    const result = reserve(snapshot(), g);
    if (count === 64) expect(result.intents[0].artifacts).toBe(64);
    else refuse(result, 'artifact count');
  });
  it('refuses sixteen maximum payloads before upload despite each transport fitting individually', () => {
    const g = generation();
    g.identity.selection = Array.from({ length: 16 }, (_, i) => ({
      product: 'cli',
      target: `synthetic-${i.toString().padStart(2, '0')}`,
    }));
    Object.values(g.observations).forEach((o) => {
      o.generationKey = rcGenerationKey(g.identity);
    });
    g.resources = [
      resource('catalog.zip', 'catalog', 2 * MiB),
      ...Array.from({ length: 16 }, (_, i) => ({
        ...resource(`payload-${i}.zip`, 'payload', 69 * MiB),
        selectionIndex: i,
      })),
    ];
    refuse(reserve(snapshot(), g), 'Generation byte');
  });
  it.each(['manifestBytes', 'diagnosticBytes'] as const)(
    'enforces %s exact and over boundaries',
    (field) => {
      const bound = field === 'manifestBytes' ? MiB : 16 * MiB;
      const g = generation();
      g.resources[0].transportBytes = 32 * MiB;
      g.resources[0][field] = bound;
      expect(reserve(snapshot(), g).status).toBe('planned');
      g.resources[0][field]++;
      refuse(reserve(snapshot(), g), 'Manifest or diagnostic');
    }
  );
  it.each([8, 9])('enforces %i open channels', (channels) => {
    const result = reserve(snapshot([], channels));
    if (channels === 8) expect(result.status).toBe('planned');
    else refuse(result, 'Open channel');
  });
  it('charges eight currents plus replacement and journal at the exact aggregate boundary', () => {
    const s = snapshot(
      Array.from({ length: 8 }, (_, i) => atBytes(published(i + 1), GiB)),
      8
    );
    const g = atBytes(generation(1, 100), GiB - journalBytes);
    expect(reserve(s, g).status).toBe('planned');
    g.resources[2].transportBytes++;
    refuse(reserve(s, g), 'Aggregate reservation');
  });
  it('retains over-ceiling unknown charges without clipping and refuses expansion', () => {
    const s = snapshot();
    s.unknown = [{ reference: 'ordinary/same-name', bytes: 9 * GiB, artifacts: 1 }];
    const result = reserve(s);
    refuse(result, 'Aggregate byte');
    expect(result.charges).toEqual({ bytes: 9 * GiB + journalBytes, artifacts: 1 });
  });
  it('reports unattributed charges red even with no generation and no possible intents', () => {
    const s = snapshot();
    s.unknown = [{ reference: 'unknown/1', bytes: MiB, artifacts: 1 }];
    const result = reconcile(s);
    expect(result.status).toBe('blocked');
    expect(result.blocked).toEqual([{ generationKey: null, obligations: ['unmatched-resources'] }]);
    expect(result.charges).toEqual({ bytes: journalBytes + MiB, artifacts: 1 });
    expect(result.intents).toEqual([]);
  });
  it.each(['mutation', 'inventory', 'absence'] as const)(
    'keeps uncertain/contradictory current %s red and in the incomplete slot',
    (kind) => {
      const g = published();
      if (kind === 'mutation') g.mutation = 'unknown';
      if (kind === 'inventory') g.observations.inventory.state = 'unknown';
      if (kind === 'absence') {
        g.observations.absence.state = 'confirmed';
        g.observations.absence.artifactIds = [100, 101];
      }
      const s = snapshot([g]);
      const result = reconcile(s);
      expect(result.status).toBe('blocked');
      expect(result.blocked[0].obligations).toContain('current-publication-unconfirmed');
      expect(result.charges!.bytes).toBe(journalBytes + 71 * MiB + 2);
      expect(result.intents).toEqual([]);
      expect(reserve(s, generation(1, 2)).status).toBe('refused');
    }
  );
  it('requires transport and metadata worst cases, not raw member sizes', () => {
    const g = generation();
    g.resources[1].transportBytes = 64 * MiB;
    refuse(reserve(snapshot(), g), 'transport worst case');
    g.resources[1].transportBytes = 69 * MiB;
    g.resources[1].metadataBytes = 0;
    refuse(reserve(snapshot(), g), 'complete resource reservation');
  });
  it('binds one reserved payload to every selected pair without accepting missing, duplicate or extra bindings', () => {
    const g = generation();
    g.identity.selection.push({ product: 'cli', target: 'x86_64-unknown-linux-musl' });
    Object.values(g.observations).forEach((o) => {
      o.generationKey = rcGenerationKey(g.identity);
    });
    refuse(reserve(snapshot(), g), 'Incomplete selected payload');
    g.resources.push({ ...resource('second.zip', 'payload', 69 * MiB), selectionIndex: 1 });
    expect(reserve(snapshot(), g).status).toBe('planned');
    g.resources[2].selectionIndex = 0;
    refuse(reserve(snapshot(), g), 'Incomplete selected payload');
    g.resources[2].selectionIndex = 2;
    refuse(reserve(snapshot(), g), 'Unbound product/target');
    g.resources[2].selectionIndex = 1;
    g.resources[0].selectionIndex = 0;
    refuse(reserve(snapshot(), g), 'Unbound product/target');
  });
  it('does not turn a fresh reservation containing existing effects into upload permission', () => {
    const g = generation();
    g.observations.reservation.state = 'confirmed';
    refuse(reserve(snapshot(), g), 'effect claims');
    const s = snapshot([generation()]);
    refuse(reserve(s, generation(1, 2)), 'Replacement slot');
    refuse(reserve(s, generation()), 'already reserved');
  });
  it('requires reservation readback and catalog-last independent verification obligations', () => {
    const g = generation();
    const s = snapshot([g]);
    expect(reconcile(s).intents).toEqual([]);
    expect(reconcile(s).blocked[0].obligations).toContain('durable-reservation');
    g.observations.reservation.state = 'confirmed';
    const result = reconcile(s);
    expect(result.intents.map((i) => i.path)).toEqual(['payload.zip', 'catalog.zip']);
    expect(result.intents[1].requires).toContain('all-payloads-independently-verified-and-settled');
    g.mutation = 'unknown';
    expect(reconcile(s).intents).toEqual([]);
    expect(reconcile(s).charges).toEqual(result.charges);
  });
  it('blocks complete recorded-output disappearance while retaining pending charges and input', () => {
    const g = published();
    g.disposition = 'pending';
    g.observations.absence.state = 'confirmed';
    g.observations.absence.artifactIds = [100, 101];
    const s = snapshot([g]);
    const original = JSON.stringify(s);
    const result = reconcile(s);
    expect(result.status).toBe('blocked');
    expect(result.blocked).toEqual([
      { generationKey: rcGenerationKey(g.identity), obligations: ['recorded-output-absent'] },
    ]);
    expect(result.intents).toEqual([]);
    expect(result.charges).toEqual({ bytes: journalBytes + 71 * MiB + 2, artifacts: 2 });
    expect(JSON.stringify(s)).toBe(original);
  });
  it('blocks payload upload when only the recorded catalog is confirmed absent', () => {
    const g = generation();
    g.resources[0].artifactId = 100;
    for (const kind of ['reservation', 'settlement', 'inventory', 'absence'] as const)
      g.observations[kind].state = 'confirmed';
    g.observations.inventory.artifactIds = [100];
    g.observations.absence.artifactIds = [100];
    const s = snapshot([g]);
    const original = JSON.stringify(s);
    const result = reconcile(s);
    expect(result.status).toBe('blocked');
    expect(result.blocked[0].obligations).toContain('recorded-output-absent');
    expect(result.intents).toEqual([]);
    expect(result.charges).toEqual({ bytes: journalBytes + 71 * MiB + 2, artifacts: 2 });
    expect(JSON.stringify(s)).toBe(original);
  });
  it('preserves empty pre-upload absence with conditional catalog-last upload obligations', () => {
    const g = generation();
    for (const kind of ['reservation', 'settlement', 'inventory', 'absence'] as const)
      g.observations[kind].state = 'confirmed';
    const s = snapshot([g]);
    const original = JSON.stringify(s);
    const result = reconcile(s);
    expect(result.status).toBe('planned');
    expect(result.blocked).toEqual([]);
    expect(result.intents.map((i) => [i.kind, i.path])).toEqual([
      ['upload', 'payload.zip'],
      ['upload', 'catalog.zip'],
    ]);
    for (const intent of result.intents)
      expect(intent.requires).toContain(
        'trusted-adapter-revalidate-durable-reservation-and-current-Core-eligibility'
      );
    expect(result.intents[1].requires).toContain('all-payloads-independently-verified-and-settled');
    expect(result.charges).toEqual({ bytes: journalBytes + 71 * MiB + 2, artifacts: 2 });
    expect(JSON.stringify(s)).toBe(original);
  });
  it.each(['workflowId', 'workflowPath', 'workflowSha256', 'toolingCommit'] as const)(
    'keeps a current generation with changed approved %s charged and unresolved',
    (field) => {
      const g = published();
      const s = snapshot([g]);
      if (field === 'workflowId') s.approvedProducer.workflowId++;
      if (field === 'workflowPath') s.approvedProducer.workflowPath = '.github/workflows/other.yml';
      if (field === 'workflowSha256') s.approvedProducer.workflowSha256 = 'f'.repeat(64);
      if (field === 'toolingCommit') s.approvedProducer.toolingCommit = 'f'.repeat(40);
      const original = JSON.stringify(s);
      const result = reconcile(s);
      expect(result.status).toBe('blocked');
      expect(result.blocked).toEqual([
        {
          generationKey: rcGenerationKey(g.identity),
          obligations: ['current-publication-unconfirmed', 'approved-producer'],
        },
      ]);
      expect(result.intents).toEqual([]);
      expect(result.charges).toEqual({ bytes: journalBytes + 71 * MiB + 2, artifacts: 2 });
      expect(JSON.stringify(s)).toBe(original);
    }
  );
  it('charges a mismatched current producer against the replacement slot, not candidate approval', () => {
    const s = snapshot([published()]);
    s.approvedProducer.toolingCommit = 'f'.repeat(40);
    const replacement = generation(1, 2);
    replacement.identity.producer = { ...s.approvedProducer };
    Object.values(replacement.observations).forEach((o) => {
      o.generationKey = rcGenerationKey(replacement.identity);
    });
    expect(reserve(snapshot(), replacement).reason).toContain('not current/approved');
    const original = JSON.stringify({ s, replacement });
    const result = reserve(s, replacement);
    refuse(result, 'Replacement slot');
    expect(result.charges).toEqual({ bytes: journalBytes + 71 * MiB + 2, artifacts: 2 });
    expect(JSON.stringify({ s, replacement })).toBe(original);
    const matching = snapshot([published()]);
    expect(reconcile(matching).status).toBe('planned');
    expect(reserve(matching, generation(1, 2)).status).toBe('planned');
  });
  it('requires exclusive revalidation for competing triggers; the recorded winner blocks another reservation', () => {
    const s = snapshot();
    const first = generation(1, 1);
    const second = generation(1, 2);
    for (const proposal of [first, second]) {
      const result = reserve(s, proposal);
      expect(result.intents.map((i) => i.kind)).toEqual(['reserve']);
      expect(result.intents[0].requires).toContain('exclusive-durable-journal-commit-and-readback');
      expect(result.evidence).toEqual(['snapshot/1']);
    }
    s.generations.push(first);
    refuse(reserve(s, second), 'Replacement slot');
    expect(s.generations).toEqual([first]);
  });
});

describe('exact identity and complete finite observations', () => {
  it.each([
    'repository',
    'pr',
    'sourceSha',
    'epoch',
    'producer',
    'runId',
    'attempt',
    'selection',
  ] as const)('binds %s into every observation owner', (field) => {
    const g = generation();
    const old = rcGenerationKey(g.identity);
    if (field === 'repository') g.identity.repository = 'fork/tmt';
    else if (field === 'epoch') g.identity.epoch.eventId++;
    else if (field === 'producer') g.identity.producer.toolingCommit = 'f'.repeat(40);
    else if (field === 'selection') g.identity.selection[0].target = 'x86_64-unknown-linux-musl';
    else if (field === 'sourceSha') g.identity.sourceSha = 'f'.repeat(40);
    else g.identity[field]++;
    const result = reserve(snapshot(), g);
    refuse(result, field === 'repository' ? 'Invalid generation identity' : 'Mismatched');
    if (field !== 'repository') expect(rcGenerationKey(g.identity)).not.toBe(old);
  });
  it.each(['pr', 'sourceSha', 'epoch', 'producer'] as const)(
    'refuses rebound but noncurrent %s instead of trusting observation references',
    (field) => {
      const g = generation();
      if (field === 'pr') g.identity.pr = 2;
      if (field === 'sourceSha') g.identity.sourceSha = 'f'.repeat(40);
      if (field === 'epoch') g.identity.epoch.eventId++;
      if (field === 'producer') g.identity.producer.workflowSha256 = 'f'.repeat(64);
      Object.values(g.observations).forEach((o) => {
        o.generationKey = rcGenerationKey(g.identity);
      });
      refuse(reserve(snapshot(), g), 'not current/approved');
    }
  );
  it.each([
    'complete',
    'fork',
    'duplicate',
    'unsafe',
    'unknown',
    'incomplete',
    'path',
    'catalog',
    'selection',
  ] as const)('refuses %s input independently', (caseName) => {
    const s = snapshot();
    const g = generation();
    if (caseName === 'complete') s.complete = false;
    if (caseName === 'fork') s.channels[0].sourceRepository = 'fork/tmt';
    if (caseName === 'duplicate') s.channels.push(structuredClone(s.channels[0]));
    if (caseName === 'unsafe') g.resources[1].transportBytes = Number.MAX_SAFE_INTEGER + 1;
    if (caseName === 'unknown') Object.assign(g.resources[0], { unknownField: true });
    if (caseName === 'incomplete') delete (g.resources[0] as Partial<RCResource>).metadataBytes;
    if (caseName === 'path') g.resources[1].path = 'payload/../other.zip';
    if (caseName === 'catalog') g.resources[0].kind = 'intermediate';
    if (caseName === 'selection')
      g.identity.selection.push(structuredClone(g.identity.selection[0]));
    expect(reserve(s, g).status).toBe('refused');
  });
  it('refuses duplicate generations, current slots and exact run/path ownership', () => {
    refuse(reconcile(snapshot([published(), published()])), 'Duplicate generation');
    const g = published();
    const other = published();
    other.identity.runId = 2;
    Object.values(other.observations).forEach((o) => {
      o.generationKey = rcGenerationKey(other.identity);
    });
    other.resources.forEach((r, i) => {
      r.artifactId = 200 + i;
    });
    other.observations.inventory.artifactIds = [200, 201];
    refuse(reconcile(snapshot([g, other])), 'Multiple current');
    const pending = generation(2, 1);
    refuse(reconcile(snapshot([g, pending], 2)), 'output ownership');
  });
  it('protects upstream provenance and ordinary same-name inventory, never adopting them', () => {
    const g = published();
    g.reuse[0].artifactId = g.resources[0].artifactId!;
    refuse(reconcile(snapshot([g])), 'adopted output');
    const s = snapshot([generation()]);
    s.generations[0].observations.reservation.state = 'confirmed';
    s.unknown = [{ reference: 'ordinary/catalog.zip', bytes: 2 * MiB, artifacts: 1 }];
    expect(reconcile(s).intents).toEqual([]);
    expect(reconcile(s).charges!.bytes).toBe(journalBytes + 73 * MiB + 2);
    refuse(reserve(s, generation(1, 2)), 'Uncertain');
  });
  it('refuses duplicate paths/IDs and proposed collisions before reservation', () => {
    const duplicate = generation();
    duplicate.resources[1].path = duplicate.resources[0].path;
    refuse(reserve(snapshot(), duplicate), 'Duplicate output');
    const g = published();
    g.resources[1].artifactId = g.resources[0].artifactId;
    refuse(reconcile(snapshot([g])), 'Duplicate output');
    const current = published();
    const s = snapshot([current], 2);
    refuse(reserve(s, generation(2, 1)), 'Conflicting proposed');
    const proposed = generation(1, 2);
    proposed.reuse[0].artifactId = 100;
    refuse(reserve(s, proposed), 'Conflicting proposed');
  });
  it('refuses a second incomplete or crash-uncertain generation with both reservations intact', () => {
    const first = generation();
    first.mutation = 'unknown';
    const second = generation(2);
    const s = snapshot([first, second], 2);
    const original = JSON.stringify(s);
    const result = reconcile(s);
    refuse(result, 'Incomplete generation');
    expect(result.charges!.bytes).toBe(journalBytes + 142 * MiB + 4);
    expect(JSON.stringify(s)).toBe(original);
  });
  it.each(['checkpointBytes', 'checkpoints', 'terminalEntries'] as const)(
    'refuses unbounded %s with no eviction',
    (field) => {
      const s = snapshot();
      s.journal[field]++;
      refuse(reconcile(s), 'journal representation');
    }
  );
  it('refuses oversized checkpoint content and missing unknown sizes, never assuming empty', () => {
    const s = snapshot();
    s.reference = 'a'.repeat(128 * 1024);
    refuse(reconcile(s), 'Checkpoint representation');
    s.reference = 'snapshot/1';
    s.unknown = [{ reference: 'unknown/1', bytes: 0, artifacts: 1 }];
    refuse(reconcile(s), 'conservative charge');
  });
});

describe('retirement, interleavings and conditional capacity release', () => {
  it.each(['merged', 'closed', 'disabled', 'new-head', 'new-epoch', 'missed-close'] as const)(
    'retires %s discovery before exact owned payload intents',
    (event) => {
      const g = published();
      const s = snapshot([g]);
      if (event === 'new-head') s.channels[0].sourceSha = 'f'.repeat(40);
      else if (event === 'new-epoch') s.channels[0].epoch.eventId++;
      else s.channels[0].state = event === 'missed-close' ? 'closed' : event;
      const original = JSON.stringify(s);
      const result = reconcile(s);
      expect(result.intents.map((i) => i.kind)).toEqual(['retire-discovery', 'delete', 'delete']);
      expect(result.intents.map((i) => i.artifactId)).toEqual([undefined, 100, 101]);
      expect(result.intents[2].requires).toContain('catalog-absence-confirmed');
      expect(result.charges!.bytes).toBe(journalBytes + 71 * MiB + 2);
      expect(reconcile(s)).toEqual(result);
      expect(JSON.stringify(s)).toBe(original);
    }
  );
  it.each(['retired', 'closed'] as const)(
    'preserves historical %s ownership and conditional release after producer approval changes',
    (disposition) => {
      const g = published();
      const s = snapshot([g]);
      s.approvedProducer.toolingCommit = 'f'.repeat(40);
      if (disposition === 'retired') g.disposition = 'retired';
      else s.channels[0].state = 'closed';
      const original = JSON.stringify(s);
      const deletion = reconcile(s);
      expect(deletion.intents.map((i) => [i.kind, i.artifactId])).toEqual([
        ['retire-discovery', undefined],
        ['delete', 100],
        ['delete', 101],
      ]);
      expect(deletion.intents[2].requires).toContain('catalog-absence-confirmed');
      expect(deletion.blocked[0].obligations).toEqual(['absence']);
      expect(JSON.stringify(s)).toBe(original);
      g.observations.absence.state = 'confirmed';
      g.observations.absence.artifactIds = [100, 101];
      const absent = JSON.stringify(s);
      const release = reconcile(s);
      expect(release.status).toBe('planned');
      expect(release.intents.map((i) => i.kind)).toEqual([
        'retire-discovery',
        'release-reservation',
      ]);
      expect(release.intents[1].requires).toContain(
        'trusted-adapter-revalidate-settlement-inventory-absence'
      );
      expect(release.intents[1].requires).toContain('exclusive-durable-release-commit');
      expect(release.charges).toEqual(deletion.charges);
      expect(JSON.stringify(s)).toBe(absent);
    }
  );
  it('does not equate cancellation202/204/404/TTL or missing logs with settlement or absence', () => {
    const g = published();
    g.observations.settlement.state = 'unknown';
    const s = snapshot([g]);
    s.channels[0].state = 'closed';
    for (const reference of [
      'cancel/202',
      'delete/204',
      'absence/404',
      'expired/TTL',
      'missing/logs',
    ]) {
      g.observations.settlement.reference = reference;
      const result = reconcile(s);
      expect(result.intents.map((i) => i.kind)).toEqual(['retire-discovery', 'cancel']);
      expect(result.intents[1]).toMatchObject({ runId: 1, attempt: 1 });
      expect(result.blocked[0].obligations).toContain('settlement');
      expect(result.charges!.bytes).toBe(journalBytes + 71 * MiB + 2);
    }
  });
  it.each(['inventory', 'absence'] as const)(
    'refuses additional/missing/duplicate/wrong-identity %s',
    (kind) => {
      for (const ids of [[100], [100, 101, 102], [100, 100], [999]]) {
        const g = published();
        g.observations[kind].state = 'confirmed';
        g.observations[kind].artifactIds = ids;
        expect(reconcile(snapshot([g])).status).toBe('refused');
      }
      const g = published();
      g.observations[kind].generationKey = 'f'.repeat(64);
      refuse(reconcile(snapshot([g])), 'Mismatched');
    }
  );
  it('retains unknown late uploads and incomplete inventory instead of deleting by name', () => {
    const g = published();
    const s = snapshot([g]);
    s.channels[0].state = 'closed';
    g.observations.inventory.state = 'unknown';
    expect(reconcile(s).intents.map((i) => i.kind)).toEqual(['retire-discovery']);
    g.observations.inventory.state = 'confirmed';
    g.observations.inventory.artifactIds.push(102);
    refuse(reconcile(s), 'Unmatched inventory');
    g.observations.inventory.artifactIds.pop();
    s.unknown = [{ reference: 'late/upload', bytes: MiB, artifacts: 1 }];
    expect(reconcile(s).intents.map((i) => i.kind)).toEqual(['retire-discovery']);
    expect(reconcile(s).charges!.bytes).toBe(journalBytes + 72 * MiB + 2);
  });
  it('requires all three observations and durable release separately; no output subtracts charges', () => {
    const g = published();
    g.disposition = 'retired';
    g.observations.absence.state = 'confirmed';
    g.observations.absence.artifactIds = [100, 101];
    const s = snapshot([g]);
    const result = reconcile(s);
    expect(result.intents.map((i) => i.kind)).toEqual(['retire-discovery', 'release-reservation']);
    expect(result.intents[1].requires).toContain('exclusive-durable-release-commit');
    expect(result.charges!.bytes).toBe(journalBytes + 71 * MiB + 2);
    g.mutation = 'unknown';
    expect(reconcile(s).intents.map((i) => i.kind)).toEqual(['retire-discovery']);
    expect(reconcile(s).charges).toEqual(result.charges);
  });
  it('re-enable/reopen cannot resurrect a retired generation or use a charged replacement slot', () => {
    const g = published();
    g.disposition = 'retired';
    const s = snapshot([g]);
    s.channels[0].epoch.eventId = 30;
    const result = reconcile(s);
    expect(result.intents.every((i) => i.kind !== 'upload')).toBe(true);
    const replacement = generation(1, 2);
    replacement.identity.epoch.eventId = 30;
    Object.values(replacement.observations).forEach((o) => {
      o.generationKey = rcGenerationKey(replacement.identity);
    });
    refuse(reserve(s, replacement), 'Replacement slot');
  });
  it('reader observations do not acquire cleanup leases or mutate upstream/local installation state', () => {
    const g = published();
    const s = snapshot([g]);
    s.channels[0].state = 'closed';
    const original = structuredClone(g.reuse);
    const result = reconcile(s);
    expect(result.intents.filter((i) => i.kind === 'delete').map((i) => i.artifactId)).toEqual([
      100, 101,
    ]);
    expect(g.reuse).toEqual(original);
    expect(result.intents.some((i) => i.path?.includes('install'))).toBe(false);
  });
  it('keeps the planner unarmed and independent of effectful owners', () => {
    const source = readFileSync(
      new URL('../../scripts/pr-rc-resources.mjs', import.meta.url),
      'utf8'
    );
    expect([...source.matchAll(/^import .* from '([^']+)'/gm)].map((m) => m[1])).toEqual([
      'node:crypto',
    ]);
    expect(source).not.toMatch(/\b(?:fetch|execSync|spawn|writeFile|unlink|process\.env)\b/);
  });
});
