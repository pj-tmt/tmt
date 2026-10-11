import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import {
  FIXTURE,
  assertOwnedPull,
  assertPRReceipt,
  assertRefusal,
  assertPositive,
  cleanupExact,
  safeEvidence,
  readOwnedReceipt,
} from '../../scripts/pr-rc-selftest.mjs';

const pull = (consumed = true) => ({
  number: FIXTURE.pr,
  base: { repo: { full_name: 'pj-tmt/tmt' } },
  head: {
    repo: { full_name: 'pj-tmt/tmt' },
    ref: FIXTURE.branch,
    sha: consumed ? 'f'.repeat(40) : FIXTURE.head,
  },
  state: 'open',
  labels: [{ name: 'rc-build' }],
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
    expect(() => assertPRReceipt(receipt(), { candidates: [candidate] })).not.toThrow();
    for (const modify of [
      (r: ReturnType<typeof receipt>) => (r.source.evidence.head_sha = 'f'.repeat(40)),
      (r: ReturnType<typeof receipt>) => r.source.evidence.producer.run_id++,
      (r: ReturnType<typeof receipt>) => r.source.evidence.catalog_artifact_id++,
      (r: ReturnType<typeof receipt>) => r.source.evidence.eligibility.enabled_event_id++,
      (r: ReturnType<typeof receipt>) => (r.channel = 'alpha'),
    ]) {
      const value = receipt();
      modify(value);
      expect(() => assertPRReceipt(value, { candidates: [candidate] })).toThrow();
    }
    expect(() =>
      assertPRReceipt(receipt(), { candidates: [{ ...candidate, payload_artifact: { id: 999 } }] })
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
        if (route.startsWith('actions/artifacts/')) {
          const id = Number(route.split('/').at(-1));
          return rows.ordinary.includes(id) ? { status: 200, data: artifact(id) } : { status: 404 };
        }
        return { status: 204 };
      },
      { ...pull(), state: 'closed' }
    );
    expect(result.retained).toBe(false);
    expect(calls.slice(0, 2)).toEqual([
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
