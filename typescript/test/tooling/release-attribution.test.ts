import {
  cpSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { readCargoWorkspace } from '../../scripts/cargo-workspace.mjs';
import {
  parseComponentMap,
  releasedComponentsForPath,
  selectCiAreas,
  ownerOf,
  selectNativeScope,
} from '../../scripts/ci-scope.mjs';
import {
  affectedProducts,
  deriveEvidence,
  type ProjectItem,
  type Closing,
  type ClosingPr,
} from '../../scripts/project-release.mjs';
import { attributeCutCommits } from '../../scripts/release-cut.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const { productOfComponent } = (await import(
  pathToFileURL(`${root}typescript/scripts/native-release-policy.mjs`).href
)) as { productOfComponent: (name: string) => string };
const source = readFileSync(`${root}.github/components.json`, 'utf8');
const map = parseComponentMap(source);
const workspace = readCargoWorkspace(root);

describe('Project-only never-shipped declarations', () => {
  it('preserves ownership, CI scope and cuts for the current map', () => {
    const original = JSON.parse(source);
    for (const component of Object.values(original.components) as Record<string, unknown>[]) {
      delete component.neverShippedPaths;
      delete component.generatedInputs;
    }
    const base = parseComponentMap(JSON.stringify(original));
    const files = runGitFiles();
    const paths = [...files, 'unmapped/input', 'extensions/tmt-colab/rust/tmt-colab/new-input'];
    for (const path of paths) {
      expect(ownerOf(path, map), path).toBe(ownerOf(path, base));
      expect(selectCiAreas([path], map), path).toEqual(selectCiAreas([path], base));
      expect(selectNativeScope([path], map), path).toBe(selectNativeScope([path], base));
    }
    const commits = [{ sha: 'a'.repeat(40), message: 'fix: changed input', files: paths }];
    for (const component of map.components.filter(
      (component) => component.package && component.release !== false
    )) {
      expect(
        attributeCutCommits(commits, map, productOfComponent(component.name), workspace)
      ).toEqual(attributeCutCommits(commits, base, productOfComponent(component.name), workspace));
    }
  });

  it.each([
    ['empty root', { root: '' }],
    ['absolute root', { root: '/tmp/tests' }],
    ['parent traversal', { root: 'rust/../tests' }],
    ['glob', { root: 'rust/**' }],
    ['backslash', { root: 'rust\\tests' }],
    ['empty reason', { reason: ' ' }],
  ])('refuses %s', (_label, patch) => {
    const invalid = JSON.parse(source);
    Object.assign(invalid.components.cli.neverShippedPaths[0], patch);
    expect(() => parseComponentMap(JSON.stringify(invalid))).toThrow();
  });

  it('keeps actual skills, build, app, dependency and undeclared inputs attributed', () => {
    for (const path of [
      'extensions/tmt-colab/skills/tmt-colab/SKILL.md',
      'extensions/tmt-colab/skills/tmt-colab/references/future.md',
      'extensions/tmt-colab/rust/tmt-colab/build.rs',
      'extensions/tmt-colab/rust/tmt-colab/Cargo.toml',
      'extensions/tmt-colab/rust/tmt-colab/src/main.rs',
      'extensions/tmt-colab/typescript/app/src/main.ts',
      'extensions/tmt-colab/typescript/colab-client/src/index.ts',
      'extensions/tmt-colab/rust/tmt-colab-model/src/lib.rs',
    ])
      expect(affectedProducts([path], map, workspace).products, path).toContain('colab');
    expect(
      affectedProducts(['rust/crates/tmt-invoke/src/lib.rs'], map, workspace).products
    ).toContain('colab');
  });
});

function runGitFiles(): string[] {
  const result = spawnSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8' });
  expect(result.status, result.stderr).toBe(0);
  return result.stdout.split('\0').filter(Boolean);
}
const sha = 'a'.repeat(40);
const office = (name: string) => `extensions/tmt-office/rust/tmt-office-${name}/src/lib.rs`;

