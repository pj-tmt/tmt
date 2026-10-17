// Cut metadata's only network boundary: bounded REST GETs, never a publishing client.
import { writeFileSync } from 'node:fs';
import { isDeepStrictEqual } from 'node:util';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { retryReleaseRead } from './release-read-retry.mjs';
import { runPackedCommand } from './packed-command.mjs';

const SHA = /^[a-f0-9]{40}$/;

/** Shared bounded catalogue; only readCutMetadata attests trusted draft visibility. */
export function readReleaseCatalog({ repository, token }, execute = runPackedCommand) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository ?? ''))
    throw new Error('Release catalogue needs a repository.');
  let requests = 0;
  // A quiet P-page catalogue costs P + 2 requests: discovery, terminal anchor,
  // middle pages, terminal verification and page-one verification. The shared
  // 60-request/90-second budget admits 58 pages (5,800 releases); all three
  // attempts spend this same budget. The live cut can exhaust its separate
  // 180-request budget earlier across snapshots. Exhaustion never returns partial history.
  const deadline = Date.now() + 90_000;
  const get = (page) => {
    if (++requests > 60 || Date.now() >= deadline) throw new Error('Cut REST budget exceeded.');
    const output = execute(
      'gh',
      [
        'api',
        `repos/${repository}/releases?per_page=100&page=${page}`,
        '--include',
        '--method',
        'GET',
      ],
      {
        cwd: process.cwd(),
        env: { ...process.env, ...(token ? { GH_TOKEN: token } : {}) },
        timeoutMs: Math.min(10_000, deadline - Date.now()),
      }
    );
    const boundary = /\r?\n\r?\n/.exec(output);
    if (!boundary || !/^HTTP\/\S+ 200(?: |\r?\n)/.test(output))
      throw new Error('Invalid cut REST response headers.');
    const headers = output.slice(0, boundary.index);
    const batch = JSON.parse(output.slice(boundary.index + boundary[0].length));
    if (!Array.isArray(batch) || batch.length > 100) throw new Error('Invalid cut REST page.');
    const links = headers.match(/^link:\s*(.*)$/im)?.[1] ?? '';
    const last = [...links.matchAll(/<([^>]+)>;\s*rel="last"/g)];
    let lastPage = 1;
    if (last.length) {
      const url = new URL(last[0][1]);
      lastPage = Number(url.searchParams.get('page'));
      if (
        last.length !== 1 ||
        url.searchParams.get('per_page') !== '100' ||
        !Number.isSafeInteger(lastPage) ||
        lastPage < 1
      )
        throw new Error('Invalid cut REST pagination headers.');
    } else if ((page === 1 && links) || /rel="next"/.test(links)) {
      throw new Error('Incomplete cut REST pagination headers.');
    }
    const rows = batch.map((row) => {
      if (!Number.isSafeInteger(row?.id)) throw new Error('Missing cut REST record identity.');
      const { id, tag_name, draft, body, target_commitish, assets } = row;
      if (!Array.isArray(assets) || assets.some((asset) => typeof asset?.name !== 'string'))
        throw new Error('Invalid release asset metadata.');
      if (typeof draft !== 'boolean' || typeof tag_name !== 'string')
        throw new Error('Invalid release metadata.');
      return {
        id,
        tag_name,
        draft,
        body,
        target_commitish,
        assets: assets.map(({ name }) => ({ name })),
      };
    });
    return { rows, lastPage };
  };
  const membership = ({ rows, lastPage }) => ({ ids: rows.map(({ id }) => id), lastPage });
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const first = get(1);
    const pages = first.lastPage;
    if (requests + pages + 1 > 60) throw new Error('Cut REST budget exceeded.');
    // Capture the terminal boundary before traversing the middle. A verification
    // taken only after traversal would miss a removal that already shifted it.
    const terminal = pages === 1 ? first : get(pages);
    const rows = [...first.rows];
    let shifted = pages > 1 && (first.rows.length !== 100 || terminal.rows.length === 0);
    for (let page = 2; page < pages; page += 1) {
      const middle = get(page);
      if (middle.rows.length !== 100) shifted = true;
      rows.push(...middle.rows);
    }
    if (pages > 1) rows.push(...terminal.rows);
    if (new Set(rows.map((row) => row.id)).size !== rows.length) continue;
    const verifiedTerminal = get(pages);
    const verifiedFirst = get(1);
    if (
      shifted ||
      !isDeepStrictEqual(membership(terminal), membership(verifiedTerminal)) ||
      !isDeepStrictEqual(membership(first), membership(verifiedFirst))
    )
      continue;
    return rows;
  }
  throw new Error(
    'Unstable cut REST catalogue after three attempts: duplicate records or shifted pages.'
  );
}

