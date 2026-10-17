import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import {
  DECLARATION_OUTPUT,
  DECLARATION_SUPPORT,
  declarationBytes,
  captureDeclaration,
  readDeclarationReleases,
  declarationMarker,
  declarationSource,
  gitDeclarationAncestry,
  readDeclarationMarker,
  resolveDeclaration,
  verifyDeclaration,
  verifyDeclarationBytes,
  writeDeclaration,
  type DeclarationRecord,
} from '../../scripts/release-declaration.mjs';
import { publishDraft } from '../../scripts/release-publish.mjs';
import { writeReleaseWorkspace } from '../support/release-workspace-fixture.js';
import { parseComponentMap } from '../../scripts/ci-scope.mjs';
import { ghApi, type DraftRelease } from '../../scripts/release-draft-assets.mjs';

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'tmt-release-declaration-'));
  roots.push(root);
  const git = (args: string[]) => {
    const result = spawnSync('git', args, { cwd: root, encoding: 'utf8' });
    if (result.status !== 0) throw new Error(result.stderr, { cause: { status: result.status } });
    return result.stdout.trim();
  };
  git(['init', '-q', '-b', 'main']);
  const commit = () => {
    git(['add', '.']);
    git([
      '-c',
      'user.name=Release fixture',
      '-c',
      'user.email=fixture@example.test',
      '-c',
      'commit.gpgsign=false',
      'commit',
      '-qm',
      'test: release declaration fixture\n\nCo-authored-by: Codex <codex@openai.com>',
    ]);
    return git(['rev-parse', 'HEAD']);
  };
  writeReleaseWorkspace(root);
  writeFileSync(join(root, '.gitignore'), 'rust/target/\n');
  const before = commit();
  git(['tag', 'v5.0.0-alpha.1']);
  writeFileSync(join(root, 'registration'), 'CLI registration and declaration support\n');
  const registration = commit();
  git(['tag', 'v5.0.0-alpha.3']);
  mkdirSync(join(root, '.github'));
  const mapText = JSON.stringify({
    components: {
      cli: { owns: ['rust'], package: 'tmt-cli' },
      ops: {
        owns: ['extensions/ops'],
        package: 'tmt-ops',
        requiresCliSha: registration,
        uses: [
          {
            feature: 'browser-access',
            label: 'Browser access',
            extension: 'remote',
            requires: '>=0.1.0-alpha.1',
          },
        ],
      },
    },
  });
  writeFileSync(join(root, '.github/components.json'), mapText);
  const cut = commit();
  git(['tag', 'v5.0.0-alpha.5']);
  const releases = [
    'v5.0.0-alpha.1',
    'v5.0.0-alpha.3',
    'v5.0.0-alpha.5',
    'tmt-remote-v0.1.0-alpha.1',
  ].map((tag_name, index): DraftRelease => ({
    id: index + 1,
    tag_name,
    draft: false,
    target_commitish: cut,
    created_at: '2026-10-17T00:00:00Z',
    assets: [],
  }));
  // The injected Git port maps the repository's fixed support commit into this minimal history.
  const ancestry = () =>
    gitDeclarationAncestry((args) =>
      git(args.map((arg) => (arg === DECLARATION_SUPPORT ? registration : arg)))
    );
  const observeCli = ancestry();
  const context = { map: declarationSource(cut, root), product: 'ops', cut, releases, observeCli };
  const record = resolveDeclaration(context)!;
  const snapshot = {
    ...context,
    declaration: record,
    hashes: { '.github/components.json': 'source' },
  };
  return { root, git, before, registration, context, record, snapshot, mapText, ancestry };
}

