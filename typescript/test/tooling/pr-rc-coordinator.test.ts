import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { describe, expect, it } from 'vite-plus/test';
import {
  capturePRRCVerification,
  validatePRRCEligibility,
  stagePRRCPayloads,
  buildPRRCCatalog,
  prRCPayloadName,
  prRCCatalogName,
  createPRRCAPI,
  runPRRCCommand,
  RC_TARGETS,
  RC_TTL_MS,
} from '../../scripts/pr-rc-coordinator.mjs';
// Independent Python hashlib/json/USTAR/gzip literals; synthetic bytes, no executed binary.
const SOURCE_FILES = {
  'rust/crates/tmt-adapters/src/storage/migrations.rs': 'synthetic migration owner\n',
  'rust/crates/tmt-adapters/src/storage/migrations/host_names.rs': 'synthetic host owner\n',
  'rust/crates/tmt-adapters/src/storage/schema/001_initial.sql': 'synthetic schema\n',
};
const ARCHIVES = [
  'H4sIAAAAAAAC/+3WQU+DMBjGcT4KX6ATRlvPZJJIQtAMYuKxAQxNoJDyTue3t2qiBhO9MA7b87u06aWnf99ST6zqNFPKVq3kTI1j17Ba2RdtrqgnbwGBcy3Ex+rM1yDYiu/9+3kYScE9P/BWcJhIWXe9d5mmV0NtQ7ryd1nqP+kjHWzjwaWgf/pntdXPjWVtY2t7sv6jWf9yywX6X7f/auhHZfRgEAX6/+w/S3dJXiRLzH/J+R/9h7P+eeieBPS/av+drhozYfaj/6/+87hMHxKW5kUZZ9mmr0/Ufyhn/QsRSfS/bv9mIMSP/n/2X96m+xt2H+/LR5bfle43UGzoSIv3/2v+SxEF6H/1/t0PYEISAAAAAAAAAAAAAGfhDRvGnfAAKAAA',
  'H4sIAAAAAAAC/+3WQWuDMBjGcT+KXyCdVmPO0gkNiBtVBjuKOgwzscTXzX37uV02LIxBOw/r87sk5JLTn/clTazqFCtLW7VRyEbzbPpXwzplxonpcehuSJNzFm8mOP88Z8vT87b86/7x7nPfCxzXc1YwDlTa+XvnOg1vhtqGVOXuUuk+qYlG2zhwLehX/bPaqpfGsraxtf2D/oNF/yIUIfpft/+q18fSqN4gCvT/vf9U7pIsT86b/1EY/tC/v5z/Ad+i/3X771TVmAGzH/0v+s/iQj4kTGZ5EafpRtcX79+PFv1HIuLof93+TU+IH/2f9l/s5eGW3ceH4pFld8W8DeQbmuiC/Z/MfyG4QP+r9z9vAAOSAAAAAAAAAAAAAPgX3gHkOVj0ACgAAA==',
  'H4sIAAAAAAAC/+3WQWuDMBjGcT+KXyCdGk13lU6YIG5UGew0RDMa0Cjx7ea+/bLC2HDQk/WwPr9LRARPf55QR6xuFZtuxYsIWTUMrWRNZd6VvqGOnCV41jaKTqc1Pz0viH6ev977nIvAcT1nBceRKmN/71yn8UPTQZKq3V2Wuq9qoqORDlwLOt8/a4x6k4YdpGnM5frns/6jbcTR/7r91303VFr1GlGg/1P/WbpL8iJZZP9FGJ7p35/vv+A++l+3/1bVUo/YfvT/3X8el+lTwtK8KOMs23TNpfr3xXz/A/s5+l+1f90T4kf/v/ov79P9HXuM9+Uzyx9KexsoNjTR8v3/2X8RcIH+V+/f3gBGJAEAAAAAAAAAAADwL3wCHMimGQAoAAA=',
  'H4sIAAAAAAAC/+3WQWuDMBjGcT+KXyBdtDHuKp1QQdyoMthpFHU0zMQSXzf37ed22WZhDNp6WJ/fJSGXnP68L2liZaPYcC0fpWC9eTbtq2GNMv3AdN81V6TJOQ4fhUHweY6mJ+d+8HX/ePeE9DzH5c4M+o62dvzeuUzdm6FdTap0V2niPqmBels7cCnoL/2zyqqX2rJdbSt7jv6Xk/5Dn/vof97+y1bvt0a1BlGg/2/9p8kqzvL4yPkvhfilf2/Sf8AlR//z9t+osjYdZj/6/9l/FhXJfcySLC+iNF3o6vT9e3LSvxThEv3P279pCfGj/4P+i3WyuWF30aZ4YNltMW4D+YIGOmX/B/M/FDJA/7P3P24AHZIAAAAAAAAAAAAA+BfeAVDqJ1sAKAAA',
];
const MANIFEST =
  '{\n  "artifacts": {\n    "tmt-cli-aarch64-apple-darwin.tar.gz": {\n      "kind": "executable-zip",\n      "name": "tmt-cli-aarch64-apple-darwin.tar.gz",\n      "target_triples": [\n        "aarch64-apple-darwin"\n      ],\n      "checksums": {\n        "sha256": "d3b5d904be830b6808e5860c9ade91179d8d00b82740291f6578592bd8223353"\n      },\n      "assets": [\n        {\n          "path": "tmt"\n        },\n        {\n          "path": "tmt-driver-herdr"\n        },\n        {\n          "path": "LICENSE"\n        },\n        {\n          "path": "NATIVE-INSTALL.md"\n        },\n        {\n          "path": "THIRD-PARTY-NOTICES.txt"\n        }\n      ]\n    },\n    "tmt-cli-aarch64-unknown-linux-musl.tar.gz": {\n      "kind": "executable-zip",\n      "name": "tmt-cli-aarch64-unknown-linux-musl.tar.gz",\n      "target_triples": [\n        "aarch64-unknown-linux-musl"\n      ],\n      "checksums": {\n        "sha256": "5839e5ef876128ae8a3daa27573b1f27ece6016a6fbad02be7ee614d2380689f"\n      },\n      "assets": [\n        {\n          "path": "tmt"\n        },\n        {\n          "path": "tmt-driver-herdr"\n        },\n        {\n          "path": "LICENSE"\n        },\n        {\n          "path": "NATIVE-INSTALL.md"\n        },\n        {\n          "path": "THIRD-PARTY-NOTICES.txt"\n        }\n      ]\n    },\n    "tmt-cli-x86_64-apple-darwin.tar.gz": {\n      "kind": "executable-zip",\n      "name": "tmt-cli-x86_64-apple-darwin.tar.gz",\n      "target_triples": [\n        "x86_64-apple-darwin"\n      ],\n      "checksums": {\n        "sha256": "939106922eac7d5d320d62a6a97ad3ecdf3c79529c974ab4bf31eb2363d5ae88"\n      },\n      "assets": [\n        {\n          "path": "tmt"\n        },\n        {\n          "path": "tmt-driver-herdr"\n        },\n        {\n          "path": "LICENSE"\n        },\n        {\n          "path": "NATIVE-INSTALL.md"\n        },\n        {\n          "path": "THIRD-PARTY-NOTICES.txt"\n        }\n      ]\n    },\n    "tmt-cli-x86_64-unknown-linux-musl.tar.gz": {\n      "kind": "executable-zip",\n      "name": "tmt-cli-x86_64-unknown-linux-musl.tar.gz",\n      "target_triples": [\n        "x86_64-unknown-linux-musl"\n      ],\n      "checksums": {\n        "sha256": "a0ea4bafed73aa2039f4222dd61a4ae4f9506b1e0e945002a903b31fc28c1c94"\n      },\n      "assets": [\n        {\n          "path": "tmt"\n        },\n        {\n          "path": "tmt-driver-herdr"\n        },\n        {\n          "path": "LICENSE"\n        },\n        {\n          "path": "NATIVE-INSTALL.md"\n        },\n        {\n          "path": "THIRD-PARTY-NOTICES.txt"\n        }\n      ]\n    }\n  },\n  "releases": [\n    {\n      "app_name": "tmt-cli",\n      "app_version": "5.0.0-alpha.92",\n      "artifacts": [\n        "tmt-cli-aarch64-apple-darwin.tar.gz",\n        "tmt-cli-aarch64-unknown-linux-musl.tar.gz",\n        "tmt-cli-x86_64-apple-darwin.tar.gz",\n        "tmt-cli-x86_64-unknown-linux-musl.tar.gz"\n      ]\n    }\n  ],\n  "tmt_application_schema": {\n    "schema_version": 1,\n    "product": "cli",\n    "source_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",\n    "databases": [\n      {\n        "domain": "tmt-core-db",\n        "version": 48\n      }\n    ],\n    "source_files": [\n      {\n        "path": "rust/crates/tmt-adapters/src/storage/migrations.rs",\n        "sha256": "785c119863d25bf07df836140552f56adf09e1afa137b3f11157dcfe97cb6b95"\n      },\n      {\n        "path": "rust/crates/tmt-adapters/src/storage/migrations/host_names.rs",\n        "sha256": "409b432374bf273c3a17455df0fe172f09a9475eb552a852a2a0866a9048dcd4"\n      },\n      {\n        "path": "rust/crates/tmt-adapters/src/storage/schema/001_initial.sql",\n        "sha256": "553101feebd4c71b6544d16a606f169e08df5dcf053315d624b00a1f2ac08513"\n      }\n    ]\n  }\n}\n';
