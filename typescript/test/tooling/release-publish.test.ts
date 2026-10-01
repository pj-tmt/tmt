import path from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  checkPublishedRelease,
  publishBlocker,
  publishDraft,
  renderFailureIssue,
  renderVerifySummary,
  reportFailure,
  verifyPublication,
  type CheckResult,
  type Outcome,
  type PublishApi,
  type PublishedApi,
  type PublishedRelease,
} from '../../scripts/release-publish.mjs';
import type { DraftRelease } from '../../scripts/release-draft-assets.mjs';

const SHA = 'a'.repeat(40);
const TAG = 'v5.0.0-alpha.9';
const EXTENSION_TAG = 'tmt-squad-v0.1.0-alpha.2';
const assets = (...names: string[]) => names.map((name, index) => ({ id: index + 1, name }));

function draft(tag: string, names: string[], overrides: Partial<DraftRelease> = {}): DraftRelease {
  return {
    id: 1,
    draft: true,
    tag_name: tag,
    target_commitish: SHA,
    created_at: '2026-09-30T01:00:00Z',
    assets: assets(...names),
    ...overrides,
  };
}

describe('publishBlocker', () => {
  const complete = draft(TAG, ['release-publication.json', 'dist-manifest.json']);

  it('lets a complete draft without a hold or a failure marker be published', () => {
    expect(publishBlocker({ release: complete, product: 'cli', tag: TAG })).toBe('');
  });

  it.each([
    ['a tag of another product', complete, 'office', TAG, 'is not a office release tag'],
    ['no such release', undefined, 'cli', TAG, 'there is no release'],
    ['a published release', { ...complete, draft: false }, 'cli', TAG, 'already published'],
    [
      'a draft without its bundle',
      draft(TAG, ['partial.tar.gz']),
      'cli',
      TAG,
      'no verified bundle',
    ],
    [
      'a held draft',
      draft(TAG, ['release-publication.json', 'publication-held.json']),
      'cli',
      TAG,
      'is held',
    ],
    [
      'a draft with a recorded failure',
      draft(TAG, ['release-publication.json', 'verification-failed.json']),
      'cli',
      TAG,
      'recorded failure',
    ],
    [
      'a stable release',
      draft('v5.0.0', ['release-publication.json']),
      'cli',
      'v5.0.0',
      'not an alpha release',
    ],
    [
      'a beta release',
      draft('tmt-squad-v0.1.0-beta.1', ['release-publication.json']),
      'squad',
      'tmt-squad-v0.1.0-beta.1',
      'not an alpha release',
    ],
    [
      'a release candidate',
      draft('v5.0.0-rc.1', ['release-publication.json']),
      'cli',
      'v5.0.0-rc.1',
      'not an alpha release',
    ],
    [
      'a draft whose target is a branch',
      draft(TAG, ['release-publication.json'], { target_commitish: 'main' }),
      'cli',
      TAG,
      'is not a commit',
    ],
  ])('refuses %s', (_name, release, product, tag, message) => {
    expect(publishBlocker({ release, product, tag })).toContain(message);
  });

  it('refuses a complete alpha draft of a component that is not released', () => {
    expect(publishBlocker({ release: complete, product: 'cli', tag: TAG, released: false })).toBe(
      'cli is not released (release: false in .github/components.json)'
    );
  });

  it('judges the tag before the draft, so a stable release is never reported as merely unbundled', () => {
    expect(
      publishBlocker({ release: draft('v5.0.0', []), product: 'cli', tag: 'v5.0.0' })
    ).toContain('not an alpha release');
  });
});