/** Exact release reads share one 60-request/90-second budget; no listing or pagination. */
export function createReleaseReader({ repository, token, sleep }, execute = runPackedCommand) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository ?? ''))
    throw new Error('Release reader needs a repository.');
  const [owner, name] = repository.split('/');
  let requests = 0;
  const deadline = Date.now() + 90_000;
  const budgetFailure = new Error('Exact release read budget exceeded.');
  const get = (args) => {
    const result = retryReleaseRead(
      () => {
        if (++requests > 60 || Date.now() >= deadline) throw budgetFailure;
        return JSON.parse(
          execute('gh', ['api', ...args], {
            cwd: process.cwd(),
            env: { ...process.env, ...(token ? { GH_TOKEN: token } : {}) },
            timeoutMs: Math.min(10_000, deadline - Date.now()),
          })
        );
      },
      { label: 'Exact release read', retryIf: (failure) => failure !== budgetFailure, sleep }
    );
    if (result?.errors) throw new Error('Exact release discovery returned GraphQL errors.');
    return result;
  };
  const byId = (id) => {
    if (!Number.isSafeInteger(id) || id <= 0) throw new Error('Invalid release identity.');
    const release = get([`repos/${repository}/releases/${id}`, '--method', 'GET']);
    if (
      release?.id !== id ||
      typeof release.tag_name !== 'string' ||
      typeof release.draft !== 'boolean' ||
      typeof release.created_at !== 'string' ||
      typeof release.target_commitish !== 'string' ||
      (release.body != null && typeof release.body !== 'string') ||
      !Array.isArray(release.assets) ||
      release.assets.some(
        (asset) =>
          !Number.isSafeInteger(asset?.id) || asset.id <= 0 || typeof asset.name !== 'string'
      )
    )
      throw new Error('Invalid exact release metadata.');
    return { ...release, body: release.body ?? '' };
  };
  return {
    byId,
    byTag: (tag) => {
      if (typeof tag !== 'string' || !tag || tag.length > 255)
        throw new Error('Invalid release tag.');
      // REST's /releases/tags endpoint excludes tagless drafts. Resolve only this
      // tag's identity through GraphQL, then retain the ordinary REST record shape.
      const result = get([
        'graphql',
        '-f',
        'query=query($owner:String!,$name:String!,$tag:String!){repository(owner:$owner,name:$name){release(tagName:$tag){databaseId tagName}}}',
        '-f',
        `owner=${owner}`,
        '-f',
        `name=${name}`,
        '-f',
        `tag=${tag}`,
      ]);
      if (!result?.data?.repository || !Object.hasOwn(result.data.repository, 'release'))
        throw new Error('Incomplete exact release discovery.');
      const identity = result.data.repository.release;
      if (identity === null) return null;
      if (identity.tagName !== tag) throw new Error('Release discovery tag changed.');
      const release = byId(identity.databaseId);
      if (release.tag_name !== tag) throw new Error('Release identity changed during discovery.');
      return release;
    },
  };
}

export function readCutMetadata(
  { repository, cut, token, draftVisibility },
  execute = runPackedCommand
) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository ?? '') || !SHA.test(cut ?? ''))
    throw new Error('Cut metadata needs a repository and exact main cut SHA.');
  if (!token || draftVisibility !== 'trusted')
    throw new Error('Draft visibility requires the trusted contents-write metadata reader.');
  const releases = readReleaseCatalog({ repository, token }, execute);
  return {
    schema: 1,
    repository,
    cut,
    draftVisibility: 'trusted',
    capturedAt: new Date().toISOString(),
    releases,
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [output, ...extra] = process.argv.slice(2);
  if (!output || extra.length) throw new Error('Usage: release-cut-read.mjs <metadata.json>');
  const input = {
    repository: process.env.GITHUB_REPOSITORY,
    cut: process.env.CUT_SHA,
    token: process.env.GH_TOKEN,
    draftVisibility: process.env.DRAFT_VISIBILITY,
  };
  let result;
  try {
    result = readCutMetadata(input);
  } catch (error) {
    // An unavailable snapshot is visible, never interpreted as an empty release list.
    result = {
      schema: 1,
      repository: input.repository,
      cut: input.cut,
      evidenceError: error.message,
    };
  }
  writeFileSync(output, `${JSON.stringify(result, null, 2)}\n`);
}
