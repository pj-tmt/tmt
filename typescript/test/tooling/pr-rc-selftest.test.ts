import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { CLI_SCHEMA_TARGETS } from '../../scripts/native-application-schema.mjs';
import { describe, expect, it } from 'vite-plus/test';
import { CLI_NAMESPACE, LEGACY_CLI_NAMESPACE } from '../../scripts/native-install-layout.mjs';
import {
  QUALIFICATION,
  schemaPreflight,
  api,
  producerRunInput,
  waitForPull,
  deriveFixture,
  assertOwnedPull,
  assertPRReceipt,
  assertRefusal,
  REFUSAL_MESSAGES,
  assertPositive,
  cleanupExact,
  safeEvidence,
  readOwnedReceipt,
} from '../../scripts/pr-rc-selftest.mjs';

const FIXTURE = {
  ...QUALIFICATION,
  head: 'a'.repeat(40),
  run: 1234,
  catalog: 10,
  payloads: [11, 12, 13, 14],
  label: 100,
  event: 101,
};
const pull = (consumed = true) => ({
  number: FIXTURE.pr,
  base: { repo: { full_name: 'pj-tmt/tmt' } },
  head: {
    repo: { full_name: 'pj-tmt/tmt' },
    ref: FIXTURE.branch,
    sha: consumed ? 'f'.repeat(40) : FIXTURE.head,
  },
  state: 'open',
  labels: [{ name: 'rc-build', id: FIXTURE.label }],
});
const artifact = (id: number) => ({
  id,
  name: `owned-${id}`,
  size_in_bytes: 100,
  digest: `sha256:${'a'.repeat(64)}`,
  workflow_run: { id: FIXTURE.run },
});
const manifest = () => ({
  run: FIXTURE.run,
  fixture: FIXTURE,
  rc: [FIXTURE.catalog, ...FIXTURE.payloads].map(artifact),
  ordinary: [20, 21],
});
const candidate = {
  target: 'x86_64-unknown-linux-musl',
  version: '5.0.0-alpha.185',
  archive: { sha256: 'b'.repeat(64) },
  payload_artifact: { id: 11 },
};
function receipt() {
  return {
    channel: 'pr2715',
    version: candidate.version,
    archive_sha256: candidate.archive.sha256,
    source: {
      kind: 'github-actions',
      evidence: {
        repository: 'pj-tmt/tmt',
        pr: FIXTURE.pr,
        head_sha: FIXTURE.head,
        producer: { workflow_id: FIXTURE.workflow, run_id: FIXTURE.run },
        catalog_artifact_id: FIXTURE.catalog,
        eligibility: { label_id: FIXTURE.label, enabled_event_id: FIXTURE.event },
        candidate,
      },
    },
  };
}
describe('one-shot PR RC reader qualification', () => {
  it('requires a bounded decimal producer run input', () => {
    expect(producerRunInput('38129567413')).toBe(38129567413);
    for (const value of [
      undefined,
      '',
      '0',
      '01',
      '-1',
      '1;echo',
      '1\n',
      '9999999999999999',
      '12345678901234567',
    ])
      expect(() => producerRunInput(value)).toThrow('Invalid producer run ID');
  });
  it('waits through lagging PR objects and keeps the final assertion identity on exhaustion', async () => {
    let reads = 0;
    const waits: number[] = [];
    const current = pull(false);
    const final = { ...current, head: { ...current.head, sha: 'b'.repeat(40) } };
    const result = await waitForPull(
      () => ({ data: ++reads < 3 ? current : final }),
      (value) => expect(value.head.sha).toBe(final.head.sha),
      (ms) => {
        waits.push(ms);
      }
    );
    expect(result).toEqual(final);
    expect(waits).toEqual([3000, 3000]);
    const failure = new Error('exact original failure');
    reads = 0;
    waits.length = 0;
    await expect(
      waitForPull(
        () => {
          reads++;
          return { data: current };
        },
        () => {
          throw failure;
        },
        (ms) => {
          waits.push(ms);
        }
      )
    ).rejects.toBe(failure);
    expect(reads).toBe(20);
    expect(waits).toEqual(Array(19).fill(3000));
    const apiFailure = new Error('read failed');
    await expect(
      waitForPull(
        () => {
          throw apiFailure;
        },
        () => {}
      )
    ).rejects.toBe(apiFailure);
  });
  it('derives the fixture only from a successful main producer and the current catalog epoch', () => {
    const producer = {
      id: FIXTURE.run,
      workflow_id: FIXTURE.workflow,
      path: '.github/workflows/pr-rc.yml',
      run_attempt: 1,
      status: 'completed',
      head_branch: 'main',
      event: 'workflow_dispatch',
      conclusion: 'success',
      repository: { full_name: 'pj-tmt/tmt' },
      display_title: `pr-rc #2715 ${FIXTURE.head.slice(0, 12)}`,
    };
    const catalog = {
      schema_version: 2,
      kind: 'tmt-pr-rc-catalog',
      repository: 'pj-tmt/tmt',
      producer: { workflow_id: FIXTURE.workflow, run_id: FIXTURE.run },
      pr: FIXTURE.pr,
      head_sha: FIXTURE.head,
      eligibility: { label_id: FIXTURE.label, enabled_event_id: FIXTURE.event },
      candidates: [
        'aarch64-apple-darwin',
        'aarch64-unknown-linux-musl',
        'x86_64-apple-darwin',
        'x86_64-unknown-linux-musl',
      ].map((target, index) => ({ target, payload_artifact: { id: FIXTURE.payloads[index] } })),
    };
    const row = { ...artifact(FIXTURE.catalog), name: 'tmt-pr-rc-catalog-v2-pr2715' };
    const timeline = [{ event: 'labeled', label: { name: 'rc-build' }, id: FIXTURE.event }];
    const derive = (p = producer, live = pull(false), events = timeline) =>
      deriveFixture(FIXTURE.run, p, row, catalog, manifest().rc, live, events);
    expect(derive()).toEqual({
      head: FIXTURE.head,
      run: FIXTURE.run,
      catalog: FIXTURE.catalog,
      payloads: FIXTURE.payloads,
      label: FIXTURE.label,
      event: FIXTURE.event,
    });
    for (const delta of [
      { workflow_id: 9 },
      { head_branch: 'other' },
      { conclusion: 'failure' },
      { event: 'pull_request' },
      { display_title: 'wrong' },
      { id: 999 },
    ])
      expect(() => derive({ ...producer, ...delta })).toThrow();
    expect(() => derive(producer, pull())).toThrow();
    expect(() => derive(producer, { ...pull(false), state: 'closed' })).toThrow();
    expect(() => derive(producer, { ...pull(false), labels: [] })).toThrow();
    expect(() =>
      derive(producer, pull(false), [...timeline, { ...timeline[0], id: 999 }])
    ).toThrow();
    expect(() => derive(producer, pull(false), Array(100).fill(timeline[0]))).toThrow();
  });
  it('reports the bounded 403 message and accepted permission using the real API parser', () => {
    const spawn = (file: string, args: string[]) => {
      expect(file).toBe('gh');
      expect(args).toContain('DELETE');
      expect(args).toContain('repos/pj-tmt/tmt/issues/2715/labels/rc-build');
      return {
        stdout:
          'HTTP/2.0 403 Forbidden\r\nX-Accepted-GitHub-Permissions: pull_requests=write\r\n\r\n{"message":"Resource not accessible by integration"}',
      };
    };
    expect(() => api('DELETE', 'issues/2715/labels/rc-build', null, false, spawn)).toThrow(
      'API DELETE failed (403): Resource not accessible by integration; x-accepted-github-permissions: pull_requests=write'
    );
  });
  it('bounds decoded API diagnostics and rejects credentials or signed URLs before truncation', () => {
    const response =
      (message: string, permissions = 'pull_requests=write') =>
      () => ({
        stdout: `HTTP/2.0 403 Forbidden\nX-Accepted-GitHub-Permissions: ${permissions}\n\n${JSON.stringify({ message })}`,
      });
    expect(() => api('DELETE', 'owned', null, false, response('x'.repeat(300)))).toThrow(
      `API DELETE failed (403): ${'x'.repeat(200)}; x-accepted-github-permissions: pull_requests=write`
    );
    expect(() => api('DELETE', 'owned', null, false, response('safe', 'y'.repeat(300)))).toThrow(
      `x-accepted-github-permissions: ${'y'.repeat(200)}`
    );
    for (const unsafe of [
      'ghp_' + 'x'.repeat(30),
      'https://download.example/?X-Amz-Signature=secret',
    ]) {
      expect(() => api('DELETE', 'owned', null, false, response('x'.repeat(300) + unsafe))).toThrow(
        'possible credential'
      );
      expect(() => api('DELETE', 'owned', null, false, response('safe', unsafe))).toThrow(
        'possible credential'
      );
    }
    expect(() =>
      api('DELETE', 'owned', null, false, () => ({ stdout: 'HTTP/2.0 403 Forbidden\n\nnot JSON' }))
    ).toThrow('API DELETE failed (403): not reported; x-accepted-github-permissions: not reported');
  });
  it('keeps successful JSON and allowed 404 API reads unchanged', () => {
    expect(
      api('GET', 'owned', null, false, () => ({ stdout: 'HTTP/2.0 200 OK\n\n{"id":123}' }))
    ).toEqual({ status: 200, data: { id: 123 } });
    expect(
      api('GET', 'owned', null, true, () => ({
        stdout: 'HTTP/2.0 404 Not Found\n\n{"message":"Not Found"}',
      }))
    ).toEqual({ status: 404, data: { message: 'Not Found' } });
  });
  it.each([LEGACY_CLI_NAMESPACE, CLI_NAMESPACE])(
    'reads the active %s receipt and refuses escape',
    (namespace) => {
      const world = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'pr-rc-selftest-')));
      try {
        const prefix = path.join(world, 'prefix');
        const root = path.join(prefix, namespace);
        const id = '10000000-0000-4000-8000-000000000001';
        const active = path.join(root, 'releases', id);
        fs.mkdirSync(active, { recursive: true });
        fs.mkdirSync(path.join(prefix, 'bin'));
        fs.writeFileSync(path.join(active, 'tmt'), 'verified fixture bytes');
        const expected = { ...receipt(), schema_version: 1, prefix, release_id: id };
        fs.writeFileSync(path.join(active, 'receipt.json'), JSON.stringify(expected));
        fs.symlinkSync(`releases/${id}`, path.join(root, 'current'));
        fs.symlinkSync(`../${namespace}/current/tmt`, path.join(prefix, 'bin/tmt'));
        expect(readOwnedReceipt(world)).toEqual(expected);
        // Scan raw bytes before parsing: malformed evidence must not print a credential in its error.
        fs.writeFileSync(path.join(active, 'receipt.json'), '{"bad":"ghp_' + 'x'.repeat(30));
        expect(() => readOwnedReceipt(world)).toThrow('possible credential');
        fs.writeFileSync(path.join(active, 'receipt.json'), JSON.stringify(expected));
        fs.unlinkSync(path.join(root, 'current'));
        fs.symlinkSync(world, path.join(root, 'current'));
        expect(() => readOwnedReceipt(world)).toThrow('escapes owned prefix');
      } finally {
        fs.rmSync(world, { recursive: true, force: true });
      }
    }
  );

  it('binds the standing same-repository branch and refuses unrelated PRs/forks/branches', () => {
    const owned = pull();
    expect(() => assertOwnedPull(owned)).not.toThrow();
    expect(() => assertOwnedPull({ ...owned, number: 42 })).toThrow();
    expect(() =>
      assertOwnedPull({ ...owned, head: { ...owned.head, repo: { full_name: 'someone/tmt' } } })
    ).toThrow();
    expect(() => assertOwnedPull({ ...owned, head: { ...owned.head, ref: 'main' } })).toThrow();
  });
  it('checks version/channel plus real receipt run/head/catalog/payload provenance', () => {
    expect(() => assertPRReceipt(receipt(), { candidates: [candidate] }, FIXTURE)).not.toThrow();
    for (const modify of [
      (r: ReturnType<typeof receipt>) => (r.source.evidence.head_sha = 'f'.repeat(40)),
      (r: ReturnType<typeof receipt>) => r.source.evidence.producer.run_id++,
      (r: ReturnType<typeof receipt>) => r.source.evidence.catalog_artifact_id++,
      (r: ReturnType<typeof receipt>) => r.source.evidence.eligibility.enabled_event_id++,
      (r: ReturnType<typeof receipt>) => (r.channel = 'alpha'),
    ]) {
      const value = receipt();
      modify(value);
      expect(() => assertPRReceipt(value, { candidates: [candidate] }, FIXTURE)).toThrow();
    }
    expect(() =>
      assertPRReceipt(
        receipt(),
        { candidates: [{ ...candidate, payload_artifact: { id: 999 } }] },
        FIXTURE
      )
    ).toThrow();
  });
  it.each([
    ['disabled', 'PR is closed, disabled, ineligible or outside the official repository.'],
    ['closed', 'PR is closed, disabled, ineligible or outside the official repository.'],
    ['epoch', 'Catalog producer or eligibility epoch does not match current API evidence.'],
    ['stale', 'PR catalog source, producer or availability evidence does not match.'],
  ] as const)('pins the %s JSON failure code/message and identical state', (stage, message) => {
    expect(REFUSAL_MESSAGES[stage]).toBe(message);
    const state = { receipt: 'original', links: { tmt: 'original' }, sentinel: 'untouched' };
    const result = {
      status: 1,
      stdout: JSON.stringify({ error: { code: 'NATIVE_UPGRADE_FAILED', message } }),
      stderr: '',
    };
    expect(() => assertRefusal(stage, result, state, structuredClone(state))).not.toThrow();
    for (const status of [0, null])
      expect(() => assertRefusal(stage, { ...result, status }, state, state)).toThrow();
    for (const otherMessage of new Set(Object.values(REFUSAL_MESSAGES))) {
      if (otherMessage === message) continue;
      expect(() =>
        assertRefusal(
          stage,
          {
            ...result,
            stdout: JSON.stringify({
              error: { code: 'NATIVE_UPGRADE_FAILED', message: otherMessage },
            }),
          },
          state,
          state
        )
      ).toThrow();
    }
    for (const stdout of [
      'network error',
      JSON.stringify({ error: { code: 'NATIVE_PR_CHANNEL_UNAVAILABLE', message } }),
      JSON.stringify({ error: { code: 'NATIVE_UPGRADE_FAILED' } }),
    ])
      expect(() => assertRefusal(stage, { ...result, stdout }, state, state)).toThrow();
    expect(() => assertRefusal(stage, result, state, { ...state, receipt: 'changed' })).toThrow();
    expect(() => assertRefusal(stage, result, state, { ...state, sentinel: 'changed' })).toThrow();
  });
  it('requires eligible success and identical receipt, prefix and application data before mutation', () => {
    const before = { receipt: receipt(), prefix: { binary: 'bytes' }, data: { sentinel: 'kept' } };
    expect(() => assertPositive({ status: 0 }, before, structuredClone(before))).not.toThrow();
    for (const status of [1, null])
      expect(() => assertPositive({ status }, before, before)).toThrow();
    for (const key of ['receipt', 'prefix', 'data'])
      expect(() =>
        assertPositive({ status: 0 }, before, { ...before, [key]: 'changed' })
      ).toThrow();
  });
  it('retains and verifies unconsumed artifacts without any label, PR or artifact mutation', async () => {
    const rows = manifest();
    const calls: string[] = [];
    const alive = new Map([...rows.rc, artifact(20), artifact(21)].map((item) => [item.id, item]));
    const read = (method: string, route: string) => {
      calls.push(method);
      if (route.includes('/timeline?'))
        return {
          status: 200,
          data: [{ event: 'labeled', label: { name: 'rc-build' }, id: FIXTURE.event }],
        };
      return { status: 200, data: alive.get(Number(route.split('/').at(-1)))! };
    };
    const result = await cleanupExact(rows, read, pull(false));
    expect(result.retained).toBe(true);
    expect(result.artifacts.map((item) => item.id)).toEqual(rows.rc.map((item) => item.id));
    expect(calls).toEqual(Array(8).fill('GET'));
    alive.delete(FIXTURE.catalog);
    await expect(
      cleanupExact(
        rows,
        (method, route) => {
          if (route.includes('/timeline?'))
            return {
              status: 200,
              data: [{ event: 'labeled', label: { name: 'rc-build' }, id: FIXTURE.event }],
            };
          const data = alive.get(Number(route.split('/').at(-1)));
          return data ? { status: 200, data } : { status: 404 };
        },
        pull(false)
      )
    ).rejects.toThrow('Unconsumed fixture artifact disappeared');
  });
  it.each(['removed', 'readded'] as const)(
    'consumes a same-head fixture when its label is %s',
    async (labelState) => {
      const rows = manifest();
      const alive = new Map(
        [...rows.rc, artifact(20), artifact(21)].map((item) => [item.id, item])
      );
      const deleted: number[] = [];
      const current = {
        ...pull(false),
        labels: labelState === 'removed' ? [] : pull(false).labels,
      };
      const result = await cleanupExact(
        rows,
        (method, route) => {
          if (route.includes('/timeline?'))
            return {
              status: 200,
              data: [{ event: 'labeled', label: { name: 'rc-build' }, id: FIXTURE.event + 1 }],
            };
          if (route.startsWith('issues/')) return { status: 204 };
          if (route.startsWith('pulls/')) return { status: 200, data: { ...current, labels: [] } };
          const id = Number(route.split('/').at(-1));
          if (method === 'DELETE') {
            deleted.push(id);
            alive.delete(id);
            return { status: 204 };
          }
          const data = alive.get(id);
          return data ? { status: 200, data } : { status: 404 };
        },
        current
      );
      expect(result.retained).toBe(false);
      expect(deleted).toEqual(rows.rc.map((item) => item.id));
      expect([...alive.keys()]).toEqual(rows.ordinary);
    }
  );
  it('rejects token/signed-url evidence without retaining it', () => {
    expect(safeEvidence({ result: 'safe' })).toBe('{"result":"safe"}');
    for (const value of [
      'ghp_' + 'x'.repeat(30),
      'github_pat_' + 'x'.repeat(30),
      'https://download.example/?X-Amz-Signature=secret',
    ])
      expect(() => safeEvidence(value)).toThrow();
  });
  it('deletes captured exact IDs, catalog first, records already-absent404 and preserves ordinary artifacts', async () => {
    const rows = manifest();
    const alive = new Map([...rows.rc, artifact(20), artifact(21)].map((item) => [item.id, item]));
    alive.delete(FIXTURE.payloads[1]);
    const deleted: number[] = [];
    const result = await cleanupExact(
      rows,
      (method, route) => {
        if (route.startsWith('issues/')) return { status: 204 };
        if (route.startsWith('pulls/')) return { status: 200, data: { ...pull(), labels: [] } };
        const id = Number(route.split('/').at(-1));
        if (method === 'DELETE') {
          deleted.push(id);
          alive.delete(id);
          return { status: 204 };
        }
        const data = alive.get(id);
        return data ? { status: 200, data } : { status: 404 };
      },
      pull()
    );
    expect(result.retained).toBe(false);
    expect(deleted).toEqual([
      FIXTURE.catalog,
      FIXTURE.payloads[0],
      FIXTURE.payloads[2],
      FIXTURE.payloads[3],
    ]);
    expect(result.artifacts.find((item) => item.id === FIXTURE.payloads[1])?.alreadyAbsent).toBe(
      true
    );
    expect([...alive.keys()]).toEqual([20, 21]);
  });
  it('reopens and removes the label only for a consumed closed fixture', async () => {
    const rows = manifest();
    const calls: string[] = [];
    const result = await cleanupExact(
      rows,
      (method, route) => {
        calls.push(`${method} ${route}`);
        if (route.startsWith('pulls/')) return { status: 200, data: { ...pull(), labels: [] } };
        if (route.startsWith('actions/artifacts/')) {
          const id = Number(route.split('/').at(-1));
          return rows.ordinary.includes(id) ? { status: 200, data: artifact(id) } : { status: 404 };
        }
        return { status: 204 };
      },
      { ...pull(), state: 'closed' }
    );
    expect(result.retained).toBe(false);
    expect(calls.filter((item) => !item.startsWith('GET')).slice(0, 2)).toEqual([
      `DELETE issues/${FIXTURE.pr}/labels/rc-build`,
      `PATCH pulls/${FIXTURE.pr}`,
    ]);
  });
  it('refuses changed ownership and a deletion that does not settle', async () => {
    const rows = manifest();
    let deletes = 0;
    await expect(
      cleanupExact(
        rows,
        () => ({
          status: 200,
          data: { ...rows.rc[0], workflow_run: { id: 999 } },
        }),
        pull()
      )
    ).rejects.toThrow();
    await expect(
      cleanupExact(
        rows,
        (method, route) => {
          if (route.startsWith('issues/')) return { status: 204 };
          if (route.startsWith('pulls/')) return { status: 200, data: { ...pull(), labels: [] } };
          if (method === 'DELETE') deletes++;
          return { status: 200, data: { ...rows.rc[0], workflow_run: { id: FIXTURE.run } } };
        },
        pull()
      )
    ).rejects.toThrow();
    expect(deletes).toBe(1);
    await expect(
      cleanupExact(
        { ...rows, rc: [rows.rc[1], ...rows.rc.slice(1)] },
        () => ({ status: 404 }),
        pull()
      )
    ).rejects.toThrow();
  });
  it('keeps all executable reader jobs read-only and never downloads/executes candidate code in write jobs', () => {
    const workflow = fs.readFileSync(
      new URL('../../../.github/workflows/pr-rc-selftest.yml', import.meta.url),
      'utf8'
    );
    expect(workflow).toMatch(/^on:\n {2}workflow_dispatch:/m);
    expect(workflow).toContain('PR_RC_SELFTEST_RUN: ${{ inputs.run }}');
    expect(workflow).toContain('required: true');
    expect(workflow).not.toMatch(/run:.*\$\{\{ inputs.run/);
    expect(workflow).not.toMatch(
      /workflow_call|workflow run|pull_request:|schedule:|gh auth token|--allow-schema-ahead/
    );
    expect(workflow).toContain('GH_TOKEN: ${{ github.token }}');
    const jobs = workflow.split(/^ {2}(?=[a-z]+:\n)/m).slice(1);
    for (const job of jobs) {
      expect(job).toContain("github.ref == 'refs/heads/main'");
      expect(job).toMatch(/timeout-minutes: (3|10)/);
      if (/: write/.test(job)) {
        expect(job).not.toContain('pr-rc-selftest-world');
        expect(job).not.toContain('tar -xzf');
        expect(job).not.toMatch(/pr-rc-selftest.mjs (baseline|stale|disabled|epoch|closed)\b/);
      } else expect(job).toContain('actions: read');
    }
    const expected = {
      baseline: 'preflight',
      disable: 'baseline',
      disabled: 'disable',
      enable: 'disabled',
      epoch: 'enable',
      advance: 'epoch',
      stale: 'advance',
      close: 'stale',
      closed: 'close',
    };
    for (const [name, dependency] of Object.entries(expected)) {
      const job = jobs.find((block) => block.startsWith(`${name}:`));
      expect(job).toContain(`needs: [${dependency}]`);
    }
    for (const [name, scope] of [
      ['disable', 'issues'],
      ['enable', 'issues'],
      ['advance', 'contents'],
      ['close', 'pull-requests'],
    ])
      expect(jobs.find((block) => block.startsWith(`${name}:`))).toContain(`${scope}: write`);
    for (const name of ['disable', 'enable']) {
      const job = jobs.find((block) => block.startsWith(`${name}:`));
      expect(job).toContain('issues: write');
      expect(job).toContain('pull-requests: write');
    }
    expect(jobs.length).toBe(11);
    expect(workflow).toContain("always() && needs.preflight.result == 'success'");
    expect(workflow).toContain('pr-rc-selftest.mjs seal');
  });
});

