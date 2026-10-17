import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vite-plus/test';
import {
  DECLARATION_OUTPUT,
  DECLARATION_SUPPORT,
  declarationBytes,
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
import type { DraftRelease } from '../../scripts/release-draft-assets.mjs';

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
  const observeCli = gitDeclarationAncestry((args) =>
    git(args.map((arg) => (arg === DECLARATION_SUPPORT ? registration : arg)))
  );
  const context = { map: declarationSource(cut, root), product: 'ops', cut, releases, observeCli };
  const record = resolveDeclaration(context)!;
  const snapshot = {
    ...context,
    declaration: record,
    hashes: { '.github/components.json': 'source' },
  };
  return { root, git, before, registration, context, record, snapshot, mapText };
}

describe('generated minimum CLI declarations', () => {
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
    expect(resolveDeclaration(f.context)!.cli.version).toBe('5.0.0-alpha.2');
    expect(verifyDeclaration({ ...f.context, record: f.record })).toEqual(f.record);
    expect(declarationBytes({ ...f.context, record: f.record })).toEqual(expected);
  });

  it('accepts a published beta minimum under ordinary SemVer, without admitting a draft', () => {
    const f = fixture();
    f.git(['tag', 'v5.0.0-beta.1', f.registration]);
    const beta = { ...f.context.releases[0], tag_name: 'v5.0.0-beta.1' };
    const releases = [beta, f.context.releases.at(-1)!];
    expect(resolveDeclaration({ ...f.context, releases })!.cli.version).toBe('5.0.0-beta.1');
    expect(() =>
      resolveDeclaration({ ...f.context, releases: [{ ...beta, draft: true }] })
    ).toThrow('minimum-cli-unpublished');
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
