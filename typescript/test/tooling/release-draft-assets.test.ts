import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import {
  attachBundle,
  bundleFiles,
  checkDraft,
  ghApi,
  recordFailure,
  type DraftRelease,
  type ReleaseApi,
} from '../../scripts/release-draft-assets.mjs';

const script = fileURLToPath(new URL('../../scripts/release-draft-assets.mjs', import.meta.url));
const targets = [
  'aarch64-apple-darwin',
  'aarch64-unknown-linux-musl',
  'x86_64-apple-darwin',
  'x86_64-unknown-linux-musl',
];
const digestOf = (text: string) => `sha256:${createHash('sha256').update(text).digest('hex')}`;

let root: string;
beforeAll(() => {
  root = mkdtempSync(path.join(os.tmpdir(), 'release-draft-assets-'));
});
afterAll(() => rmSync(root, { recursive: true, force: true }));

/** A verified bundle as the assemble job leaves it: archives, final manifest, installers, marker. */
function bundle(product: string, tag: string) {
  const directory = mkdtempSync(path.join(root, 'bundle-'));
  const prefix = product === 'cli' ? 'tmt-cli' : `tmt-${product}`;
  const artifacts: Record<string, { kind: string }> = {};
  for (const target of targets) {
    artifacts[`${prefix}-${target}.tar.gz`] = { kind: 'executable-zip' };
    artifacts[`${prefix}-${target}.tar.gz.sha256`] = { kind: 'checksum' };
    writeFileSync(path.join(directory, `${prefix}-${target}.tar.gz`), `archive ${target}`);
  }
  artifacts['sha256.sum'] = { kind: 'unified-checksum' };
  writeFileSync(
    path.join(directory, 'dist-manifest.json'),
    JSON.stringify({ announcement_tag: tag, artifacts })
  );
  if (product === 'cli') {
    writeFileSync(path.join(directory, 'tmt-installer.sh'), '#!/bin/sh\n');
    writeFileSync(path.join(directory, 'install.sh'), '#!/bin/sh\n');
  }
  const policy = {
    cli: { tagPrefix: 'v', prerelease: false, latest: true, flags: ['--latest=true'] },
    office: {
      tagPrefix: 'tmt-office-v',
      prerelease: true,
      latest: false,
      flags: ['--prerelease', '--latest=false'],
    },
    squad: {
      tagPrefix: 'tmt-squad-v',
      prerelease: true,
      latest: false,
      flags: ['--prerelease', '--latest=false'],
    },
  }[product];
  writeFileSync(
    path.join(directory, 'release-publication.json'),
    `${JSON.stringify({ product, ...policy })}\n`
  );
  // Files that stay in the bundle directory but are never attached.
  writeFileSync(path.join(directory, 'sha256.sum'), 'sums');
  writeFileSync(path.join(directory, 'x86_64-apple-darwin-notices.txt'), 'notices');
  return directory;
}

/** An asset as the fake store keeps it: its digest can be withheld. */
interface StoredAsset {
  id: number;
  name: string;
  digest?: string | null;
}

interface Fake {
  api: ReleaseApi;
  releases: DraftRelease[];
  calls: string[];
}

/** An in-memory release store; uploads record the digest of the file's bytes unless corrupted. */
function fakeApi(
  releases: DraftRelease[],
  options: { corrupt?: string; withheldDigests?: number } = {}
): Fake {
  const calls: string[] = [];
  let nextId = 100;
  let withheld = options.withheldDigests ?? 0;
  const mutable = releases as (Omit<DraftRelease, 'assets'> & { assets: StoredAsset[] })[];
  const api: ReleaseApi = {
    listReleases: () => {
      const listed = JSON.parse(JSON.stringify(mutable)) as typeof mutable;
      if (withheld > 0) {
        withheld -= 1;
        for (const release of listed) for (const asset of release.assets) asset.digest = null;
      }
      return listed;
    },
    upload: (release, name, file) => {
      calls.push(`upload ${name}`);
      const target = mutable.find(({ id }) => id === release.id);
      const bytes = readFileSync(file, 'utf8');
      const digest = options.corrupt === name ? digestOf(`${bytes} corrupted`) : digestOf(bytes);
      target?.assets.push({ id: (nextId += 1), name, digest });
    },
    deleteAsset: (id) => {
      calls.push(`delete ${id}`);
      for (const release of mutable) release.assets = release.assets.filter((a) => a.id !== id);
    },
  };
  return { api, releases: mutable, calls };
}