describe('generated minimum CLI declarations', () => {
  it.each([2, 1000])(
    'bounds declaration Git work for %i CLI releases, including rechecks',
    (count) => {
      const sha = (n: number) => n.toString(16).padStart(40, '0');
      const tag = (n: number) => `v5.0.0-alpha.${n}`;
      const map = parseComponentMap(
        JSON.stringify({
          components: {
            cli: { owns: ['rust'], package: 'tmt-cli' },
            ops: { owns: ['ops'], package: 'tmt-ops', requiresCliSha: sha(10) },
          },
        })
      );
      const releases = Array.from({ length: count }, (_, i) => ({
        tag_name: tag(i + 1),
        draft: i === 0,
      }));
      const calls: string[][] = [];
      const observeCli = gitDeclarationAncestry((args) => {
        calls.push(args);
        if (args[0] === 'for-each-ref')
          return Array.from(
            { length: count + 1 },
            (_, i) => `${tag(i)}\t${sha(i + 100)}\t\tcommit\t`
          ).join('\n');
        if (args[0] === 'tag' && args[1] === '--contains')
          return [tag(0), tag(1), tag(count)].join('\n');
        // Also model the old per-tag operations: the regression must fail the count, not its stub.
        if (args[0] === 'rev-parse') return sha(Number(args[2].match(/alpha\.(\d+)/)![1]) + 100);
        if (args[0] === 'merge-base') {
          if ([sha(101), sha(count + 100)].includes(args[3])) return '';
          throw new Error('not ancestor', { cause: { status: 1 } });
        }
        throw new Error(`Unexpected Git operation: ${args}`);
      });
      const context = { map, product: 'ops', cut: sha(20), releases, observeCli };
      const record = resolveDeclaration(context)!;
      expect(record.cli).toEqual({
        tag: tag(count),
        version: `5.0.0-alpha.${count}`,
        sha: sha(count + 100),
      });
      expect(verifyDeclaration({ ...context, record })).toEqual(record);
      expect(resolveDeclaration(context)).toEqual(record);
      expect(calls).toHaveLength(3);
      expect(calls.filter((args) => args[0] === 'for-each-ref')).toHaveLength(1);
      expect(
        calls
          .filter((args) => args[0] === 'tag')
          .map((args) => args[2])
          .sort()
      ).toEqual([DECLARATION_SUPPORT, sha(10)].sort());
    }
  );

  it('peels annotated CLI tags while excluding a bare tag and a draft from selection', () => {
    const f = fixture();
    f.git(['tag', 'v5.0.0-alpha.0', f.registration]);
    f.git([
      '-c',
      'user.name=Release fixture',
      '-c',
      'user.email=fixture@example.test',
      '-c',
      'tag.gpgsign=false',
      'tag',
      '-a',
      'v5.0.0-alpha.2',
      f.registration,
      '-m',
      'test: annotated release',
    ]);
    const releases = [
      ...f.context.releases,
      { ...f.context.releases[0], tag_name: 'v5.0.0-alpha.2' },
    ];
    const context = { ...f.context, releases, observeCli: f.ancestry() };
    expect(resolveDeclaration(context)!.cli).toEqual({
      tag: 'v5.0.0-alpha.2',
      version: '5.0.0-alpha.2',
      sha: f.registration,
    });
    const draftReleases = releases.map((release) =>
      release.tag_name === 'v5.0.0-alpha.2' ? { ...release, draft: true } : release
    );
    expect(resolveDeclaration({ ...context, releases: draftReleases })!.cli.tag).toBe(
      'v5.0.0-alpha.3'
    );
  });

  it('rejects divergent published tags even when their version precedes a supporting release', () => {
    const f = fixture();
    f.git(['checkout', '--orphan', 'divergent']);
    f.git([
      '-c',
      'user.name=Release fixture',
      '-c',
      'user.email=fixture@example.test',
      '-c',
      'commit.gpgsign=false',
      'commit',
      '-qm',
      'test: divergent release\n\nCo-authored-by: Codex <codex@openai.com>',
    ]);
    f.git(['tag', 'v5.0.0-alpha.2']);
    f.git(['checkout', 'main']);
    const divergent = { ...f.context.releases[0], tag_name: 'v5.0.0-alpha.2' };
    const context = {
      ...f.context,
      releases: [...f.context.releases, divergent],
      observeCli: f.ancestry(),
    };
    expect(resolveDeclaration(context)!.cli.tag).toBe('v5.0.0-alpha.3');
    expect(() => resolveDeclaration({ ...context, releases: [divergent] })).toThrow(
      'minimum-cli-unpublished'
    );
  });

  it('uses the oldest containing publication and identical bytes on all preparation hosts', () => {
    const f = fixture();
    expect(f.record.cli).toEqual({
      tag: 'v5.0.0-alpha.3',
      version: '5.0.0-alpha.3',
      sha: f.registration,
    });
    const expected = declarationBytes({ ...f.context, record: f.record });
    for (const target of [
      'aarch64-apple-darwin',
      'x86_64-apple-darwin',
      'aarch64-unknown-linux-musl',
      'x86_64-unknown-linux-musl',
    ]) {
      const host = join(f.root, 'rust/target', target);
      writeDeclaration(host, f.snapshot, f.context.map);
      expect(readFileSync(join(host, DECLARATION_OUTPUT))).toEqual(expected);
      verifyDeclarationBytes(join(host, DECLARATION_OUTPUT), f.snapshot, f.context.map);
    }
    expect(JSON.parse(expected.toString())).toEqual({
      version: 1,
      requiresCli: '>=5.0.0-alpha.3',
      uses: [
        {
          feature: 'browser-access',
          label: 'Browser access',
          extension: 'remote',
          requires: '>=0.1.0-alpha.1',
        },
      ],
    });
    expect(readFileSync(join(f.root, '.github/components.json'), 'utf8')).toBe(f.mapText);
    expect(readDeclarationMarker(declarationMarker(f.record))).toEqual(f.record);
    // A lower version published later cannot change an already captured cut.
    f.git(['tag', 'v5.0.0-alpha.2', f.registration]);
    f.context.releases.push({ ...f.context.releases[0], id: 99, tag_name: 'v5.0.0-alpha.2' });
    // A new planner/catalogue takes a new tag snapshot; the captured record remains unchanged.
    f.context.observeCli = f.ancestry();
    expect(resolveDeclaration(f.context)!.cli.version).toBe('5.0.0-alpha.2');
    expect(verifyDeclaration({ ...f.context, record: f.record })).toEqual(f.record);
    expect(declarationBytes({ ...f.context, record: f.record })).toEqual(expected);
  });

  it('accepts a published beta minimum under ordinary SemVer, without admitting a draft', () => {
    const f = fixture();
    f.git(['tag', 'v5.0.0-beta.1', f.registration]);
    const beta = { ...f.context.releases[0], tag_name: 'v5.0.0-beta.1' };
    const releases = [beta, f.context.releases.at(-1)!];
    expect(
      resolveDeclaration({ ...f.context, releases, observeCli: f.ancestry() })!.cli.version
    ).toBe('5.0.0-beta.1');
    expect(() =>
      resolveDeclaration({ ...f.context, releases: [{ ...beta, draft: true }] })
    ).toThrow('minimum-cli-unpublished');
  });

  it('regenerates standalone proof bytes from the existing captured snapshot without a catalogue read', () => {
    const f = fixture();
    const transport = mkdtempSync(join(tmpdir(), 'tmt-declaration-transport-'));
    roots.push(transport);
    const snapshot = join(transport, 'release-version-state.json');
    writeFileSync(snapshot, JSON.stringify(f.snapshot));
    const script = new URL('../../scripts/release-declaration.mjs', import.meta.url);
    const result = spawnSync(
      process.execPath,
      [script.pathname, 'generate', f.root, snapshot, 'ops'],
      {
        cwd: f.root,
        encoding: 'utf8',
        env: { PATH: process.env.PATH, HOME: transport },
        timeout: 10000,
      }
    );
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toBe('');
    expect(readFileSync(join(f.root, DECLARATION_OUTPUT))).toEqual(
      declarationBytes({ ...f.context, record: f.record })
    );
  });

  it('rejects hand edits, source copies, wrong cut/product and duplicate cut metadata', () => {
    const f = fixture();
    writeDeclaration(f.root, f.snapshot, f.context.map);
    const file = join(f.root, DECLARATION_OUTPUT);
    writeFileSync(file, readFileSync(file, 'utf8').replace('>=5.0.0-alpha.3', '>=5.0.0-alpha.1'));
    expect(() => verifyDeclarationBytes(file, f.snapshot, f.context.map)).toThrow(
      'differs from generated bytes'
    );
    expect(() =>
      writeDeclaration(
        f.root,
        { ...f.snapshot, hashes: { 'extensions/ops/TMT-USES.json': 'hand-authored' } },
        f.context.map
      )
    ).toThrow('never source-copied');
    expect(() => declarationBytes({ ...f.context, cut: f.before, record: f.record })).toThrow(
      'cut changed'
    );
    expect(() => declarationBytes({ ...f.context, product: 'remote', record: f.record })).toThrow(
      'product changed'
    );
    expect(() => readDeclarationMarker(declarationMarker(f.record).repeat(2))).toThrow('Duplicate');
  });

  it('blocks a cut with only drafts/bare tags and with an unpublished authored minimum', () => {
    const f = fixture();
    const drafts = f.context.releases.map((r) => ({
      ...r,
      draft: r.tag_name !== 'v5.0.0-alpha.1',
    }));
    expect(() => resolveDeclaration({ ...f.context, releases: drafts })).toThrow(
      'minimum-cli-unpublished'
    );
    expect(() =>
      resolveDeclaration({
        ...f.context,
        releases: f.context.releases.filter((r) => r.tag_name !== 'tmt-remote-v0.1.0-alpha.1'),
      })
    ).toThrow('uses-unpublished');
    expect(resolveDeclaration({ ...f.context, product: 'cli' })).toBeNull();
    expect(resolveDeclaration({ ...f.context, product: 'driver-herdr' })).toBeNull();
  });
});

