// Trusted-main adapter: the cut planner remains pure; only this owner drafts and dispatches.
import assert from 'node:assert/strict';
import { appendFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readReleaseSourceAtRef } from './release-source-at-ref.mjs';
import { releasePolicy } from './native-release-policy.mjs';
import { planReleaseCuts } from './release-cut.mjs';
import { readCutMetadata } from './release-cut-read.mjs';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const SHA = /^[a-f0-9]{40}$/;

/** REST only, with one shared deadline/request budget including metadata pagination. */
export function createCutClient({ repository, token, ref }, execute = runPackedCommand) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository ?? '') || !token)
    throw new Error('Release cut needs repository and draft-visible credentials.');
  let requests = 0;
  const deadline = Date.now() + 180_000;
  const command = (executable, args, options) => {
    if (++requests > 180 || Date.now() >= deadline)
      throw new Error('Live cut REST budget exceeded.');
    return execute(executable, args, {
      ...options,
      timeoutMs: Math.min(options.timeoutMs, deadline - Date.now()),
    });
  };
  const api = (path, method = 'GET', fields = []) => {
    if (method !== 'GET' && ref !== 'refs/heads/main')
      throw new Error('Release cut writes are main-only.');
    const output = command(
      'gh',
      ['api', `repos/${repository}/${path}`, '--method', method, ...fields],
      {
        cwd: ROOT,
        env: { ...process.env, GH_TOKEN: token },
        timeoutMs: 10_000,
      }
    );
    return output.trim() ? JSON.parse(output) : null;
  };
  return {
    main: () => {
      const sha = api('git/ref/heads/main')?.object?.sha;
      if (!SHA.test(sha ?? '')) throw new Error('Main ref has no exact cut SHA.');
      return sha;
    },
    metadata: (cut) =>
      readCutMetadata({ repository, cut, token, draftVisibility: 'trusted' }, command),
    release: (id) => {
      if (!Number.isSafeInteger(id) || id <= 0) throw new Error('Invalid draft identity.');
      return api(`releases/${id}`);
    },
    tagged: (tag) => {
      const refs = api(`git/matching-refs/tags/${encodeURIComponent(tag)}`);
      if (!Array.isArray(refs) || refs.some((row) => typeof row.ref !== 'string'))
        throw new Error('Incomplete tag discovery.');
      return refs.some((row) => row.ref === `refs/tags/${tag}`);
    },
    draft: ({ tag, cut, body, product }) => {
      const policy = releasePolicy(product);
      return api('releases', 'POST', [
        '-f',
        `tag_name=${tag}`,
        '-f',
        `target_commitish=${cut}`,
        '-f',
        `name=${tag}`,
        '-f',
        `body=${body}`,
        '-F',
        'draft=true',
        '-F',
        `prerelease=${policy.prerelease}`,
        '-f',
        'make_latest=false',
      ]);
    },
    dispatch: (product, tag) => {
      if (!tag || !tag.startsWith(releasePolicy(product).tagPrefix))
        throw new Error('Dispatch needs its allocated product tag.');
      api('actions/workflows/native-release.yml/dispatches', 'POST', [
        '-f',
        'ref=main',
        '-f',
        `inputs[product]=${product}`,
        '-f',
        'inputs[prepare]=false',
        '-f',
        `inputs[tag]=${tag}`,
      ]);
    },
  };
}

export function cutDraftBody(row) {
  if (!SHA.test(row.cut ?? '') || !row.notes || !row.commits?.length)
    throw new Error('A cut draft requires notes and releasable source commits.');
  return `${row.notes}\n\nRelease cut: ${row.cut}\n`;
}

function verifiedDraft(release, row, body) {
  assert.ok(Number.isSafeInteger(release?.id) && release.id > 0, 'Missing created draft identity.');
  assert.equal(release.draft, true, 'Cut is no longer a draft.');
  assert.equal(release.tag_name, row.tag, 'Cut draft tag changed.');
  assert.equal(release.target_commitish, row.cut, 'Cut draft target changed.');
  assert.equal(release.body, body, 'Cut draft notes changed.');
}