const draft = (
  tag: string,
  assets: string[] = [],
  overrides: Partial<DraftRelease> = {}
): DraftRelease => ({
  id: 1,
  draft: true,
  tag_name: tag,
  target_commitish: 'a'.repeat(40),
  created_at: '2026-09-30T01:00:00Z',
  assets: assets.map((name, index) => ({ id: 10 + index, name, digest: 'sha256:old' })),
  ...overrides,
});

describe('bundleFiles', () => {
  const manifest = (tag: string, product = 'cli') =>
    JSON.parse(readFileSync(path.join(bundle(product, tag), 'dist-manifest.json'), 'utf8'));

  it('lists the four archives and the manifest, plus both installers for the CLI', () => {
    expect(bundleFiles('cli', manifest('v5.0.0-alpha.9'), 'v5.0.0-alpha.9')).toEqual([
      ...targets.map((target) => `tmt-cli-${target}.tar.gz`),
      'dist-manifest.json',
      'tmt-installer.sh',
      'install.sh',
    ]);
    expect(
      bundleFiles(
        'squad',
        manifest('tmt-squad-v0.1.0-alpha.2', 'squad'),
        'tmt-squad-v0.1.0-alpha.2'
      )
    ).toEqual([...targets.map((target) => `tmt-squad-${target}.tar.gz`), 'dist-manifest.json']);
  });

  it('refuses a manifest for another tag, a missing target and a foreign archive', () => {
    expect(() => bundleFiles('cli', manifest('v5.0.0-alpha.8'), 'v5.0.0-alpha.9')).toThrow(
      'The bundle is for v5.0.0-alpha.8, not for v5.0.0-alpha.9.'
    );
    const three = manifest('v5.0.0-alpha.9');
    delete three.artifacts['tmt-cli-x86_64-apple-darwin.tar.gz'];
    expect(() => bundleFiles('cli', three, 'v5.0.0-alpha.9')).toThrow('found 3');
    expect(() =>
      bundleFiles(
        'office',
        manifest('tmt-office-v0.1.0-alpha.4', 'squad'),
        'tmt-office-v0.1.0-alpha.4'
      )
    ).toThrow('Unexpected archive');
  });
});

describe('checkDraft', () => {
  it('asks for a build when the draft has neither a bundle nor a recorded failure', () => {
    const { api } = fakeApi([draft('v5.0.0-alpha.9', ['partial.tar.gz'])]);
    expect(checkDraft({ api, tag: 'v5.0.0-alpha.9' })).toEqual({ todo: true, reason: '' });
  });

  it('skips a draft that already carries the bundle', () => {
    const { api, calls } = fakeApi([draft('v5.0.0-alpha.9', ['release-publication.json'])]);
    expect(checkDraft({ api, tag: 'v5.0.0-alpha.9', retry: true }).todo).toBe(false);
    expect(calls).toEqual([]);
  });

  it('skips a parked draft, and a retry removes only the failure marker and takes it back', () => {
    const { api, calls, releases } = fakeApi([
      draft('v5.0.0-alpha.9', ['verification-failed.json', 'keep.txt']),
    ]);
    expect(checkDraft({ api, tag: 'v5.0.0-alpha.9' }).todo).toBe(false);
    expect(calls).toEqual([]);
    expect(checkDraft({ api, tag: 'v5.0.0-alpha.9', retry: true }).todo).toBe(true);
    expect(calls).toEqual(['delete 10']);
    expect(releases[0].assets?.map(({ name }) => name)).toEqual(['keep.txt']);
  });

  it('refuses a published release and an unknown tag', () => {
    const { api } = fakeApi([draft('v5.0.0-alpha.9', [], { draft: false })]);
    expect(() => checkDraft({ api, tag: 'v5.0.0-alpha.9' })).toThrow('is published');
    expect(() => checkDraft({ api, tag: 'v9.9.9' })).toThrow('There is no release v9.9.9.');
  });
});

