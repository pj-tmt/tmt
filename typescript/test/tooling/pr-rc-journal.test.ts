import { createHash } from 'node:crypto';
import { describe, expect, it, vi } from 'vite-plus/test';
import {
  commitRCCheckpoint,
  decodeRCCheckpoint,
  encodeRCCheckpoint,
  prepareRCCheckpoint,
  type RCCheckpoint,
  type RCCheckpointCandidate,
  type RCCheckpointRecord,
  type RCCheckpointPorts,
  type RCCheckpointRecovery,
  type RCCheckpointResponse,
} from '../../scripts/pr-rc-journal.mjs';
// Independently computed literal fixture bytes, identities and digests; not adapter output.
const EMPTY =
  '{"predecessor":null,"revision":1,"schema":1,"snapshot":{"approvedProducer":{"toolingCommit":"cccccccccccccccccccccccccccccccccccccccc","workflowId":10,"workflowPath":".github/workflows/synthetic-rc.yml","workflowSha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"channels":[{"epoch":{"enabledAtMs":1000,"eventId":20,"labelId":10},"pr":1,"reference":"channel/1","sourceRepository":"pj-tmt/tmt","sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"enabled"}],"complete":true,"generations":[],"journal":{"checkpointBytes":131072,"checkpoints":3,"terminalEntries":0},"reference":"snapshot/1","repository":"pj-tmt/tmt","unknown":[]},"snapshotDigest":"6ef9bbf939fa0bea543f98545ae82163ff82b3a72708b87142d5a168baf63c10","sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","terminal":[],"writer":{"attempt":2,"ownerId":77,"producer":{"toolingCommit":"cccccccccccccccccccccccccccccccccccccccc","workflowId":10,"workflowPath":".github/workflows/synthetic-rc.yml","workflowSha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"repository":"pj-tmt/tmt","runId":20}}';
const RESERVED =
  '{"predecessor":{"digest":"ece312f1ca0c0f2f5374cf2fbaa31725eb2102776a50fe66f37aae7f2a9f3f79","id":91},"revision":2,"schema":1,"snapshot":{"approvedProducer":{"toolingCommit":"cccccccccccccccccccccccccccccccccccccccc","workflowId":10,"workflowPath":".github/workflows/synthetic-rc.yml","workflowSha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"channels":[{"epoch":{"enabledAtMs":1000,"eventId":20,"labelId":10},"pr":1,"reference":"channel/1","sourceRepository":"pj-tmt/tmt","sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"enabled"}],"complete":true,"generations":[{"disposition":"pending","identity":{"attempt":2,"epoch":{"enabledAtMs":1000,"eventId":20,"labelId":10},"pr":1,"producer":{"toolingCommit":"cccccccccccccccccccccccccccccccccccccccc","workflowId":10,"workflowPath":".github/workflows/synthetic-rc.yml","workflowSha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"repository":"pj-tmt/tmt","runId":20,"selection":[{"product":"cli","target":"aarch64-apple-darwin"}],"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"mutation":"none","observations":{"absence":{"artifactIds":[],"generationKey":"e0acee3505fb0710994fdf6a7564f0ec021a2f66797fa65b4722bcfd93bb71ec","reference":"absence/1","state":"unknown"},"inventory":{"artifactIds":[],"generationKey":"e0acee3505fb0710994fdf6a7564f0ec021a2f66797fa65b4722bcfd93bb71ec","reference":"inventory/1","state":"unknown"},"reservation":{"artifactIds":[],"generationKey":"e0acee3505fb0710994fdf6a7564f0ec021a2f66797fa65b4722bcfd93bb71ec","reference":"reservation/1","state":"unknown"},"settlement":{"artifactIds":[],"generationKey":"e0acee3505fb0710994fdf6a7564f0ec021a2f66797fa65b4722bcfd93bb71ec","reference":"settlement/1","state":"unknown"}},"resources":[{"artifactId":null,"diagnosticBytes":0,"kind":"catalog","manifestBytes":0,"metadataBytes":1,"path":"catalog.zip","selectionIndex":null,"transportBytes":2097152},{"artifactId":null,"diagnosticBytes":0,"kind":"payload","manifestBytes":0,"metadataBytes":1,"path":"payload.zip","selectionIndex":0,"transportBytes":72351744}],"reuse":[]}],"journal":{"checkpointBytes":131072,"checkpoints":3,"terminalEntries":0},"reference":"snapshot/1","repository":"pj-tmt/tmt","unknown":[]},"snapshotDigest":"19752230488ca234e8a15bbd1d13e4901b44abd09bf8b1d528480b92bf389485","sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","terminal":[],"writer":{"attempt":2,"ownerId":77,"producer":{"toolingCommit":"cccccccccccccccccccccccccccccccccccccccc","workflowId":10,"workflowPath":".github/workflows/synthetic-rc.yml","workflowSha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"repository":"pj-tmt/tmt","runId":20}}';