describe('publishDraft', () => {
  function fakeApi(releases: DraftRelease[]) {
    const published: { tag: string; flags: readonly string[] }[] = [];
    const api: PublishApi = {
      listReleases: () => releases,
      upload: () => {
        throw new Error('nothing is uploaded');
      },
      deleteAsset: () => {
        throw new Error('nothing is deleted');
      },
      publish: (tag, flags) => {
        published.push({ tag, flags });
      },
    };
    return { api, published };
  }

  it.each([
    ['cli', TAG, ['--draft=false', '--prerelease=false', '--latest=true']],
    ['squad', EXTENSION_TAG, ['--draft=false', '--prerelease=true', '--latest=false']],
  ])('publishes a %s draft with the flags of its policy, once', (product, tag, flags) => {
    const { api, published } = fakeApi([
      draft('v5.0.0-alpha.8', ['release-publication.json']),
      draft(tag, ['release-publication.json']),
    ]);
    expect(publishDraft({ api, product, tag })).toEqual({ flags });
    expect(published).toEqual([{ tag, flags }]);
  });

  it('publishes nothing for a draft that is held, failed, incomplete or of another product', () => {
    for (const names of [
      ['release-publication.json', 'publication-held.json'],
      ['release-publication.json', 'verification-failed.json'],
      ['dist-manifest.json'],
    ]) {
      const { api, published } = fakeApi([draft(TAG, names)]);
      expect(() => publishDraft({ api, product: 'cli', tag: TAG })).toThrow('Not publishing');
      expect(published).toEqual([]);
    }
    const { api, published } = fakeApi([draft(TAG, ['release-publication.json'])]);
    expect(() => publishDraft({ api, product: 'squad', tag: TAG })).toThrow('Not publishing');
    expect(() => publishDraft({ api, product: 'cli', tag: 'v5.0.0-alpha.99' })).toThrow(
      'there is no release v5.0.0-alpha.99'
    );
    expect(published).toEqual([]);
  });

  it('publishes nothing that is not an alpha release, however complete the draft', () => {
    for (const tag of ['v5.0.0', 'v5.0.0-beta.1', 'v5.0.0-rc.1']) {
      const { api, published } = fakeApi([draft(tag, ['release-publication.json'])]);
      expect(() => publishDraft({ api, product: 'cli', tag })).toThrow('not an alpha release');
      expect(published, tag).toEqual([]);
    }
  });

  it('publishes nothing of a component that is not released, even a complete alpha draft', () => {
    const { api, published } = fakeApi([draft(TAG, ['release-publication.json'])]);
    expect(() => publishDraft({ api, product: 'cli', tag: TAG, released: false })).toThrow(
      'cli is not released'
    );
    expect(published).toEqual([]);
  });
});

const published = (overrides: Partial<PublishedRelease> = {}): PublishedRelease => ({
  draft: false,
  immutable: true,
  prerelease: false,
  tag_name: TAG,
  target_commitish: SHA,
  assets: assets('release-publication.json', 'dist-manifest.json', 'tmt-cli-x.tar.gz'),
  ...overrides,
});

const failures = (results: readonly CheckResult[]) =>
  Object.fromEntries(results.filter(({ ok }) => !ok).map(({ check, reason }) => [check, reason]));

describe('checkPublishedRelease', () => {
  const cli = (overrides: Partial<PublishedRelease> = {}) =>
    checkPublishedRelease({
      release: published(overrides),
      latest: published(),
      tagCommit: SHA,
      product: 'cli',
      tag: TAG,
    });

  it('passes a public, immutable CLI release that is latest, on its commit, with the bundle marker', () => {
    const results = cli();
    expect(results.map(({ check }) => check)).toEqual([
      'published',
      'immutable',
      'flags',
      'tag',
      'bundle',
    ]);
    expect(failures(results)).toEqual({});
  });

  it('fails a release that is still a draft or is not immutable', () => {
    expect(failures(cli({ draft: true }))).toHaveProperty('published');
    expect(failures(cli({ immutable: false })).immutable).toContain('immutability may be off');
    expect(failures(cli({ immutable: undefined }))).toHaveProperty('immutable');
  });

  it('fails a CLI release that stayed a prerelease or is not the latest release', () => {
    expect(failures(cli({ prerelease: true })).flags).toContain('prerelease is true');
    const results = checkPublishedRelease({
      release: published(),
      latest: published({ tag_name: 'v5.0.0-alpha.8' }),
      tagCommit: SHA,
      product: 'cli',
      tag: TAG,
    });
    expect(failures(results).flags).toContain(
      'the latest release is v5.0.0-alpha.8, not v5.0.0-alpha.9'
    );
    const none = checkPublishedRelease({
      release: published(),
      latest: null,
      tagCommit: SHA,
      product: 'cli',
      tag: TAG,
    });
    expect(failures(none).flags).toContain('missing');
  });

  describe('for an extension', () => {
    const extension = (
      latest: PublishedRelease | null,
      overrides: Partial<PublishedRelease> = {}
    ) =>
      checkPublishedRelease({
        release: published({ tag_name: EXTENSION_TAG, prerelease: true, ...overrides }),
        latest,
        tagCommit: SHA,
        product: 'squad',
        tag: EXTENSION_TAG,
      });

    it('passes a prerelease while the latest release is still a CLI release', () => {
      expect(failures(extension(published()))).toEqual({});
    });

    it('fails an extension that is not a prerelease or became the latest release', () => {
      expect(failures(extension(published(), { prerelease: false })).flags).toContain(
        'prerelease is false'
      );
      expect(failures(extension(published({ tag_name: EXTENSION_TAG }))).flags).toContain(
        'became the latest release'
      );
    });

    it('fails when the latest release is not a CLI release or does not exist', () => {
      expect(
        failures(extension(published({ tag_name: 'tmt-office-v0.1.0-alpha.4' }))).flags
      ).toContain('install.sh would not resolve');
      expect(failures(extension(null)).flags).toContain('no latest release');
    });
  });

  it('fails a tag that is not on the release commit, or that cannot be read', () => {
    for (const tagCommit of ['b'.repeat(40), null]) {
      const results = checkPublishedRelease({
        release: published(),
        latest: published(),
        tagCommit,
        product: 'cli',
        tag: TAG,
      });
      expect(failures(results).tag, String(tagCommit)).toContain(`not at ${SHA}`);
    }
  });

  it('fails a release whose bundle marker is missing', () => {
    expect(failures(cli({ assets: assets('dist-manifest.json') })).bundle).toContain(
      'release-publication.json is missing'
    );
  });
});