describe('attachBundle', () => {
  const attach = (fake: Fake, directory: string, tag = 'v5.0.0-alpha.9', product = 'cli') =>
    attachBundle({ api: fake.api, product, tag, directory, sleep: () => {} });

  it('uploads the CLI files in order, checks their digests, and the marker last', () => {
    const fake = fakeApi([draft('v5.0.0-alpha.9')]);
    const { uploaded } = attach(fake, bundle('cli', 'v5.0.0-alpha.9'));
    const expected = [
      ...targets.map((target) => `tmt-cli-${target}.tar.gz`),
      'dist-manifest.json',
      'tmt-installer.sh',
      'install.sh',
      'release-publication.json',
    ];
    expect(uploaded).toEqual(expected);
    expect(fake.calls).toEqual(expected.map((name) => `upload ${name}`));
    expect(fake.releases[0].assets?.map(({ name }) => name)).toEqual(expected);
  });

  it('attaches an extension bundle without installers, under its own tag', () => {
    const fake = fakeApi([draft('tmt-office-v0.1.0-alpha.4')]);
    const { uploaded } = attach(
      fake,
      bundle('office', 'tmt-office-v0.1.0-alpha.4'),
      'tmt-office-v0.1.0-alpha.4',
      'office'
    );
    expect(uploaded).toHaveLength(6);
    expect(uploaded.at(-1)).toBe('release-publication.json');
    expect(uploaded).not.toContain('install.sh');
  });

  it('replaces the stale files of an interrupted upload instead of failing on them', () => {
    const fake = fakeApi([draft('v5.0.0-alpha.9', ['tmt-cli-aarch64-apple-darwin.tar.gz'])]);
    attach(fake, bundle('cli', 'v5.0.0-alpha.9'));
    expect(fake.calls.slice(0, 2)).toEqual([
      'delete 10',
      'upload tmt-cli-aarch64-apple-darwin.tar.gz',
    ]);
    expect(
      fake.releases[0].assets?.filter(({ name }) => name === 'tmt-cli-aarch64-apple-darwin.tar.gz')
    ).toHaveLength(1);
  });

  it('never uploads the marker when a stored file does not match the local bytes', () => {
    const fake = fakeApi([draft('v5.0.0-alpha.9')], { corrupt: 'install.sh' });
    expect(() => attach(fake, bundle('cli', 'v5.0.0-alpha.9'))).toThrow('install.sh was stored as');
    expect(fake.calls).not.toContain('upload release-publication.json');
  });

  it('waits for digests GitHub has not computed yet, and gives up with the missing names', () => {
    const slow = fakeApi([draft('v5.0.0-alpha.9')], { withheldDigests: 2 });
    expect(attach(slow, bundle('cli', 'v5.0.0-alpha.9')).uploaded.at(-1)).toBe(
      'release-publication.json'
    );
    const never = fakeApi([draft('v5.0.0-alpha.9')], { withheldDigests: 99 });
    expect(() => attach(never, bundle('cli', 'v5.0.0-alpha.9'))).toThrow('reports no digest for');
    expect(never.calls).not.toContain('upload release-publication.json');
  });

  it('refuses a published release, a draft that is already complete, and a product that is not the tag', () => {
    const directory = bundle('cli', 'v5.0.0-alpha.9');
    expect(() =>
      attach(fakeApi([draft('v5.0.0-alpha.9', [], { draft: false })]), directory)
    ).toThrow('is published');
    expect(() =>
      attach(fakeApi([draft('v5.0.0-alpha.9', ['release-publication.json'])]), directory)
    ).toThrow('already carries a bundle');
    expect(() =>
      attach(fakeApi([draft('v5.0.0-alpha.9')]), directory, 'v5.0.0-alpha.9', 'office')
    ).toThrow('is not a office tag');
  });

  it('refuses a bundle for another tag, a marker that is not the policy, and a missing file', () => {
    const fake = fakeApi([draft('v5.0.0-alpha.9')]);
    expect(() => attach(fake, bundle('cli', 'v5.0.0-alpha.8'))).toThrow(
      'The bundle is for v5.0.0-alpha.8'
    );
    const wrongMarker = bundle('cli', 'v5.0.0-alpha.9');
    writeFileSync(
      path.join(wrongMarker, 'release-publication.json'),
      '{"product":"cli","latest":false}\n'
    );
    expect(() => attach(fake, wrongMarker)).toThrow('does not match the cli publication policy');
    const missing = bundle('cli', 'v5.0.0-alpha.9');
    rmSync(path.join(missing, 'install.sh'));
    expect(() => attach(fake, missing)).toThrow('The bundle has no install.sh.');
    expect(fake.calls).toEqual([]);
  });
});

