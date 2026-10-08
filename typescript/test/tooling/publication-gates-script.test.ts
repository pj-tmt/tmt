import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vite-plus/test';
import { publishDraft } from '../../scripts/release-publish.mjs';
import { planReleaseBuilds } from '../../scripts/plan-release-builds.mjs';
import type { DraftRelease } from '../../scripts/release-draft-assets.mjs';
import { REQUIRED_CONTEXTS } from '../../scripts/publication-gates.mjs';

import { writeReleaseWorkspace } from '../support/release-workspace-fixture.js';

const script = fileURLToPath(new URL('../../scripts/publication-gates.mjs', import.meta.url));
const repositoryRoot = fileURLToPath(new URL('../../../', import.meta.url));
const MIGRATIONS = 'rust/crates/tmt-adapters/src/storage/migrations.rs';

let root: string;
let predecessorScript: string;
beforeAll(() => {
  root = mkdtempSync(path.join(os.tmpdir(), 'publication-gates-'));
  // Real gate code; only private registry data is synthetic, as in release-predecessor.test.ts.
  const tooling = path.join(root, 'synthetic/typescript');
  cpSync(path.join(repositoryRoot, 'typescript/scripts'), path.join(tooling, 'scripts'), {
    recursive: true,
  });
  symlinkSync(
    path.join(repositoryRoot, 'typescript/node_modules'),
    path.join(tooling, 'node_modules'),
    'dir'
  );
  const policy = path.join(tooling, 'scripts/native-release-policy.mjs');
  const source = readFileSync(policy, 'utf8');
  expect(source.split('const PRODUCTS = {')).toHaveLength(2);
  writeFileSync(
    policy,
    source.replace(
      'const PRODUCTS = {',
      `const PRODUCTS = {
    'fixture-old': { tagPrefix: 'tmt-fixture-old-v', prerelease: true, latest: false, retired: true },
    'fixture-new': { tagPrefix: 'tmt-fixture-new-v', prerelease: true, latest: false },`
    )
  );
  // Node canonicalizes import.meta.url; its CLI guard needs the same path through macOS /var.
  predecessorScript = realpathSync(path.join(tooling, 'scripts/publication-gates.mjs'));
});
afterAll(() => rmSync(root, { recursive: true, force: true }));

/** A fake `gh` that answers from a state file, records every call and keeps every upload. */
const FAKE_GH = `#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const state = JSON.parse(fs.readFileSync(process.env.FAKE_GH_STATE, 'utf8'));
const args = process.argv.slice(2);
fs.appendFileSync(state.calls, JSON.stringify(args) + '\\n');
const joined = args.join(' ');
const out = (value) => process.stdout.write(typeof value === 'string' ? value : JSON.stringify(value));
const method = args.includes('--method') ? args[args.indexOf('--method') + 1] : 'GET';
if (method === 'POST') {
  const url = new URL(args.find((arg) => arg.startsWith('https://uploads')));
  fs.copyFileSync(args[args.indexOf('--input') + 1], path.join(state.uploads, url.searchParams.get('name')));
  out('{}');
} else if (method === 'DELETE') {
  const id = Number(args.find((arg) => arg.includes('/assets/')).split('/assets/')[1]);
  for (const release of state.releases) release.assets = release.assets.filter((asset) => asset.id !== id);
  fs.writeFileSync(process.env.FAKE_GH_STATE, JSON.stringify(state));
  out('{}');
} else if (joined.includes('releases/assets/')) {
  out(state.assetTexts[joined.split('releases/assets/')[1]] ?? '');
} else if (joined.includes('/check-runs')) {
  out([{ check_runs: state.checkRuns }]);
} else if (joined.includes('/pulls')) {
  out(state.pulls);
} else if (joined.includes('/releases')) {
  out([state.releases]);
} else {
  process.stderr.write('unexpected gh call: ' + joined);
  process.exit(9);
}
`;

const list = (count: number) =>
  `const MIGRATIONS: &[Migration] = &[\n${Array.from({ length: count }, (_, index) => `    Migration { name: "m${index}", sql: "" },`).join('\n')}\n];\n`;

