import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vite-plus/test';
import type { DraftAsset, DraftRelease } from '../../scripts/release-draft-assets.mjs';
import {
  PROOF_FILES,
  archiveTargets,
  assessUpgrade,
  combineFailures,
  failureCause,
  fetchUpgrade,
  ghAssetDownloader,
  proveStaged,
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

const TARGETS = [
  'aarch64-apple-darwin',
  'aarch64-unknown-linux-musl',
  'x86_64-apple-darwin',
  'x86_64-unknown-linux-musl',
];

function release(
  tag: string,
  {
    draft = false,
    sha = COMMIT,
    digests = true,
    targets = [TARGET],
  }: { draft?: boolean; sha?: string; digests?: boolean; targets?: string[] } = {}
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
    assets: [
      ...targets.map((target) => asset(`${prefixOf(product)}-${target}.tar.gz`)),
      asset('dist-manifest.json'),
    ],
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
    expect(staged.digests).toEqual({
      [`tmt-cli-${TARGET}.tar.gz`]: digestOf(`v5.0.0-alpha.8:tmt-cli-${TARGET}.tar.gz`),
      'dist-manifest.json': digestOf('v5.0.0-alpha.8:dist-manifest.json'),
    });

    const corrupted = (asset: DraftAsset, file: string) =>
      writeFileSync(file, `${contents.get(asset.id)} tampered`);
    expect(() =>
      stageRelease({ download: corrupted, release: cli, product: 'cli', target: TARGET, directory })
    ).toThrow('does not match its recorded digest');
  });
});

describe('archiveTargets', () => {
  it('lists the targets a release carries an archive for, and ignores every other asset', () => {
    const cli = release('v5.0.0-alpha.9', { draft: true, targets: [...TARGETS].reverse() });
    expect(archiveTargets({ release: cli, product: 'cli' })).toEqual(TARGETS);
    expect(archiveTargets({ release: cli, product: 'office' })).toEqual([]);
    const withChecksum = {
      ...cli,
      assets: [...(cli.assets ?? []), { id: 999, name: `tmt-cli-${TARGET}.tar.gz.sha256` }],
    };
    expect(archiveTargets({ release: withChecksum, product: 'cli' })).toEqual(TARGETS);
  });
});

