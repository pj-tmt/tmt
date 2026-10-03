import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { parseComponentMap } from '../../scripts/ci-scope.mjs';
import { queuedReleaseNotesCover } from '../../scripts/release-please-queue.mjs';
import { loadReleasePleaseCommitRules } from '../../scripts/release-please-commits.mjs';
import {
  checkReleaseNotes,
  createSafetyReader,
  inspectManifestDrafts,
  verifyReleasePrNotes,
  type SafetyReader,
} from '../../scripts/release-pr-safety.mjs';

const { runPackedCommand } = await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
);

const root = fileURLToPath(new URL('../../../', import.meta.url));
const components = parseComponentMap(
  readFileSync(path.join(root, '.github/components.json'), 'utf8')
).components;
const repository = 'pj-tmt/tmt';
const heldPathsOf = (input: Parameters<typeof inspectManifestDrafts>[0]) =>
  inspectManifestDrafts(input).heldPaths;
const cliTag = 'v5.0.0-alpha.34';
const squadTag = 'tmt-squad-v0.1.0-alpha.8';
const published = (tag_name = cliTag) => ({
  tag_name,
  draft: false,
  published_at: '2026-10-02T01:00:00Z',
});
const pr = (body: string, component = 'tmt-cli') => ({
  head: { ref: `release-please--branches--main--components--${component}` },
  body,
});
const notes = (sha: string, tag = cliTag) =>
  `## [next](https://github.com/${repository}/compare/${tag}...next)\n\n* fix ([abc](https://github.com/${repository}/commit/${sha}))\n`;

