// Trusted-main adapter: the cut planner remains pure; only this owner drafts and dispatches.
import assert from 'node:assert/strict';
import { appendFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseComponentMap } from './ci-scope.mjs';
import { releasePolicy, productOfTag } from './native-release-policy.mjs';
import { planReleaseBuilds } from './plan-release-builds.mjs';
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
    dispatch: (product) => {
      releasePolicy(product);
      api('actions/workflows/native-release.yml/dispatches', 'POST', [
        '-f',
        'ref=main',
        '-f',
        `inputs[product]=${product}`,
        '-f',
        'inputs[prepare]=false',
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
export async function runReleaseCuts({ client, git, live = false, date }) {
  const cut = client.main();
  git(['fetch', '--quiet', 'origin', 'main', '--tags']);
  git(['merge-base', '--is-ancestor', cut, 'refs/remotes/origin/main']);
  const map = parseComponentMap(git(['show', `${cut}:.github/components.json`]));
  const metadata = client.metadata(cut);
  const cutDate = date ?? metadata.capturedAt?.slice(0, 10);
  const plan = await planReleaseCuts({ metadata, map, git, date: cutDate });
  if (plan.unavailable) throw new Error(plan.unavailable);
  const actions = [];
  for (const row of plan.components) {
    try {
      // A fresh bounded read precedes each component mutation. Other components progress independently.
      const fresh = live ? client.metadata(cut) : metadata;
      const checked = await planReleaseCuts({ metadata: fresh, map, git, date: cutDate });
      if (checked.unavailable) throw new Error(checked.unavailable);
      const current = checked.components.find((item) => item.product === row.product);
      if (!current) throw new Error('Component disappeared from fresh cut plan.');
      const drafts = fresh.releases.filter(
        (release) => release.draft === true && productOfTag(release.tag_name) === row.product
      );
      if (drafts.length) {
        const active = fresh.runs.some(
          (run) => run.display_title === `Native release: ${row.product}`
        );
        // Unknown active-run identity also blocks recovery, as it blocks new cuts in the planner.
        const unknown = fresh.runs.some(
          (run) => !map.components.some((c) => run.display_title === `Native release: ${c.name}`)
        );
        if (active || unknown) {
          actions.push({ product: row.product, status: 'in-flight', reason: current.reason });
          continue;
        }
        if (drafts.length !== 1)
          throw new Error('Multiple component drafts require investigation.');
        const release = client.release(drafts[0].id);
        assert.equal(release.id, drafts[0].id, 'Draft identity changed during recovery.');
        assert.equal(release.tag_name, drafts[0].tag_name, 'Draft tag changed during recovery.');
        assert.equal(
          release.target_commitish,
          drafts[0].target_commitish,
          'Draft target changed during recovery.'
        );
        assert.equal(release.draft, true, 'Draft published during recovery.');
        if (
          !Array.isArray(release.assets) ||
          release.assets.some((asset) => typeof asset.name !== 'string')
        )
          throw new Error(
            'Draft assets unavailable; recovery cannot infer bundle/hold/failure state.'
          );
        if (client.tagged(release.tag_name)) {
          actions.push({
            product: row.product,
            status: 'parked',
            reason: 'Draft already has a git tag; investigate before recovery.',
          });
          continue;
        }
        const pending = planReleaseBuilds({ releases: [release], product: row.product });
        if (pending.builds.length || pending.awaiting.length) {
          git([
            'merge-base',
            '--is-ancestor',
            release.target_commitish,
            'refs/remotes/origin/main',
          ]);
          if (live) client.dispatch(row.product);
          actions.push({
            product: row.product,
            status: live ? 'resumed' : 'would-resume',
            tag: release.tag_name,
          });
        } else {
          actions.push({ product: row.product, status: 'parked', reason: current.reason });
        }
        continue;
      }
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
        client.dispatch(row.product);
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
    const result = await runReleaseCuts({ client, git, live: process.env.LIVE === 'true' });
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