const GENERATION_KEY = 'e0acee3505fb0710994fdf6a7564f0ec021a2f66797fa65b4722bcfd93bb71ec';
const root = '/repos/pj-tmt/tmt/deployments';
const environment = 'tmt-pr-rc-checkpoint';
const bytes = (s: string) => new TextEncoder().encode(s);
const digest = (s: string | Uint8Array) => createHash('sha256').update(s).digest('hex');
const empty = (): RCCheckpoint => JSON.parse(EMPTY);
const reserved = (): RCCheckpoint => JSON.parse(RESERVED);
const candidate = (): RCCheckpointCandidate => ({
  bytes: bytes(RESERVED),
  digest: digest(RESERVED),
  charges: { bytes: 74842114, artifacts: 2 },
});
const old = (): RCCheckpointRecord => ({
  bytes: bytes(EMPTY),
  digest: digest(EMPTY),
  id: 91,
  charges: { bytes: 393216, artifacts: 0 },
});
const deployment = (id: number, payload: string) => ({
  id,
  repository_url: 'https://api.github.com/repos/pj-tmt/tmt',
  creator: { id: 77 },
  sha: 'a'.repeat(40),
  task: environment,
  environment,
  payload,
  transient_environment: false,
  production_environment: false,
});
const unrelated = { id: 7, environment: 'production', task: 'deploy', payload: { ordinary: true } };
const inactive = {
  id: 51,
  creator: { id: 77 },
  state: 'inactive',
  environment,
  deployment_url: `https://api.github.com${root}/91`,
};
interface Step {
  method: string;
  path: string;
  status: number;
  value: unknown;
  body?: unknown;
}
const success = (): Step[] => [
  {
    method: 'GET',
    path: `${root}?per_page=100&page=1`,
    status: 200,
    value: [unrelated, deployment(91, EMPTY)],
  },
  { method: 'GET', path: `${root}/91`, status: 200, value: deployment(91, EMPTY) },
  {
    method: 'POST',
    path: root,
    status: 201,
    value: deployment(42, RESERVED),
    body: {
      ref: 'a'.repeat(40),
      task: environment,
      auto_merge: false,
      required_contexts: [],
      payload: RESERVED,
      environment,
      description: 'Unarmed PR RC checkpoint candidate',
      transient_environment: false,
      production_environment: false,
    },
  },
  { method: 'GET', path: `${root}/42`, status: 200, value: deployment(42, RESERVED) },
  {
    method: 'GET',
    path: `${root}?per_page=100&page=1`,
    status: 200,
    value: [deployment(42, RESERVED), unrelated, deployment(91, EMPTY)],
  },
  {
    method: 'POST',
    path: `${root}/91/statuses`,
    status: 201,
    value: inactive,
    body: { state: 'inactive', auto_inactive: false, environment },
  },
  {
    method: 'GET',
    path: `${root}/91/statuses?per_page=100&page=1`,
    status: 200,
    value: [inactive],
  },
  {
    method: 'GET',
    path: `${root}?per_page=100&page=1`,
    status: 200,
    value: [deployment(42, RESERVED), unrelated, deployment(91, EMPTY)],
  },
  { method: 'GET', path: `${root}/91`, status: 200, value: deployment(91, EMPTY) },
  { method: 'DELETE', path: `${root}/91`, status: 204, value: null },
  { method: 'GET', path: `${root}/91`, status: 404, value: { message: 'Not Found' } },
  {
    method: 'GET',
    path: `${root}?per_page=100&page=1`,
    status: 200,
    value: [unrelated, deployment(42, RESERVED)],
  },
];
function fixture(steps = success()) {
  const calls: { method: string; path: string; body: unknown; timeoutMs: number }[] = [];
  let durable: RCCheckpointRecovery | null = null;
  const saved: RCCheckpointRecovery[] = [];
  const protocolErrors: unknown[] = [];
  let crash: RCCheckpointRecovery['phase'] | null = null;
  let clock = 100;
  let mutate: ((response: RCCheckpointResponse, index: number) => void) | null = null;
  let capture = {
    writer: empty().writer,
    reference: 'custody/synthetic',
    concurrencyDomain: 'tmt-pr-rc-writers',
    cancelInProgress: false,
  };
  const ports: RCCheckpointPorts = {
    now: () => clock,
    custody: { capture: async () => structuredClone(capture) },
    recovery: {
      load: async () => structuredClone(durable),
      save: async (state) => {
        durable = structuredClone(state);
        saved.push(structuredClone(state));
        if (crash === state.phase) throw new Error(`Injected crash ${crash}`);
      },
    },
    transport: async (request) => {
      const body =
        request.body === null ? null : JSON.parse(new TextDecoder().decode(request.body));
      const index = calls.length;
      calls.push({
        method: request.method,
        path: request.path,
        body,
        timeoutMs: request.timeoutMs,
      });
      const step = steps[index];
      try {
        expect(step, 'No unplanned effect or retry').toBeDefined();
        expect([request.method, request.path, body]).toEqual([
          step.method,
          step.path,
          step.body ?? null,
        ]);
      } catch (error) {
        protocolErrors.push(error);
        throw error;
      }
      const response: RCCheckpointResponse = {
        status: step.status,
        body: step.status === 204 ? new Uint8Array() : bytes(JSON.stringify(step.value)),
        elapsedMs: 1,
        nextPage: null,
        authenticated: { repository: 'pj-tmt/tmt', ownerId: 77 },
      };
      if (mutate) mutate(response, index);
      return response;
    },
  };
  return {
    ports,
    protocolErrors,
    calls,
    saved,
    get durable() {
      return durable;
    },
    setCrash: (phase: typeof crash) => {
      crash = phase;
    },
    setClock: (value: number) => {
      clock = value;
    },
    setMutation: (fn: typeof mutate) => {
      mutate = fn;
    },
    setCapture: (value: typeof capture) => {
      capture = value;
    },
  };
}
const run = async (f: ReturnType<typeof fixture>, input = candidate(), recorded = [old()]) => {
  const result = await commitRCCheckpoint({
    candidate: input,
    recorded,
    bootstrap: null,
    ports: f.ports,
    startedAtMs: 0,
  });
  // Adapter failure translation must never hide an unrelated protocol assertion failure.
  expect(f.protocolErrors).toEqual([]);
  return result;
};
const expectedCharges = { bytes: 3 * 128 * 1024 + 71 * 1024 ** 2 + 2, artifacts: 2 };