describe('fetchUpgrade and proveStaged', () => {
  const releases = [
    release('v5.0.0-alpha.7', { targets: TARGETS }),
    release('v5.0.0-alpha.8', { targets: TARGETS }),
    release('v5.0.0-alpha.9', { draft: true, targets: TARGETS }),
    release('tmt-office-v0.1.0-alpha.3', { targets: TARGETS }),
    release('tmt-office-v0.1.0-alpha.4', { draft: true, targets: TARGETS }),
    release('tmt-squad-v0.1.0-alpha.1', { draft: true, targets: TARGETS }),
  ];
  const download = (asset: DraftAsset, file: string) =>
    writeFileSync(file, contents.get(asset.id) ?? '');
  const fetchInto = (product: string, tag: string, input: { releases?: DraftRelease[] } = {}) => {
    const directory = mkdtempSync(path.join(root, 'fetch-'));
    const downloads: string[] = [];
    const plan = fetchUpgrade({
      releases: input.releases ?? releases,
      download: (asset, file) => {
        downloads.push(asset.name);
        download(asset, file);
      },
      product,
      tag,
      directory,
    });
    return { plan, directory, downloads };
  };
  const prove = (
    directory: string,
    input: { product: string; tag: string; target?: string; skill?: string }
  ) => {
    const calls: { script: string; args: string[] }[] = [];
    const result = proveStaged({
      directory,
      target: TARGET,
      run: (name, args) => calls.push({ script: name, args }),
      ...input,
    });
    return { result, calls };
  };
  const value = (args: string[], flag: string) => {
    expect(args, flag).toContain(flag);
    return args[args.indexOf(flag) + 1];
  };

  it('stages the candidate and the previous CLI release for every target, with a plan', () => {
    const { plan, directory, downloads } = fetchInto('cli', 'v5.0.0-alpha.9');
    expect(plan.previous).toBe('v5.0.0-alpha.8');
    expect(plan.driver).toBeNull();
    expect(downloads).toHaveLength(2 * 2 * TARGETS.length);
    expect(Object.keys(plan.files).sort()).toEqual(
      TARGETS.flatMap((target) =>
        ['candidate', 'previous'].flatMap((kind) => [
          `${target}/${kind}/dist-manifest.json`,
          `${target}/${kind}/tmt-cli-${target}.tar.gz`,
        ])
      ).sort()
    );
    expect(plan.files[`${TARGET}/candidate/tmt-cli-${TARGET}.tar.gz`]).toBe(
      digestOf(`v5.0.0-alpha.9:tmt-cli-${TARGET}.tar.gz`)
    );
    expect(
      readFileSync(path.join(directory, TARGET, 'previous', `tmt-cli-${TARGET}.tar.gz`), 'utf8')
    ).toBe(`v5.0.0-alpha.8:tmt-cli-${TARGET}.tar.gz`);
    expect(JSON.parse(readFileSync(path.join(directory, 'plan.json'), 'utf8'))).toEqual(plan);
  });

  it('adds the newest published CLI as the driver of an extension', () => {
    const { plan, directory } = fetchInto('office', 'tmt-office-v0.1.0-alpha.4');
    expect(plan.previous).toBe('tmt-office-v0.1.0-alpha.3');
    expect(plan.driver).toBe('v5.0.0-alpha.8');
    expect(plan.files[`${TARGET}/driver/tmt-cli-${TARGET}.tar.gz`]).toBe(
      digestOf(`v5.0.0-alpha.8:tmt-cli-${TARGET}.tar.gz`)
    );
    expect(
      readFileSync(path.join(directory, TARGET, 'candidate', `tmt-office-${TARGET}.tar.gz`), 'utf8')
    ).toBe(`tmt-office-v0.1.0-alpha.4:tmt-office-${TARGET}.tar.gz`);
  });

  it('has nothing to fetch for the first release of a product', () => {
    const { plan, directory, downloads } = fetchInto('squad', 'tmt-squad-v0.1.0-alpha.1');
    expect(plan.previous).toBeNull();
    expect(plan.files).toEqual({});
    expect(downloads).toEqual([]);
    expect(prove(directory, { product: 'squad', tag: 'tmt-squad-v0.1.0-alpha.1' })).toEqual({
      result: { previous: null },
      calls: [],
    });
  });

  it('stages a published release too, against the one before it', () => {
    expect(fetchInto('cli', 'v5.0.0-alpha.8').plan.previous).toBe('v5.0.0-alpha.7');
  });

  it('refuses an unknown tag, an extension without a published CLI and a previous release that lacks a target', () => {
    expect(() => fetchInto('cli', 'v9.9.9')).toThrow('There is no release v9.9.9.');
    expect(() =>
      fetchInto('office', 'tmt-office-v0.1.0-alpha.4', {
        releases: releases.filter((entry) => !entry.tag_name.startsWith('v')),
      })
    ).toThrow('needs a published CLI release');
    expect(() =>
      fetchInto('cli', 'v5.0.0-alpha.9', {
        releases: [
          release('v5.0.0-alpha.8', { targets: [TARGET] }),
          release('v5.0.0-alpha.9', { draft: true, targets: TARGETS }),
        ],
      })
    ).toThrow('has no tmt-cli-aarch64-unknown-linux-musl.tar.gz');
    expect(() =>
      fetchInto('cli', 'v5.0.0-alpha.9', {
        releases: [
          release('v5.0.0-alpha.8', { targets: TARGETS }),
          release('v5.0.0-alpha.9', { draft: true, targets: [] }),
        ],
      })
    ).toThrow('has no archive to upgrade to');
  });

  it('refuses a download that is not what GitHub recorded', () => {
    expect(() =>
      fetchUpgrade({
        releases,
        download: (_asset, file) => writeFileSync(file, 'something else'),
        product: 'cli',
        tag: 'v5.0.0-alpha.9',
        directory: mkdtempSync(path.join(root, 'fetch-')),
      })
    ).toThrow('does not match its recorded digest');
  });

  it('runs the managed-install lifecycle for a CLI over the staged files of one target', () => {
    const { directory } = fetchInto('cli', 'v5.0.0-alpha.9');
    const { result, calls } = prove(directory, {
      product: 'cli',
      tag: 'v5.0.0-alpha.9',
      target: 'x86_64-unknown-linux-musl',
      skill: 'skills/tmux-team/SKILL.md',
    });
    expect(result.previous).toBe('v5.0.0-alpha.8');
    expect(calls).toHaveLength(1);
    const [{ script: name, args }] = calls;
    const target = 'x86_64-unknown-linux-musl';
    expect(name).toBe('verify-native-installation.mjs');
    expect(value(args, '--archive')).toBe(
      path.join(directory, target, 'candidate', `tmt-cli-${target}.tar.gz`)
    );
    expect(value(args, '--previous-archive')).toBe(
      path.join(directory, target, 'previous', `tmt-cli-${target}.tar.gz`)
    );
    expect(value(args, '--previous-manifest')).toBe(
      path.join(directory, target, 'previous', 'dist-manifest.json')
    );
    expect(value(args, '--target')).toBe(target);
    expect(value(args, '--skill')).toBe('skills/tmux-team/SKILL.md');
  });

  it('drives an extension with the staged CLI', () => {
    const { directory } = fetchInto('office', 'tmt-office-v0.1.0-alpha.4');
    const { result, calls } = prove(directory, {
      product: 'office',
      tag: 'tmt-office-v0.1.0-alpha.4',
    });
    expect(result.previous).toBe('tmt-office-v0.1.0-alpha.3');
    const [{ script: name, args }] = calls;
    expect(name).toBe('verify-native-extension-upgrade.mjs');
    expect(value(args, '--product')).toBe('office');
    expect(value(args, '--driver-archive')).toBe(
      path.join(directory, TARGET, 'driver', `tmt-cli-${TARGET}.tar.gz`)
    );
    expect(value(args, '--archive')).toBe(
      path.join(directory, TARGET, 'candidate', `tmt-office-${TARGET}.tar.gz`)
    );
    expect(args).not.toContain('--skill');
  });

  it('checks the staged files again, since they crossed a job boundary', () => {
    const { directory } = fetchInto('cli', 'v5.0.0-alpha.9');
    const file = path.join(directory, TARGET, 'candidate', `tmt-cli-${TARGET}.tar.gz`);
    writeFileSync(file, 'tampered on the way');
    expect(() => prove(directory, { product: 'cli', tag: 'v5.0.0-alpha.9', skill: 's' })).toThrow(
      'does not match its recorded digest'
    );
    rmSync(file);
    expect(() => prove(directory, { product: 'cli', tag: 'v5.0.0-alpha.9', skill: 's' })).toThrow(
      'missing or does not match'
    );
  });

  it('refuses assets staged for another release or target, and a CLI proof without a skill', () => {
    const { directory } = fetchInto('cli', 'v5.0.0-alpha.9');
    expect(() => prove(directory, { product: 'cli', tag: 'v5.0.0-alpha.8', skill: 's' })).toThrow(
      'staged assets are for v5.0.0-alpha.9'
    );
    expect(() => prove(directory, { product: 'office', tag: 'v5.0.0-alpha.9' })).toThrow(
      'staged assets are for'
    );
    expect(() =>
      prove(directory, { product: 'cli', tag: 'v5.0.0-alpha.9', target: 'riscv64', skill: 's' })
    ).toThrow('no files for riscv64');
    expect(() => prove(directory, { product: 'cli', tag: 'v5.0.0-alpha.9' })).toThrow(
      'needs --skill'
    );
  });

  it('fails when the verifier fails', () => {
    const { directory } = fetchInto('cli', 'v5.0.0-alpha.9');
    expect(() =>
      proveStaged({
        directory,
        product: 'cli',
        tag: 'v5.0.0-alpha.9',
        target: TARGET,
        skill: 's',
        run: () => {
          throw new Error('verify-native-installation.mjs failed with 1.');
        },
      })
    ).toThrow('failed with 1');
  });
});

