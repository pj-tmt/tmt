import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { parseComponentMap } from '../../scripts/ci-scope.mjs';
import {
  checkReleaseNotes,
  createSafetyReader,
  taglessDrafts,
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

function fixture(
  run: (context: {
    reader: SafetyReader;
    git: (args: string[]) => string;
    commit: (text: string) => string;
    old: string;
    anchor: string;
    base: string;
    directory: string;
  }) => void
) {
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt-release-pr-safety-'));
  const env = { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' };
  const git = (args: string[]) => runPackedCommand('git', args, { cwd: directory, env }).trim();
  const commit = (text: string) => {
    writeFileSync(path.join(directory, 'input'), text);
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
    run({ reader, git, commit, old, anchor, base, directory });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe('release PR linked-commit gate', () => {
  it('accepts CLI and Squad notes with their own latest published anchors', () =>
    fixture(({ reader, base }) => {
      expect(checkReleaseNotes({ pr: pr(notes(base)), base, components, reader })).toEqual({
        tag: cliTag,
        linkedCommits: 1,
      });
      expect(
        checkReleaseNotes({ pr: pr(notes(base, squadTag), 'tmt-squad'), base, components, reader })
          ?.tag
      ).toBe(squadTag);
    }));
  it('catches full-history notes even when the compare URL has the correct base', () =>
    fixture(({ reader, old, base }) => {
      expect(() => checkReleaseNotes({ pr: pr(notes(old)), base, components, reader })).toThrow(
        'outside'
      );
    }));
  it('excludes the anchor itself and future commits', () =>
    fixture(({ reader, anchor, base, commit }) => {
      const future = commit('future');
      for (const sha of [anchor, future])
        expect(() => checkReleaseNotes({ pr: pr(notes(sha)), base, components, reader })).toThrow(
          'outside'
        );
    }));
  it('rejects an ancestor of the candidate that is not a descendant of the tag', () =>
    fixture(({ reader, git, commit, old, base }) => {
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
      expect(() =>
        checkReleaseNotes({ pr: pr(notes(side)), base: head, components, reader })
      ).toThrow('outside');
      expect(
        checkReleaseNotes({ pr: pr(notes(base)), base: head, components, reader })?.linkedCommits
      ).toBe(1);
    }));
  it('uses the published release rather than a newer draft or another component', () =>
    fixture(({ reader, base }) => {
      expect(
        checkReleaseNotes({
          pr: pr(notes(base)),
          base,
          components,
          reader,
          releases: [
            published(),
            published(squadTag),
            { tag_name: 'v5.0.0-alpha.35', draft: true },
          ],
        })?.tag
      ).toBe(cliTag);
      expect(() =>
        checkReleaseNotes({ pr: pr(notes(base, 'v5.0.0-alpha.33')), base, components, reader })
      ).toThrow('compare base');
    }));
  it('does not own generation, conventional parsing, duplicate policy or expected entry counts', () =>
    fixture(({ reader, base }) => {
      expect(
        checkReleaseNotes({
          pr: pr(notes(base) + notes(base).split('\n').at(-2)),
          base,
          components,
          reader,
        })?.linkedCommits
      ).toBe(2);
      expect(
        checkReleaseNotes({ pr: pr(notes(base).split('\n')[0]), base, components, reader })
          ?.linkedCommits
      ).toBe(0);
    }));
  it('skips ordinary PRs without reading releases', () =>
    fixture(({ reader, base }) => {
      reader.list = () => {
        throw new Error('no read allowed');
      };
      expect(
        checkReleaseNotes({ pr: { head: { ref: 'feature' } }, base, components, reader })
      ).toBeNull();
    }));
  it('fails closed on missing anchors, notes, tags and malformed linked data', () =>
    fixture(({ reader, base, git }) => {
      for (const body of [
        '',
        notes(base) + notes(base),
        notes('bad'),
        notes(''),
        notes(base).replace(repository + '/commit', 'other/repo/commit'),
      ]) {
        expect(() => checkReleaseNotes({ pr: pr(body), base, components, reader })).toThrow();
      }
      expect(() =>
        checkReleaseNotes({ pr: pr(notes(base)), base: 'missing', components, reader })
      ).toThrow('base SHA');
      expect(() =>
        checkReleaseNotes({ pr: pr(notes(base)), base, components, reader, releases: [] })
      ).toThrow('No published');
      git(['tag', '-d', cliTag]);
      expect(() => checkReleaseNotes({ pr: pr(notes(base)), base, components, reader })).toThrow();
    }));
  it('rejects missing branch and parked or unknown release components', () =>
    fixture(({ reader, base }) => {
      for (const pull of [{}, pr(notes(base), 'tmt-office'), pr(notes(base), 'unknown')])
        expect(() => checkReleaseNotes({ pr: pull, base, components, reader })).toThrow();
    }));
});

describe('HEADGREEN release PR discovery', () => {
  it('checks an earlier release candidate when the tip and event base are site-only', () =>
    fixture(({ reader, git, commit, anchor, base }) => {
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
        verifyReleasePrNotes({
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
      expect(() =>
        verifyReleasePrNotes({
          eventName: 'merge_group',
          event: { merge_group: { head_sha: tip } },
          components,
          reader,
        })
      ).toThrow('outside');
    }));
  it('skips an edited ordinary PR title while still checking an earlier release candidate', () =>
    fixture(({ reader, git, commit, base }) => {
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
        verifyReleasePrNotes({
          eventName: 'merge_group',
          event: { merge_group: { head_sha: tip } },
          components,
          reader,
        })
      ).toBe(1);
    }));
  it('fails closed on unavailable queue PR data and title mismatch', () =>
    fixture(({ reader, git, commit, base }) => {
      git(['update-ref', 'refs/remotes/origin/main', base]);
      const tip = commit('chore(main): release next (#100)');
      reader.get = () => null;
      expect(() =>
        verifyReleasePrNotes({
          eventName: 'merge_group',
          event: { merge_group: { head_sha: tip } },
          components,
          reader,
        })
      ).toThrow('Missing PR branch');
      reader.get = () => ({ ...pr(notes(base)), number: 100, title: 'changed' });
      expect(() =>
        verifyReleasePrNotes({
          eventName: 'merge_group',
          event: { merge_group: { head_sha: tip } },
          components,
          reader,
        })
      ).toThrow('does not match');
      reader.get = () => ({ ...pr(notes(base)), number: 100, title: 'chore(main): release next' });
      expect(() =>
        verifyReleasePrNotes({
          eventName: 'merge_group',
          event: { merge_group: { head_sha: tip } },
          components,
          reader,
        })
      ).toThrow('does not match');
      git(['update-ref', '-d', 'refs/remotes/origin/main']);
      expect(() =>
        verifyReleasePrNotes({
          eventName: 'merge_group',
          event: { merge_group: { head_sha: tip } },
          components,
          reader,
        })
      ).toThrow();
    }));
  it('checks pull request events and rejects unsupported/missing event data', () =>
    fixture(({ reader, base }) => {
      expect(
        verifyReleasePrNotes({
          eventName: 'pull_request',
          event: { pull_request: { ...pr(notes(base)), base: { sha: base } } },
          components,
          reader,
        })
      ).toBe(1);
      for (const eventName of ['push', 'merge_group'])
        expect(() => verifyReleasePrNotes({ eventName, event: {}, components, reader })).toThrow();
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
  it('skips for either or both manifest component drafts without tags', () => {
    for (const tags of [[draftTags[0]], [draftTags[1]], draftTags])
      expect(
        taglessDrafts({
          manifest,
          components,
          reader: draftReader(tags.map((tag_name) => ({ tag_name, draft: true }))),
        })
      ).toEqual(tags);
  });
  it('runs for tagged drafts, unrelated versions, and published releases', () => {
    for (const releases of [
      [published(draftTags[0])],
      [{ tag_name: 'v5.0.0-alpha.99', draft: true }],
      [],
    ])
      expect(taglessDrafts({ manifest, components, reader: draftReader(releases) })).toEqual([]);
    expect(
      taglessDrafts({
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
      taglessDrafts({
        manifest,
        components,
        reader: draftReader(
          [{ tag_name: draftTags[0], draft: true }],
          [{ ref: `refs/tags/${draftTags[0]}0`, object: { sha: 'a'.repeat(40) } }]
        ),
      })
    ).toEqual([draftTags[0]]);
  });
  it('fails closed on incomplete manifest, release and tag data', () => {
    for (const candidate of [null, {}, { '.': 'bad' }, { ...manifest, '.': 'bad' }])
      expect(() =>
        taglessDrafts({ manifest: candidate, components, reader: draftReader([]) })
      ).toThrow();
    for (const releases of [
      [{}],
      [{ tag_name: draftTags[0] }],
      [{ tag_name: draftTags[0], draft: false }],
    ])
      expect(() => taglessDrafts({ manifest, components, reader: draftReader(releases) })).toThrow(
        'release data'
      );
    expect(() =>
      taglessDrafts({
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
    .split('\n      # Release pull requests')[0]
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
      writeFileSync(
        path.join(directory, 'node'),
        '#!/bin/sh\ncase "$1" in\n*/release-please-queue.mjs) echo run ;;\n*/release-please-run.mjs) echo "$2" >> "$RUNNER_TEMP/commands" ;;\nesac\n',
        { mode: 0o700 }
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
  it('runs the real draft adapter with fixture REST data and writes durable outputs', () => {
    const directory = mkdtempSync(path.join(tmpdir(), 'tmt-release-draft-cli-'));
    try {
      const current = JSON.parse(
        readFileSync(path.join(root, '.release-please-manifest.json'), 'utf8')
      ) as Record<string, string>;
      const tag = `v${current['.']}`;
      writeFileSync(
        path.join(directory, 'gh'),
        `#!${process.execPath}\nif (process.env.GH_TOKEN !== 'fixture') process.exit(23);\nconsole.log(JSON.stringify(process.argv[3].includes('/releases?') ? [{tag_name:${JSON.stringify(tag)},draft:true}] : []));\n`,
        { mode: 0o700 }
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
      expect(result.stdout).toBe('skip\n');
      expect(readFileSync(output, 'utf8')).toBe('skip=true\n');
      expect(readFileSync(summary, 'utf8')).toContain(tag);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});