function schemaCase(candidateVersion: number, alphaVersion = 3) {
  const bytes = (value: unknown) => Buffer.from(JSON.stringify(value));
  const schema = (version: number) => ({
    schema_version: 1 as const,
    product: 'cli' as const,
    source_sha: FIXTURE.head,
    databases: [{ domain: 'tmt-core-db' as const, version }],
    source_files: [{ path: 'rust/schema.sql', sha256: 'a'.repeat(64) }],
  });
  const tag = 'v5.0.0-alpha.191';
  const base = `https://github.com/pj-tmt/tmt/releases/download/${tag}/`;
  const pointerUrl =
    'https://raw.githubusercontent.com/pj-tmt/tmt/release-index/channels/cli/alpha.json';
  const manifestUrl = `${base}dist-manifest.json`;
  const recordUrl = `${base}tmt-release-record.json`;
  const manifest = { announcement_tag: tag, tmt_application_schema: schema(alphaVersion) };
  const descriptor = (name: string, body: Buffer) => ({
    name,
    url: `${base}${name}`,
    size: body.length,
    sha256: createHash('sha256').update(body).digest('hex'),
  });
  const record = {
    schemaVersion: 1,
    product: 'cli',
    version: '5.0.0-alpha.191',
    tag,
    releaseId: 123,
    sourceSha: FIXTURE.head,
    manifest: descriptor('dist-manifest.json', bytes(manifest)),
    archives: Object.fromEntries(
      CLI_SCHEMA_TARGETS.map((target) => [
        target,
        descriptor(`tmt-cli-${target}.tar.gz`, Buffer.from('opaque archive')),
      ])
    ),
  };
  const pointer = {
    schemaVersion: 1,
    product: 'cli',
    channel: 'alpha',
    version: record.version,
    tag,
    record: descriptor('tmt-release-record.json', bytes(record)),
  };
  const catalog = {
    head_sha: FIXTURE.head,
    candidates: CLI_SCHEMA_TARGETS.map((target) => ({
      target,
      application_schema: schema(candidateVersion),
    })),
  };
  const routes = new Map([
    [pointerUrl, bytes(pointer)],
    [recordUrl, bytes(record)],
    [manifestUrl, bytes(manifest)],
  ]);
  const reads: { url: string; limit: number }[] = [];
  const get = (url: string, limit: number) => {
    reads.push({ url, limit });
    const body = routes.get(url);
    if (!body) throw new Error(`Unexpected public GET: ${url}`);
    return body;
  };
  const seal = () => {
    routes.set(manifestUrl, bytes(manifest));
    record.manifest = descriptor('dist-manifest.json', bytes(manifest));
    routes.set(recordUrl, bytes(record));
    pointer.record = descriptor('tmt-release-record.json', bytes(record));
    routes.set(pointerUrl, bytes(pointer));
  };
  return {
    catalog,
    get,
    reads,
    routes,
    manifest,
    record,
    pointer,
    seal,
    pointerUrl,
    recordUrl,
    manifestUrl,
  };
}
describe('selftest schema admission before baseline', () => {
  it.each([2, 3])(
    'admits candidate schema %i at or below the published alpha schema',
    (version) => {
      const test = schemaCase(version);
      const result = schemaPreflight(test.catalog, test.get);
      expect(result.alpha).toEqual({
        tag: test.record.tag,
        manifest_sha256: test.record.manifest.sha256,
        schema: 3,
      });
      expect(result.candidates.map((item) => item.schema)).toEqual([
        version,
        version,
        version,
        version,
      ]);
      expect(test.reads).toEqual([
        { url: test.pointerUrl, limit: 16 * 1024 },
        { url: test.recordUrl, limit: test.pointer.record.size },
        { url: test.manifestUrl, limit: test.record.manifest.size },
      ]);
    }
  );
  it('stops an ahead or mixed candidate with both schema numbers and the wait instruction', () => {
    for (const mixed of [false, true]) {
      const test = schemaCase(mixed ? 3 : 4);
      test.catalog.candidates[3].application_schema.databases[0].version = 4;
      expect(() => schemaPreflight(test.catalog, test.get)).toThrow(
        'Candidate application schema 4 is ahead of latest published alpha schema 3 (v5.0.0-alpha.191); wait for a published alpha carrying schema 4 before running the selftest.'
      );
    }
  });
  it('refuses absent, malformed, unbound or wrong-channel evidence without another source', () => {
    const mutations = [
      (test: ReturnType<typeof schemaCase>) => test.routes.delete(test.pointerUrl),
      (test: ReturnType<typeof schemaCase>) => test.routes.set(test.recordUrl, Buffer.from('{')),
      (test: ReturnType<typeof schemaCase>) => {
        test.pointer.record.sha256 = 'b'.repeat(64);
        test.routes.set(test.pointerUrl, Buffer.from(JSON.stringify(test.pointer)));
      },
      (test: ReturnType<typeof schemaCase>) => {
        test.pointer.record.size++;
        test.routes.set(test.pointerUrl, Buffer.from(JSON.stringify(test.pointer)));
      },
      (test: ReturnType<typeof schemaCase>) => {
        test.manifest.announcement_tag = 'v5.0.0-alpha.190';
        test.seal();
      },
      (test: ReturnType<typeof schemaCase>) => {
        test.record.sourceSha = 'b'.repeat(40);
        test.seal();
      },
      (test: ReturnType<typeof schemaCase>) => {
        test.manifest.tmt_application_schema.databases[0].version = -1;
        test.seal();
      },
      (test: ReturnType<typeof schemaCase>) => {
        test.catalog.candidates[0].application_schema.source_sha = 'b'.repeat(40);
      },
      (test: ReturnType<typeof schemaCase>) => {
        test.catalog.candidates[0].application_schema.databases = [];
      },
      (test: ReturnType<typeof schemaCase>) => {
        test.catalog.candidates.pop();
      },
      (test: ReturnType<typeof schemaCase>) => {
        test.pointer.product = 'ops';
        test.seal();
      },
      (test: ReturnType<typeof schemaCase>) => {
        test.pointer.channel = 'stable';
        test.seal();
      },
    ];
    for (const mutate of mutations) {
      const test = schemaCase(3);
      mutate(test);
      expect(() => schemaPreflight(test.catalog, test.get)).toThrow();
      expect(test.reads.every(({ url }) => !url.includes('api.github.com'))).toBe(true);
    }
  });
  it('gates manifest emission before baseline and refuses oversized evidence', () => {
    const source = fs.readFileSync(
      new URL('../../scripts/pr-rc-selftest.mjs', import.meta.url),
      'utf8'
    );
    expect(source).toContain('const schemaAdmission = schemaPreflight(catalog);');
    expect(source.indexOf('const schemaAdmission = schemaPreflight(catalog);')).toBeLessThan(
      source.indexOf('fs.writeFileSync(manifestFile')
    );
    const test = schemaCase(3);
    expect(() => schemaPreflight(test.catalog, () => Buffer.alloc(16 * 1024 + 1))).toThrow(
      'Schema evidence exceeds its byte cap.'
    );
  });
});