const green = REQUIRED_CONTEXTS.map((name) => ({
  name,
  status: 'completed',
  conclusion: 'success',
  completed_at: '2026-09-30T01:00:00Z',
  check_suite: { id: 100 },
  app: { slug: 'github-actions' },
}));

interface Scenario {
  predecessor?: boolean;
  migrations?: number;
  failedCut?: 'breaking' | 'migration';
  subject?: string;
  body?: string;
  checkRuns?: typeof green;
  immutable?: boolean;
  hold?: { gate: string; reason: string; tag?: string; sha?: string } | null;
  draftCommit?: 'candidate' | 'branch';
  published?: boolean;
  draftTag?: string;
}

function scenario(options: Scenario = {}) {
  const directory = mkdtempSync(path.join(root, 'case-'));
  const repo = path.join(directory, 'repo');
  mkdirSync(repo);
  const git = (...args: string[]) => {
    const result = spawnSync('git', args, { cwd: repo, encoding: 'utf8' });
    expect(result.status, `git ${args.join(' ')}: ${result.stderr}`).toBe(0);
    return result.stdout.trim();
  };
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'test@example.test');
  git('config', 'user.name', 'Test');
  mkdirSync(path.join(repo, '.github'));
  mkdirSync(path.join(repo, path.dirname(MIGRATIONS)), { recursive: true });
  copyFileSync(
    path.join(repositoryRoot, '.github/components.json'),
    path.join(repo, '.github/components.json')
  );
  if (options.predecessor)
    writeFileSync(
      path.join(repo, '.github/components.json'),
      JSON.stringify({
        components: {
          'fixture-new': {
            owns: ['rust/lib.rs'],
            package: 'tmt-fixture-new',
            predecessor: 'fixture-old',
            migrations: [MIGRATIONS],
          },
        },
      })
    );
  writeReleaseWorkspace(repo, [
    'remote',
    'colab',
    'driver-herdr',
    ...(options.predecessor ? ['fixture-new'] : []),
  ]);
  const previousTag = options.predecessor ? 'tmt-fixture-old-v0.1.0-alpha.8' : 'v5.0.0-alpha.8';
  const candidateTag =
    options.draftTag ?? (options.predecessor ? 'tmt-fixture-new-v0.1.0-alpha.9' : 'v5.0.0-alpha.9');
  // Exact-ref metadata must use the checkout's installed toolchain, not the host default.
  copyFileSync(
    path.join(repositoryRoot, 'rust/rust-toolchain.toml'),
    path.join(repo, 'rust/rust-toolchain.toml')
  );
  writeFileSync(path.join(repo, MIGRATIONS), list(2));
  writeFileSync(path.join(repo, 'rust/lib.rs'), 'fn a() {}\n');
  git('add', '-A');
  git('commit', '-q', '-m', 'feat: the published release');
  git('tag', previousTag);
  const previous = git('rev-parse', 'HEAD');
  writeFileSync(path.join(repo, 'rust/lib.rs'), 'fn a() { 1; }\n');
  if (options.failedCut === 'migration') writeFileSync(path.join(repo, MIGRATIONS), list(3));
  git(
    'commit',
    '-q',
    '-am',
    options.failedCut === 'breaking' ? 'feat!: remove the old API' : 'fix: a bug',
    '-m',
    'Co-authored-by: Codex <codex@openai.com>'
  );
  const failed = git('rev-parse', 'HEAD');
  writeFileSync(
    path.join(repo, MIGRATIONS),
    list(options.migrations ?? (options.failedCut === 'migration' ? 3 : 2))
  );
  writeFileSync(path.join(repo, 'rust/lib.rs'), 'fn a() { 2; }\n');
  git(
    'commit',
    '-q',
    '-am',
    options.subject ?? 'fix: the candidate',
    ...(options.body ? ['-m', options.body] : [])
  );
  const candidate = git('rev-parse', 'HEAD');

  const assets = (names: string[]) => names.map((name, index) => ({ id: 100 + index, name }));
  const state = {
    calls: path.join(directory, 'calls'),
    uploads: path.join(directory, 'uploads'),
    pulls: [
      {
        number: 7,
        merged_at: '2026-09-30T00:30:00Z',
        merge_commit_sha: candidate,
        head: { sha: 'f'.repeat(40) },
      },
    ],
    checkRuns: options.checkRuns ?? green,
    assetTexts: {
      '150': JSON.stringify({
        tag: candidateTag,
        sha: candidate,
        ...(options.hold ?? { gate: 'migration', reason: 'held earlier' }),
      }),
    } as Record<string, string>,
    releases: [
      {
        id: 1,
        draft: false,
        tag_name: previousTag,
        target_commitish: previous,
        created_at: '2026-09-29T00:00:00Z',
        published_at: '2026-09-29T15:00:00Z',
        immutable: options.immutable ?? true,
        assets: [],
      },
      {
        id: 2,
        draft: options.published !== true,
        tag_name: candidateTag,
        target_commitish: options.draftCommit === 'branch' ? 'main' : candidate,
        created_at: '2026-09-30T00:40:00Z',
        published_at: options.published ? '2026-09-30T02:00:00Z' : null,
        immutable: false,
        assets: assets([
          'release-publication.json',
          ...(options.hold === null ? [] : ['publication-held.json']),
        ]).map((asset) => (asset.name === 'publication-held.json' ? { ...asset, id: 150 } : asset)),
      },
    ],
  };
  if (options.failedCut)
    state.releases.push({
      id: 3,
      draft: true,
      tag_name: 'v5.0.0-alpha.9',
      target_commitish: failed,
      created_at: '2026-09-30T00:35:00Z',
      published_at: null,
      immutable: false,
      assets: assets(['verification-failed.json']),
    });
  mkdirSync(state.uploads);
  writeFileSync(state.calls, '');
  const stateFile = path.join(directory, 'state.json');
  writeFileSync(stateFile, JSON.stringify(state));
  const bin = path.join(directory, 'bin');
  mkdirSync(bin);
  writeExecutable(path.join(bin, 'gh'), FAKE_GH, 0o755);
  const output = path.join(directory, 'output');
  const summary = path.join(directory, 'summary');
  writeFileSync(output, '');
  writeFileSync(summary, '');
  const run = (args: string[], environment: NodeJS.ProcessEnv = process.env) => {
    const entry = options.predecessor ? predecessorScript : script;
    const result = spawnSync(process.execPath, [entry, ...args], {
      cwd: repo,
      encoding: 'utf8',
      env: {
        ...environment,
        // This dependency-free workspace needs no shared cache or caller Cargo configuration.
        CARGO_HOME: path.join(directory, 'cargo'),
        PATH: `${bin}${path.delimiter}${environment.PATH}`,
        FAKE_GH_STATE: stateFile,
        GITHUB_REPOSITORY: 'wkh237/tmt',
        GITHUB_OUTPUT: output,
        GITHUB_STEP_SUMMARY: summary,
        GITHUB_SERVER_URL: 'https://github.test',
        GITHUB_RUN_ID: '4242',
      },
      timeout: 30_000,
    });
    return {
      status: result.status,
      stderr: result.stderr,
      output: readFileSync(output, 'utf8'),
      summary: readFileSync(summary, 'utf8'),
    };
  };
  const calls = () =>
    readFileSync(state.calls, 'utf8')
      .split('\n')
      .filter(Boolean)
      .map((line) => JSON.parse(line) as string[]);
  const uploaded = (name: string) => {
    const file = path.join(state.uploads, name);
    return existsSync(file) ? JSON.parse(readFileSync(file, 'utf8')) : null;
  };
  const readState = () =>
    JSON.parse(readFileSync(stateFile, 'utf8')) as {
      releases: DraftRelease[];
      assetTexts: Record<string, string>;
    };
  const publication = () =>
    publishDraft({
      api: {
        upload: () => {
          throw new Error('Unexpected publication upload');
        },
        deleteAsset: () => {
          throw new Error('Unexpected publication delete');
        },
        listReleases: () => readState().releases,
        latestRelease: () => ({ tag_name: 'v5.0.0-alpha.9' }),
        setLatest: () => {
          throw new Error('Unexpected latest correction in the gate fixture');
        },
        publish: (tag) => {
          const current = readState();
          current.releases = current.releases.map((release) =>
            release.tag_name === tag ? { ...release, draft: false } : release
          );
          writeFileSync(stateFile, JSON.stringify(current));
        },
      },
      product: 'cli',
      tag: 'v5.0.0-alpha.9',
    });
  return { run, calls, uploaded, candidate, readState, publication };
}