it.each(['model', 'command', 'service'])(
  'attributes the linked Office %s crate to CLI in both cut and project sweep',
  (name) => {
    const path = office(name);
    expect(releasedComponentsForPath(path, map, workspace).map((c) => c.name)).toContain('cli');
    expect(affectedProducts([path], map, workspace).products).toEqual(['cli', 'office']);
    expect(
      attributeCutCommits(
        [{ sha, message: 'fix: linked Office crate', files: [path] }],
        map,
        'cli',
        workspace
      )
    ).toHaveLength(1);
  }
);
it.each([
  office('storage'),
  office('pairing'),
  'extensions/tmt-office/rust/tmt-office/src/main.rs',
  'rust/crates/tmt-test-support/src/lib.rs',
])('does not attribute runtime-only or dev-only path %s to CLI', (path) => {
  expect(releasedComponentsForPath(path, map, workspace).map((c) => c.name)).not.toContain('cli');
  expect(
    attributeCutCommits(
      [{ sha, message: 'fix: private change', files: [path] }],
      map,
      'cli',
      workspace
    )
  ).toEqual([]);
});
it('preserves Herdr attribution and CI selection', () => {
  expect(
    affectedProducts(['rust/crates/tmt-driver-herdr/src/lib.rs'], map, workspace).products
  ).toEqual(['driver-herdr']);
  const before = JSON.parse(source);
  delete before.components.office.releaseStatus;
  delete before.components['tmt-test-support'].releaseStatus;
  delete before.components['colab-app'].releaseConsumers;
  delete before.components['colab-client'].releaseConsumers;
  // Release ownership must not change CI selection, including the pre-activation map.
  before.components['tmt-colab'].release = false;
  before.components['tmt-remote'].release = false;
  const paths = [
    office('model'),
    office('storage'),
    'extensions/tmt-colab/typescript/app/src/main.ts',
  ];
  expect(selectCiAreas(paths, map)).toEqual(
    selectCiAreas(paths, parseComponentMap(JSON.stringify(before)))
  );
});
it('attributes embedded app changes to Colab, rejecting invalid marker combinations', () => {
  expect(
    affectedProducts(['extensions/tmt-colab/typescript/app/src/main.ts'], map, workspace)
  ).toEqual({ products: ['colab'], unpublished: [] });
  for (const patch of [
    { releaseStatus: 'unknown' },
    { release: true, releaseStatus: 'never' },
    { releaseStatus: 'never', releaseConsumers: ['cli'] },
  ]) {
    const value = JSON.parse(source);
    Object.assign(value.components['tmt-test-support'], patch);
    expect(() => parseComponentMap(JSON.stringify(value))).toThrow();
  }
});

it('attributes shared token changes to CLI and the embedded Colab consumer', () => {
  const path = 'design/tokens/tokens.json';
  expect(affectedProducts([path], map, workspace)).toEqual({
    products: ['cli', 'colab'],
    unpublished: [],
  });
  for (const product of ['cli', 'colab']) {
    expect(
      attributeCutCommits(
        [{ sha, message: 'fix: shared tokens', files: [path] }],
        map,
        product,
        workspace
      )
    ).toHaveLength(1);
  }
  expect(affectedProducts(['design/tokens-other/tokens.json'], map, workspace).products).toEqual([
    'cli',
  ]);
});

describe('explicit private delivery status', () => {
  const item = { content: { id: 'issue' } } as ProjectItem;
  const closing: Closing = {
    prs: new Map([['pr', { number: 1, mergeCommit: { oid: sha } } as ClosingPr]]),
    issues: new Map([['issue', new Set(['pr'])]]),
  };
  const evidence = (paths: string[], componentMap = map) =>
    deriveEvidence(
      [item],
      closing,
      [],
      { validateTags: () => {}, paths: () => paths, containingTags: () => new Set() },
      componentMap,
      workspace
    ).get('issue');
  it('marks an explicitly never-shipped leaf Done with no release evidence', () => {
    expect(evidence(['rust/crates/tmt-test-support/src/lib.rs'])).toMatchObject({
      status: 'Done',
      text: '',
      waiting: [],
    });
  });
  it('marks Office-only waits Done with the agreed note', () => {
    expect(evidence([office('storage')])).toMatchObject({
      status: 'Done',
      text: 'ships with the first Office release',
      waiting: ['Awaiting office'],
    });
  });
  it('names each parked product actually waited on without adding unrelated parked products', () => {
    const value = JSON.parse(source);
    value.components['tmt-remote'].release = false;
    value.components['tmt-remote'].releaseStatus = 'parked';
    const parkedMap = parseComponentMap(JSON.stringify(value));
    const remote = 'extensions/tmt-remote/rust/tmt-remote/src/main.rs';
    expect(evidence([remote], parkedMap)).toMatchObject({
      status: 'Done',
      text: 'ships with the first Remote release',
      waiting: ['Awaiting remote'],
    });
    expect(evidence([office('storage')], parkedMap)?.text).toBe(
      'ships with the first Office release'
    );
    expect(evidence([remote, office('storage')], parkedMap)).toMatchObject({
      status: 'Done',
      text: 'ships with the first Office release\nships with the first Remote release',
      waiting: ['Awaiting office', 'Awaiting remote'],
    });
    expect(evidence([remote, office('model')], parkedMap)).toMatchObject({
      status: 'Merged',
      text: '',
    });
  });
  it('retains published CLI evidence when Office is the sole remaining wait', () => {
    const row = deriveEvidence(
      [item],
      closing,
      [
        {
          tag_name: 'v5.0.0-alpha.1',
          draft: false,
          published_at: '2026-10-03T00:00:00Z',
          body: '',
        },
      ],
      {
        validateTags: () => {},
        paths: () => [office('model')],
        containingTags: () => new Set(['v5.0.0-alpha.1']),
      },
      map,
      workspace
    ).get('issue');
    expect(row).toMatchObject({
      status: 'Done',
      text: 'tmt-cli 5.0.0-alpha.1\nships with the first Office release',
      waiting: ['Awaiting office'],
    });
  });
  it('keeps Office plus CLI awaiting Merged', () => {
    expect(evidence([office('model')])).toMatchObject({
      status: 'Merged',
      text: '',
      waiting: ['Awaiting cli', 'Awaiting office'],
    });
  });
  it.each([
    'extensions/tmt-colab/rust/tmt-colab/src/main.rs',
    'extensions/tmt-remote/rust/tmt-remote/src/main.rs',
  ])('keeps activated products Merged until their containing release publishes: %s', (path) => {
    expect(evidence([path])?.status).toBe('Merged');
  });
  it('does not turn missing path evidence into Done', () => {
    expect(evidence([])?.status).toBe('Merged');
  });
});