const hash = (bytes: string | Buffer) => createHash('sha256').update(bytes).digest('hex');
const head = 'a'.repeat(40);
const tooling = 'e'.repeat(40);
const now = Date.parse('2026-10-10T10:00:00Z');
const identity = { pr: 1639, head, head12: head.slice(0, 12), runId: 8001, attempt: 1 };
function observation() {
  return {
    pull: {
      number: 1639,
      state: 'open',
      head: { sha: head, repo: { full_name: 'pj-tmt/tmt' } },
      base: { repo: { full_name: 'pj-tmt/tmt' } },
      labels: [{ id: 44, name: 'rc-build' }],
    },
    timeline: [
      {
        id: 55,
        event: 'labeled',
        label: { id: 44, name: 'rc-build' },
        created_at: '2026-10-10T09:00:00Z',
      },
    ],
    run: {
      id: 8001,
      run_attempt: 1,
      workflow_id: 702,
      path: '.github/workflows/pr-rc.yml',
      head_sha: tooling,
      head_branch: 'main',
      event: 'workflow_dispatch',
      status: 'in_progress',
      repository: { full_name: 'pj-tmt/tmt' },
      display_title: `pr-rc #1639 ${identity.head12}`,
    },
  };
}
function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-pr-rc-'));
  const directory = path.join(root, 'bundle');
  fs.mkdirSync(directory);
  for (const [name, bytes] of Object.entries(SOURCE_FILES)) {
    fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
    fs.writeFileSync(path.join(root, name), bytes);
  }
  const schema = JSON.parse(MANIFEST).tmt_application_schema;
  fs.writeFileSync(path.join(directory, 'dist-manifest.json'), MANIFEST);
  const snapshot = path.join(root, 'snapshot.json');
  fs.writeFileSync(
    snapshot,
    JSON.stringify({
      schema: 1,
      product: 'cli',
      cut: head,
      version: '5.0.0-alpha.92',
      hashes: Object.fromEntries(
        Object.entries(SOURCE_FILES).map(([name, bytes]) => [name, hash(bytes)])
      ),
    })
  );
  const reports = RC_TARGETS.map((target, i) => {
    const archive = Buffer.from(ARCHIVES[i], 'base64');
    fs.writeFileSync(path.join(directory, `tmt-cli-${target}.tar.gz`), archive);
    fs.writeFileSync(path.join(directory, `${target}-notices.txt`), 'verified notices\n');
    fs.writeFileSync(
      path.join(directory, `${target}-application-schema.json`),
      JSON.stringify({
        schema_version: 1,
        target,
        source_snapshot_sha256: 'c'.repeat(64),
        archive_sha256: hash(archive),
        record: schema,
        output_sha256: hash(`${JSON.stringify(schema)}\n`),
        binary_sha256: 'b'.repeat(64),
      }) + '\n'
    );
    return capturePRRCVerification({
      root,
      directory,
      target,
      snapshot,
      runId: 8001,
      attempt: 1,
      toolingSha: tooling,
    });
  });
  return {
    root,
    directory,
    reports,
    input: {
      directory,
      reports,
      output: path.join(root, 'payload'),
      head,
      runId: 8001,
      attempt: 1,
      toolingSha: tooling,
    },
    cleanup: () => fs.rmSync(root, { recursive: true, force: true }),
  };
}