describe('strict immutable checkpoint and planner ownership', () => {
  it('matches independent complete literal bytes, key, predecessor, reservation and charges', () => {
    const g = reserved().snapshot.generations[0];
    expect(g.observations.reservation.generationKey).toBe(GENERATION_KEY);
    const prepared = prepareRCCheckpoint({
      writer: empty().writer,
      sourceSha: 'a'.repeat(40),
      previous: old(),
      snapshot: empty().snapshot,
      request: { kind: 'reserve', generation: g },
      terminal: [],
    });
    expect(new TextDecoder().decode(prepared.bytes)).toBe(RESERVED);
    expect(prepared.digest).toBe(digest(RESERVED));
    expect(prepared.charges).toEqual(expectedCharges);
    expect(decodeRCCheckpoint(prepared.bytes)).toEqual(reserved());
    g.resources[0].transportBytes++;
    expect(new TextDecoder().decode(prepared.bytes)).toBe(RESERVED);
  });
  it.each([
    'duplicate',
    'escaped-duplicate',
    'unknown',
    'unsafe',
    'fraction',
    'nested',
    'unicode',
    'utf8',
    'oversized',
    'digest',
  ])('rejects %s rather than truncating or normalizing away evidence', (kind) => {
    let payload = RESERVED;
    if (kind === 'duplicate') payload = payload.replace('{', '{"schema":1,');
    if (kind === 'escaped-duplicate') payload = payload.replace('{', '{"\\u0073chema":1,');
    if (kind === 'unknown') payload = payload.replace('{', '{"extra":1,');
    if (kind === 'unsafe') payload = payload.replace('"revision":2', '"revision":9007199254740992');
    if (kind === 'fraction') payload = payload.replace('"revision":2', '"revision":2.1');
    if (kind === 'nested') payload = '['.repeat(17) + '0' + ']'.repeat(17);
    if (kind === 'unicode') payload = payload.replace('snapshot/1', '\\ud800');
    if (kind === 'oversized') payload = ' '.repeat(131073);
    if (kind === 'digest') payload = payload.replace('snapshot/1', 'snapshot/2');
    expect(() =>
      decodeRCCheckpoint(kind === 'utf8' ? new Uint8Array([255]) : bytes(payload))
    ).toThrow();
  });
  it.each(['repository', 'owner', 'run', 'attempt', 'tooling', 'predecessor', 'revision'])(
    'binds %s independently before mutation',
    async (kind) => {
      const cp = reserved();
      if (kind === 'repository') cp.writer.repository = 'fork/tmt';
      if (kind === 'owner') cp.writer.ownerId++;
      if (kind === 'run') cp.writer.runId++;
      if (kind === 'attempt') cp.writer.attempt++;
      if (kind === 'tooling') cp.writer.producer.toolingCommit = 'f'.repeat(40);
      if (kind === 'predecessor') cp.predecessor!.id++;
      if (kind === 'revision') cp.revision++;
      const f = fixture();
      const data = bytes(JSON.stringify(cp));
      const result = await run(f, { ...candidate(), bytes: data, digest: digest(data) });
      expect(result.status).toBe('refused');
      expect(f.calls).toEqual([]);
    }
  );
  it('refuses dropping uncertainty or generations and never evicts terminal records', () => {
    const cp = reserved();
    const prior = { ...candidate(), id: 42 };
    expect(() =>
      prepareRCCheckpoint({
        writer: cp.writer,
        sourceSha: cp.sourceSha,
        previous: prior,
        snapshot: empty().snapshot,
        request: { kind: 'reconcile' },
        terminal: [],
      })
    ).toThrow('changed or dropped');
    cp.terminal = Array.from({ length: 33 }, (_, index) => ({
      generationKey: digest(String(index)),
      reference: `terminal/${index}`,
    }));
    expect(() => encodeRCCheckpoint(cp)).toThrow('Terminal ring');
  });
});