const early = ['early', '--product', 'cli', '--tag', 'v5.0.0-alpha.9'];
const finish = (result: string, outcome: string, more: string[] = []) => [
  'finish',
  '--product',
  'cli',
  '--tag',
  'v5.0.0-alpha.9',
  '--upgrade-result',
  result,
  '--upgrade-outcome',
  outcome,
  ...more,
];

describe('publication-gates.mjs early', () => {
  it('holds the first successor publication for a breaking commit after its predecessor', () => {
    const { run, uploaded, candidate } = scenario({
      predecessor: true,
      subject: 'feat!: remove predecessor API',
      hold: null,
    });
    const result = run([
      'early',
      '--product',
      'fixture-new',
      '--tag',
      'tmt-fixture-new-v0.1.0-alpha.9',
    ]);
    expect(result.status, result.stderr).toBe(0);
    expect(result.output).toBe('held=migration\nskip=\n');
    expect(result.summary).toContain('- FAILED `migration`');
    expect(uploaded('publication-held.json')).toMatchObject({
      tag: 'tmt-fixture-new-v0.1.0-alpha.9',
      sha: candidate,
      gate: 'migration',
      reason: `commit ${candidate.slice(0, 8)} is a breaking change: feat!: remove predecessor API`,
    });
  });

  it('passes a draft whose commit, immutability, order and migrations are all in order', () => {
    const { run, uploaded, calls } = scenario({ hold: null });
    const result = run(early);
    expect(result.status, result.stderr).toBe(0);
    expect(result.output).toBe('held=\nskip=\n');
    for (const gate of ['channel', 'commit', 'immutability', 'monotonic', 'migration']) {
      expect(result.summary).toContain(`- passed \`${gate}\``);
    }
    expect(result.summary).toContain('Every gate passed. The next job publishes the release.');
    expect(uploaded('publication-held.json')).toBeNull();
    expect(calls().filter((call) => call.includes('--method'))).toEqual([]);
  });

  it('passes a scope-skipped required check using the same-suite native aggregate from REST', () => {
    const checkRuns = green.map((entry) =>
      entry.name === 'Unit tests' ? { ...entry, conclusion: 'skipped' } : entry
    );
    const { run, uploaded, calls } = scenario({ checkRuns, hold: null });
    const result = run(early);
    expect(result.status, result.stderr).toBe(0);
    expect(result.output).toBe('held=\nskip=\n');
    expect(result.summary).toContain('- passed `commit`');
    expect(uploaded('publication-held.json')).toBeNull();
    expect(calls()).toContainEqual([
      'api',
      '--paginate',
      '--slurp',
      `repos/wkh237/tmt/commits/${'f'.repeat(40)}/check-runs`,
    ]);
  });

  it('resolves captured metadata independently of caller Cargo configuration', () => {
    const callerCargo = mkdtempSync(path.join(root, 'caller-cargo-'));
    writeFileSync(path.join(callerCargo, 'config.toml'), 'invalid caller Cargo configuration');
    const { run, uploaded, calls } = scenario({ hold: null });
    const result = run(early, { ...process.env, CARGO_HOME: callerCargo });
    expect(result.status, result.stderr).toBe(0);
    expect(result.output).toBe('held=\nskip=\n');
    expect(result.summary).toContain('- passed `migration`');
    expect(uploaded('publication-held.json')).toBeNull();
    expect(calls().filter((call) => call.includes('--method'))).toEqual([]);
  });

  it.each(['missing', 'failure', 'cancelled', 'unexpected-skip'])(
    'keeps a durable commit hold for %s required-check evidence',
    (evidence) => {
      const checkRuns = green.flatMap((entry) => {
        if (entry.name === 'Unit tests') {
          if (evidence === 'missing') return [];
          return [{ ...entry, conclusion: evidence === 'unexpected-skip' ? 'skipped' : evidence }];
        }
        return [
          evidence === 'unexpected-skip' && entry.name === 'Native package matrix'
            ? { ...entry, check_suite: { id: 101 } }
            : entry,
        ];
      });
      const { run, uploaded, candidate } = scenario({ checkRuns, hold: null });
      const result = run(early);
      expect(result.status, result.stderr).toBe(0);
      expect(result.output).toBe('held=commit\nskip=\n');
      expect(uploaded('publication-held.json')).toMatchObject({
        tag: 'v5.0.0-alpha.9',
        sha: candidate,
        gate: 'commit',
        reason:
          evidence === 'missing'
            ? 'required check "Unit tests" did not complete on #7'
            : `required check "Unit tests" ${evidence === 'unexpected-skip' ? 'skipped' : evidence} on #7`,
      });
    }
  );

  it.each(['v5.0.0', 'v5.0.0-beta.1', 'v5.0.0-rc.2'])(
    'holds the release %s at the channel gate, without gathering any other evidence',
    (tag) => {
      const { run, uploaded, calls } = scenario({ hold: null, draftTag: tag });
      const result = run(['early', '--product', 'cli', '--tag', tag]);
      expect(result.status, result.stderr).toBe(0);
      expect(result.output).toBe('held=channel\nskip=\n');
      expect(result.summary).toContain('- FAILED `channel`');
      expect(result.summary).toContain('**Held** at `channel`');
      expect(uploaded('publication-held.json')).toMatchObject({ tag, gate: 'channel' });
      expect(uploaded('publication-held.json').reason).toContain('not an alpha release');
      // Nothing after the channel gate ran: no pull request and no check-run lookup.
      expect(calls().filter((call) => call.join(' ').match(/\/pulls|\/check-runs/))).toEqual([]);
    }
  );

  it('never releases a hold of the channel gate, since a release that is not an alpha is published by hand', () => {
    const { run, uploaded } = scenario({
      draftTag: 'v5.0.0',
      hold: { gate: 'channel', reason: 'v5.0.0 is not an alpha release' },
    });
    const result = run(['early', '--product', 'cli', '--tag', 'v5.0.0', '--release-hold']);
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('held by the channel gate');
    expect(result.stderr).toContain('published by hand');
    expect(result.output).toBe('');
    expect(uploaded('publication-held.json')).toBeNull();
  });

  it('holds a draft whose pull request failed a required check, with the marker on the draft', () => {
    const failed = green.map((run) =>
      run.name === 'Unit tests' ? { ...run, conclusion: 'failure' } : run
    );
    const { run, uploaded, candidate } = scenario({ checkRuns: failed, hold: null });
    const result = run(early);
    expect(result.status, result.stderr).toBe(0);
    expect(result.output).toBe('held=commit\nskip=\n');
    expect(result.summary).toContain('**Held** at `commit`');
    const marker = uploaded('publication-held.json');
    expect(marker).toMatchObject({
      tag: 'v5.0.0-alpha.9',
      sha: candidate,
      gate: 'commit',
      reason: 'required check "Unit tests" failure on #7',
      runUrl: 'https://github.test/wkh237/tmt/actions/runs/4242',
    });
    expect(Date.parse(marker.recordedAt)).not.toBeNaN();
  });

  it('publishes an alpha draft that adds migrations and reports the count in the summary', () => {
    const migration = scenario({ migrations: 3, hold: null });
    const result = migration.run(early);
    expect(result.output).toBe('held=\nskip=\n');
    expect(result.summary).toContain(`${MIGRATIONS} has 3 migrations, 2 in v5.0.0-alpha.8`);
    expect(migration.uploaded('publication-held.json')).toBeNull();
  });

  it('keeps an unpublished failed draft out of the breaking-change boundary', () => {
    const fixture = scenario({ failedCut: 'breaking', draftTag: 'v5.0.0-alpha.10', hold: null });
    const result = fixture.run(['early', '--product', 'cli', '--tag', 'v5.0.0-alpha.10']);
    expect(result.status, result.stderr).toBe(0);
    expect(result.output).toBe('held=migration\nskip=\n');
    expect(fixture.uploaded('publication-held.json')?.reason).toContain(
      'feat!: remove the old API'
    );
    expect(
      fixture.readState().releases.find((release) => release.tag_name === 'v5.0.0-alpha.9')?.draft
    ).toBe(true);
  });

  it('counts migrations from the published ancestor despite a newer failed draft', () => {
    const fixture = scenario({ failedCut: 'migration', draftTag: 'v5.0.0-alpha.10', hold: null });
    const result = fixture.run(['early', '--product', 'cli', '--tag', 'v5.0.0-alpha.10']);
    expect(result.status, result.stderr).toBe(0);
    expect(result.output).toBe('held=\nskip=\n');
    expect(result.summary).toContain(`${MIGRATIONS} has 3 migrations, 2 in v5.0.0-alpha.8`);
    expect(fixture.uploaded('publication-held.json')).toBeNull();
  });

  it('holds a draft that carries a breaking commit, with or without a new migration', () => {
    const both = scenario({ migrations: 3, subject: 'feat!: drop a table', hold: null });
    expect(both.run(early).output).toBe('held=migration\nskip=\n');
    expect(both.uploaded('publication-held.json')?.reason).toContain('is a breaking change');
    const breaking = scenario({ subject: 'feat(api)!: drop a flag', hold: null });
    expect(breaking.run(early).output).toBe('held=migration\nskip=\n');
    expect(breaking.uploaded('publication-held.json')?.reason).toContain('is a breaking change');
    const footer = scenario({ body: 'BREAKING CHANGE: it is gone', hold: null });
    expect(footer.run(early).output).toBe('held=migration\nskip=\n');
  });

  it('holds when the newest published release is not immutable', () => {
    const { run, uploaded } = scenario({ immutable: false, hold: null });
    expect(run(early).output).toBe('held=immutability\nskip=\n');
    expect(uploaded('publication-held.json')?.reason).toContain('v5.0.0-alpha.8 is not immutable');
  });

  it('holds a draft that points at a branch, so no commit can be checked', () => {
    const { run } = scenario({ draftCommit: 'branch', hold: null });
    const result = run(early);
    expect(result.status, result.stderr).toBe(0);
    expect(result.output).toBe('held=commit\nskip=\n');
  });

  it('refuses a published release and a missing one', () => {
    expect(scenario({ published: true, hold: null }).run(early).stderr).toContain(
      'already published'
    );
    const { run } = scenario({ hold: null });
    expect(run(['early', '--product', 'cli', '--tag', 'v9.9.9']).stderr).toContain(
      'There is no release v9.9.9.'
    );
    expect(run(['bogus', '--product', 'cli', '--tag', 'v5.0.0-alpha.9']).stderr).toContain(
      'Usage:'
    );
  });
});