// Exercise the actual cut entry point: Cargo reads an exported ref, and both successful
// planning and offline acquisition failure remove the exported checkout.
it.each([false, true])(
  'cleans the immutable cut checkout (offline cache unavailable: %s)',
  (missingCache) => {
    const directory = realpathSync(mkdtempSync(join(tmpdir(), 'tmt-cut-entry-')));
    try {
      const checkout = join(directory, 'checkout');
      mkdirSync(join(checkout, 'rust/src'), { recursive: true });
      mkdirSync(join(checkout, '.github'));
      // Use the actual entry point and imports in a small independent checkout.
      cpSync(join(root, 'typescript/scripts'), join(checkout, 'typescript/scripts'), {
        recursive: true,
      });
      symlinkSync(
        join(root, 'typescript/node_modules'),
        join(checkout, 'typescript/node_modules'),
        'dir'
      );
      cpSync(join(root, 'rust/rust-toolchain.toml'), join(checkout, 'rust/rust-toolchain.toml'));
      writeFileSync(
        join(checkout, '.github/components.json'),
        JSON.stringify({
          components: { cli: { package: 'tmt-cli', release: true, owns: ['rust/**'] } },
        })
      );
      writeFileSync(
        join(checkout, 'rust/Cargo.toml'),
        '[package]\nname = "tmt-cli"\nversion = "0.1.0"\nedition = "2024"\n[dependencies]\nlibc = "0.2"\n'
      );
      writeFileSync(join(checkout, 'rust/src/lib.rs'), 'pub fn fixture() {}\n');
      const locked = spawnSync('cargo', ['generate-lockfile', '--offline'], {
        cwd: join(checkout, 'rust'),
        encoding: 'utf8',
        timeout: 30_000,
      });
      expect(locked.status, locked.stderr).toBe(0);
      const git = (args: string[]) => {
        const result = spawnSync('git', args, { cwd: checkout, encoding: 'utf8', timeout: 10_000 });
        expect(result.status, result.stderr).toBe(0);
        return result.stdout.trim();
      };
      git(['init', '--quiet']);
      git(['add', 'rust', '.github/components.json']);
      git([
        '-c',
        'user.name=TMT Test',
        '-c',
        'user.email=test@example.invalid',
        '-c',
        'commit.gpgsign=false',
        'commit',
        '--quiet',
        '-m',
        'fixture\n\nCo-authored-by: Codex <codex@openai.com>',
      ]);
      const cut = git(['rev-parse', 'HEAD']);
      git(['update-ref', 'refs/remotes/origin/main', cut]);
      // Reading the running checkout instead of the captured commit would fail.
      writeFileSync(join(checkout, 'rust/Cargo.toml'), 'invalid current checkout manifest\n');
      const snapshot = join(directory, 'metadata.json');
      writeFileSync(
        snapshot,
        JSON.stringify({
          schema: 1,
          repository: 'pj-tmt/tmt',
          cut,
          draftVisibility: 'trusted',
          releases: [],
          runs: [],
        })
      );
      const temporary = join(directory, 'temporary');
      mkdirSync(temporary);
      // Resolve Apple's Git shim before changing TMPDIR: xcrun cache initialization
      // otherwise emits unrelated diagnostics inside the strict command runner.
      const selectedGit =
        process.platform === 'darwin'
          ? spawnSync('xcrun', ['--find', 'git'], { encoding: 'utf8', timeout: 30_000 })
          : undefined;
      if (selectedGit) expect(selectedGit.status).toBe(0);
      const env = {
        ...process.env,
        TMPDIR: temporary,
        PATH: selectedGit
          ? `${dirname(selectedGit.stdout.trim())}:${process.env.PATH}`
          : process.env.PATH,
      };
      if (missingCache)
        Object.assign(env, {
          CARGO_HOME: join(directory, 'empty-cargo-home'),
          CARGO_NET_OFFLINE: 'true',
        });
      const result = spawnSync(
        process.execPath,
        [join(checkout, 'typescript/scripts/release-cut.mjs'), snapshot],
        { cwd: checkout, env, encoding: 'utf8', timeout: 60_000, maxBuffer: 2 * 1024 * 1024 }
      );
      if (missingCache) {
        expect(result.status).toBe(1);
        expect(result.stderr).toContain('offline');
      } else {
        expect(result.status, result.stderr).toBe(0);
        expect(JSON.parse(result.stdout).cut).toBe(cut);
      }
      expect(readdirSync(temporary)).toEqual([]);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  },
  70_000
);