describe('injected Deployment candidate protocol', () => {
  it('proves exact create/readback/inactive/status/delete/absence/inventory order, preserving unrelated rows and all charges', async () => {
    const f = fixture();
    const result = await run(f);
    expect(result.status).toBe('readback-confirmed');
    expect(result.successor?.id).toBe(42); // Smaller than predecessor: no numeric-latest inference.
    expect(result.charges).toEqual(expectedCharges);
    expect(f.calls).toHaveLength(12);
    expect(result.evidence.map((e) => [e.method, e.path])).toEqual(
      success().map((s) => [s.method, s.path])
    );
    expect(f.saved.map((s) => s.phase)).toEqual([
      'before-create',
      'after-returned-id',
      'after-readback',
      'before-inactive-status',
      'after-inactive-status',
      'before-delete',
      'after-delete',
      'complete',
    ]);
    expect(f.saved[1].createdId).toBe(42);
    expect(JSON.parse(new TextDecoder().decode(f.saved[1].evidence[2].response.body))).toEqual(
      deployment(42, RESERVED)
    );
    expect(f.durable?.phase).toBe('complete');
    expect(f.calls.some((c) => c.path.endsWith('/7'))).toBe(false);
    expect(f.calls.every((c) => c.timeoutMs <= 10000)).toBe(true);
  });
  it('requires explicit admitted empty bootstrap and refuses absent checkpoints as empty', async () => {
    const data = {
      bytes: bytes(EMPTY),
      digest: digest(EMPTY),
      charges: { bytes: 393216, artifacts: 0 },
    };
    const f = fixture();
    expect((await run(f, data, [])).status).toBe('refused');
    expect(f.calls).toEqual([]);
    const steps: Step[] = [
      { method: 'GET', path: `${root}?per_page=100&page=1`, status: 200, value: [unrelated] },
      {
        method: 'POST',
        path: root,
        status: 201,
        value: deployment(91, EMPTY),
        body: {
          ref: 'a'.repeat(40),
          task: environment,
          auto_merge: false,
          required_contexts: [],
          payload: EMPTY,
          environment,
          description: 'Unarmed PR RC checkpoint candidate',
          transient_environment: false,
          production_environment: false,
        },
      },
      { method: 'GET', path: `${root}/91`, status: 200, value: deployment(91, EMPTY) },
      {
        method: 'GET',
        path: `${root}?per_page=100&page=1`,
        status: 200,
        value: [unrelated, deployment(91, EMPTY)],
      },
    ];
    const boot = fixture(steps);
    const result = await commitRCCheckpoint({
      candidate: data,
      recorded: [],
      bootstrap: {
        writer: empty().writer,
        admissionReference: 'bootstrap/once',
        emptySnapshotDigest: empty().snapshotDigest,
      },
      ports: boot.ports,
      startedAtMs: 0,
    });
    expect(result.status).toBe('readback-confirmed');
    expect(boot.calls).toHaveLength(4);
  });
  it.each([
    'custody',
    'concurrency',
    'cancel',
    'deadline',
    'fork',
    'missing',
    'duplicate',
    'partial',
    'denied',
    'wrong-owner',
    'wrong-repository',
    'wrong-payload',
    'wrong-sha',
  ])('refuses %s without later mutation and retains charges', async (kind) => {
    const f = fixture();
    const custody = {
      writer: empty().writer,
      reference: 'custody/1',
      concurrencyDomain: 'tmt-pr-rc-writers',
      cancelInProgress: false,
    };
    if (kind === 'custody') custody.reference = '';
    if (kind === 'concurrency') custody.concurrencyDomain = 'other';
    if (kind === 'cancel') custody.cancelInProgress = true;
    f.setCapture(custody);
    if (kind === 'deadline') f.setClock(600000);
    f.setMutation((response, index) => {
      if (index !== 0) return;
      let rows = [unrelated, deployment(91, EMPTY)];
      if (kind === 'fork') rows.push(deployment(92, EMPTY));
      if (kind === 'missing') rows = [unrelated];
      if (kind === 'duplicate') rows.push(deployment(91, EMPTY));
      if (kind === 'partial') response.nextPage = 2;
      if (kind === 'denied') response.status = 404;
      if (kind === 'wrong-owner') rows[1] = { ...deployment(91, EMPTY), creator: { id: 78 } };
      if (kind === 'wrong-repository')
        rows[1] = {
          ...deployment(91, EMPTY),
          repository_url: 'https://api.github.com/repos/fork/tmt',
        };
      if (kind === 'wrong-payload') rows[1] = deployment(91, RESERVED);
      if (kind === 'wrong-sha') rows[1] = { ...deployment(91, EMPTY), sha: 'f'.repeat(40) };
      response.body = bytes(JSON.stringify(rows));
    });
    const result = await run(f);
    expect(result.status).toBe('refused');
    expect(result.charges).toEqual(expectedCharges);
    expect(f.calls.every((call) => call.method === 'GET')).toBe(true);
  });
  it.each([2, 3, 4, 5, 6, 7, 8, 9, 10, 11])(
    'unknown HTTP at phase %i freezes all subsequent effects and recovery',
    async (index) => {
      const f = fixture();
      f.setMutation((response, current) => {
        if (current === index) response.status = 503;
      });
      const result = await run(f);
      expect(result.status).toBe('frozen');
      expect(result.charges).toEqual(expectedCharges);
      expect(f.calls).toHaveLength(index + 1);
      const originalCalls = f.calls.length;
      const recovered = await run(f);
      expect(recovered.status).toBe('frozen');
      expect(recovered.charges).toEqual(expectedCharges);
      expect(recovered.recovery?.candidate.bytes).toEqual(bytes(RESERVED));
      expect(f.calls).toHaveLength(originalCalls);
    }
  );
  it.each([
    'before-create',
    'after-returned-id',
    'after-readback',
    'after-inactive-status',
    'after-delete',
  ] as const)(
    'persists independent crash boundary %s with full unresolved state',
    async (phase) => {
      const f = fixture();
      f.setCrash(phase);
      const result = await run(f);
      expect(result.status).toBe('frozen');
      expect(f.durable?.phase).toBe(phase);
      expect(f.durable?.recorded[0].id).toBe(91);
      expect(f.durable?.candidate.bytes).toEqual(bytes(RESERVED));
      expect(result.charges).toEqual(expectedCharges);
      expect(f.calls).toHaveLength(
        {
          'before-create': 2,
          'after-returned-id': 3,
          'after-readback': 5,
          'after-inactive-status': 7,
          'after-delete': 10,
        }[phase]
      );
      const count = f.calls.length;
      f.setCrash(null);
      const recovered = await run(f);
      expect(recovered.status).toBe('frozen');
      expect(recovered.charges).toEqual(expectedCharges);
      expect(f.calls).toHaveLength(count);
    }
  );
  it.each([
    'lost-response',
    'unauthenticated',
    'oversized',
    'slow',
    'malformed',
    'duplicate-json',
    'missing-id',
    'inactive-failure',
    'status-owner',
    'delete-still-present',
  ])('keeps %s red, without retry or releasing reservation', async (kind) => {
    const steps = success();
    if (kind === 'delete-still-present')
      steps[11].value = [deployment(91, EMPTY), deployment(42, RESERVED), unrelated];
    const f = fixture(steps);
    f.setMutation((response, index) => {
      const target = ['inactive-failure', 'status-owner'].includes(kind) ? 6 : 2;
      if (index !== target) return;
      if (kind === 'lost-response') throw new Error('Lost POST response');
      if (kind === 'unauthenticated') response.authenticated.ownerId = 78;
      if (kind === 'oversized') response.body = new Uint8Array(8 * 1024 ** 2 + 1);
      if (kind === 'slow') response.elapsedMs = 10001;
      if (kind === 'malformed') response.body = bytes('{');
      if (kind === 'duplicate-json') response.body = bytes('{"id":42,"id":43}');
      if (kind === 'missing-id') response.body = bytes('{}');
      if (kind === 'inactive-failure')
        response.body = bytes(JSON.stringify([{ ...inactive, state: 'failure' }]));
      if (kind === 'status-owner')
        response.body = bytes(JSON.stringify([{ ...inactive, creator: { id: 78 } }]));
    });
    const result = await run(f);
    expect(result.status).toBe('frozen');
    expect(result.charges).toEqual(expectedCharges);
    expect(f.calls.filter((call) => call.method === 'POST' && call.path === root)).toHaveLength(1);
    if (['inactive-failure', 'status-owner'].includes(kind))
      expect(f.calls.some((call) => call.method === 'DELETE')).toBe(false);
  });
});

