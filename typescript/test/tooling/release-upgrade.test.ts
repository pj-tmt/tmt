import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { DraftAsset, DraftRelease } from '../../scripts/release-draft-assets.mjs';
import {
  ghAssetDownloader,
  proveUpgrade,
  releaseCommit,
  selectAssets,
  selectPrevious,
  stageRelease,
} from '../../scripts/release-upgrade.mjs';
import {
  compareVersions,
  publishedReleases,
  versionOfTag,
} from '../../scripts/release-versions.mjs';

const script = fileURLToPath(new URL('../../scripts/release-upgrade.mjs', import.meta.url));
const TARGET = 'aarch64-apple-darwin';
const COMMIT = 'a'.repeat(40);

let root: string;
beforeAll(() => {
  root = mkdtempSync(path.join(os.tmpdir(), 'release-upgrade-'));
});
afterAll(() => rmSync(root, { recursive: true, force: true }));

const digestOf = (text: string) => `sha256:${createHash('sha256').update(text).digest('hex')}`;
const prefixOf = (product: string) => (product === 'cli' ? 'tmt-cli' : `tmt-${product}`);

/** The content of every asset by id, so a download can be faked and its digest computed. */
const contents = new Map<number, string>();
let nextId = 1;

function release(
  tag: string,
  {
    draft = false,
    sha = COMMIT,
    digests = true,
  }: { draft?: boolean; sha?: string; digests?: boolean } = {}
): DraftRelease {
  const product = tag.startsWith('tmt-office-v')
    ? 'office'
    : tag.startsWith('tmt-squad-v')
      ? 'squad'
      : 'cli';
  const asset = (name: string): DraftAsset => {
    const id = nextId++;
    const text = `${tag}:${name}`;
    contents.set(id, text);
    return { id, name, ...(digests ? { digest: digestOf(text) } : {}) };
  };
  return {
    id: nextId++,
    draft,
    tag_name: tag,
    target_commitish: sha,
    created_at: '2026-09-30T00:00:00Z',
    assets: [asset(`${prefixOf(product)}-${TARGET}.tar.gz`), asset('dist-manifest.json')],
  };
}

describe('versions', () => {
  it('orders versions by Semantic Versioning precedence', () => {
    const ordered = [
      '0.1.0-alpha.2',
      '0.1.0-alpha.3',
      '0.1.0-alpha.10',
      '0.1.0-alpha.beta',
      '0.1.0-beta',
      '0.1.0',
      '0.1.1',
      '4.10.9',
      '5.0.0-alpha.1',
      '5.0.0-alpha.1.1',
      '5.0.0-alpha.9',
      '5.0.0-alpha.10',
      '5.0.0',
    ];
    for (let index = 1; index < ordered.length; index += 1) {
      expect(compareVersions(ordered[index - 1], ordered[index]), ordered[index]).toBe(-1);
      expect(compareVersions(ordered[index], ordered[index - 1]), ordered[index]).toBe(1);
    }
    expect(compareVersions('5.0.0-alpha.9', '5.0.0-alpha.9')).toBe(0);
    expect(() => compareVersions('5.0', '5.0.0')).toThrow('5.0 is not a version.');
  });

  it("names a tag's version by its product and refuses another product's tag", () => {
    expect(versionOfTag('v5.0.0-alpha.9', 'cli')).toBe('5.0.0-alpha.9');
    expect(versionOfTag('tmt-office-v0.1.0-alpha.4', 'office')).toBe('0.1.0-alpha.4');
    expect(versionOfTag('tmt-squad-v0.1.0-alpha.2', 'squad')).toBe('0.1.0-alpha.2');
    expect(() => versionOfTag('tmt-office-v0.1.0', 'cli')).toThrow('not a cli tag');
  });

  it('lists the published releases of one product, newest version first', () => {
    const releases = [
      release('v5.0.0-alpha.7'),
      release('v5.0.0-alpha.10', { draft: true }),
      release('v5.0.0-alpha.8'),
      release('tmt-office-v0.1.0-alpha.3'),
      release('v5.0.0-alpha.9'),
    ];
    expect(publishedReleases(releases, 'cli').map((entry) => entry.tag_name)).toEqual([
      'v5.0.0-alpha.9',
      'v5.0.0-alpha.8',
      'v5.0.0-alpha.7',
    ]);
    expect(publishedReleases(releases, 'squad')).toEqual([]);
  });
});

