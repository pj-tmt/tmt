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
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { readCargoWorkspace } from '../../scripts/cargo-workspace.mjs';
import {
  parseComponentMap,
  releasedComponentsForPath,
  selectCiAreas,
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
const source = readFileSync(`${root}.github/components.json`, 'utf8');
const map = parseComponentMap(source);
const workspace = readCargoWorkspace(root);
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
  delete before.components['tmt-colab'].package;
  const paths = [
    office('model'),
    office('storage'),
    'extensions/tmt-colab/typescript/app/src/main.ts',
  ];
  expect(selectCiAreas(paths, map)).toEqual(
    selectCiAreas(paths, parseComponentMap(JSON.stringify(before)))
  );
});
it('accepts future packaged consumers, rejecting invalid marker combinations', () => {
  expect(
    affectedProducts(['extensions/tmt-colab/typescript/app/src/main.ts'], map, workspace)
  ).toEqual({ products: [], unpublished: ['tmt-colab'] });
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

describe('explicit private delivery status', () => {
  const item = { content: { id: 'issue' } } as ProjectItem;
  const closing: Closing = {
    prs: new Map([['pr', { number: 1, mergeCommit: { oid: sha } } as ClosingPr]]),
    issues: new Map([['issue', new Set(['pr'])]]),
  };
  const evidence = (paths: string[]) =>
    deriveEvidence(
      [item],
      closing,
      [],
      { validateTags: () => {}, paths: () => paths, containingTags: () => new Set() },
      map,
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
  ])('keeps not-yet-activated products awaiting: %s', (path) => {
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
      if (missingCache) Object.assign(env, { CARGO_HOME: join(directory, 'empty-cargo-home') });
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