describe('finite recovery, overlap and independent guard sensitivity', () => {
  it('admits complete pagination and retains every unrelated Deployment', async () => {
    const steps = success();
    const ordinary = Array.from({ length: 99 }, (_, index) => ({ ...unrelated, id: 1000 + index }));
    steps[0].value = [...ordinary, deployment(91, EMPTY)];
    steps.splice(1, 0, {
      method: 'GET',
      path: `${root}?per_page=100&page=2`,
      status: 200,
      value: [],
    });
    const f = fixture(steps);
    f.setMutation((response, index) => {
      if (index === 0) response.nextPage = 2;
    });
    const result = await run(f);
    expect(result.status).toBe('readback-confirmed');
    expect(f.calls).toHaveLength(13);
    expect(f.calls.filter((call) => call.method === 'DELETE').map((call) => call.path)).toEqual([
      `${root}/91`,
    ]);
  });
  it('refuses ten full pages instead of dropping records to fit finite inventory', async () => {
    const steps: Step[] = Array.from({ length: 10 }, (_, page) => ({
      method: 'GET',
      path: `${root}?per_page=100&page=${page + 1}`,
      status: 200,
      value: Array.from({ length: 100 }, (_, index) => ({
        ...unrelated,
        id: page * 100 + index + 1000,
      })),
    }));
    const f = fixture(steps);
    f.setMutation((response, index) => {
      response.nextPage = index + 2;
    });
    const result = await run(f);
    expect(result.reason).toBe('Incomplete bounded checkpoint inventory.');
    expect(result.charges).toEqual(expectedCharges);
    expect(f.calls).toHaveLength(10);
    expect(f.calls.every((call) => call.method === 'GET')).toBe(true);
  });
  it('refuses overlap beyond three checkpoints before any transport effect', async () => {
    const f = fixture();
    const result = await run(f, candidate(), [old(), { ...old(), id: 92 }, { ...old(), id: 93 }]);
    expect(result.reason).toBe('Recorded checkpoint count/IDs.');
    expect(f.calls).toEqual([]);
    expect(result.charges).toEqual(expectedCharges);
  });
  it('validates the full recorded chain rather than only a head digest or numeric maximum', async () => {
    const fork = reserved();
    fork.predecessor!.id = 999;
    const forkBytes = encodeRCCheckpoint(fork);
    const second = { ...candidate(), bytes: forkBytes, digest: digest(forkBytes), id: 42 };
    const next = prepareRCCheckpoint({
      writer: fork.writer,
      sourceSha: fork.sourceSha,
      previous: second,
      snapshot: fork.snapshot,
      request: { kind: 'reconcile' },
      terminal: [],
    });
    const f = fixture();
    const result = await run(f, next, [old(), second]);
    expect(result.reason).toBe('Recorded checkpoint chain/fork mismatch.');
    expect(f.calls).toEqual([]);
    expect(result.charges).toEqual(expectedCharges);
  });
  it('never drops unmatched uncertainty or terminal records to manufacture a new ledger', () => {
    const snapshot = empty().snapshot;
    snapshot.unknown = [{ reference: 'unknown/lost-upload', bytes: 1000000, artifacts: 1 }];
    const terminal = [{ generationKey: GENERATION_KEY, reference: 'terminal/1' }];
    snapshot.journal.terminalEntries = 1;
    const held = prepareRCCheckpoint({
      writer: empty().writer,
      sourceSha: empty().sourceSha,
      previous: null,
      snapshot,
      request: { kind: 'reconcile' },
      terminal,
    });
    const record = { ...held, id: 99 };
    expect(held.charges).toEqual({ bytes: 1393216, artifacts: 1 });
    expect(() =>
      prepareRCCheckpoint({
        writer: empty().writer,
        sourceSha: empty().sourceSha,
        previous: record,
        snapshot: empty().snapshot,
        request: { kind: 'reconcile' },
        terminal: [],
      })
    ).toThrow('changed or dropped');
    const nextSnapshot = structuredClone(snapshot);
    nextSnapshot.journal.terminalEntries = 0;
    expect(() =>
      prepareRCCheckpoint({
        writer: empty().writer,
        sourceSha: empty().sourceSha,
        previous: record,
        snapshot: nextSnapshot,
        request: { kind: 'reconcile' },
        terminal: [],
      })
    ).toThrow('Terminal record dropped');
  });
  it('captures source bytes independently of caller aliases before awaiting custody', async () => {
    const input = candidate();
    const recorded = [old()];
    const f = fixture();
    const capture = f.ports.custody.capture;
    f.ports.custody.capture = async () => {
      input.bytes.fill(0);
      recorded[0].bytes.fill(0);
      return capture();
    };
    const result = await run(f, input, recorded);
    expect(result.status).toBe('readback-confirmed');
    expect(result.recovery?.candidate.bytes).toEqual(bytes(RESERVED));
    const callCount = f.calls.length;
    const recovered = await run(f);
    expect(recovered.status).toBe('frozen');
    expect(recovered.recovery?.phase).toBe('complete');
    expect(recovered.recovery?.createdId).toBe(42);
    expect(f.calls).toHaveLength(callCount);
  });
  it.each(['newer-status', 'duplicate-status', 'wrong-deployment', 'wrong-environment'])(
    'requires exact verified inactive status for %s before delete',
    async (kind) => {
      const f = fixture();
      f.setMutation((response, index) => {
        if (index !== 6) return;
        if (kind === 'newer-status')
          response.body = bytes(
            JSON.stringify([{ ...inactive, id: 52, state: 'failure' }, inactive])
          );
        if (kind === 'duplicate-status')
          response.body = bytes(JSON.stringify([inactive, inactive]));
        if (kind === 'wrong-deployment')
          response.body = bytes(
            JSON.stringify([{ ...inactive, deployment_url: `https://api.github.com${root}/7` }])
          );
        if (kind === 'wrong-environment')
          response.body = bytes(JSON.stringify([{ ...inactive, environment: 'production' }]));
      });
      const result = await run(f);
      expect(result.status).toBe('frozen');
      expect(result.reason).toMatch(/Inactive status readback|Duplicate\/oversized/);
      expect(result.charges).toEqual(expectedCharges);
      expect(f.calls).toHaveLength(7);
      expect(f.calls.some((call) => call.method === 'DELETE')).toBe(false);
    }
  );
  it('counts original queue/custody wait and stops after the deadline crosses on readback', async () => {
    const f = fixture();
    f.setMutation((_, index) => {
      if (index === 3) f.setClock(600000);
    });
    const result = await run(f);
    expect(result.reason).toBe('Checkpoint response byte/time budget exceeded.');
    expect(result.status).toBe('frozen');
    expect(result.charges).toEqual(expectedCharges);
    expect(f.calls).toHaveLength(4);
  });
  it('does not claim transport 202 or DELETE204 alone proves absence or releases artifact accounting', async () => {
    const created = fixture();
    created.setMutation((response, index) => {
      if (index === 2) response.status = 202;
    });
    expect((await run(created)).reason).toBe('Unexpected checkpoint HTTP 202.');
    expect(created.calls).toHaveLength(3);
    const steps = success();
    steps[10].status = 200;
    steps[10].value = deployment(91, EMPTY);
    const deleted = fixture(steps);
    const result = await run(deleted);
    expect(result.reason).toBe('Unexpected checkpoint HTTP 200.');
    expect(result.status).toBe('frozen');
    expect(result.charges).toEqual(expectedCharges);
    expect(deleted.calls).toHaveLength(11);
  });
});