/** Drafts survive dispatch failure. Existing held/failed drafts are never automatically retried. */
export async function runReleaseCuts({
  client,
  git,
  live = false,
  date,
  product,
  version,
  root = ROOT,
}) {
  const cut = client.main();
  git(['fetch', '--quiet', 'origin', 'main', '--tags']);
  git(['merge-base', '--is-ancestor', cut, 'refs/remotes/origin/main']);
  const { map, workspace } = readReleaseSourceAtRef(cut, { root, warm: true });
  if (
    product &&
    !map.components.some((c) => c.name === product && c.package && c.release !== false)
  )
    throw new Error(`Cut selection names an unreleased component ${product}.`);
  if (version && !product) throw new Error('An explicit cut version needs exactly one product.');
  const versions = version ? { [product]: version } : {};
  const metadata = client.metadata(cut);
  const cutDate = date ?? metadata.capturedAt?.slice(0, 10);
  const plan = await planReleaseCuts({ metadata, map, workspace, git, date: cutDate, versions });
  if (plan.unavailable) throw new Error(plan.unavailable);
  const actions = [];
  for (const row of plan.components) {
    if (product && row.product !== product) continue;
    try {
      // A fresh bounded read precedes each component mutation. Other components progress independently.
      const fresh = live ? client.metadata(cut) : metadata;
      const checked = await planReleaseCuts({
        metadata: fresh,
        map,
        workspace,
        git,
        date: cutDate,
        versions,
      });
      if (checked.unavailable) throw new Error(checked.unavailable);
      const current = checked.components.find((item) => item.product === row.product);
      if (!current) throw new Error('Component disappeared from fresh cut plan.');
      if (current.status !== 'proposed') {
        actions.push({ product: row.product, status: current.status, reason: current.reason });
        continue;
      }
      // A concurrent publication changes the version/range. Do not create the stale proposal.
      assert.deepEqual(current, row, 'Release state changed; recompute the cut on the next run.');
      if (client.tagged(row.tag))
        throw new Error('Cut tag already exists; refusing an unverified source.');
      const body = cutDraftBody(row);
      if (live) {
        const created = client.draft({ product: row.product, tag: row.tag, cut, body });
        verifiedDraft(created, row, body);
        verifiedDraft(client.release(created.id), row, body);
        if (client.tagged(row.tag))
          throw new Error('Draft unexpectedly has a tag before publication.');
        client.dispatch(row.product, row.tag);
      }
      actions.push({
        product: row.product,
        status: live ? 'created' : 'would-create',
        tag: row.tag,
        cut,
      });
    } catch (error) {
      // Report an uncertain POST once; never retry a create/dispatch inside this run.
      actions.push({ product: row.product, status: 'failed', reason: error.message });
    }
  }
  return { ...plan, mode: live ? 'live' : 'dry-run', actions };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 2) throw new Error('Usage: release-cut-live.mjs');
    const client = createCutClient({
      repository: process.env.GITHUB_REPOSITORY,
      token: process.env.GH_TOKEN,
      ref: process.env.GITHUB_REF,
    });
    const git = (args) => runPackedCommand('git', args, { cwd: ROOT, env: process.env }).trimEnd();
    if (
      (process.env.PRODUCT || process.env.VERSION) &&
      process.env.GITHUB_EVENT_NAME !== 'workflow_dispatch'
    )
      throw new Error('Product/version selection requires an owner-dispatched cut.');
    const result = await runReleaseCuts({
      client,
      git,
      live: process.env.LIVE === 'true',
      product: process.env.PRODUCT || undefined,
      version: process.env.VERSION || undefined,
    });
    console.log(JSON.stringify(result, null, 2));
    if (process.env.GITHUB_STEP_SUMMARY)
      appendFileSync(
        process.env.GITHUB_STEP_SUMMARY,
        `## Release cuts (${result.mode})\n\nCut: ${result.cut}\n\n${result.actions.map((a) => `- ${a.product}: ${a.status}${a.reason ? ` — ${a.reason}` : ''}`).join('\n')}\n`
      );
    if (result.actions.some((action) => action.status === 'failed')) process.exitCode = 1;
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