describe('publication-gates.mjs dry', () => {
  const dry = (sha: string, more: string[] = []) => [
    'dry',
    '--product',
    'cli',
    '--tag',
    'v5.0.0-alpha.999999',
    '--sha',
    sha,
    ...more,
  ];

  it('evaluates the gates for a synthetic candidate without a commit gate or any write', () => {
    const { run, uploaded, calls, candidate } = scenario({ hold: null });
    const result = run(dry(candidate));
    expect(result.status, result.stderr).toBe(0);
    for (const gate of ['channel', 'immutability', 'monotonic', 'migration']) {
      expect(result.summary).toContain(`- passed \`${gate}\``);
    }
    expect(result.summary).not.toContain('`commit`');
    expect(result.summary).toContain('(dry run)');
    expect(result.summary).toContain('nothing was recorded or published');
    expect(result.output).toBe('');
    expect(uploaded('publication-held.json')).toBeNull();
    expect(calls().filter((call) => call.includes('--method'))).toEqual([]);
  });

  it('runs the commit gate on main, where the candidate is a merged commit', () => {
    const { run, candidate } = scenario({ hold: null });
    const result = run(dry(candidate, ['--on-main']));
    expect(result.status, result.stderr).toBe(0);
    expect(result.summary).toContain('- passed `commit`');
  });

  it('fails the run for a breaking change, naming the gate, and records no hold', () => {
    const { run, uploaded, calls, candidate } = scenario({ failedCut: 'breaking', hold: null });
    const result = run(dry(candidate));
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('would hold v5.0.0-alpha.999999 at migration');
    expect(result.summary).toContain('**Would hold** at `migration`');
    expect(uploaded('publication-held.json')).toBeNull();
    expect(calls().filter((call) => call.includes('--method'))).toEqual([]);
  });

  it('holds when the newest published release is not immutable', () => {
    const { run, candidate } = scenario({ immutable: false, hold: null });
    const result = run(dry(candidate));
    expect(result.status).toBe(1);
    expect(result.summary).toContain('**Would hold** at `immutability`');
  });

  it('requires the candidate commit', () => {
    const { run } = scenario({ hold: null });
    const result = run(dry('').slice(0, 5));
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('--sha is required');
  });
});