describe('assessUpgrade', () => {
  it('has nothing to upgrade from for the first release of a product', () => {
    const asked: string[] = [];
    expect(
      assessUpgrade({
        plan: { previous: null },
        hasFileAt: (file) => {
          asked.push(file);
          return false;
        },
      })
    ).toEqual({ outcome: 'nothing', reason: '' });
    expect(asked).toEqual([]);
  });

  it('can prove when the release commit carries every script the proof runs', () => {
    const asked: string[] = [];
    expect(
      assessUpgrade({
        plan: { previous: 'v5.0.0-alpha.8' },
        hasFileAt: (file) => {
          asked.push(file);
          return true;
        },
      })
    ).toEqual({ outcome: 'proved', reason: '' });
    expect(asked).toEqual(PROOF_FILES.map((file) => `typescript/scripts/${file}`));
  });

  it('names the first missing script and words the reason for the owner', () => {
    const missing = PROOF_FILES[1];
    const { outcome, reason } = assessUpgrade({
      plan: { previous: 'v5.0.0-alpha.8' },
      hasFileAt: (file) => !file.endsWith(missing),
    });
    expect(outcome).toBe('predates');
    expect(reason).toBe(
      `This release's commit has no ${missing}: it predates the automated upgrade proof, so prove the upgrade by hand, as the native release verification guide describes.`
    );
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

describe('failureCause and combineFailures', () => {
  const trace = (error: string) =>
    `Run the proof\nnode:internal/process/execution:1\n    triggerUncaughtException(\n    ^\n\n${error}\n    at file:///x.mjs:1:1 {\n  code: 'ERR_ASSERTION'\n}\n\nNode.js v22.23.2\nverify-native-extension-upgrade.mjs failed with 1.\n`;

  it('takes the first error line a verifier threw, not the runner’s own closing line', () => {
    expect(failureCause(trace('Error: Native archive checksum mismatch'))).toBe(
      'Native archive checksum mismatch'
    );
    expect(
      failureCause(
        trace(
          'AssertionError [ERR_ASSERTION]: Packed command failed (exited 1, expected 0): tmt extension install squad: unrecognized subcommand squad\ncommand: x'
        )
      )
    ).toBe(
      'Packed command failed (exited 1, expected 0): tmt extension install squad: unrecognized subcommand squad'
    );
  });

  it('says so when the log shows no error, and keeps one bounded line of printable text', () => {
    // Only a line that starts with the error counts, not text that mentions one.
    expect(failureCause('expected output: Error: not thrown\nError: the real one\n')).toBe(
      'the real one'
    );
    expect(failureCause('Run the proof\nverify.mjs failed with 1.\n')).toBe(
      'the proof failed without an error message'
    );
    expect(failureCause('Error: a\u0007b\tc\u001b[31m red')).toBe('a b c [31m red');
    const long = failureCause(`Error: ${'x'.repeat(1000)}`);
    expect(long).toHaveLength(300);
    expect(long.endsWith('...')).toBe(true);
  });

  it('names each distinct cause once, with the hosts it happened on, sorted', () => {
    const squad = trace('Error: no squad command');
    expect(
      combineFailures([
        { target: 'x86_64-unknown-linux-musl', log: squad },
        { target: 'aarch64-apple-darwin', log: squad },
        { target: 'x86_64-apple-darwin', log: trace('Error: a downgrade was accepted') },
      ])
    ).toBe(
      'no squad command (aarch64-apple-darwin, x86_64-unknown-linux-musl); a downgrade was accepted (x86_64-apple-darwin)'
    );
    const many = combineFailures(
      Array.from({ length: 8 }, (_, index) => ({
        target: `host-${index}`,
        log: trace(`Error: cause number ${index} ${'y'.repeat(150)}`),
      }))
    );
    expect(many.length).toBeLessThanOrEqual(600);
  });
});

describe('release-upgrade.mjs', () => {
  function fakeGh(releases: DraftRelease[]) {
    const directory = mkdtempSync(path.join(root, 'cli-'));
    const bin = path.join(directory, 'bin');
    mkdirSync(bin);
    const calls = path.join(directory, 'gh-calls');
    writeFileSync(calls, '');
    writeExecutable(
      path.join(bin, 'gh'),
      `#!/bin/sh\necho "$*" >> '${calls}'\ncase "$*" in\n  *commits/*) echo ${'c'.repeat(40)} ;;\n  *) echo '${JSON.stringify([releases])}' ;;\nesac\n`,
      0o755
    );
    const output = path.join(directory, 'output');
    writeFileSync(output, '');
    const run = (args: string[], environment: Record<string, string> = {}) =>
      spawnSync('node', [script, ...args], {
        encoding: 'utf8',
        env: {
          PATH: `${bin}${path.delimiter}${process.env.PATH}`,
          GITHUB_REPOSITORY: 'wkh237/tmt',
          GITHUB_OUTPUT: output,
          ...environment,
        },
        timeout: 10_000,
      });
    return Object.assign(run, {
      output: () => readFileSync(output, 'utf8'),
      ghCalls: () => readFileSync(calls, 'utf8').split('\n').filter(Boolean).length,
    });
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

  it('fetches nothing for a first release, and proves offline without gh or a repository', () => {
    const run = fakeGh([release('tmt-squad-v0.1.0-alpha.1', { draft: true })]);
    const directory = path.join(root, 'none');
    const fetched = run([
      'fetch',
      '--product',
      'squad',
      '--tag',
      'tmt-squad-v0.1.0-alpha.1',
      '--directory',
      directory,
    ]);
    expect(fetched.status).toBe(0);
    expect(fetched.stderr).toContain(
      'No published squad release precedes tmt-squad-v0.1.0-alpha.1'
    );
    expect(JSON.parse(readFileSync(path.join(directory, 'plan.json'), 'utf8')).previous).toBeNull();

    // `prove` runs the release's own code: it must not need gh, a token or a repository.
    const offline = run(
      [
        'prove',
        '--product',
        'squad',
        '--tag',
        'tmt-squad-v0.1.0-alpha.1',
        '--target',
        TARGET,
        '--directory',
        directory,
      ],
      { GITHUB_REPOSITORY: '', GH_TOKEN: '' }
    );
    expect(offline.status).toBe(0);
    expect(offline.stderr).toContain('there is nothing to upgrade from');
    expect(run.ghCalls()).toBe(1);
  });

  it('names why the hosts failed from their logs, offline, and is empty when none failed', () => {
    const run = fakeGh([]);
    const logs = path.join(root, 'logs');
    for (const target of ['aarch64-apple-darwin', 'x86_64-apple-darwin']) {
      mkdirSync(path.join(logs, `upgrade-proof-log-${target}`), { recursive: true });
      writeFileSync(
        path.join(logs, `upgrade-proof-log-${target}`, 'upgrade-proof.log'),
        'noise\nError: Native archive checksum mismatch\n  at x\n'
      );
    }
    mkdirSync(path.join(logs, 'unrelated'), { recursive: true });
    const failed = run(['reason', '--directory', logs], { GITHUB_REPOSITORY: '', GH_TOKEN: '' });
    expect(failed.status).toBe(0);
    expect(failed.stderr).toContain('The upgrade proof failed: Native archive checksum mismatch');
    expect(run.output()).toBe(
      'reason=Native archive checksum mismatch (aarch64-apple-darwin, x86_64-apple-darwin)\n'
    );
    const none = run(['reason', '--directory', path.join(root, 'no-logs')]);
    expect(none.status).toBe(0);
    expect(run.output().endsWith('\nreason=\n')).toBe(true);
  });

  it('refuses missing options and an unknown command', () => {
    const run = fakeGh([]);
    expect(run(['prove', '--product', 'cli']).stderr).toContain('--tag is required.');
    expect(run(['bogus']).stderr).toContain(
      'Usage: release-upgrade.mjs resolve|fetch|assess|prove|reason'
    );
  });
});