describe('recordFailure', () => {
  const failure = (fake: Fake) =>
    recordFailure({
      api: fake.api,
      tag: 'v5.0.0-alpha.9',
      runUrl: 'https://github.com/wkh237/tmt/actions/runs/1',
      sha: 'a'.repeat(40),
      jobs: ['Verify final cli bundle (x86_64-apple-darwin)'],
      now: new Date('2026-09-30T02:00:00Z'),
    });

  it('uploads what failed and where, replacing an older record', () => {
    const fake = fakeApi([draft('v5.0.0-alpha.9', ['verification-failed.json'])]);
    expect(failure(fake)).toEqual({ recorded: true });
    expect(fake.calls).toEqual(['delete 10', 'upload verification-failed.json']);
  });

  it('records nothing on a draft that already carries the bundle', () => {
    const fake = fakeApi([draft('v5.0.0-alpha.9', ['release-publication.json'])]);
    expect(failure(fake)).toEqual({ recorded: false });
    expect(fake.calls).toEqual([]);
  });

  it('refuses a published release', () => {
    expect(() => failure(fakeApi([draft('v5.0.0-alpha.9', [], { draft: false })]))).toThrow(
      'is published'
    );
  });

  it('writes the run URL, commit, failed jobs and time into the uploaded file', () => {
    let written = '';
    const fake = fakeApi([draft('v5.0.0-alpha.9')]);
    const api: ReleaseApi = {
      ...fake.api,
      upload: (release, name, file) => {
        written = readFileSync(file, 'utf8');
        return fake.api.upload(release, name, file);
      },
    };
    recordFailure({
      api,
      tag: 'v5.0.0-alpha.9',
      runUrl: 'https://github.com/wkh237/tmt/actions/runs/1',
      sha: 'a'.repeat(40),
      jobs: ['a', 'b'],
      now: new Date('2026-09-30T02:00:00Z'),
    });
    expect(JSON.parse(written)).toEqual({
      tag: 'v5.0.0-alpha.9',
      sha: 'a'.repeat(40),
      runUrl: 'https://github.com/wkh237/tmt/actions/runs/1',
      failedJobs: ['a', 'b'],
      recordedAt: '2026-09-30T02:00:00.000Z',
    });
  });
});

describe('ghApi', () => {
  const runner = (stdout: string, status = 0, stderr = '') => {
    const seen: string[][] = [];
    const spawn = (_command: string, args: readonly string[]) => {
      seen.push([...args]);
      return { status, stdout, stderr };
    };
    return { seen, spawn };
  };

  it('lists releases across the pages gh slurps', () => {
    const { seen, spawn } = runner(JSON.stringify([[draft('a')], [draft('b')]]));
    const api = ghApi({ repository: 'wkh237/tmt', spawn });
    expect(api.listReleases().map((release) => release.tag_name)).toEqual(['a', 'b']);
    expect(seen[0]).toEqual(['api', '--paginate', '--slurp', 'repos/wkh237/tmt/releases']);
  });

  it('uploads through the uploads host with the file as the body, and deletes by asset id', () => {
    const { seen, spawn } = runner('{}');
    const api = ghApi({ repository: 'wkh237/tmt', spawn });
    api.upload({ ...draft('v1'), id: 7 }, 'a b.tar.gz', '/tmp/file');
    api.deleteAsset(9);
    expect(seen[0]).toEqual([
      'api',
      '--method',
      'POST',
      '-H',
      'Content-Type: application/octet-stream',
      'https://uploads.github.com/repos/wkh237/tmt/releases/7/assets?name=a%20b.tar.gz',
      '--input',
      '/tmp/file',
    ]);
    expect(seen[1]).toEqual(['api', '--method', 'DELETE', 'repos/wkh237/tmt/releases/assets/9']);
  });

  it('reports the status and stderr of a failed call', () => {
    const { spawn } = runner('', 1, 'HTTP 404');
    expect(() => ghApi({ repository: 'wkh237/tmt', spawn }).deleteAsset(1)).toThrow(
      'failed with 1: HTTP 404'
    );
  });
});

describe('release-draft-assets.mjs', () => {
  it('checks a draft through gh and writes the answer to the step output', () => {
    const directory = mkdtempSync(path.join(root, 'cli-'));
    const bin = path.join(directory, 'bin');
    mkdirSync(bin);
    writeFileSync(
      path.join(bin, 'gh'),
      `#!/bin/sh\necho '${JSON.stringify([[draft('v5.0.0-alpha.9', ['verification-failed.json'])]])}'\n`
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
    const parked = run(['check', '--tag', 'v5.0.0-alpha.9']);
    expect(parked.status).toBe(0);
    expect(readFileSync(output, 'utf8')).toBe('todo=false\n');
    expect(parked.stderr).toContain('skipped, its failure is recorded');
    const unknown = run(['check', '--tag', 'v9.9.9']);
    expect(unknown.status).toBe(1);
    expect(unknown.stderr).toContain('There is no release v9.9.9.');
    expect(run(['bogus', '--tag', 'x']).stderr).toContain('Usage: release-draft-assets.mjs');
  });
});