describe('selectPrevious', () => {
  const releases = [
    release('v5.0.0-alpha.7'),
    release('v5.0.0-alpha.8'),
    release('v5.0.0-alpha.9', { draft: true }),
    release('v5.0.0-alpha.10', { draft: true }),
    release('tmt-office-v0.1.0-alpha.3'),
    release('tmt-office-v0.1.0-alpha.4', { draft: true }),
    release('tmt-squad-v0.1.0-alpha.1'),
  ];
  const previous = (product: string, candidateTag: string) =>
    selectPrevious({ releases, product, candidateTag })?.tag_name ?? null;

  it('is the newest published release of the same product below the candidate', () => {
    expect(previous('cli', 'v5.0.0-alpha.9')).toBe('v5.0.0-alpha.8');
    expect(previous('cli', 'v5.0.0-alpha.10')).toBe('v5.0.0-alpha.8');
    expect(previous('office', 'tmt-office-v0.1.0-alpha.4')).toBe('tmt-office-v0.1.0-alpha.3');
  });

  it('never picks a draft, the candidate itself, a higher version or another product', () => {
    expect(previous('cli', 'v5.0.0-alpha.8')).toBe('v5.0.0-alpha.7');
    expect(previous('cli', 'v5.0.0-alpha.7')).toBeNull();
    expect(previous('cli', 'v4.9.0')).toBeNull();
    expect(previous('squad', 'tmt-squad-v0.1.0-alpha.2')).toBe('tmt-squad-v0.1.0-alpha.1');
    expect(previous('squad', 'tmt-squad-v0.1.0-alpha.1')).toBeNull();
  });

  it('refuses a candidate tag of another product', () => {
    expect(() => previous('cli', 'tmt-office-v0.1.0-alpha.4')).toThrow('not a cli tag');
  });
});

describe('selectAssets and stageRelease', () => {
  it('selects the archive of the product and target and the manifest, each with a digest', () => {
    for (const [tag, product] of [
      ['v5.0.0-alpha.8', 'cli'],
      ['tmt-office-v0.1.0-alpha.3', 'office'],
      ['tmt-squad-v0.1.0-alpha.1', 'squad'],
    ]) {
      const assets = selectAssets({ release: release(tag), product, target: TARGET });
      expect(assets.archive.name).toBe(`${prefixOf(product)}-${TARGET}.tar.gz`);
      expect(assets.manifest.name).toBe('dist-manifest.json');
    }
  });

  it('refuses a missing asset and an asset without a usable digest', () => {
    const cli = release('v5.0.0-alpha.8');
    expect(() =>
      selectAssets({ release: cli, product: 'cli', target: 'x86_64-apple-darwin' })
    ).toThrow('has no tmt-cli-x86_64-apple-darwin.tar.gz');
    expect(() =>
      selectAssets({
        release: release('v5.0.0-alpha.8', { digests: false }),
        product: 'cli',
        target: TARGET,
      })
    ).toThrow('no usable digest for tmt-cli-aarch64-apple-darwin.tar.gz');
    const malformed = {
      ...cli,
      assets: cli.assets?.map((asset) => ({ ...asset, digest: 'sha1:abc' })),
    };
    expect(() => selectAssets({ release: malformed, product: 'cli', target: TARGET })).toThrow(
      'no usable digest'
    );
  });

  it('downloads both files and checks them against the recorded digests', () => {
    const directory = mkdtempSync(path.join(root, 'stage-'));
    const cli = release('v5.0.0-alpha.8');
    const download = (asset: DraftAsset, file: string) =>
      writeFileSync(file, contents.get(asset.id) ?? '');
    const staged = stageRelease({
      download,
      release: cli,
      product: 'cli',
      target: TARGET,
      directory,
    });
    expect(readFileSync(staged.archive, 'utf8')).toBe(`v5.0.0-alpha.8:tmt-cli-${TARGET}.tar.gz`);
    expect(readFileSync(staged.manifest, 'utf8')).toBe('v5.0.0-alpha.8:dist-manifest.json');

    const corrupted = (asset: DraftAsset, file: string) =>
      writeFileSync(file, `${contents.get(asset.id)} tampered`);
    expect(() =>
      stageRelease({ download: corrupted, release: cli, product: 'cli', target: TARGET, directory })
    ).toThrow('does not match its recorded digest');
  });
});

