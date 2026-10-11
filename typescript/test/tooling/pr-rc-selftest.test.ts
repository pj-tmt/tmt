import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import {
  QUALIFICATION,
  producerRunInput,
  waitForPull,
  deriveFixture,
  assertOwnedPull,
  assertPRReceipt,
  assertRefusal,
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
  it('reads the real CLI namespace and refuses an active receipt outside the isolated prefix', () => {
    const world = fs.mkdtempSync(path.join(os.tmpdir(), 'pr-rc-selftest-'));
    try {
      const root = path.join(world, 'prefix/lib/tmux-team');
      const active = path.join(root, 'releases/owned');
      fs.mkdirSync(active, { recursive: true });
      fs.writeFileSync(path.join(active, 'receipt.json'), JSON.stringify(receipt()));
      fs.symlinkSync('releases/owned', path.join(root, 'current'));
      expect(readOwnedReceipt(world)).toEqual(receipt());
      fs.unlinkSync(path.join(root, 'current'));
      fs.symlinkSync(world, path.join(root, 'current'));
      expect(() => readOwnedReceipt(world)).toThrow('escapes owned prefix');
    } finally {
      fs.rmSync(world, { recursive: true, force: true });
    }
  });

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
  it('requires the pinned unavailable error and byte-identical state, not just a nonzero exit', () => {
    const state = { receipt: 'original', links: { tmt: 'original' }, sentinel: 'untouched' };
    const result = {
      status: 1,
      stdout: '{"error":{"code":"NATIVE_PR_CHANNEL_UNAVAILABLE"}}',
      stderr: '',
    };
    expect(() => assertRefusal(result, state, structuredClone(state))).not.toThrow();
    expect(() => assertRefusal({ ...result, status: 0 }, state, state)).toThrow();
    expect(() => assertRefusal({ ...result, status: null }, state, state)).toThrow();
    expect(() => assertRefusal({ ...result, stdout: 'network error' }, state, state)).toThrow();
    expect(() => assertRefusal(result, state, { ...state, receipt: 'changed' })).toThrow();
    expect(() => assertRefusal(result, state, { ...state, sentinel: 'changed' })).toThrow();
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
      return { status: 200, data: alive.get(Number(route.split('/').at(-1)))! };
    };
    const result = await cleanupExact(rows, read, pull(false));
    expect(result.retained).toBe(true);
    expect(result.artifacts.map((item) => item.id)).toEqual(rows.rc.map((item) => item.id));
    expect(calls).toEqual(Array(7).fill('GET'));
    alive.delete(FIXTURE.catalog);
    await expect(
      cleanupExact(
        rows,
        (method, route) => {
          const data = alive.get(Number(route.split('/').at(-1)));
          return data ? { status: 200, data } : { status: 404 };
        },
        pull(false)
      )
    ).rejects.toThrow('Unconsumed fixture artifact disappeared');
  });
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
    expect(jobs.length).toBe(11);
    expect(workflow).toContain("always() && needs.preflight.result == 'success'");
    expect(workflow).toContain('pr-rc-selftest.mjs seal');
  });
});