describe('trusted main PR RC producer', () => {
  it('stages four unchanged root payloads and the revision-2 Core literal catalog', async () => {
    const f = fixture();
    try {
      const candidates = stagePRRCPayloads(f.input);
      const calls: string[] = [];
      const bytes = await buildPRRCCatalog(
        {
          identity,
          candidates,
          producer: {
            workflow_id: 702,
            workflow_path: '.github/workflows/pr-rc.yml',
            workflow_sha256: 'd'.repeat(64),
            tooling_sha: tooling,
            run_id: 8001,
            run_attempt: 1,
          },
          publishedAtMs: now,
        },
        {
          observe: () => {
            calls.push('observe');
            return observation();
          },
          payload: (name) => {
            calls.push(name);
            const i = RC_TARGETS.findIndex(
              (target) => name === prRCPayloadName(1639, target, 8001, 1)
            );
            return {
              id: 900 + i,
              name,
              zip_sha256: 'f'.repeat(64),
              zip_bytes: 1024,
              members: [candidates[i].dist_manifest, candidates[i].archive],
            };
          },
        }
      );
      expect(calls).toEqual([
        'observe',
        ...RC_TARGETS.map((target) => prRCPayloadName(1639, target, 8001, 1)),
        'observe',
      ]);
      const catalog = JSON.parse(bytes.toString());
      expect(Object.keys(catalog).sort()).toEqual(
        [
          'schema_version',
          'kind',
          'repository',
          'pr',
          'head_sha',
          'producer',
          'eligibility',
          'published_at_ms',
          'expires_at_ms',
          'candidates',
        ].sort()
      );
      expect(catalog).toMatchObject({
        schema_version: 2,
        kind: 'tmt-pr-rc-catalog',
        repository: 'pj-tmt/tmt',
        pr: 1639,
        head_sha: head,
        published_at_ms: now,
        expires_at_ms: now + RC_TTL_MS,
        eligibility: {
          label: 'rc-build',
          label_id: 44,
          enabled_event_id: 55,
          enabled_at_ms: Date.parse('2026-10-10T09:00:00Z'),
        },
      });
      expect(catalog.candidates).toEqual(
        candidates.map((candidate, i) => ({
          ...candidate,
          payload_artifact: {
            id: 900 + i,
            name: prRCPayloadName(1639, RC_TARGETS[i], 8001, 1),
            zip_sha256: 'f'.repeat(64),
            zip_bytes: 1024,
          },
        }))
      );
      expect(prRCCatalogName(1639)).toBe('tmt-pr-rc-catalog-v2-pr1639');
      for (const target of RC_TARGETS)
        expect(fs.readdirSync(path.join(f.input.output, target)).sort()).toEqual([
          'dist-manifest.json',
          `tmt-cli-${target}.tar.gz`,
        ]);
    } finally {
      f.cleanup();
    }
  });

  it.each([
    'fork',
    'closed',
    'unlabeled',
    'head',
    'branch',
    'workflow',
    'run',
    'epoch',
    'reopen',
  ] as const)('refuses %s eligibility before any output', (kind) => {
    const o = observation();
    if (kind === 'fork') o.pull.head.repo.full_name = 'other/repo';
    if (kind === 'closed') o.pull.state = 'closed';
    if (kind === 'unlabeled') o.pull.labels = [];
    if (kind === 'head') o.pull.head.sha = 'b'.repeat(40);
    if (kind === 'branch') o.run.head_branch = 'feature';
    if (kind === 'workflow') o.run.path = '.github/workflows/ci.yml';
    if (kind === 'run') o.run.id++;
    if (kind === 'epoch') o.timeline[0].event = 'unlabeled';
    if (kind === 'reopen')
      (o.timeline as unknown[]).push({
        id: 56,
        event: 'reopened',
        created_at: '2026-10-10T09:30:00Z',
      });
    expect(() => validatePRRCEligibility(o, identity, now)).toThrow();
  });
  it('records changing main tooling as diagnostics rather than a per-commit trust anchor', () => {
    const o = observation();
    o.run.head_sha = 'f'.repeat(40);
    expect(validatePRRCEligibility(o, identity, now)).toEqual(
      validatePRRCEligibility(observation(), identity, now)
    );
  });
  it.each([
    'archive',
    'manifest',
    'notices',
    'schema',
    'source',
    'version',
    'target',
    'duplicate',
    'tooling',
    'attempt',
  ] as const)('refuses changed %s preparation', (kind) => {
    const f = fixture();
    try {
      if (kind === 'archive')
        fs.appendFileSync(path.join(f.directory, `tmt-cli-${RC_TARGETS[0]}.tar.gz`), 'changed');
      if (kind === 'manifest') fs.appendFileSync(path.join(f.directory, 'dist-manifest.json'), ' ');
      if (kind === 'notices')
        fs.appendFileSync(path.join(f.directory, `${RC_TARGETS[0]}-notices.txt`), 'changed');
      if (kind === 'schema') f.reports[0].application_schema.databases[0].version++;
      if (kind === 'source') f.reports[0].source_sha = 'b'.repeat(40);
      if (kind === 'version') f.reports[0].version = '5.0.0-alpha.93';
      if (kind === 'target') f.reports.pop();
      if (kind === 'duplicate') f.reports[1] = f.reports[0];
      if (kind === 'tooling') f.reports[0].prepare_tooling_sha = 'f'.repeat(40);
      if (kind === 'attempt') f.reports[0].prepare_run_attempt++;
      expect(() => stagePRRCPayloads(f.input)).toThrow();
    } finally {
      f.cleanup();
    }
  });
  it('withholds the catalog on a changed final epoch and never retries a failed payload read', async () => {
    const f = fixture();
    try {
      const candidates = stagePRRCPayloads(f.input);
      const input = {
        identity,
        candidates,
        producer: {
          workflow_id: 702,
          workflow_path: '.github/workflows/pr-rc.yml',
          workflow_sha256: 'd'.repeat(64),
          tooling_sha: tooling,
          run_id: 8001,
          run_attempt: 1,
        },
        publishedAtMs: now,
      };
      let reads = 0,
        observations = 0;
      const payload = (name: string) => {
        const i = reads++;
        return {
          id: 900 + i,
          name,
          zip_sha256: 'f'.repeat(64),
          zip_bytes: 1024,
          members: [candidates[i].dist_manifest, candidates[i].archive],
        };
      };
      await expect(
        buildPRRCCatalog(input, {
          observe: () => {
            const o = observation();
            if (observations++) o.timeline[0].id++;
            return o;
          },
          payload,
        })
      ).rejects.toThrow('epoch changed');
      expect(reads).toBe(4);
      reads = 0;
      await expect(
        buildPRRCCatalog(input, {
          observe: observation,
          payload: () => {
            reads++;
            throw new Error('read unavailable');
          },
        })
      ).rejects.toThrow('read unavailable');
      expect(reads).toBe(1);
    } finally {
      f.cleanup();
    }
  });
});