describe('mandatory publication compatibility preflight', () => {
  for (const mode of [
    'published',
    'draft CLI',
    'missing CLI',
    'unpublished use',
    'diverged',
    'moved tag',
    'missing metadata',
  ] as const) {
    it(`${mode}: records a publish mutation only for a valid published ancestor`, () => {
      const f = fixture();
      const tag = 'tmt-ops-v0.1.0-alpha.2';
      const mutations: string[] = [];
      let record: DeclarationRecord | null = f.record;
      let releases = [...f.context.releases];
      if (mode === 'draft CLI')
        releases = releases.map((r) => ({ ...r, draft: r.tag_name === f.record.cli.tag }));
      if (mode === 'missing CLI')
        releases = releases.filter((r) => r.tag_name !== f.record.cli.tag);
      if (mode === 'unpublished use')
        releases = releases.filter((r) => !r.tag_name.startsWith('tmt-remote'));
      if (mode === 'missing metadata') record = null;
      releases.push({
        id: 100,
        tag_name: tag,
        draft: true,
        target_commitish: f.context.cut,
        created_at: '2026-10-17T00:00:00Z',
        body: declarationMarker(record),
        assets: [{ id: 1, name: 'release-publication.json' }],
      });
      const observeCli =
        mode === 'diverged'
          ? () => ({ sha: f.registration, status: 'diverged' })
          : mode === 'moved tag'
            ? () => ({ sha: f.context.cut, status: 'ahead' })
            : f.context.observeCli;
      const api = {
        readRelease: (tag: string) => releases.find((release) => release.tag_name === tag) ?? null,
        listReleases: () => releases,
        publish: (name: string) => {
          mutations.push(name);
        },
        latestRelease: () => null,
        setLatest: () => {
          throw new Error('extension cannot set latest');
        },
        upload: () => {
          throw new Error('no upload');
        },
        deleteAsset: () => {
          throw new Error('no delete');
        },
      };
      const act = () =>
        publishDraft({
          api,
          product: 'ops',
          tag,
          declarationEvidence: { map: f.context.map, observeCli },
        });
      if (mode === 'published') {
        act();
        expect(mutations).toEqual([tag]);
      } else {
        expect(act).toThrow();
        expect(mutations).toEqual([]);
      }
    });
  }
});