describe('publication-gates.mjs early --release-hold', () => {
  it('skips exactly the gate the marker names and runs the others', () => {
    const { run, uploaded } = scenario({
      subject: 'feat!: drop a flag',
      hold: { gate: 'migration', reason: 'held earlier' },
    });
    const result = run([...early, '--release-hold']);
    expect(result.status, result.stderr).toBe(0);
    expect(result.output).toBe('held=\nskip=migration\n');
    expect(result.summary).toContain('- skipped `migration`');
    expect(uploaded('publication-held.json')).toBeNull();
  });

  it('never skips a different gate: another failure replaces the marker', () => {
    const { run, uploaded } = scenario({
      subject: 'feat!: drop a flag',
      immutable: false,
      hold: { gate: 'migration', reason: 'held earlier' },
    });
    const result = run([...early, '--release-hold']);
    expect(result.output).toBe('held=immutability\nskip=migration\n');
    expect(uploaded('publication-held.json')?.gate).toBe('immutability');
  });

  it('reports the upgrade gate as the one to skip, without running a gate of its own', () => {
    const { run } = scenario({ hold: { gate: 'upgrade', reason: 'predates' } });
    expect(run([...early, '--release-hold']).output).toBe('held=\nskip=upgrade\n');
  });

  it('refuses a draft that carries no hold marker', () => {
    const { run } = scenario({ hold: null });
    const result = run([...early, '--release-hold']);
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('carries no publication-held.json to release');
  });
});