describe('verifyPublication', () => {
  const ok: Outcome = { ok: true, output: '' };

  function fakeApi(overrides: Partial<PublishedApi> = {}) {
    const calls: string[] = [];
    const api: PublishedApi = {
      getRelease: (tag) => {
        calls.push(`get ${tag}`);
        return published();
      },
      latestRelease: () => published(),
      tagCommit: () => SHA,
      download: (tag, directory) => {
        calls.push(`download ${tag} ${directory}`);
        return ok;
      },
      verifyRelease: (tag) => {
        calls.push(`verify ${tag}`);
        return ok;
      },
      verifyAsset: (tag, file) => {
        calls.push(`verify-asset ${tag} ${file}`);
        return ok;
      },
      ...overrides,
    };
    return { api, calls };
  }
  const verify = (api: PublishedApi, sleeps: number[] = [], attempts = 3) =>
    verifyPublication({
      api,
      product: 'cli',
      tag: TAG,
      directory: '/work/published',
      attempts,
      sleep: (milliseconds) => sleeps.push(milliseconds),
    });

  it('checks the release, its attestation and every asset, without waiting when all is ready', () => {
    const { api, calls } = fakeApi();
    const sleeps: number[] = [];
    const results = verify(api, sleeps);
    expect(results.map(({ check }) => check)).toEqual([
      'published',
      'immutable',
      'flags',
      'tag',
      'bundle',
      'attestation',
      'assets',
    ]);
    expect(failures(results)).toEqual({});
    expect(sleeps).toEqual([]);
    expect(calls).toEqual([
      `get ${TAG}`,
      `verify ${TAG}`,
      `download ${TAG} /work/published`,
      ...['release-publication.json', 'dist-manifest.json', 'tmt-cli-x.tar.gz'].map(
        (name) => `verify-asset ${TAG} ${path.join('/work/published', name)}`
      ),
    ]);
  });

  it('waits and tries again while GitHub has not finished immutability or the attestation', () => {
    let reads = 0;
    let verifications = 0;
    const { api } = fakeApi({
      getRelease: () => published({ immutable: ++reads >= 3 }),
      verifyRelease: () => {
        verifications += 1;
        return verifications >= 2 ? ok : { ok: false, output: 'no attestation yet' };
      },
    });
    const sleeps: number[] = [];
    expect(failures(verify(api, sleeps))).toEqual({});
    expect(sleeps).toEqual([15_000, 15_000, 15_000]);
  });

  it('waits for latest to move to the release it was published as', () => {
    let reads = 0;
    const { api } = fakeApi({
      latestRelease: () => published({ tag_name: ++reads >= 2 ? TAG : 'v5.0.0-alpha.8' }),
    });
    const sleeps: number[] = [];
    expect(failures(verify(api, sleeps))).toEqual({});
    expect(sleeps).toEqual([15_000]);
  });

  it('reports a latest release that never moves, after the bounded attempts', () => {
    const { api } = fakeApi({ latestRelease: () => published({ tag_name: 'v5.0.0-alpha.8' }) });
    const sleeps: number[] = [];
    expect(failures(verify(api, sleeps)).flags).toContain('the latest release is v5.0.0-alpha.8');
    expect(sleeps).toHaveLength(2);
  });

  it('gives up after the bounded attempts and reports what gh said', () => {
    let verifications = 0;
    const { api } = fakeApi({
      verifyRelease: () => {
        verifications += 1;
        return { ok: false, output: 'attestation for v5.0.0-alpha.9 not found' };
      },
    });
    const sleeps: number[] = [];
    const results = verify(api, sleeps);
    expect(verifications).toBe(3);
    expect(sleeps).toHaveLength(2);
    expect(failures(results).attestation).toContain('attestation for v5.0.0-alpha.9 not found');
  });

  it('tries each asset once, names the ones that do not verify and still checks the others', () => {
    const { api, calls } = fakeApi({
      verifyAsset: (_tag, file) =>
        path.basename(file) === 'dist-manifest.json'
          ? { ok: false, output: 'does not contain subject' }
          : ok,
    });
    const results = verify(api, [], 1);
    expect(failures(results).assets).toContain('dist-manifest.json: does not contain subject');
    expect(failures(results).assets).not.toContain('tmt-cli-x.tar.gz');
    expect(calls.filter((call) => call.startsWith('get'))).toHaveLength(1);
  });

  it('fails the assets when they cannot be downloaded, and verifies none', () => {
    const { api, calls } = fakeApi({ download: () => ({ ok: false, output: 'HTTP 502' }) });
    const results = verify(api, [], 1);
    expect(failures(results).assets).toContain('could not be downloaded: HTTP 502');
    expect(calls.some((call) => call.startsWith('verify-asset'))).toBe(false);
  });

  it('reports a release that is not public as the one failure, after the bounded attempts', () => {
    const sleeps: number[] = [];
    const { api } = fakeApi({ getRelease: () => null });
    const results = verify(api, sleeps);
    expect(results).toEqual([
      { check: 'published', ok: false, reason: `there is no published release ${TAG}` },
    ]);
    expect(sleeps).toHaveLength(2);
  });
});