type CleanupOptions = {
  call(method: string, route: string): { status: number; data: unknown };
  now(): number;
  sleep(ms: number): Promise<void>;
  pr: number | null;
  keepRun: number | null;
  runId: number;
};
const cleanupWorkflow = fs.readFileSync(
  new URL('../../../.github/workflows/pr-rc-cleanup.yml', import.meta.url),
  'utf8'
);
const cleanupSource = cleanupWorkflow
  .split('          // BEGIN_CLEANUP\n')[1]
  .split('          // END_CLEANUP')[0]
  .replace(/^ {10}/gm, '')
  .trim();
const cleanupPRRC = new Function(`return (${cleanupSource});`)() as (
  options: CleanupOptions
) => Promise<{ requests: number; deleted: number[]; bytes: number; retainedLimit: number }>;
function cleanupFixture() {
  const runs = [11, 12, 13].map((id) => ({
    id,
    workflow_id: 702,
    path: '.github/workflows/pr-rc.yml',
    head_branch: 'main',
    event: 'workflow_dispatch',
    repository: { full_name: 'pj-tmt/tmt' },
    display_title: `pr-rc #1639 ${identity.head12}`,
    created_at: '2026-10-10T09:00:00Z',
    status: 'completed',
    conclusion: 'success',
  }));
  const all = new Map(
    runs.map((run) => [
      run.id,
      [
        {
          id: run.id * 10,
          name: prRCCatalogName(1639),
          size_in_bytes: 10,
          workflow_run: { id: run.id },
        },
        ...RC_TARGETS.map((target, i) => ({
          id: run.id * 10 + i + 1,
          name: prRCPayloadName(1639, target, run.id, 1),
          size_in_bytes: 20,
          workflow_run: { id: run.id },
        })),
        {
          id: run.id * 10 + 9,
          name: 'ordinary-preparation',
          size_in_bytes: 100,
          workflow_run: { id: run.id },
        },
      ],
    ])
  );
  const calls: { method: string; route: string }[] = [];
  const pull = observation().pull;
  const call = (method: string, route: string): { status: number; data: unknown } => {
    calls.push({ method, route });
    const suffix = route.replace('repos/pj-tmt/tmt/', '');
    if (suffix === 'actions/workflows/pr-rc.yml')
      return { status: 200, data: { id: 702, path: '.github/workflows/pr-rc.yml' } };
    if (suffix.startsWith('actions/workflows/702/runs?'))
      return { status: 200, data: { total_count: runs.length, workflow_runs: runs } };
    if (suffix === 'pulls/1639') return { status: 200, data: pull };
    const run = /^actions\/runs\/(\d+)(\/artifacts\?per_page=100|\/cancel)?$/.exec(suffix);
    if (run) {
      const id = Number(run[1]);
      if (run[2] === '/cancel') {
        runs.find((item) => item.id === id)!.status = 'completed';
        return { status: 202, data: null };
      }
      if (run[2])
        return { status: 200, data: { total_count: all.get(id)!.length, artifacts: all.get(id) } };
      return { status: 200, data: runs.find((item) => item.id === id) };
    }
    const artifact = /^actions\/artifacts\/(\d+)$/.exec(suffix);
    if (artifact) {
      const id = Number(artifact[1]);
      const runId = [...all].find(([, items]) => items.some((item) => item.id === id))?.[0];
      if (runId === undefined) return { status: 404, data: null };
      if (method === 'DELETE') {
        all.set(
          runId,
          all.get(runId)!.filter((item) => item.id !== id)
        );
        return { status: 204, data: null };
      }
      return { status: 200, data: all.get(runId)!.find((item) => item.id === id) };
    }
    throw new Error(`Unexpected cleanup request ${method} ${suffix}`);
  };
  return {
    runs,
    all,
    calls,
    pull,
    call,
    options: {
      call,
      now: () => now,
      sleep: async () => {},
      pr: 1639,
      keepRun: null,
      runId: 9000,
    } as CleanupOptions,
  };
}