async function fixture(
  run: (context: {
    reader: SafetyReader;
    git: (args: string[]) => string;
    commit: (text: string, file?: string) => string;
    old: string;
    anchor: string;
    base: string;
    directory: string;
  }) => void | Promise<void>
) {
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt-release-pr-safety-'));
  const env = { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' };
  const git = (args: string[]) => runPackedCommand('git', args, { cwd: directory, env }).trim();
  const commit = (text: string, file = 'input') => {
    mkdirSync(path.dirname(path.join(directory, file)), { recursive: true });
    writeFileSync(path.join(directory, file), text);
    git(['add', '.']);
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
      text,
    ]);
    return git(['rev-parse', 'HEAD']);
  };
  try {
    git(['init', '--quiet']);
    writeFileSync(
      path.join(directory, 'release-please-config.json'),
      readFileSync(path.join(root, 'release-please-config.json'))
    );
    const old = commit('old history');
    const anchor = commit('published');
    git(['tag', cliTag]);
    git(['tag', squadTag]);
    const base = commit('fix: in range');
    const reader: SafetyReader = {
      repository,
      git,
      get() {
        throw new Error('unexpected REST read');
      },
      list() {
        return [published(), published(squadTag)];
      },
    };
    await run({ reader, git, commit, old, anchor, base, directory });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe('release PR linked-commit gate', () => {
  it('accepts CLI and Squad notes with their own latest published anchors', () =>
    fixture(async ({ reader, base }) => {
      expect(await checkReleaseNotes({ pr: pr(notes(base)), base, components, reader })).toEqual({
        tag: cliTag,
        linkedCommits: 1,
      });
      expect(
        (
          await checkReleaseNotes({
            pr: pr(notes(base, squadTag), 'tmt-squad'),
            base,
            components,
            reader,
          })
        )?.tag
      ).toBe(squadTag);
    }));
  it('catches full-history notes even when the compare URL has the correct base', () =>
    fixture(async ({ reader, old, base }) => {
      await expect(
        async () => await checkReleaseNotes({ pr: pr(notes(old)), base, components, reader })
      ).rejects.toThrow('outside');
    }));
  it('excludes the anchor itself and future commits', () =>
    fixture(async ({ reader, anchor, base, commit }) => {
      const future = commit('future');
      for (const sha of [anchor, future])
        await expect(
          async () => await checkReleaseNotes({ pr: pr(notes(sha)), base, components, reader })
        ).rejects.toThrow('outside');
    }));
  it('rejects an ancestor of the candidate that is not a descendant of the tag', () =>
    fixture(async ({ reader, git, commit, old, base }) => {
      git(['checkout', '--quiet', '--detach', old]);
      writeFileSync(path.join(reader.git(['rev-parse', '--show-toplevel']), 'side'), 'side');
      const side = commit('side branch');
      git(['checkout', '--quiet', '--detach', base]);
      git([
        '-c',
        'user.name=TMT Test',
        '-c',
        'user.email=test@example.invalid',
        '-c',
        'commit.gpgsign=false',
        'merge',
        '--quiet',
        '--no-ff',
        '-m',
        'Merge fixture',
        '-X',
        'ours',
        side,
      ]);
      const head = git(['rev-parse', 'HEAD']);
      await expect(
        async () => await checkReleaseNotes({ pr: pr(notes(side)), base: head, components, reader })
      ).rejects.toThrow('outside');
      expect(
        (await checkReleaseNotes({ pr: pr(notes(base)), base: head, components, reader }))
          ?.linkedCommits
      ).toBe(1);
    }));
  it('uses the published release rather than a newer draft or another component', () =>
    fixture(async ({ reader, base }) => {
      expect(
        (
          await checkReleaseNotes({
            pr: pr(notes(base)),
            base,
            components,
            reader,
            releases: [
              published(),
              published(squadTag),
              { tag_name: 'v5.0.0-alpha.35', draft: true },
            ],
          })
        )?.tag
      ).toBe(cliTag);
      await expect(
        async () =>
          await checkReleaseNotes({
            pr: pr(notes(base, 'v5.0.0-alpha.33')),
            base,
            components,
            reader,
          })
      ).rejects.toThrow('compare base');
    }));
  it('requires linked coverage without imposing duplicate policy or entry counts', () =>
    fixture(async ({ reader, base }) => {
      expect(
        (
          await checkReleaseNotes({
            pr: pr(notes(base) + notes(base).split('\n').at(-2)),
            base,
            components,
            reader,
          })
        )?.linkedCommits
      ).toBe(2);
      await expect(
        checkReleaseNotes({ pr: pr(notes(base).split('\n')[0]), base, components, reader })
      ).rejects.toThrow('COVERAGE');
    }));
  it('skips ordinary PRs without reading releases', () =>
    fixture(async ({ reader, base }) => {
      reader.list = () => {
        throw new Error('no read allowed');
      };
      expect(
        await checkReleaseNotes({ pr: { head: { ref: 'feature' } }, base, components, reader })
      ).toBeNull();
    }));
  it('fails closed on missing anchors, notes, tags and malformed linked data', () =>
    fixture(async ({ reader, base, git }) => {
      for (const body of [
        '',
        notes(base) + notes(base),
        notes('bad'),
        notes(''),
        notes(base).replace(repository + '/commit', 'other/repo/commit'),
      ]) {
        await expect(
          async () => await checkReleaseNotes({ pr: pr(body), base, components, reader })
        ).rejects.toThrow();
      }
      await expect(
        async () =>
          await checkReleaseNotes({ pr: pr(notes(base)), base: 'missing', components, reader })
      ).rejects.toThrow('base SHA');
      await expect(
        async () =>
          await checkReleaseNotes({ pr: pr(notes(base)), base, components, reader, releases: [] })
      ).rejects.toThrow('No published');
      git(['tag', '-d', cliTag]);
      await expect(
        async () => await checkReleaseNotes({ pr: pr(notes(base)), base, components, reader })
      ).rejects.toThrow();
    }));
  it('rejects missing branch and parked or unknown release components', () =>
    fixture(async ({ reader, base }) => {
      for (const pull of [{}, pr(notes(base), 'tmt-office'), pr(notes(base), 'unknown')])
        await expect(
          async () => await checkReleaseNotes({ pr: pull, base, components, reader })
        ).rejects.toThrow();
    }));
});

describe('pinned release-please coverage API', () => {
  it('loads the pinned interfaces and fails loudly if any internal import disappears', () => {
    const rules = loadReleasePleaseCommitRules();
    expect(Object.keys(rules)).toEqual([
      'CommitSplit',
      'CommitExclude',
      'parseConventionalCommits',
      'DefaultChangelogNotes',
    ]);
    for (const missing of ['commit-split.js', 'commit-exclude.js', 'commit.js', 'default.js']) {
      expect(() =>
        loadReleasePleaseCommitRules((specifier) => {
          if (specifier.endsWith(missing)) throw new Error(`Missing ${missing}`);
          return Object.fromEntries(Object.entries(rules));
        })
      ).toThrow('Unsupported release-please commit API');
    }
    expect(() => loadReleasePleaseCommitRules(() => ({}))).toThrow(
      'Unsupported release-please commit API'
    );
  });
});

describe('release notes coverage', () => {
  it('skips a covered queued PR, refreshes a missing late fix, and skips again after regeneration', () =>
    fixture(async ({ reader, git, commit, base, directory }) => {
      const late = commit(
        'fix(core): merged while release is queued',
        'rust/crates/tmt-core/src/lib.rs'
      );
      let body = notes(base);
      const queued = {
        number: 100,
        headRefOid: 'a'.repeat(40),
        headRefName: 'release-please--branches--main--components--tmt-cli',
        baseRefName: 'main',
        isDraft: false,
        headRepository: { nameWithOwner: repository },
        mergeQueueEntry: { id: 'entry' } as { id: string } | null,
        autoMergeRequest: null,
      };
      const execute = (command: string, args: string[]) => {
        if (command === 'git') return reader.git(args);
        if (args[1] === 'graphql')
          return JSON.stringify({
            data: {
              repository: {
                pullRequests: {
                  nodes: [queued],
                  pageInfo: { hasNextPage: false, endCursor: null },
                },
              },
            },
          });
        if (args[1] === `repos/${repository}/pulls/100`)
          return JSON.stringify({
            ...pr(body),
            number: 100,
            state: 'open',
            head: {
              ref: queued.headRefName,
              sha: queued.headRefOid,
              repo: { full_name: repository },
            },
            base: { ref: 'main', repo: { full_name: repository } },
          });
        if (args[1].startsWith(`repos/${repository}/releases?`))
          return JSON.stringify(reader.list('releases'));
        throw new Error('unexpected queue acquisition');
      };
      git(['update-ref', 'refs/remotes/origin/main', late]);
      const options = { repository, token: 'workflow-fixture', cwd: directory };
      // The push-event checkout can lag the main ref fetched by a serialized workflow.
      git(['checkout', '--quiet', '--detach', base]);
      expect(await checkReleaseNotes({ pr: pr(body), base, components, reader })).not.toBeNull();
      expect(await queuedReleaseNotesCover(options, execute)).toBe(false);
      body += notes(late).split('\n').at(-2);
      expect(await queuedReleaseNotesCover(options, execute)).toBe(true);
      git(['checkout', '--quiet', '--detach', late]);
      const prose = commit('docs: no newly releasable content');
      git(['update-ref', 'refs/remotes/origin/main', prose]);
      expect(await queuedReleaseNotesCover(options, execute)).toBe(true);
      body = notes(late, 'v5.0.0-alpha.33');
      expect(await queuedReleaseNotesCover(options, execute)).toBe(false);
      body = notes(base) + notes('f'.repeat(40)).split('\n').at(-2);
      expect(await queuedReleaseNotesCover(options, execute)).toBe(false);
      queued.mergeQueueEntry = null;
      body = '';
      expect(await queuedReleaseNotesCover(options, execute)).toBe(false);
    }));

  it('rejects an unlinked late fix and passes when the refreshed notes cover the range', () =>
    fixture(async ({ reader, commit, base }) => {
      const late = commit(
        'fix(core): late merged change (#102)',
        'rust/crates/tmt-core/src/lib.rs'
      );
      const body = notes(base);
      await expect(
        checkReleaseNotes({ pr: pr(body), base: late, components, reader })
      ).rejects.toThrow(`COVERAGE missing commit(s): ${late}`);
      expect(
        (
          await checkReleaseNotes({
            pr: pr(body + notes(late).split('\n').at(-2)),
            base: late,
            components,
            reader,
          })
        )?.linkedCommits
      ).toBe(2);
    }));

  it('ignores unlinked hidden types and component-excluded paths', () =>
    fixture(async ({ reader, commit, base }) => {
      commit('chore: maintenance');
      commit('ci: maintain checks');
      const tip = commit('fix: another product', 'extensions/tmt-squad/rust/tmt-squad/src/main.rs');
      expect(
        (await checkReleaseNotes({ pr: pr(notes(base)), base: tip, components, reader }))
          ?.linkedCommits
      ).toBe(1);
    }));

  it('ignores another component but requires direct and private-leaf Squad changes', () =>
    fixture(async ({ reader, commit }) => {
      const header = notes('unused', squadTag).split('\n')[0];
      const other = commit('feat: core only', 'rust/crates/tmt-core/src/lib.rs');
      expect(
        (await checkReleaseNotes({ pr: pr(header, 'tmt-squad'), base: other, components, reader }))
          ?.linkedCommits
      ).toBe(0);
      const direct = commit('fix: Squad change', 'extensions/tmt-squad/rust/tmt-squad/src/main.rs');
      const leaf = commit('fix: private TUI change', 'rust/crates/tmt-tui/src/text.rs');
      const body = notes(direct, squadTag);
      await expect(
        checkReleaseNotes({ pr: pr(body, 'tmt-squad'), base: leaf, components, reader })
      ).rejects.toThrow(`COVERAGE missing commit(s): ${leaf}`);
      expect(
        (
          await checkReleaseNotes({
            pr: pr(body + notes(leaf, squadTag).split('\n').at(-2), 'tmt-squad'),
            base: leaf,
            components,
            reader,
          })
        )?.linkedCommits
      ).toBe(2);
    }));

  it('uses configured visible sections and overrides hidden types at the candidate base', () =>
    fixture(async ({ reader, commit, base, directory }) => {
      const config = JSON.parse(
        readFileSync(path.join(directory, 'release-please-config.json'), 'utf8')
      );
      config['changelog-sections'] = [
        { type: 'fix', section: 'Fixes', hidden: true },
        { type: 'refactor', section: 'Refactors', hidden: false },
      ];
      writeFileSync(path.join(directory, 'release-please-config.json'), JSON.stringify(config));
      commit('chore: change global sections');
      const hidden = commit('fix: now hidden');
      expect(
        (await checkReleaseNotes({ pr: pr(notes(base)), base: hidden, components, reader }))
          ?.linkedCommits
      ).toBe(1);
      const visible = commit('refactor: now visible');
      await expect(
        checkReleaseNotes({ pr: pr(notes(base)), base: visible, components, reader })
      ).rejects.toThrow(`COVERAGE missing commit(s): ${visible}`);
      config.packages['.']['changelog-sections'] = [{ type: 'chore', section: 'Maintenance' }];
      writeFileSync(path.join(directory, 'release-please-config.json'), JSON.stringify(config));
      const override = commit('chore: package sections override global');
      await expect(
        checkReleaseNotes({ pr: pr(notes(base)), base: override, components, reader })
      ).rejects.toThrow('COVERAGE');
    }));

  it('includes hidden breaking changes and nested visible commits using the pinned parser', () =>
    fixture(async ({ reader, commit, base }) => {
      const breaking = commit('chore!: incompatible change');
      const nested = commit('chore: maintenance\n\nfix: nested user-visible correction');
      await expect(
        checkReleaseNotes({ pr: pr(notes(base)), base: nested, components, reader })
      ).rejects.toThrow(`COVERAGE missing commit(s): ${nested}, ${breaking}`);
      expect(
        (
          await checkReleaseNotes({
            pr: pr(
              notes(base) +
                [breaking, nested].map((sha) => notes(sha).split('\n').at(-2)).join('\n')
            ),
            base: nested,
            components,
            reader,
          })
        )?.linkedCommits
      ).toBe(3);
    }));

  it('requires coverage at an earlier release squash parent in the cumulative merge group', () =>
    fixture(async ({ reader, git, commit, base }) => {
      git(['update-ref', 'refs/remotes/origin/main', base]);
      const late = commit('fix: late queued feature (#99)');
      commit('chore(main): release next (#100)');
      const tip = commit('docs: later prose (#101)');
      reader.get = (url) =>
        url === 'pulls/100'
          ? {
              ...pr(notes(base)),
              number: 100,
              title: 'chore(main): release next',
              base: { ref: 'main', repo: { full_name: repository } },
            }
          : { head: { ref: 'ordinary' } };
      await expect(
        verifyReleasePrNotes({
          eventName: 'merge_group',
          event: { merge_group: { head_sha: tip } },
          components,
          reader,
        })
      ).rejects.toThrow(`COVERAGE missing commit(s): ${late}`);
      const get = reader.get;
      reader.get = (url) =>
        url === 'pulls/100'
          ? {
              ...pr(notes(base) + notes(late).split('\n').at(-2)),
              number: 100,
              title: 'chore(main): release next',
              base: { ref: 'main', repo: { full_name: repository } },
            }
          : get(url);
      expect(
        await verifyReleasePrNotes({
          eventName: 'merge_group',
          event: { merge_group: { head_sha: tip } },
          components,
          reader,
        })
      ).toBe(1);
    }));

  it('fails closed when local coverage history or package configuration is incomplete', () =>
    fixture(async ({ reader, base }) => {
      const git = reader.git;
      for (const bad of [
        'malformed',
        Array.from({ length: 501 }, () => `${base}\0chore: hidden\0`).join('\n'),
      ]) {
        reader.git = (args) =>
          args[0] === 'log' ? bad : args[0] === 'diff-tree' ? 'input\0' : git(args);
        await expect(
          checkReleaseNotes({ pr: pr(notes(base)), base, components, reader })
        ).rejects.toThrow(/coverage/i);
      }
      reader.git = (args) => (args[0] === 'show' ? '{"packages":{}}' : git(args));
      await expect(
        checkReleaseNotes({ pr: pr(notes(base)), base, components, reader })
      ).rejects.toThrow('package config');
    }));
});

describe('HEADGREEN release PR discovery', () => {
  it('checks an earlier release candidate when the tip and event base are site-only', () =>
    fixture(async ({ reader, git, commit, anchor, base }) => {
      git(['update-ref', 'refs/remotes/origin/main', anchor]);
      const release = commit('chore(main): release 5.0.0-alpha.35 (#100)');
      const tip = commit('docs(site): change text (#101)');
      reader.get = (url) => ({
        ...pr(notes(base)),
        number: url === 'pulls/100' ? 100 : 101,
        title:
          url === 'pulls/100' ? 'chore(main): release 5.0.0-alpha.35' : 'docs(site): change text',
        head: { ref: url === 'pulls/100' ? pr('').head.ref : 'docs' },
        base: { ref: 'main', repo: { full_name: repository } },
      });
      // The pending pre-release change represents its own ordinary PR too.
      // Pin main at base so only the synthetic release and site commits are pending.
      git(['update-ref', 'refs/remotes/origin/main', base]);
      expect(
        await verifyReleasePrNotes({
          eventName: 'merge_group',
          event: { merge_group: { head_sha: tip, base_sha: release } },
          components,
          reader,
        })
      ).toBe(1);
      reader.get = (url) => ({
        ...pr(notes(anchor)),
        number: url === 'pulls/100' ? 100 : 101,
        title:
          url === 'pulls/100' ? 'chore(main): release 5.0.0-alpha.35' : 'docs(site): change text',
        base: { ref: 'main', repo: { full_name: repository } },
      });
      await expect(
        async () =>
          await verifyReleasePrNotes({
            eventName: 'merge_group',
            event: { merge_group: { head_sha: tip } },
            components,
            reader,
          })
      ).rejects.toThrow('outside');
    }));
  it('skips an edited ordinary PR title while still checking an earlier release candidate', () =>
    fixture(async ({ reader, git, commit, base }) => {
      git(['update-ref', 'refs/remotes/origin/main', base]);
      commit('chore(main): release next (#100)');
      const tip = commit('docs(site): original title (#101)');
      reader.get = (url) =>
        url === 'pulls/100'
          ? {
              ...pr(notes(base)),
              number: 100,
              title: 'chore(main): release next',
              base: { ref: 'main', repo: { full_name: repository } },
            }
          : {
              number: 101,
              title: 'docs(site): edited title',
              head: { ref: 'docs' },
              // Ordinary PR base metadata is not a release-notes gate input either.
              base: { ref: 'another-base', repo: { full_name: 'other/repo' } },
            };
      expect(
        await verifyReleasePrNotes({
          eventName: 'merge_group',
          event: { merge_group: { head_sha: tip } },
          components,
          reader,
        })
      ).toBe(1);
    }));
  it('fails closed on unavailable queue PR data and title mismatch', () =>
    fixture(async ({ reader, git, commit, base }) => {
      git(['update-ref', 'refs/remotes/origin/main', base]);
      const tip = commit('chore(main): release next (#100)');
      reader.get = () => null;
      await expect(
        async () =>
          await verifyReleasePrNotes({
            eventName: 'merge_group',
            event: { merge_group: { head_sha: tip } },
            components,
            reader,
          })
      ).rejects.toThrow('Missing PR branch');
      reader.get = () => ({ ...pr(notes(base)), number: 100, title: 'changed' });
      await expect(
        async () =>
          await verifyReleasePrNotes({
            eventName: 'merge_group',
            event: { merge_group: { head_sha: tip } },
            components,
            reader,
          })
      ).rejects.toThrow('does not match');
      reader.get = () => ({ ...pr(notes(base)), number: 100, title: 'chore(main): release next' });
      await expect(
        async () =>
          await verifyReleasePrNotes({
            eventName: 'merge_group',
            event: { merge_group: { head_sha: tip } },
            components,
            reader,
          })
      ).rejects.toThrow('does not match');
      git(['update-ref', '-d', 'refs/remotes/origin/main']);
      await expect(
        async () =>
          await verifyReleasePrNotes({
            eventName: 'merge_group',
            event: { merge_group: { head_sha: tip } },
            components,
            reader,
          })
      ).rejects.toThrow();
    }));
  it('checks pull request events and rejects unsupported/missing event data', () =>
    fixture(async ({ reader, base }) => {
      expect(
        await verifyReleasePrNotes({
          eventName: 'pull_request',
          event: { pull_request: { ...pr(notes(base)), base: { sha: base } } },
          components,
          reader,
        })
      ).toBe(1);
      for (const eventName of ['push', 'merge_group'])
        await expect(
          async () => await verifyReleasePrNotes({ eventName, event: {}, components, reader })
        ).rejects.toThrow();
    }));
});

const squadRoot = components.find((component) => component.name === 'squad')!.owns[0];
const manifest = { '.': '5.0.0-alpha.35', [squadRoot]: '0.1.0-alpha.9' };
const draftTags = ['v5.0.0-alpha.35', 'tmt-squad-v0.1.0-alpha.9'];
function draftReader(releases: unknown[], refs: unknown[] = []): SafetyReader {
  return {
    repository,
    get() {
      throw new Error('unexpected read');
    },
    git() {
      throw new Error('unexpected git');
    },
    list(path) {
      return path === 'releases' ? releases : refs;
    },
  };
}
describe('tagless manifest draft skip', () => {
  it('exports sanitized matching tagged-draft evidence from the one existing draft discovery', () => {
    const input = {
      id: 77,
      tag_name: draftTags[0],
      draft: true,
      created_at: '2026-10-03T10:00:00Z',
      body: 'not transported',
      assets: [{ name: 'private' }],
    };
    const reader = draftReader(
      [input, { ...input, id: 78, tag_name: 'unrelated' }],
      [{ ref: `refs/tags/${draftTags[0]}`, object: { sha: 'a'.repeat(40) } }]
    );
    const list = reader.list;
    const calls: string[] = [];
    reader.list = (path) => {
      calls.push(path);
      return list(path);
    };
    expect(inspectManifestDrafts({ manifest, components, reader })).toEqual({
      heldPaths: [],
      drafts: [{ path: '.', id: 77, tag_name: draftTags[0], created_at: input.created_at }],
    });
    expect(calls.filter((path) => path === 'releases')).toHaveLength(1);
  });

  it('skips for either or both manifest component drafts without tags', () => {
    for (const tags of [[draftTags[0]], [draftTags[1]], draftTags])
      expect(
        heldPathsOf({
          manifest,
          components,
          reader: draftReader(tags.map((tag_name) => ({ tag_name, draft: true }))),
        })
      ).toEqual(tags.map((tag) => (tag === draftTags[0] ? '.' : squadRoot)));
  });
  it('runs for tagged drafts, unrelated versions, and published releases', () => {
    for (const releases of [
      [published(draftTags[0])],
      [{ tag_name: 'v5.0.0-alpha.99', draft: true }],
      [],
    ])
      expect(heldPathsOf({ manifest, components, reader: draftReader(releases) })).toEqual([]);
    expect(
      heldPathsOf({
        manifest,
        components,
        reader: draftReader(
          [{ tag_name: draftTags[0], draft: true }],
          [{ ref: `refs/tags/${draftTags[0]}`, object: { sha: 'a'.repeat(40) } }]
        ),
      })
    ).toEqual([]);
  });
  it('does not mistake prefix-matching tags for the exact manifest tag', () => {
    expect(
      heldPathsOf({
        manifest,
        components,
        reader: draftReader(
          [{ tag_name: draftTags[0], draft: true }],
          [{ ref: `refs/tags/${draftTags[0]}0`, object: { sha: 'a'.repeat(40) } }]
        ),
      })
    ).toEqual(['.']);
  });
  it('fails closed on incomplete manifest, release and tag data', () => {
    for (const candidate of [null, {}, { '.': 'bad' }, { ...manifest, '.': 'bad' }])
      expect(() =>
        heldPathsOf({ manifest: candidate, components, reader: draftReader([]) })
      ).toThrow();
    for (const releases of [
      [{}],
      [{ tag_name: draftTags[0] }],
      [{ tag_name: draftTags[0], draft: false }],
    ])
      expect(() => heldPathsOf({ manifest, components, reader: draftReader(releases) })).toThrow(
        'release data'
      );
    expect(() =>
      heldPathsOf({
        manifest,
        components,
        reader: draftReader([{ tag_name: draftTags[0], draft: true }], [{}]),
      })
    ).toThrow('git tag data');
  });
});

describe('bounded workflow REST adapter', () => {
  it('uses explicit workflow credentials and complete REST pagination, never GraphQL', () => {
    const calls: string[] = [];
    const reader = createSafetyReader(
      { repository, token: 'workflow', env: { GH_TOKEN: 'agent' } },
      (command, args, options) => {
        expect(command).toBe('gh');
        expect(args[0]).toBe('api');
        expect(args).not.toContain('graphql');
        expect(options.env.GH_TOKEN).toBe('workflow');
        calls.push(args[1]);
        return JSON.stringify(/[?&]page=1$/.test(args[1]) ? Array(100).fill({}) : []);
      }
    );
    expect(reader.list('releases')).toHaveLength(100);
    expect(calls).toHaveLength(2);
  });
  it('refuses malformed/incomplete responses, missing credentials and exhausted budgets', () => {
    for (const response of ['{}', 'invalid', JSON.stringify(Array(100).fill({}))]) {
      const reader = createSafetyReader({ repository, token: 'workflow' }, () => response);
      expect(() => reader.list('releases')).toThrow();
    }
    expect(() => createSafetyReader({ repository })).toThrow('token');
    expect(() => createSafetyReader({ repository: '../bad', token: 'x' })).toThrow('owner/repo');
    const reader = createSafetyReader({ repository, token: 'workflow' }, () => '{}');
    for (let count = 0; count < 60; count++) reader.get('fixture');
    expect(() => reader.get('fixture')).toThrow('budget');
  });
});

describe('workflow safety wiring', () => {
  const ci = readFileSync(path.join(root, '.github/workflows/ci.yml'), 'utf8');
  const release = readFileSync(path.join(root, '.github/workflows/release.yml'), 'utf8');
  const loop = release
    .split('      - name: Run release-please\n')[1]
    .split('\n      # Only one release PR')[0]
    .split('        run: |\n')[1]
    .split('\n')
    .map((line) => line.slice(10))
    .join('\n');
  it('requires notes on PR updates and merge groups without restarting CI on edits', () => {
    expect(ci).toContain('types: [opened, synchronize, reopened]');
    expect(ci.split('  pull_request:\n')[1].split('  merge_group:')[0]).not.toContain('edited');
    expect(ci.split('  code-quality:\n')[1]).toContain('fetch-depth: 0');
    expect(ci).toContain('run: node typescript/scripts/release-pr-safety.mjs notes');
    expect(ci).toContain('GITHUB_TOKEN: ${{ github.token }}');
  });
  it('preserves github-release after the draft skip, in both live and dry-run mode', () => {
    const directory = mkdtempSync(path.join(tmpdir(), 'tmt-release-draft-loop-'));
    try {
      writeExecutable(
        path.join(directory, 'node'),
        '#!/bin/sh\ncase "$1" in\n*/release-please-queue.mjs) echo run ;;\n*/release-please-run.mjs) echo "$2" >> "$RUNNER_TEMP/commands" ;;\nesac\n',
        0o700
      );
      for (const live of ['true', 'false']) {
        const commands = path.join(directory, 'commands');
        writeFileSync(commands, '');
        const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', loop], {
          cwd: path.join(root, '.github/release-please'),
          env: {
            ...process.env,
            PATH: `${directory}:${process.env.PATH}`,
            RUNNER_TEMP: directory,
            GITHUB_STEP_SUMMARY: path.join(directory, 'summary'),
            GITHUB_OUTPUT: path.join(directory, 'output'),
            TAGLESS_DRAFT: 'true',
            LIVE: live,
          },
          encoding: 'utf8',
          timeout: 5000,
        });
        expect(result.status).toBe(0);
        expect(readFileSync(commands, 'utf8')).toBe('github-release\n');
      }
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
  it.each([
    { heldPaths: ['.'] },
    { heldPaths: [squadRoot] },
    { heldPaths: ['.', squadRoot] },
    { heldPaths: [] },
  ])('reports held manifest paths $heldPaths with real draft adapter outputs', ({ heldPaths }) => {
    const directory = mkdtempSync(path.join(tmpdir(), 'tmt-release-draft-cli-'));
    try {
      const current = JSON.parse(
        readFileSync(path.join(root, '.release-please-manifest.json'), 'utf8')
      ) as Record<string, string>;
      const drafts = heldPaths.map((path) => ({
        tag_name: `${path === '.' ? 'v' : 'tmt-squad-v'}${current[path]}`,
        draft: true,
        id: path === '.' ? 1 : 2,
        created_at: '2026-10-03T10:00:00Z',
        body: 'not transported',
      }));
      const allHeld = heldPaths.length === Object.keys(current).length;
      writeExecutable(
        path.join(directory, 'gh'),
        `#!${process.execPath}\nif (process.env.GH_TOKEN !== 'fixture') process.exit(23);\nconsole.log(JSON.stringify(process.argv[3].includes('/releases?') ? ${JSON.stringify(drafts)} : []));\n`,
        0o700
      );
      const output = path.join(directory, 'output'),
        summary = path.join(directory, 'summary');
      const result = spawnSync(
        process.execPath,
        [path.join(root, 'typescript/scripts/release-pr-safety.mjs'), 'draft'],
        {
          env: {
            ...process.env,
            PATH: `${directory}:${process.env.PATH}`,
            RELEASE_TOKEN: 'fixture',
            GH_TOKEN: 'wrong-agent',
            GITHUB_REPOSITORY: repository,
            GITHUB_OUTPUT: output,
            GITHUB_STEP_SUMMARY: summary,
          },
          encoding: 'utf8',
          timeout: 5000,
        }
      );
      expect(result.status).toBe(0);
      expect(result.stdout).toBe(allHeld ? 'skip\n' : 'run\n');
      expect(readFileSync(output, 'utf8')).toBe(
        `skip=${allHeld}\nheld_paths=${JSON.stringify(heldPaths)}\ndrafts=${JSON.stringify(drafts.map(({ id, tag_name, created_at }, index) => ({ path: heldPaths[index], id, tag_name, created_at })))}\n`
      );
      if (heldPaths.length) expect(readFileSync(summary, 'utf8')).toContain(heldPaths.join(', '));
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});
