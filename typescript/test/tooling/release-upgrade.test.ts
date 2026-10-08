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
  ACCEPTANCE_TEST,
  acceptanceApplicability,
  archiveTargets,
  assessUpgrade,
  combineFailures,
  failureCause,
  fetchUpgrade,
  ghAssetDownloader,
  ghCliAncestry,
  localCandidate,
  proveStaged,
  proveArchiveAcceptance,
  releaseCommit,
  selectAssets,
  selectPrevious,
  selectSupportFloor,
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
    : tag.startsWith('tmt-ops-v')
      ? 'ops'
      : tag.startsWith('tmt-driver-herdr-v')
        ? 'driver-herdr'
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
    expect(versionOfTag('tmt-ops-v0.1.0-alpha.2', 'ops')).toBe('0.1.0-alpha.2');
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
    expect(publishedReleases(releases, 'ops')).toEqual([]);
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
    release('tmt-ops-v0.1.0-alpha.1'),
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
    expect(previous('ops', 'tmt-ops-v0.1.0-alpha.2')).toBe('tmt-ops-v0.1.0-alpha.1');
    expect(previous('ops', 'tmt-ops-v0.1.0-alpha.1')).toBeNull();
  });

  it('refuses a candidate tag of another product', () => {
    expect(() => previous('cli', 'tmt-office-v0.1.0-alpha.4')).toThrow('not a cli tag');
  });

  it('never treats a dev candidate as newer than a published alpha with the same core', () => {
    expect(previous('cli', 'v5.0.0-dev')).toBeNull();
    expect(previous('office', 'tmt-office-v0.1.0-dev')).toBeNull();
    expect(previous('ops', 'tmt-ops-v0.1.0-dev')).toBeNull();
  });
});