describe('proveUpgrade', () => {
  const releases = [
    release('v5.0.0-alpha.7'),
    release('v5.0.0-alpha.8'),
    release('v5.0.0-alpha.9', { draft: true }),
    release('tmt-office-v0.1.0-alpha.3'),
    release('tmt-office-v0.1.0-alpha.4', { draft: true }),
    release('tmt-squad-v0.1.0-alpha.1', { draft: true }),
  ];
  const download = (asset: DraftAsset, file: string) =>
    writeFileSync(file, contents.get(asset.id) ?? '');
  const prove = (input: { product: string; tag: string; skill?: string }) => {
    const calls: { script: string; args: string[] }[] = [];
    const directory = mkdtempSync(path.join(root, 'prove-'));
    const result = proveUpgrade({
      releases,
      download,
      run: (name, args) => calls.push({ script: name, args }),
      target: TARGET,
      directory,
      ...input,
    });
    return { result, calls, directory };
  };
  const value = (args: string[], flag: string) => {
    expect(args, flag).toContain(flag);
    return args[args.indexOf(flag) + 1];
  };

  it('runs the managed-install lifecycle for a CLI draft over the two staged archives', () => {
    const { result, calls, directory } = prove({
      product: 'cli',
      tag: 'v5.0.0-alpha.9',
      skill: 'skills/tmux-team/SKILL.md',
    });
    expect(result.previous).toBe('v5.0.0-alpha.8');
    expect(calls).toHaveLength(1);
    const [{ script: name, args }] = calls;
    expect(name).toBe('verify-native-installation.mjs');
    expect(value(args, '--archive')).toBe(
      path.join(directory, 'candidate', `tmt-cli-${TARGET}.tar.gz`)
    );
    expect(value(args, '--previous-archive')).toBe(
      path.join(directory, 'previous', `tmt-cli-${TARGET}.tar.gz`)
    );
    expect(readFileSync(value(args, '--previous-archive'), 'utf8')).toBe(
      `v5.0.0-alpha.8:tmt-cli-${TARGET}.tar.gz`
    );
    expect(readFileSync(value(args, '--archive'), 'utf8')).toBe(
      `v5.0.0-alpha.9:tmt-cli-${TARGET}.tar.gz`
    );
    expect(value(args, '--target')).toBe(TARGET);
    expect(value(args, '--skill')).toBe('skills/tmux-team/SKILL.md');
  });

  it('drives an extension draft with the newest published CLI', () => {
    const { result, calls, directory } = prove({
      product: 'office',
      tag: 'tmt-office-v0.1.0-alpha.4',
    });
    expect(result.previous).toBe('tmt-office-v0.1.0-alpha.3');
    const [{ script: name, args }] = calls;
    expect(name).toBe('verify-native-extension-upgrade.mjs');
    expect(value(args, '--product')).toBe('office');
    expect(readFileSync(value(args, '--driver-archive'), 'utf8')).toBe(
      `v5.0.0-alpha.8:tmt-cli-${TARGET}.tar.gz`
    );
    expect(value(args, '--driver-archive')).toBe(
      path.join(directory, 'driver', `tmt-cli-${TARGET}.tar.gz`)
    );
    expect(readFileSync(value(args, '--archive'), 'utf8')).toBe(
      `tmt-office-v0.1.0-alpha.4:tmt-office-${TARGET}.tar.gz`
    );
    expect(args).not.toContain('--skill');
  });

  it('has nothing to upgrade from for the first release of a product, and runs nothing', () => {
    const { result, calls } = prove({ product: 'squad', tag: 'tmt-squad-v0.1.0-alpha.1' });
    expect(result.previous).toBeNull();
    expect(calls).toEqual([]);
  });

  it('proves a published release too, from the one before it', () => {
    const { result } = prove({ product: 'cli', tag: 'v5.0.0-alpha.8', skill: 'skill' });
    expect(result.previous).toBe('v5.0.0-alpha.7');
  });

  it('refuses an unknown tag, a CLI proof without a skill and an extension proof without a CLI', () => {
    expect(() => prove({ product: 'cli', tag: 'v9.9.9' })).toThrow('There is no release v9.9.9.');
    expect(() => prove({ product: 'cli', tag: 'v5.0.0-alpha.9' })).toThrow('needs --skill');
    expect(() =>
      proveUpgrade({
        releases: releases.filter((entry) => !entry.tag_name.startsWith('v')),
        download,
        run: () => {},
        product: 'office',
        tag: 'tmt-office-v0.1.0-alpha.4',
        target: TARGET,
        directory: mkdtempSync(path.join(root, 'prove-')),
      })
    ).toThrow('needs a published CLI release');
  });

  it('fails when a verifier fails, and when a download is not what GitHub recorded', () => {
    expect(() =>
      proveUpgrade({
        releases,
        download,
        run: () => {
          throw new Error('verify-native-installation.mjs failed with 1.');
        },
        product: 'cli',
        tag: 'v5.0.0-alpha.9',
        target: TARGET,
        directory: mkdtempSync(path.join(root, 'prove-')),
        skill: 'skill',
      })
    ).toThrow('failed with 1');
    expect(() =>
      proveUpgrade({
        releases,
        download: (_asset, file) => writeFileSync(file, 'something else'),
        run: () => {},
        product: 'cli',
        tag: 'v5.0.0-alpha.9',
        target: TARGET,
        directory: mkdtempSync(path.join(root, 'prove-')),
        skill: 'skill',
      })
    ).toThrow('does not match its recorded digest');
  });
});