describe('publication-gates.mjs finish', () => {
  it('reports that every gate passed when the upgrade was proved, and removes no marker that is not there', () => {
    const { run, uploaded, calls } = scenario({ hold: null });
    const result = run(finish('success', 'proved'));
    expect(result.output).toBe('held=\n');
    expect(result.summary).toContain('- passed `upgrade`');
    expect(uploaded('publication-held.json')).toBeNull();
    expect(calls().filter((call) => call.includes('DELETE'))).toEqual([]);
  });

  it('passes a first release that has nothing to upgrade from', () => {
    const { run } = scenario({ hold: null });
    const result = run(finish('success', 'nothing'));
    expect(result.output).toBe('held=\n');
    expect(result.summary).toContain('nothing to upgrade from');
  });

  it('holds a draft whose commit predates the proof with the reason verbatim', () => {
    const reason =
      "This release's commit has no release-upgrade.mjs: it predates the automated upgrade proof, so prove the upgrade by hand, as the native release verification guide describes.";
    const { run, uploaded } = scenario({ hold: null });
    const result = run(finish('failure', 'predates', ['--upgrade-reason', reason]));
    expect(result.output).toBe('held=upgrade\n');
    expect(uploaded('publication-held.json')).toMatchObject({ gate: 'upgrade', reason });
    expect(result.summary).toContain(reason);
  });

  it('holds a draft whose proof failed, naming the run', () => {
    const { run, uploaded } = scenario({ hold: null });
    expect(run(finish('failure', '')).output).toBe('held=upgrade\n');
    expect(uploaded('publication-held.json')?.reason).toBe(
      'the upgrade proof failure: https://github.test/wkh237/tmt/actions/runs/4242'
    );
  });

  it('removes the marker once a released hold passes, and never runs the proof it skipped', () => {
    const { run, uploaded, calls } = scenario({ hold: { gate: 'upgrade', reason: 'predates' } });
    const result = run(finish('skipped', '', ['--skip', 'upgrade']));
    expect(result.output).toBe('held=\n');
    expect(result.summary).toContain('- skipped `upgrade`');
    expect(uploaded('publication-held.json')).toBeNull();
    expect(
      calls().some(
        (call) => call.includes('DELETE') && call.some((arg) => arg.endsWith('/assets/150'))
      )
    ).toBe(true);
  });
});

