import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import {
  FIXTURE,
  assertOwnedPull,
  assertPRReceipt,
  assertRefusal,
  cleanupExact,
  safeEvidence,
  readOwnedReceipt,
} from '../../scripts/pr-rc-selftest.mjs';

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
    const pull = {
      number: FIXTURE.pr,
      base: { repo: { full_name: 'pj-tmt/tmt' } },
      head: { repo: { full_name: 'pj-tmt/tmt' }, ref: FIXTURE.branch },
    };
    expect(() => assertOwnedPull(pull)).not.toThrow();
    expect(() => assertOwnedPull({ ...pull, number: 42 })).toThrow();
    expect(() =>
      assertOwnedPull({ ...pull, head: { ...pull.head, repo: { full_name: 'someone/tmt' } } })
    ).toThrow();
    expect(() => assertOwnedPull({ ...pull, head: { ...pull.head, ref: 'main' } })).toThrow();
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
    const result = await cleanupExact(rows, (method, route) => {
      const id = Number(route.split('/').at(-1));
      if (method === 'DELETE') {
        deleted.push(id);
        alive.delete(id);
        return { status: 204 };
      }
      const data = alive.get(id);
      return data ? { status: 200, data } : { status: 404 };
    });
    expect(deleted).toEqual([
      FIXTURE.catalog,
      FIXTURE.payloads[0],
      FIXTURE.payloads[2],
      FIXTURE.payloads[3],
    ]);
    expect(result.find((item) => item.id === FIXTURE.payloads[1])?.alreadyAbsent).toBe(true);
    expect([...alive.keys()]).toEqual([20, 21]);
  });
  it('refuses changed ownership and a deletion that does not settle', async () => {
    const rows = manifest();
    let deletes = 0;
    await expect(
      cleanupExact(rows, () => ({
        status: 200,
        data: { ...rows.rc[0], workflow_run: { id: 999 } },
      }))
    ).rejects.toThrow();
    await expect(
      cleanupExact(rows, (method) => {
        if (method === 'DELETE') deletes++;
        return { status: 200, data: { ...rows.rc[0], workflow_run: { id: FIXTURE.run } } };
      })
    ).rejects.toThrow();
    expect(deletes).toBe(1);
    await expect(
      cleanupExact({ ...rows, rc: [rows.rc[1], ...rows.rc.slice(1)] }, () => ({ status: 404 }))
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