describe('one no-checkout exact-ID RC cleanup implementation', () => {
  it('keeps at most two published generations and preserves ordinary preparation artifacts', async () => {
    const f = cleanupFixture();
    const result = await cleanupPRRC(f.options);
    expect(result.deleted).toEqual([110, 111, 112, 113, 114]);
    expect(result.bytes).toBe(90);
    expect(f.all.get(11)!.map((item) => item.id)).toEqual([119]);
    expect(f.calls.filter((call) => call.method === 'DELETE').map((call) => call.route)).toEqual(
      [110, 111, 112, 113, 114].map((id) => `repos/pj-tmt/tmt/actions/artifacts/${id}`)
    );
    expect(f.all.get(12)).toHaveLength(6);
    expect(f.all.get(13)).toHaveLength(6);
  });
  it.each(['closed', 'unlabeled', 'expired'] as const)(
    'actively deletes %s RC outputs, catalog first, idempotently',
    async (kind) => {
      const f = cleanupFixture();
      if (kind === 'closed') f.pull.state = 'closed';
      if (kind === 'unlabeled') f.pull.labels = [];
      if (kind === 'expired') f.options.now = () => now + 4 * 86400000;
      const result = await cleanupPRRC(f.options);
      expect(result.deleted).toEqual([
        130, 131, 132, 133, 134, 120, 121, 122, 123, 124, 110, 111, 112, 113, 114,
      ]);
      expect(result.bytes).toBe(270);
      expect(
        [...f.all.values()].every(
          (items) => items.length === 1 && items[0].name === 'ordinary-preparation'
        )
      ).toBe(true);
      expect((await cleanupPRRC(f.options)).deleted).toEqual([]);
    }
  );
  it.each(['workflow', 'branch', 'repo', 'pr', 'artifact'] as const)(
    'refuses wrong %s ownership before deleting an ID',
    async (kind) => {
      const f = cleanupFixture();
      if (kind === 'workflow') f.runs[0].workflow_id++;
      if (kind === 'branch') f.runs[0].head_branch = 'feature';
      if (kind === 'repo') f.runs[0].repository.full_name = 'fork/repo';
      if (kind === 'pr') f.pull.head.repo.full_name = 'fork/repo';
      if (kind === 'artifact') f.all.get(11)![0].workflow_run.id = 99;
      await expect(cleanupPRRC(f.options)).rejects.toThrow();
      expect(f.calls.filter((call) => call.method === 'DELETE')).toEqual([]);
    }
  );
  it('retires catalog before cancelling only its owned pending run and re-inventories after settlement', async () => {
    const f = cleanupFixture();
    f.pull.state = 'closed';
    f.runs[2].status = 'in_progress';
    await cleanupPRRC(f.options);
    const cancelled = f.calls.findIndex((call) => call.method === 'POST');
    expect(f.calls[cancelled].route).toBe('repos/pj-tmt/tmt/actions/runs/13/cancel');
    expect(
      f.calls.findIndex((call) => call.method === 'DELETE' && call.route.endsWith('/130'))
    ).toBeLessThan(cancelled);
    expect(
      f.calls.findIndex((call) => call.method === 'DELETE' && call.route.endsWith('/131'))
    ).toBeGreaterThan(cancelled);
  });
  it('reports late uploads rather than declaring absence', async () => {
    const f = cleanupFixture();
    f.pull.state = 'closed';
    const call = f.call;
    f.options.call = (method, route) => {
      const result = call(method, route);
      if (method === 'DELETE' && route.endsWith('/134'))
        f.all.get(13)!.push({
          id: 138,
          name: prRCPayloadName(1639, RC_TARGETS[0], 13, 2),
          size_in_bytes: 20,
          workflow_run: { id: 13 },
        });
      return result;
    };
    await expect(cleanupPRRC(f.options)).rejects.toThrow('Late RC upload');
    expect(f.all.get(13)!.some((item) => item.id === 138)).toBe(true);
  });
  it.each(['inventory', 'absence', 'settlement', 'retention'] as const)(
    'fails visibly on unknown %s cleanup evidence',
    async (kind) => {
      const f = cleanupFixture();
      f.pull.state = 'closed';
      if (kind === 'retention') f.options.keepRun = 12;
      if (kind === 'settlement') f.runs[2].status = 'in_progress';
      const call = f.call;
      f.options.call = (method, route) => {
        if (kind === 'absence' && method === 'DELETE') return { status: 204, data: null };
        if (kind === 'settlement' && method === 'POST') return { status: 202, data: null };
        const result = call(method, route);
        if (kind === 'inventory' && route.includes('/artifacts?'))
          return { status: 200, data: { total_count: 101, artifacts: [] } };
        return result;
      };
      await expect(cleanupPRRC(f.options)).rejects.toThrow(
        kind === 'inventory'
          ? 'inventory incomplete'
          : kind === 'absence'
            ? 'still present'
            : kind === 'settlement'
              ? 'not settled'
              : 'current producer caller'
      );
      expect(f.all.get(13)!.some((item) => item.id === 131)).toBe(true);
    }
  );
  it('uses the same function for close, schedule, dispatch and producer retirement without a checkout', () => {
    for (const trigger of ['pull_request:', 'schedule:', 'workflow_dispatch:', 'workflow_call:'])
      expect(cleanupWorkflow).toContain(trigger);
    expect(cleanupWorkflow).toContain('actions: write');
    expect(cleanupWorkflow).not.toMatch(/uses:.*checkout|contents: write|pull_request_target/);
    expect(cleanupWorkflow.match(/async function cleanupPRRC/g)).toHaveLength(1);
  });
});