describe('bounded stalled transport', () => {
  it('aborts a stalled POST at the remaining original deadline without retry', async () => {
    vi.useFakeTimers();
    try {
      const f = fixture();
      f.setClock(599995);
      const transport = f.ports.transport;
      let aborted = false;
      f.ports.transport = (request) => {
        if (request.method !== 'POST') return transport(request);
        request.signal.addEventListener('abort', () => {
          aborted = true;
        });
        return new Promise(() => {});
      };
      const pending = run(f);
      await vi.advanceTimersByTimeAsync(5);
      const result = await pending;
      expect(result.reason).toBe('Checkpoint transport deadline exceeded.');
      expect(result.status).toBe('frozen');
      expect(result.requests).toBe(3);
      expect(aborted).toBe(true);
      expect(result.charges).toEqual(expectedCharges);
      expect(f.durable?.phase).toBe('before-create');
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('supplied accounting cannot erase planner charges', () => {
  it('refuses a zero-charge candidate before any API call', async () => {
    const f = fixture();
    const input = candidate();
    input.charges = { bytes: 0, artifacts: 0 };
    const result = await run(f, input);
    expect(result.reason).toBe('Candidate accounting mismatch.');
    expect(result.charges).toEqual(expectedCharges);
    expect(f.calls).toEqual([]);
  });
});
