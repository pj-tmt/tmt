import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { componentMap } from '../../scripts/ci-scope.mjs';
import {
  activeProducts,
  runReleaseRehearsal,
  selectReleaseRehearsal,
} from '../../scripts/release-rehearsal.mjs';

const map = componentMap();
const all = ['cli', 'colab', 'remote', 'squad'];
const select = (...paths: string[]) => selectReleaseRehearsal(paths, map);

describe('active products', () => {
  it('lists released native products from the component map, not parked or private ones', () => {
    expect(activeProducts(map)).toEqual(all);
  });
});

describe('release rehearsal selection', () => {
  it.each([
    'rust/Cargo.lock',
    'rust/Cargo.toml',
    'rust/rust-toolchain.toml',
    'rust/about.toml',
    'rust/licenses/Apache-2.0.txt',
    'dist-workspace.toml',
    'typescript/pnpm-lock.yaml',
    '.github/components.json',
    '.github/release-parity.json',
    '.github/workflows/native-release-prepare.yml',
    '.github/workflows/native-release-bundle.yml',
    '.github/workflows/native-release-upgrade.yml',
    '.github/workflows/native-release-upgrade-prove.yml',
    '.github/workflows/release-rehearsal.yml',
    '.github/actions/inject-release-version/action.yml',
    '.github/actions/setup-tooling/action.yml',
    'scripts/build-native-artifact.sh',
    'scripts/run-native-verification.sh',
    'typescript/scripts/verify-native-notices.mjs',
    'typescript/scripts/release-version-injection.mjs',
    'typescript/scripts/release-upgrade.mjs',
    'typescript/scripts/verify-native-installation.mjs',
    'typescript/scripts/verify-native-extension-upgrade.mjs',
    'rust/crates/tmt-core/Cargo.toml',
  ])('rehearses every active product for the shared release input %s', (changed) => {
    expect(select(changed)).toEqual(all);
  });

  it.each([
    ['extensions/tmt-colab/rust/tmt-colab/Cargo.toml', ['colab']],
    ['extensions/tmt-remote/rust/Cargo.toml', ['remote']],
    ['extensions/tmt-squad/Cargo.toml', ['squad']],
  ])('attributes the product manifest %s to its product only', (changed, products) => {
    expect(select(changed)).toEqual(products);
  });

  it.each([
    // Bundled frontend sources are checked by their own jobs; alone they never select a rehearsal.
    'extensions/tmt-colab/typescript/app/src/main.ts',
    'extensions/tmt-colab/typescript/colab-client/src/index.ts',
    'extensions/tmt-office/typescript/apps/office/src/App.tsx',
    'rust/crates/tmt-cli/src/main.rs',
    'docs/guide.md',
    'site/src/index.md',
    'typescript/test/tooling/ci-scope.test.ts',
    '.github/workflows/release.yml',
  ])('selects nothing for %s', (changed) => {
    expect(select(changed)).toEqual([]);
  });

  it('keeps a frontend edit from hiding a release input in the same change', () => {
    expect(
      select(
        'extensions/tmt-colab/typescript/app/src/main.ts',
        'extensions/tmt-colab/rust/tmt-colab/Cargo.toml'
      )
    ).toEqual(['colab']);
    expect(select('extensions/tmt-colab/typescript/app/src/main.ts', 'rust/Cargo.lock')).toEqual(
      all
    );
  });

  it('stays conservative for an empty change list', () => {
    expect(select()).toEqual(all);
  });
});

describe('selection from a real git diff', () => {
  function diffRepository(changes: string[]) {
    const directory = mkdtempSync(path.join(tmpdir(), 'tmt-rehearsal-select-'));
    const env = { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' };
    const git = (args: string[]) => {
      const result = spawnSync('git', args, { cwd: directory, env, encoding: 'utf8' });
      if (result.status !== 0) throw new Error(result.stderr);
      return result.stdout.trim();
    };
    try {
      git(['init', '-q']);
      const commit = (file: string, text: string) => {
        mkdirSync(path.dirname(path.join(directory, file)), { recursive: true });
        writeFileSync(path.join(directory, file), text);
        git(['add', '.']);
        git(['-c', 'user.name=T', '-c', 'user.email=t@t', 'commit', '-q', '-m', file]);
        return git(['rev-parse', 'HEAD']);
      };
      const base = commit('README.md', 'base');
      let head = base;
      for (const file of changes) head = commit(file, `change ${file}`);
      const out: string[] = [];
      runReleaseRehearsal(['select', base, head], {
        cwd: directory,
        stdout: { write: (text) => out.push(text) },
        stderr: { write: () => true },
      });
      return out.join('');
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }
  it('emits the selection outputs for a shared input, a frontend-only change and a product manifest', () => {
    expect(diffRepository(['rust/Cargo.lock'])).toBe(
      `release_rehearsal=true\nrelease_rehearsal_products=${JSON.stringify(all)}\n`
    );
    expect(diffRepository(['extensions/tmt-colab/typescript/app/src/main.ts'])).toBe(
      'release_rehearsal=false\nrelease_rehearsal_products=[]\n'
    );
    expect(diffRepository(['extensions/tmt-remote/rust/Cargo.toml'])).toBe(
      'release_rehearsal=true\nrelease_rehearsal_products=["remote"]\n'
    );
  });
  it('lists every active product for the nightly run and rejects other arguments', () => {
    const out: string[] = [];
    runReleaseRehearsal(['all'], {
      cwd: '.',
      stdout: { write: (text) => out.push(text) },
      stderr: { write: () => true },
    });
    expect(out.join('')).toBe(`products=${JSON.stringify(all)}\n`);
    for (const args of [[], ['select'], ['select', 'a'], ['other']])
      expect(() =>
        runReleaseRehearsal(args, {
          cwd: '.',
          stdout: { write: () => true },
          stderr: { write: () => true },
        })
      ).toThrow('Usage');
  });
});