describe('releaseCommit', () => {
  const commitOfTag = (tag: string) => (tag === 'v5.0.0-alpha.8' ? 'b'.repeat(40) : 'not a commit');

  it('is the commit a release points at, or its tag when it names a branch', () => {
    expect(
      releaseCommit({ release: release('v5.0.0-alpha.9', { draft: true }), commitOfTag })
    ).toBe(COMMIT);
    expect(
      releaseCommit({ release: release('v5.0.0-alpha.8', { sha: 'main' }), commitOfTag })
    ).toBe('b'.repeat(40));
  });

  it('refuses a draft that names a branch and a tag that is not a commit', () => {
    expect(() =>
      releaseCommit({
        release: release('v5.0.0-alpha.9', { draft: true, sha: 'main' }),
        commitOfTag,
      })
    ).toThrow('not a commit');
    expect(() =>
      releaseCommit({ release: release('v5.0.0-alpha.7', { sha: 'main' }), commitOfTag })
    ).toThrow('does not resolve to a commit');
  });
});

describe('ghAssetDownloader', () => {
  it('asks the asset endpoint for the bytes by id and writes them', () => {
    const seen: string[][] = [];
    const spawn = (_command: string, args: string[]) => {
      seen.push(args);
      return { status: 0, stdout: Buffer.from([0, 255, 10, 65]), stderr: Buffer.from('') };
    };
    const file = path.join(mkdtempSync(path.join(root, 'download-')), 'archive');
    ghAssetDownloader({ repository: 'wkh237/tmt', spawn })({ id: 42, name: 'a.tar.gz' }, file);
    expect(seen).toEqual([
      ['api', '-H', 'Accept: application/octet-stream', 'repos/wkh237/tmt/releases/assets/42'],
    ]);
    expect([...readFileSync(file)]).toEqual([0, 255, 10, 65]);
  });

  it('reports the status and stderr of a failed download', () => {
    const spawn = () => ({ status: 1, stdout: Buffer.from(''), stderr: Buffer.from('HTTP 404') });
    expect(() =>
      ghAssetDownloader({ repository: 'wkh237/tmt', spawn })(
        { id: 1, name: 'x' },
        path.join(root, 'x')
      )
    ).toThrow('could not download x (1): HTTP 404');
  });
});

describe('release-upgrade.mjs', () => {
  function fakeGh(releases: DraftRelease[]) {
    const directory = mkdtempSync(path.join(root, 'cli-'));
    const bin = path.join(directory, 'bin');
    mkdirSync(bin);
    writeFileSync(
      path.join(bin, 'gh'),
      `#!/bin/sh\ncase "$*" in\n  *commits/*) echo ${'c'.repeat(40)} ;;\n  *) echo '${JSON.stringify([releases])}' ;;\nesac\n`
    );
    chmodSync(path.join(bin, 'gh'), 0o755);
    const output = path.join(directory, 'output');
    writeFileSync(output, '');
    const run = (args: string[]) =>
      spawnSync('node', [script, ...args], {
        encoding: 'utf8',
        env: {
          PATH: `${bin}${path.delimiter}${process.env.PATH}`,
          GITHUB_REPOSITORY: 'wkh237/tmt',
          GITHUB_OUTPUT: output,
        },
        timeout: 10_000,
      });
    return Object.assign(run, { output: () => readFileSync(output, 'utf8') });
  }

  it('resolves the commit of a draft and of a published release', () => {
    const run = fakeGh([
      release('v5.0.0-alpha.9', { draft: true }),
      release('v5.0.0-alpha.8', { sha: 'main' }),
    ]);
    expect(run(['resolve', '--tag', 'v5.0.0-alpha.9']).status).toBe(0);
    expect(run.output()).toBe(`sha=${COMMIT}\n`);
    expect(run(['resolve', '--tag', 'v5.0.0-alpha.8']).status).toBe(0);
    expect(run.output()).toBe(`sha=${COMMIT}\nsha=${'c'.repeat(40)}\n`);
    expect(run(['resolve', '--tag', 'v1.0.0']).stderr).toContain('There is no release v1.0.0.');
  });

  it('reports that a first release has nothing to upgrade from', () => {
    const run = fakeGh([release('tmt-squad-v0.1.0-alpha.1', { draft: true })]);
    const result = run([
      'prove',
      '--product',
      'squad',
      '--tag',
      'tmt-squad-v0.1.0-alpha.1',
      '--target',
      TARGET,
      '--directory',
      path.join(root, 'none'),
    ]);
    expect(result.status).toBe(0);
    expect(result.stderr).toContain('No published squad release precedes tmt-squad-v0.1.0-alpha.1');
  });

  it('refuses missing options and an unknown command', () => {
    const run = fakeGh([]);
    expect(run(['prove', '--product', 'cli']).stderr).toContain('--tag is required.');
    expect(run(['bogus']).stderr).toContain('Usage: release-upgrade.mjs prove|resolve');
  });
});