// Planner selection and gate effects are separate evidence: a planned rerun cannot publish
// until early and upgrade proof pass, and failure must not rewrite the original marker.
describe('held-draft rerun publication decisions', () => {
  it('reruns every early gate, then clears the marker only after upgrade passes', () => {
    const { run, calls, readState, publication } = scenario({
      hold: { gate: 'upgrade', reason: 'old tooling failed' },
    });
    expect(
      planReleaseBuilds({ releases: readState().releases, product: 'cli', rerun: 'v5.0.0-alpha.9' })
        .builds
    ).toHaveLength(1);
    expect(publication).toThrow('is held');
    const planned = run([...early, '--rerun']);
    expect(planned.status).toBe(0);
    expect(planned.output).toContain('held=\nskip=\nrerun_gate=upgrade\n');
    for (const gate of ['channel', 'commit', 'immutability', 'monotonic', 'migration']) {
      expect(planned.summary).toContain(`- passed \`${gate}\``);
    }
    expect(calls().filter((call) => call.includes('--method'))).toEqual([]);
    const result = run(finish('success', 'proved', ['--rerun-gate', 'upgrade']));
    expect(result.status, result.stderr).toBe(0);
    expect(result.summary).toContain('Every gate passed. The next job publishes the release.');
    expect(calls().filter((call) => call.includes('DELETE'))).toHaveLength(1);
    expect(publication().flags).toContain('--latest=false');
    expect(
      readState().releases.find((release) => release.tag_name === 'v5.0.0-alpha.9')?.draft
    ).toBe(false);
  });

  it.each(['failure', 'cancelled', 'skipped'])(
    'keeps the original marker when upgrade is %s',
    (result) => {
      const { run, calls, uploaded, readState, publication } = scenario({
        hold: { gate: 'upgrade', reason: 'original cause' },
      });
      const marker = readState().assetTexts['150'];
      const planned = run([...early, '--rerun']);
      expect(planned.status, planned.stderr).toBe(0);
      const decision = run(finish(result, 'proved', ['--rerun-gate', 'upgrade']));
      expect(decision.status).toBe(0);
      expect(decision.output).toContain('held=upgrade');
      expect(calls().filter((call) => call.includes('--method'))).toEqual([]);
      expect(uploaded('publication-held.json')).toBeNull();
      expect(readState().assetTexts['150']).toBe(marker);
      expect(publication).toThrow('is held');
    }
  );

  it('keeps the original marker when an early gate still fails', () => {
    const { run, calls } = scenario({
      subject: 'feat!: still breaking',
      hold: { gate: 'migration', reason: 'original cause' },
    });
    const result = run([...early, '--rerun']);
    expect(result.output).toContain('held=migration');
    expect(result.summary).toContain('- FAILED `migration`');
    expect(calls().filter((call) => call.includes('--method'))).toEqual([]);
  });

  it.each([
    { gate: 'unknown', reason: 'invalid gate' },
    { gate: 'upgrade', reason: 'wrong tag', tag: 'v5.0.0-alpha.8' },
    { gate: 'upgrade', reason: 'wrong SHA', sha: 'a'.repeat(40) },
  ])('refuses mismatching marker %o without mutations', (hold) => {
    const { run, calls } = scenario({ hold });
    expect(run([...early, '--rerun']).status).toBe(1);
    expect(calls().filter((call) => call.includes('--method'))).toEqual([]);
  });

  it('refuses a non-held draft and a finish gate mismatch or skip', () => {
    expect(scenario({ hold: null }).run([...early, '--rerun']).status).toBe(1);
    const { run, calls } = scenario({ hold: { gate: 'upgrade', reason: 'original' } });
    expect(run(finish('success', 'proved', ['--rerun-gate', 'migration'])).status).toBe(1);
    expect(
      run(finish('success', 'proved', ['--rerun-gate', 'upgrade', '--skip', 'upgrade'])).status
    ).toBe(1);
    expect(calls().filter((call) => call.includes('--method'))).toEqual([]);
    expect(run([...early, '--rerun', '--release-hold']).status).toBe(1);
  });
});