describe('selectAssets and stageRelease', () => {
  it('selects the archive of the product and target and the manifest, each with a digest', () => {
    for (const [tag, product] of [
      ['v5.0.0-alpha.8', 'cli'],
      ['tmt-office-v0.1.0-alpha.3', 'office'],
      ['tmt-ops-v0.1.0-alpha.1', 'ops'],
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
    release('tmt-ops-v0.1.0-alpha.1', { draft: true, targets: TARGETS }),
    release('tmt-driver-herdr-v0.1.0-alpha.1', { targets: TARGETS }),
    release('tmt-driver-herdr-v0.1.0-alpha.2', { draft: true, targets: TARGETS }),
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
    input: { product: string; tag: string; target?: string; skill?: string; sourceRoot?: string }
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

  describe('a rehearsal candidate staged from the verified local bundle', () => {
    const SYNTHETIC = 'v5.0.0-alpha.999999';
    /** A bundle directory as the prepare run's assemble job uploads it. */
    function bundle(
      product: string,
      tag: string,
      { announce = tag, install = false }: { announce?: string; install?: boolean } = {}
    ) {
      const directory = mkdtempSync(path.join(root, 'bundle-'));
      writeFileSync(
        path.join(directory, 'dist-manifest.json'),
        JSON.stringify({ announcement_tag: announce })
      );
      for (const target of TARGETS) {
        const name = `${prefixOf(product)}-${target}.tar.gz`;
        const text = `${tag}:${name}`;
        writeFileSync(path.join(directory, name), text);
        writeFileSync(
          path.join(directory, `${name}.sha256`),
          `${digestOf(text).slice('sha256:'.length)} *${name}\n`
        );
      }
      if (install) writeFileSync(path.join(directory, 'install.sh'), 'bootstrap');
      return directory;
    }
    const stageLocal = (
      product: string,
      tag: string,
      input: { directory?: string; releases?: DraftRelease[] } = {}
    ) => {
      const directory = mkdtempSync(path.join(root, 'local-'));
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
        local: localCandidate({
          directory: input.directory ?? bundle(product, tag),
          product,
          tag,
        }),
      });
      return { plan, directory, downloads };
    };

    it('stages the local candidate beside the published previous release in the draft plan shape', () => {
      const published = [
        release('v5.0.0-alpha.36', { targets: TARGETS }),
        release('v5.0.0-alpha.45', { targets: TARGETS }),
      ];
      const { plan, directory, downloads } = stageLocal('cli', SYNTHETIC, {
        releases: published,
        directory: bundle('cli', SYNTHETIC, { install: true }),
      });
      expect(plan.previous).toBe('v5.0.0-alpha.45');
      // Only published releases are downloaded; the candidate is copied from the bundle.
      expect(downloads).toHaveLength(4 * TARGETS.length);
      expect(new Set(downloads)).toEqual(
        new Set(['dist-manifest.json', ...TARGETS.map((target) => `tmt-cli-${target}.tar.gz`)])
      );
      // The same file set a draft candidate stages: candidate, previous, declared floor and bootstrap.
      expect(Object.keys(plan.files).sort()).toEqual(
        TARGETS.flatMap((target) => [
          ...['candidate', 'previous', 'floor'].flatMap((kind) => [
            `${target}/${kind}/dist-manifest.json`,
            `${target}/${kind}/tmt-cli-${target}.tar.gz`,
          ]),
          `${target}/candidate/install.sh`,
        ]).sort()
      );
      expect(plan.files[`${TARGET}/candidate/tmt-cli-${TARGET}.tar.gz`]).toBe(
        digestOf(`${SYNTHETIC}:tmt-cli-${TARGET}.tar.gz`)
      );
      expect(
        readFileSync(path.join(directory, TARGET, 'candidate', `tmt-cli-${TARGET}.tar.gz`), 'utf8')
      ).toBe(`${SYNTHETIC}:tmt-cli-${TARGET}.tar.gz`);
      // The proof consumes it exactly like a fetched draft.
      const { result, calls } = prove(directory, {
        product: 'cli',
        tag: SYNTHETIC,
        skill: 'SKILL.md',
      });
      expect(result.previous).toBe('v5.0.0-alpha.45');
      expect(value(calls[0].args, '--archive')).toBe(
        path.join(directory, TARGET, 'candidate', `tmt-cli-${TARGET}.tar.gz`)
      );
    });

    it('stages an extension candidate with the newest published CLI as its driver', () => {
      const { plan } = stageLocal('office', 'tmt-office-v0.1.0-alpha.999999');
      expect(plan.previous).toBe('tmt-office-v0.1.0-alpha.3');
      expect(plan.driver).toBe('v5.0.0-alpha.8');
    });

    it('stages the candidate bootstrap that the declared support floor needs', () => {
      const entries = [
        release('v5.0.0-alpha.36', { targets: TARGETS }),
        release('v5.0.0-alpha.45', { targets: TARGETS }),
      ];
      const { plan } = stageLocal('cli', SYNTHETIC, {
        releases: entries,
        directory: bundle('cli', SYNTHETIC, { install: true }),
      });
      expect(plan.floor).toBe('v5.0.0-alpha.36');
      expect(plan.files[`${TARGET}/candidate/install.sh`]).toBe(digestOf('bootstrap'));
    });

    it('reports a first product release as nothing to upgrade from', () => {
      const { plan, downloads } = stageLocal('ops', 'tmt-ops-v0.1.0-alpha.999999', {
        releases: [],
      });
      expect(plan.previous).toBeNull();
      expect(downloads).toEqual([]);
    });

    it('refuses a synthetic candidate that is not newer than the newest published release', () => {
      const newer = [...releases, release('v5.0.1', { targets: TARGETS })];
      expect(() => stageLocal('cli', SYNTHETIC, { releases: newer })).toThrow(
        `The synthetic candidate ${SYNTHETIC} is not newer than the published v5.0.1.`
      );
      const same = [...releases, release(SYNTHETIC, { targets: TARGETS })];
      expect(() => stageLocal('cli', SYNTHETIC, { releases: same })).toThrow('not newer');
    });

    it('rejects a bundle with a missing manifest, another tag, no archive or a bad digest', () => {
      const missing = bundle('cli', SYNTHETIC);
      rmSync(path.join(missing, 'dist-manifest.json'));
      expect(() => localCandidate({ directory: missing, product: 'cli', tag: SYNTHETIC })).toThrow(
        'has no dist-manifest.json'
      );
      expect(() =>
        localCandidate({
          directory: bundle('cli', SYNTHETIC, { announce: 'v5.0.0-alpha.1' }),
          product: 'cli',
          tag: SYNTHETIC,
        })
      ).toThrow('announces v5.0.0-alpha.1');
      expect(() =>
        localCandidate({ directory: bundle('cli', SYNTHETIC), product: 'ops', tag: SYNTHETIC })
      ).toThrow('has no ops archive');
      const corrupted = bundle('cli', SYNTHETIC);
      writeFileSync(path.join(corrupted, `tmt-cli-${TARGET}.tar.gz`), 'tampered');
      expect(() =>
        localCandidate({ directory: corrupted, product: 'cli', tag: SYNTHETIC })
      ).toThrow(
        `tmt-cli-${TARGET}.tar.gz does not match its recorded tmt-cli-${TARGET}.tar.gz.sha256`
      );
      const unrecorded = bundle('cli', SYNTHETIC);
      rmSync(path.join(unrecorded, `tmt-cli-${TARGET}.tar.gz.sha256`));
      expect(() =>
        localCandidate({ directory: unrecorded, product: 'cli', tag: SYNTHETIC })
      ).toThrow('does not match its recorded');
    });
  });

  it('stages both the declared CLI floor and last published source with the candidate bootstrap', () => {
    const entries = [
      release('v5.0.0-alpha.36', { targets: TARGETS }),
      release('v5.0.0-alpha.45', { targets: TARGETS }),
      release('v5.0.0-alpha.46', { draft: true, targets: TARGETS }),
    ];
    let candidate = entries[2];
    const asset = { id: nextId++, name: 'install.sh', digest: digestOf('bootstrap') };
    contents.set(asset.id, 'bootstrap');
    candidate = { ...candidate, assets: [...(candidate.assets ?? []), asset] };
    entries[2] = candidate;
    const { directory, plan, downloads } = fetchInto('cli', candidate.tag_name, {
      releases: entries,
    });
    expect(plan.floor).toBe('v5.0.0-alpha.36');
    expect(plan.previous).toBe('v5.0.0-alpha.45');
    expect(downloads).toHaveLength(7 * TARGETS.length);
    const { calls } = prove(directory, {
      product: 'cli',
      tag: candidate.tag_name,
      skill: 'candidate-skill',
    });
    expect(calls).toHaveLength(2);
    expect(value(calls[0].args, '--previous-archive')).toContain('/previous/');
    expect(value(calls[1].args, '--previous-archive')).toContain('/floor/');
    for (const call of calls)
      expect(value(call.args, '--bootstrap')).toBe(
        path.join(directory, TARGET, 'candidate', 'install.sh')
      );
    const messages: string[] = [];
    const acceptanceSources: string[] = [];
    let compiles = 0;
    proveArchiveAcceptance({
      directory,
      product: 'cli',
      tag: candidate.tag_name,
      target: TARGET,
      report: (line) => messages.push(line),
      execute: (command, args, options) => {
        if (command === 'cargo') {
          compiles++;
          return JSON.stringify({
            reason: 'compiler-artifact',
            target: { name: 'tmt_adapters' },
            profile: { test: true },
            executable: '/fixture/tests',
          });
        }
        if (args.includes('--list')) return `${ACCEPTANCE_TEST}: test\n`;
        acceptanceSources.push(options.env.TMT_UPGRADE_OLD_ARCHIVE!);
        return 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out;\n';
      },
    });
    expect(compiles).toBe(1);
    expect(acceptanceSources).toEqual(calls.map((call) => value(call.args, '--previous-archive')));
    expect(
      messages.filter((message) => message.includes('Real-archive adapter acceptance: passed'))
    ).toHaveLength(2);
    writeFileSync(path.join(directory, TARGET, 'floor', `tmt-cli-${TARGET}.tar.gz`), 'corrupt');
    expect(() =>
      prove(directory, { product: 'cli', tag: candidate.tag_name, skill: 'candidate-skill' })
    ).toThrow('does not match its recorded digest');
  });

  it('reuses the previous source when it is the floor and refuses missing bootstrap evidence', () => {
    const entries = [
      release('v5.0.0-alpha.36', { targets: TARGETS }),
      release('v5.0.0-alpha.37', { draft: true, targets: TARGETS }),
    ];
    expect(() => fetchInto('cli', entries[1].tag_name, { releases: entries })).toThrow(
      'digest-checked install.sh'
    );
    const asset = { id: nextId++, name: 'install.sh', digest: digestOf('bootstrap') };
    contents.set(asset.id, 'bootstrap');
    entries[1] = { ...entries[1], assets: [...(entries[1].assets ?? []), asset] };
    for (const assets of [
      [...(entries[1].assets ?? []), asset],
      (entries[1].assets ?? []).map((item) =>
        item.name === 'install.sh' ? { ...item, digest: 'sha256:invalid' } : item
      ),
    ]) {
      expect(() =>
        fetchInto('cli', entries[1].tag_name, {
          releases: [entries[0], { ...entries[1], assets }],
        })
      ).toThrow('exactly one digest-checked install.sh');
    }
    const { directory, plan } = fetchInto('cli', entries[1].tag_name, { releases: entries });
    expect(Object.keys(plan.files).some((name) => name.includes('/floor/'))).toBe(false);
    expect(
      prove(directory, { product: 'cli', tag: entries[1].tag_name, skill: 'candidate-skill' }).calls
    ).toHaveLength(1);
    delete plan.files[`${TARGET}/candidate/install.sh`];
    writeFileSync(path.join(directory, 'plan.json'), JSON.stringify(plan));
    expect(() =>
      prove(directory, { product: 'cli', tag: entries[1].tag_name, skill: 'candidate-skill' })
    ).toThrow('install.sh has no recorded digest');
    plan.floor = null;
    writeFileSync(path.join(directory, 'plan.json'), JSON.stringify(plan));
    expect(() =>
      prove(directory, { product: 'cli', tag: entries[1].tag_name, skill: 'candidate-skill' })
    ).toThrow('missing the declared support floor');
  });

  it('selects only the exact published floor, failing closed instead of substituting another source', () => {
    const input = { product: 'cli', candidateTag: 'v5.0.0-alpha.46' };
    for (const entries of [
      [],
      [release('v5.0.0-alpha.36', { draft: true })],
      [release('v5.0.0-alpha.35')],
      [release('v5.0.0-alpha.36'), release('v5.0.0-alpha.36')],
    ])
      expect(() => selectSupportFloor({ ...input, releases: entries })).toThrow(
        'exactly one published support floor'
      );
    expect(
      selectSupportFloor({ ...input, candidateTag: 'v5.0.0-alpha.36', releases: [] })
    ).toBeNull();
    expect(
      selectSupportFloor({ product: 'ops', candidateTag: 'tmt-ops-v1.0.0', releases: [] })
    ).toBeNull();
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

  it('stages only driver versions and the published CLI, then calls the driver proof', () => {
    const { plan, directory, downloads } = fetchInto(
      'driver-herdr',
      'tmt-driver-herdr-v0.1.0-alpha.2'
    );
    expect(plan.previous).toBe('tmt-driver-herdr-v0.1.0-alpha.1');
    expect(plan.driver).toBe('v5.0.0-alpha.8');
    expect(downloads.filter((name) => name.endsWith('.tar.gz'))).toHaveLength(3 * TARGETS.length);
    expect(
      downloads
        .filter((name) => name.endsWith('.tar.gz'))
        .every((name) => name.startsWith('tmt-driver-herdr-') || name.startsWith('tmt-cli-'))
    ).toBe(true);
    const { calls } = prove(directory, {
      product: 'driver-herdr',
      tag: 'tmt-driver-herdr-v0.1.0-alpha.2',
    });
    expect(calls).toHaveLength(1);
    expect(calls[0].script).toBe('verify-native-driver-upgrade.mjs');
    expect(value(calls[0].args, '--product')).toBe('driver-herdr');
    expect(value(calls[0].args, '--driver-archive')).toContain('tmt-cli-');
  });

  it('has nothing to fetch for the first release of a product', () => {
    const { plan, directory, downloads } = fetchInto('ops', 'tmt-ops-v0.1.0-alpha.1');
    expect(plan.previous).toBeNull();
    expect(plan.files).toEqual({});
    expect(downloads).toEqual([]);
    expect(prove(directory, { product: 'ops', tag: 'tmt-ops-v0.1.0-alpha.1' })).toEqual({
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
      skill: 'release-source/skills/tmux-team/SKILL.md',
      sourceRoot: '/candidate-source',
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
    expect(value(args, '--skill')).toBe('release-source/skills/tmux-team/SKILL.md');
    expect(value(args, '--source-root')).toBe('/candidate-source');
  });

  describe('real-archive adapter acceptance', () => {
    const binary = '/task-owned/tmt-adapters-tests';
    const compiled = JSON.stringify({
      reason: 'compiler-artifact',
      target: { name: 'tmt_adapters' },
      profile: { test: true },
      executable: binary,
    });
    const listed = `${ACCEPTANCE_TEST}: test\n\n1 test, 0 benchmarks\n`;
    const passed =
      'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 42 filtered out; finished in 0.01s\n';
    const input = () => ({
      directory: fetchInto('cli', 'v5.0.0-alpha.9').directory,
      product: 'cli',
      tag: 'v5.0.0-alpha.9',
      target: TARGET,
      sourceRoot: undefined,
      environment: { CARGO_TARGET_DIR: '/shared/task-target' },
    });

    it('compiles the adapter only, binds verified archives, and executes exactly the ignored test', () => {
      const calls: { executable: string; args: string[]; env: NodeJS.ProcessEnv; cwd: string }[] =
        [];
      const fixture = input();
      const result = proveArchiveAcceptance({
        ...fixture,
        execute: (executable, args, options) => {
          calls.push({ executable, args, env: options.env, cwd: options.cwd });
          return [compiled, listed, passed][calls.length - 1];
        },
      });
      expect(result).toEqual({ outcome: 'proved' });
      expect(calls).toHaveLength(3);
      expect(calls[0].executable).toBe('cargo');
      expect(calls[0].args).toContain('--no-run');
      expect(calls[0].args).toContain('tmt-adapters');
      expect(calls[0].args).toContain('Cargo.toml');
      expect(path.basename(calls[0].cwd)).toBe('rust');
      expect(calls[1].cwd).toBe(calls[0].cwd);
      expect(calls[2].cwd).toBe(calls[0].cwd);
      expect(calls[0].args).not.toContain('--release');
      expect(calls[0].env.CARGO_BUILD_JOBS).toBeUndefined();
      expect(calls[0].env.CARGO_TARGET_DIR).toBe('/shared/task-target');
      expect(calls[0].env.TMT_UPGRADE_OLD_ARCHIVE).toBe(
        path.join(fixture.directory, TARGET, 'previous', `tmt-cli-${TARGET}.tar.gz`)
      );
      expect(calls[0].env.TMT_UPGRADE_NEW_MANIFEST).toBe(
        path.join(fixture.directory, TARGET, 'candidate', 'dist-manifest.json')
      );
      expect(calls[1].executable).toBe(binary);
      expect(calls[1].args).toEqual([ACCEPTANCE_TEST, '--exact', '--ignored', '--list']);
      expect(calls[2].args).toEqual([ACCEPTANCE_TEST, '--exact', '--ignored', '--nocapture']);
    });

    it('preserves an exported local Cargo worker limit', () => {
      const jobs: (string | undefined)[] = [];
      expect(
        proveArchiveAcceptance({
          ...input(),
          environment: { CARGO_BUILD_JOBS: '2' },
          execute: (_executable, _args, options) => {
            jobs.push(options.env.CARGO_BUILD_JOBS);
            return [compiled, listed, passed][jobs.length - 1];
          },
        })
      ).toEqual({ outcome: 'proved' });
      expect(jobs).toEqual(['2', '2', '2']);
    });

    it('compiles and executes the release adapter from sourceRoot on a rerun', () => {
      const sourceRoot = mkdtempSync(path.join(root, 'applicable-release-'));
      const source = path.join(
        sourceRoot,
        'rust/crates/tmt-adapters/src/native_install/upgrade_artifact_tests.rs'
      );
      mkdirSync(path.dirname(source), { recursive: true });
      writeFileSync(
        source,
        'fn cargo_dist_upgrade_refreshes_real_artifacts_and_preserves_conflicts() {}\n' +
          '// skill-content transition: skipped (old and candidate text are identical)'
      );
      const directories: string[] = [];
      expect(
        proveArchiveAcceptance({
          ...input(),
          sourceRoot,
          execute: (_executable, _args, options) => {
            directories.push(options.cwd);
            return [compiled, listed, passed][directories.length - 1];
          },
        })
      ).toEqual({ outcome: 'proved' });
      expect(directories).toEqual(Array(3).fill(path.join(sourceRoot, 'rust')));
    });

    it.each(['', `${compiled}\n${compiled}`])(
      'rejects missing or ambiguous compiled test binaries',
      (output) => {
        expect(() => proveArchiveAcceptance({ ...input(), execute: () => output })).toThrow(
          'Expected exactly one tmt-adapters lib-test executable'
        );
      }
    );

    it.each(['0 tests, 0 benchmarks\n', `${listed}\nother: test\n`])(
      'rejects empty or extra test discovery',
      (listing) => {
        let call = 0;
        expect(() =>
          proveArchiveAcceptance({ ...input(), execute: () => [compiled, listing][call++] })
        ).toThrow('Expected exactly one discovered real-archive upgrade acceptance test');
        expect(call).toBe(2);
      }
    );

    it.each([
      'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 42 filtered out;',
      'test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 42 filtered out;',
    ])('refuses a successful process that executed no acceptance test', (output) => {
      let call = 0;
      expect(() =>
        proveArchiveAcceptance({ ...input(), execute: () => [compiled, listed, output][call++] })
      ).toThrow('Expected exactly one passing executed real-archive upgrade acceptance test');
    });

    it('preserves compiler and test execution failures', () => {
      for (const failingCall of [0, 2]) {
        let call = 0;
        expect(() =>
          proveArchiveAcceptance({
            ...input(),
            execute: () => {
              if (call === failingCall) throw new Error('owned process failed');
              return [compiled, listed, passed][call++];
            },
          })
        ).toThrow('owned process failed');
      }
    });

    it('rechecks staged digests before any compile or historical exemption', () => {
      const fixture = input();
      writeFileSync(
        path.join(fixture.directory, TARGET, 'candidate', 'dist-manifest.json'),
        'corrupted'
      );
      let calls = 0;
      expect(() =>
        proveArchiveAcceptance({
          ...fixture,
          sourceRoot: '/missing-source',
          execute: () => {
            calls++;
            return '';
          },
        })
      ).toThrow('does not match its recorded digest');
      expect(calls).toBe(0);
    });

    it('reports predates as not applicable only for the release-source checkout on a rerun', () => {
      const sourceRoot = mkdtempSync(path.join(root, 'historical-source-'));
      const messages: string[] = [];
      let calls = 0;
      expect(
        proveArchiveAcceptance({
          ...input(),
          sourceRoot,
          execute: () => {
            calls++;
            return '';
          },
          report: (message) => messages.push(message),
        })
      ).toEqual({ outcome: 'predates' });
      expect(calls).toBe(0);
      expect(messages.join('\n')).toMatch(/predates; not applicable/);
      expect(messages.join('\n')).not.toContain('acceptance: passed');
      expect(acceptanceApplicability(sourceRoot)).toBe('predates');
      const source = path.join(
        sourceRoot,
        'rust/crates/tmt-adapters/src/native_install/upgrade_artifact_tests.rs'
      );
      mkdirSync(path.dirname(source), { recursive: true });
      writeFileSync(source, 'fn unrelated_test() {}');
      expect(acceptanceApplicability(sourceRoot)).toBe('predates');
      writeFileSync(
        source,
        'fn cargo_dist_upgrade_refreshes_real_artifacts_and_preserves_conflicts() { assert_ne!(new_skill, old_skill); }'
      );
      expect(acceptanceApplicability(sourceRoot)).toBe('predates');
      const legacyMessages: string[] = [];
      expect(
        proveArchiveAcceptance({
          ...input(),
          sourceRoot,
          execute: () => {
            throw new Error('A pre-#575 test must never compile.');
          },
          report: (message) => legacyMessages.push(message),
        })
      ).toEqual({ outcome: 'predates' });
      expect(legacyMessages.join('\n')).toMatch(/predates; not applicable/);
      expect(legacyMessages.join('\n')).not.toContain('acceptance: passed');
      writeFileSync(
        source,
        'fn cargo_dist_upgrade_refreshes_real_artifacts_and_preserves_conflicts() {}\n' +
          '// skill-content transition: skipped (old and candidate text are identical)'
      );
      expect(acceptanceApplicability(sourceRoot)).toBe('applicable');
      expect(() => acceptanceApplicability(path.join(sourceRoot, 'missing-checkout'))).toThrow(
        'release-source checkout is missing'
      );
    });
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
    expect(asked).toEqual(
      PROOF_FILES.filter((file) => file !== 'verify-native-driver-upgrade.mjs').map(
        (file) => `typescript/scripts/${file}`
      )
    );
    expect(
      assessUpgrade({
        plan: { product: 'driver-herdr', previous: 'tmt-driver-herdr-v0.1.0-alpha.1' },
        hasFileAt: (file) => !file.endsWith('verify-native-driver-upgrade.mjs'),
      }).outcome
    ).toBe('predates');
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
          'AssertionError [ERR_ASSERTION]: Packed command failed (exited 1, expected 0): tmt extension install ops: unrecognized subcommand ops\ncommand: x'
        )
      )
    ).toBe(
      'Packed command failed (exited 1, expected 0): tmt extension install ops: unrecognized subcommand ops'
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
    const ops = trace('Error: no ops command');
    expect(
      combineFailures([
        { target: 'x86_64-unknown-linux-musl', log: ops },
        { target: 'aarch64-apple-darwin', log: ops },
        { target: 'x86_64-apple-darwin', log: trace('Error: a downgrade was accepted') },
      ])
    ).toBe(
      'no ops command (aarch64-apple-darwin, x86_64-unknown-linux-musl); a downgrade was accepted (x86_64-apple-darwin)'
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
    const run = fakeGh([release('tmt-ops-v0.1.0-alpha.1', { draft: true })]);
    const directory = path.join(root, 'none');
    const fetched = run([
      'fetch',
      '--product',
      'ops',
      '--tag',
      'tmt-ops-v0.1.0-alpha.1',
      '--directory',
      directory,
    ]);
    expect(fetched.status).toBe(0);
    expect(fetched.stderr).toContain('No published ops release precedes tmt-ops-v0.1.0-alpha.1');
    expect(JSON.parse(readFileSync(path.join(directory, 'plan.json'), 'utf8')).previous).toBeNull();

    // `prove` runs the release's own code: it must not need gh, a token or a repository.
    const offline = run(
      [
        'prove',
        '--product',
        'ops',
        '--tag',
        'tmt-ops-v0.1.0-alpha.1',
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
      'Usage: release-upgrade.mjs resolve|fetch|assess|prove|acceptance|reason'
    );
  });
});

describe('REST evidence for a published CLI tag', () => {
  const registration = '3'.repeat(40);
  const driverSha = '8'.repeat(40);
  const published = release('v5.0.0-alpha.84', { sha: '9'.repeat(40) });
  const comparison = (status: string, mergeBase = registration) => ({
    status,
    base_commit: { sha: registration },
    merge_base_commit: { sha: mergeBase },
  });
  it.each([
    ['ahead', driverSha, registration],
    ['identical', registration, registration],
    ['behind', driverSha, driverSha],
    ['diverged', driverSha, '2'.repeat(40)],
  ])('reads exact tag commit then checks %s comparison evidence', (status, sha, mergeBase) => {
    const calls: { command: string; args: string[]; options: object }[] = [];
    const observe = ghCliAncestry({
      repository: 'fixture/repository',
      spawn: (command, args, options) => {
        calls.push({ command, args, options });
        return {
          status: 0,
          stdout: JSON.stringify(calls.length === 1 ? { sha } : comparison(status, mergeBase)),
        };
      },
    });
    expect(observe(published, registration)).toEqual({ sha, status });
    expect(calls.map((c) => [c.command, ...c.args])).toEqual([
      ['gh', 'api', 'repos/fixture/repository/commits/v5.0.0-alpha.84'],
      ['gh', 'api', `repos/fixture/repository/compare/${registration}...${sha}`],
    ]);
    expect(calls.map((c) => c.options)).toEqual([
      expect.objectContaining({ timeout: 60_000, maxBuffer: 4 * 1024 * 1024 }),
      expect.objectContaining({ timeout: 60_000, maxBuffer: 4 * 1024 * 1024 }),
    ]);
  });
  it.each([
    ['unknown status', comparison('unknown')],
    ['wrong base', { ...comparison('ahead'), base_commit: { sha: driverSha } }],
    ['wrong merge base', comparison('ahead', driverSha)],
    ['ancestor evidence absent', { status: 'behind', base_commit: { sha: registration } }],
    ['identical mismatch', comparison('identical')],
  ])('refuses %s', (_, response) => {
    let calls = 0;
    const observe = ghCliAncestry({
      repository: 'fixture/repository',
      spawn: () => ({
        status: 0,
        stdout: JSON.stringify(++calls === 1 ? { sha: driverSha } : response),
      }),
    });
    expect(() => observe(published, registration)).toThrow('Unknown REST CLI ancestry');
    expect(calls).toBe(2);
  });
  it.each(['tag', 'compare'])('refuses %s REST failure without a fallback', (phase) => {
    let calls = 0;
    const observe = ghCliAncestry({
      repository: 'fixture/repository',
      spawn: () => {
        calls += 1;
        return phase === 'tag' || calls === 2
          ? { status: 1, stdout: '' }
          : { status: 0, stdout: JSON.stringify({ sha: driverSha }) };
      },
    });
    expect(() => observe(published, registration)).toThrow('REST read failed');
    expect(calls).toBe(phase === 'tag' ? 1 : 2);
  });
  it.each(['main', [driverSha], null])(
    'refuses invalid tag commit %j and never asks compare',
    (sha) => {
      let calls = 0;
      const observe = ghCliAncestry({
        repository: 'fixture/repository',
        spawn: () => {
          calls += 1;
          return { status: 0, stdout: JSON.stringify({ sha }) };
        },
      });
      expect(() => observe(published, registration)).toThrow('no resolved commit');
      expect(calls).toBe(1);
    }
  );
  it('refuses draft/unknown publication and malformed registration before REST', () => {
    const observe = ghCliAncestry({
      repository: 'fixture/repository',
      spawn: () => {
        throw new Error('must not call REST');
      },
    });
    expect(() => observe({ ...published, draft: true }, registration)).toThrow('published release');
    expect(() =>
      observe({ ...published, draft: undefined } as unknown as DraftRelease, registration)
    ).toThrow('published release');
    expect(() => observe(published, 'main')).toThrow('registration SHA');
  });
  it('retains a subprocess error as unknown ancestry rather than using target_commitish', () => {
    const observe = ghCliAncestry({
      repository: 'fixture/repository',
      spawn: () => ({ error: new Error('request unavailable'), status: null, stdout: '' }),
    });
    expect(() => observe(published, registration)).toThrow('request unavailable');
  });
});