describe('targeted declaration evidence', () => {
  it.each([3000, 6000])(
    'checks frozen publications and guards publish independently of %i unrelated releases',
    (count) => {
      const f = fixture();
      const tag = 'tmt-ops-v0.1.0-alpha.2';
      const own: DraftRelease = {
        id: count + 100,
        tag_name: tag,
        draft: true,
        created_at: '2026-10-17T00:00:00Z',
        target_commitish: f.context.cut,
        body: declarationMarker(f.record),
        assets: [{ id: 10, name: 'release-publication.json' }],
      };
      const unrelated = Array.from({ length: count }, (_, index): DraftRelease => ({
        id: 100 + index,
        tag_name: `unrelated-${index}`,
        draft: false,
        created_at: '2026-10-17T00:00:00Z',
        target_commitish: f.context.cut,
        assets: [],
      }));
      const releases = [...f.context.releases, ...unrelated, own];
      const tags = new Map(releases.map((release) => [release.tag_name, release]));
      const ids = new Map(releases.map((release) => [release.id, release]));
      const calls: string[] = [];
      const api = ghApi({
        repository: 'pj-tmt/tmt',
        spawn: (_command, args) => {
          calls.push(args[1]);
          let response;
          if (args[1] === 'graphql') {
            const requested = args.find((arg) => arg.startsWith('tag='))!.slice(4);
            const release = tags.get(requested);
            response = {
              data: {
                repository: {
                  release: release ? { databaseId: release.id, tagName: requested } : null,
                },
              },
            };
          } else if (new RegExp('^repos/pj-tmt/tmt/releases/[0-9]+$').test(args[1])) {
            response = ids.get(Number(args[1].split('/').at(-1)));
          } else throw new Error(`Unexpected history access: ${args.join(' ')}`);
          return { status: 0, stdout: JSON.stringify(response), stderr: '' };
        },
      });
      const evidence = readDeclarationReleases({
        ...f.context,
        record: f.record,
        readRelease: api.readRelease,
      });
      expect(verifyDeclaration({ ...f.context, record: f.record, releases: evidence })).toEqual(
        f.record
      );
      expect(evidence.map((release) => release.tag_name)).toEqual([
        f.record.cli.tag,
        'tmt-remote-v0.1.0-alpha.1',
      ]);
      expect(calls).toHaveLength(4); // Two exact identity + REST reads, no catalogue.
      calls.length = 0;
      const mutations: string[] = [];
      publishDraft({
        api: {
          ...api,
          listReleases: () => {
            throw new Error('No history in extension publication');
          },
          publish: (requested) => {
            mutations.push(requested);
          },
          latestRelease: () => null,
          setLatest: () => {
            throw new Error('Extension cannot set latest');
          },
        },
        product: 'ops',
        tag,
        declarationEvidence: { map: f.context.map, observeCli: f.context.observeCli },
      });
      expect(mutations).toEqual([tag]);
      expect(calls).toHaveLength(6); // Own draft, frozen CLI, authored use; no other release.
    }
  );

  it.each(['published', 'draft', 'bare'] as const)(
    'tagless capture agrees with the complete oracle for an oldest %s candidate',
    (state) => {
      const f = fixture();
      f.git(['tag', 'v5.0.0-alpha.2', f.registration]); // An older bare tag is never a publication.
      let releases = f.context.releases;
      if (state === 'draft')
        releases = releases.map((release) => ({
          ...release,
          draft: release.tag_name === 'v5.0.0-alpha.3',
        }));
      if (state === 'bare')
        releases = releases.filter((release) => release.tag_name !== 'v5.0.0-alpha.3');
      const requested: string[] = [];
      const git = (args: string[]) =>
        f.git(args.map((arg) => (arg === DECLARATION_SUPPORT ? f.registration : arg)));
      const actual = captureDeclaration({
        ...f.context,
        git,
        readRelease: (tag) => {
          requested.push(tag);
          return releases.find((release) => release.tag_name === tag) ?? null;
        },
      });
      expect(actual).toEqual(resolveDeclaration({ ...f.context, releases }));
      expect(actual?.cli.tag).toBe(state === 'published' ? 'v5.0.0-alpha.3' : 'v5.0.0-alpha.5');
      expect(requested).not.toContain('v5.0.0-alpha.1'); // Before both floors, never queried.
      expect(requested).toContain('v5.0.0-alpha.2');
    }
  );
});