describe('bounded authenticated Actions readback', () => {
  const makeZip = (names: string[]) =>
    execFileSync('python3', [
      '-c',
      `import io,sys,zipfile,json
b=io.BytesIO()
with zipfile.ZipFile(b,'w',compression=zipfile.ZIP_STORED) as z:
 for name in json.loads(sys.argv[1]): z.writestr(name,b'verified bytes')
sys.stdout.buffer.write(b.getvalue())`,
      JSON.stringify(names),
    ]);
  it.each(['report.json', 'dist-manifest.json'])(
    'reads %s without extracting or executing payload data',
    (name) => {
      const zip = makeZip([name]);
      const calls: string[][] = [];
      const api = createPRRCAPI((file, args, options) => {
        if (file === 'python3') return execFileSync(file, args, options);
        calls.push(args);
        return zip;
      });
      expect(
        api.members(
          { id: 17, size_in_bytes: zip.length, digest: `sha256:${hash(zip)}`, expired: false },
          1024
        )
      ).toEqual([{ name, bytes: Buffer.from('verified bytes') }]);
      expect(calls).toEqual([
        [
          'api',
          'repos/pj-tmt/tmt/actions/artifacts/17/zip',
          '-H',
          'Accept: application/octet-stream',
        ],
      ]);
    }
  );
  it.each(['../escape', 'dir/report.json', 'back\\slash', '.', '..'])(
    'rejects non-root member %s',
    (name) => {
      const zip = makeZip([name]);
      const api = createPRRCAPI((file, args, options) =>
        file === 'python3'
          ? execFileSync(file, args, { ...options, stdio: ['pipe', 'pipe', 'pipe'] })
          : zip
      );
      expect(() =>
        api.members(
          { id: 17, size_in_bytes: zip.length, digest: `sha256:${hash(zip)}`, expired: false },
          1024
        )
      ).toThrow();
    }
  );
  it.each(['digest', 'size', 'expired', 'duplicate', 'extra', 'oversize'])(
    'refuses %s ZIP evidence',
    (kind) => {
      const zip = makeZip(
        kind === 'duplicate'
          ? ['report.json', 'report.json']
          : kind === 'extra'
            ? ['a', 'b', 'c']
            : ['report.json']
      );
      const artifact = {
        id: 17,
        size_in_bytes: zip.length,
        digest: `sha256:${hash(zip)}`,
        expired: false,
      };
      if (kind === 'digest') artifact.digest = `sha256:${'0'.repeat(64)}`;
      if (kind === 'size') artifact.size_in_bytes++;
      if (kind === 'expired') artifact.expired = true;
      const api = createPRRCAPI((file, args, options) =>
        file === 'python3'
          ? execFileSync(file, args, { ...options, stdio: ['pipe', 'pipe', 'pipe'] })
          : zip
      );
      expect(() => api.members(artifact, kind === 'oversize' ? zip.length - 1 : 1024)).toThrow();
    }
  );
  it('fails closed on truncated authenticated inventories and never retries reads', () => {
    let calls = 0;
    const api = createPRRCAPI(() => {
      calls++;
      return Buffer.from(JSON.stringify({ total_count: 101, artifacts: [] }));
    });
    expect(() => api.inventory(8001)).toThrow('incomplete');
    expect(calls).toBe(1);
  });
  it('uses current API opt-in for the runtime guard and refuses another branch before reading', async () => {
    const f = fixture();
    try {
      const output = path.join(f.root, 'output');
      fs.writeFileSync(output, '');
      const env = {
        GITHUB_REPOSITORY: 'pj-tmt/tmt',
        GITHUB_REF: 'refs/heads/main',
        REQUESTED_PR: '1639',
        REQUESTED_HEAD: head,
        REQUESTED_HEAD12: identity.head12,
        GITHUB_RUN_ID: '8001',
        GITHUB_RUN_ATTEMPT: '1',
        GITHUB_OUTPUT: output,
      };
      const reads: string[] = [];
      const o = observation();
      const api = {
        metadata: (route: string) => {
          reads.push(route);
          if (route.startsWith('pulls/')) return o.pull;
          if (route.startsWith('issues/')) return o.timeline;
          return o.run;
        },
        inventory: () => [],
        members: () => [],
      };
      await runPRRCCommand('guard', env, api);
      expect(fs.readFileSync(output, 'utf8')).toBe(`head=${head}\n`);
      expect(reads).toEqual([
        'pulls/1639',
        'issues/1639/timeline?per_page=100',
        'actions/runs/8001/attempts/1',
      ]);
      reads.length = 0;
      await expect(
        runPRRCCommand('guard', { ...env, GITHUB_REF: 'refs/heads/other' }, api)
      ).rejects.toThrow('refs/heads/main');
      expect(reads).toEqual([]);
    } finally {
      f.cleanup();
    }
  });
});