describe('the failure issue', () => {
  const results: CheckResult[] = [
    { check: 'published', ok: true, reason: '' },
    { check: 'immutable', ok: false, reason: 'v5.0.0-alpha.9 is not immutable' },
    { check: 'assets', ok: false, reason: 'gh release verify-asset failed for x: no subject' },
  ];

  it('names the release, only the failed checks and the run, and says nothing was rolled back', () => {
    const { title, body } = renderFailureIssue({
      tag: TAG,
      results,
      runUrl: 'https://github.com/wkh237/tmt/actions/runs/1',
    });
    expect(title).toBe('Release v5.0.0-alpha.9 failed its post-publication checks');
    expect(body).toContain('- `immutable`: v5.0.0-alpha.9 is not immutable');
    expect(body).toContain('- `assets`: gh release verify-asset failed for x: no subject');
    expect(body).not.toContain('`published`');
    expect(body).toContain('Run: https://github.com/wkh237/tmt/actions/runs/1');
    expect(body).toContain('Nothing was rolled back');
    expect(renderFailureIssue({ tag: TAG, results }).body).not.toContain('Run:');
  });

  it('opens one issue per release, and comments when it is already open', () => {
    const calls: string[] = [];
    const api = (open: number | null) => ({
      openIssue: (title: string) => {
        calls.push(`search ${title}`);
        return open;
      },
      createIssue: (title: string) => {
        calls.push(`create ${title}`);
        return 77;
      },
      commentIssue: (number: number) => {
        calls.push(`comment ${number}`);
      },
    });
    expect(reportFailure({ api: api(null), tag: TAG, results })).toEqual({
      issue: 77,
      created: true,
    });
    expect(reportFailure({ api: api(12), tag: TAG, results })).toEqual({
      issue: 12,
      created: false,
    });
    const title = 'Release v5.0.0-alpha.9 failed its post-publication checks';
    expect(calls).toEqual([`search ${title}`, `create ${title}`, `search ${title}`, 'comment 12']);
  });

  it('renders the checks of the run summary', () => {
    const text = renderVerifySummary({ tag: TAG, results });
    expect(text).toContain('### Published release `v5.0.0-alpha.9`');
    expect(text).toContain('- passed `published`');
    expect(text).toContain('- FAILED `immutable`: v5.0.0-alpha.9 is not immutable');
  });
});
