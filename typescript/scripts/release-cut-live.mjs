import { declarationMarker } from './release-declaration.mjs';
// Trusted-main adapter: the cut planner remains pure; only this owner drafts and dispatches.
import assert from 'node:assert/strict';
import { appendFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readReleaseSourceAtRef } from './release-source-at-ref.mjs';
import { componentOfProduct, releasePolicy } from './native-release-policy.mjs';
import { planReleaseCuts } from './release-cut.mjs';
import { readCutMetadata } from './release-cut-read.mjs';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const SHA = /^[a-f0-9]{40}$/;

/** REST only, with one shared deadline/request budget including metadata pagination. */
export function createCutClient({ repository, token, ref, observe }, execute = runPackedCommand) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository ?? '') || !token)
    throw new Error('Release cut needs repository and draft-visible credentials.');
  let requests = 0;
  let reconciliationReads = 0;
  // MEASURED incident: 67 s to first draft, then ~28 s/row. MODELED six-row
  // provision: 67 + 6*28 = 235 s, 95 s margin, 30 s read-only reserve = 360 s.
  // Normal six-row REST work <= 1 + 60 + 6*5 = 91; reserve three of 180 calls.
  const deadline = Date.now() + 360_000;
  const command = (executable, args, options, reconciliation = false) => {
    const stage = reconciliation ? 'reconciliation' : 'normal';
    const limit = reconciliation ? 180 : 177;
    const end = deadline - (reconciliation ? 0 : 30_000);
    if (requests >= limit || Date.now() >= end || (reconciliation && reconciliationReads >= 3))
      throw new Error(`Live cut REST budget exceeded (${stage}; ${requests}/180 calls).`);
    requests += 1;
    if (reconciliation) reconciliationReads += 1;
    const started = Date.now();
    try {
      return execute(executable, args, {
        ...options,
        timeoutMs: Math.min(options.timeoutMs, end - started),
      });
    } finally {
      const method = args[args.indexOf('--method') + 1];
      observe?.({
        kind: 'rest',
        stage,
        request: requests,
        method,
        route: args[1],
        elapsedMs: Date.now() - started,
      });
    }
  };
  const api = (path, method = 'GET', fields = [], reconciliation = false) => {
    if (method !== 'GET' && ref !== 'refs/heads/main')
      throw new Error('Release cut writes are main-only.');
    const output = command(
      'gh',
      ['api', `repos/${repository}/${path}`, '--method', method, ...fields],
      {
        cwd: ROOT,
        env: { ...process.env, GH_TOKEN: token },
        timeoutMs: 10_000,
      },
      reconciliation
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
    reconcileDispatch: ({ id, row, body, startedAt }) => {
      const draft = api(`releases/${id}`, 'GET', [], true);
      verifiedDraft(draft, row, body);
      assert.equal(draft.id, id, 'Cut draft identity changed.');
      // GitHub timestamps have whole-second precision and creation can finish
      // after our POST aborts. Read since its floored start, not a client-abort cap.
      const lower = Math.floor(startedAt / 1000) * 1000;
      const since = `>=${new Date(lower).toISOString()}`;
      const evidence = api(
        `actions/workflows/native-release.yml/runs?event=workflow_dispatch&branch=main&per_page=100&created=${encodeURIComponent(since)}`,
        'GET',
        [],
        true
      );
      const readAt = Date.now();
      assert.ok(
        Number.isSafeInteger(evidence?.total_count) &&
          evidence.total_count >= 0 &&
          Array.isArray(evidence.workflow_runs) &&
          evidence.total_count === evidence.workflow_runs.length &&
          evidence.total_count <= 100,
        'Incomplete dispatch run listing.'
      );
      const runs = evidence.workflow_runs;
      assert.ok(
        runs.every(
          (run) =>
            Number.isSafeInteger(run.id) &&
            run.id > 0 &&
            typeof run.created_at === 'string' &&
            Number.isFinite(Date.parse(run.created_at))
        ) && new Set(runs.map((run) => run.id)).size === runs.length,
        'Invalid dispatch run identities.'
      );
      const matched = runs.filter(
        (run) =>
          run.event === 'workflow_dispatch' &&
          run.head_branch === 'main' &&
          run.path === '.github/workflows/native-release.yml' &&
          run.display_title === `Native release: ${row.tag}` &&
          Date.parse(run.created_at) >= lower &&
          Date.parse(run.created_at) <= readAt &&
          run.html_url === `https://github.com/${repository}/actions/runs/${run.id}`
      );
      assert.equal(
        matched.length,
        1,
        'Dispatch is uncertain: expected exactly one matching native run.'
      );
      return { runId: matched[0].id, runUrl: matched[0].html_url };
    },
  };
}

export function cutDraftBody(row) {
  if (!SHA.test(row.cut ?? '') || !row.notes || !row.commits?.length)
    throw new Error('A cut draft requires notes and releasable source commits.');
  return `${row.notes}\n\nRelease cut: ${row.cut}\n${declarationMarker(row.declaration)}`;
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
  observe,
}) {
  const timed = async (stage, read) => {
    const started = Date.now();
    try {
      return await read();
    } finally {
      observe?.({ kind: 'phase', stage, elapsedMs: Date.now() - started });
    }
  };
  const originalGit = git;
  git = (args) => {
    const started = Date.now();
    try {
      return originalGit(args);
    } finally {
      observe?.({ kind: 'git', operation: args[0], elapsedMs: Date.now() - started });
    }
  };
  const cut = await timed('capture-main', () => client.main());
  git(['fetch', '--quiet', 'origin', 'main', '--tags']);
  git(['merge-base', '--is-ancestor', cut, 'refs/remotes/origin/main']);
  const { map, workspace } = await timed('source', () =>
    readReleaseSourceAtRef(cut, {
      root,
      warm: true,
      ...(observe
        ? {
            execute: (program, args, options) => {
              const started = Date.now();
              try {
                return runPackedCommand(program, args, options);
              } finally {
                observe({
                  kind: 'source-command',
                  program,
                  operation: args[0],
                  elapsedMs: Date.now() - started,
                });
              }
            },
          }
        : {}),
    })
  );
  if (
    product &&
    (!componentOfProduct(map, product).package ||
      componentOfProduct(map, product).release === false)
  )
    throw new Error(`Cut selection names an unreleased component ${product}.`);
  if (version && !product) throw new Error('An explicit cut version needs exactly one product.');
  const versions = version ? { [product]: version } : {};
  const metadata = await timed('catalogue', () => client.metadata(cut));
  const cutDate = date ?? metadata.capturedAt?.slice(0, 10);
  const plan = await timed('initial-plan', () =>
    planReleaseCuts({ metadata, map, workspace, git, date: cutDate, versions })
  );
  if (plan.unavailable) throw new Error(plan.unavailable);
  const actions = [];
  let rowStarted;
  let rowStage;
  const record = (action) => {
    actions.push(action);
    observe?.({ kind: 'row', stage: rowStage, ...action, elapsedMs: Date.now() - rowStarted });
  };
  for (const row of plan.components) {
    if (product && row.product !== product) continue;
    rowStarted = Date.now();
    rowStage = 'selection';
    if (row.status !== 'proposed') {
      record({ product: row.product, status: row.status, reason: row.reason });
      continue;
    }
    try {
      // release-cut workflow concurrency serializes allocators. Reuse the captured
      // view plus our returned mutations; publication never frees a reserved number.
      rowStage = 'revalidate';
      const checked = await timed(`revalidate:${row.product}`, () =>
        planReleaseCuts({
          metadata,
          map,
          workspace,
          git,
          date: cutDate,
          versions,
          product: row.product,
        })
      );
      if (checked.unavailable) throw new Error(checked.unavailable);
      const current = checked.components.find((item) => item.product === row.product);
      if (!current) throw new Error('Component disappeared from the updated captured plan.');
      if (current.status !== 'proposed') {
        record({ product: row.product, status: current.status, reason: current.reason });
        continue;
      }
      // Our earlier allocations must not change this product's captured proposal.
      assert.deepEqual(current, row, 'Release state changed; recompute the cut on the next run.');
      rowStage = 'tag-before-create';
      if (client.tagged(row.tag))
        throw new Error('Cut tag already exists; refusing an unverified source.');
      const body = cutDraftBody(row);
      if (live) {
        rowStage = 'create';
        const created = client.draft({ product: row.product, tag: row.tag, cut, body });
        verifiedDraft(created, row, body);
        metadata.releases.push(created);
        // This exact-ID read proves the persisted identity/body/target before dispatch;
        // it is not a second catalogue snapshot and does not reselect a frozen CLI.
        rowStage = 'verify-draft';
        verifiedDraft(client.release(created.id), row, body);
        rowStage = 'tag-before-dispatch';
        if (client.tagged(row.tag))
          throw new Error('Draft unexpectedly has a tag before publication.');
        rowStage = 'dispatch';
        const startedAt = Date.now();
        try {
          await timed(`dispatch:${row.product}`, () => client.dispatch(row.product, row.tag));
        } catch (error) {
          // A timeout or HTTP 5xx can follow an accepted POST. A known
          // rejection/budget refusal is still failed, and never gets another POST.
          if (
            !/ETIMEDOUT|ECONNRESET|EPIPE|timed? out|uncertain dispatch|socket hang up|HTTP 5\d\d\b/i.test(
              error.message
            )
          )
            throw error;
          let evidence;
          let reason = error.message.split('\n', 1)[0];
          try {
            if (!client.reconcileDispatch) throw new Error('Dispatch read-back is unavailable.');
            evidence = await timed(`reconcile:${row.product}`, () =>
              client.reconcileDispatch({ id: created.id, row, body, startedAt })
            );
          } catch (readError) {
            reason += `; ${readError.message.split('\n', 1)[0]}`;
          }
          record({
            product: row.product,
            status: 'dispatch-uncertain',
            tag: row.tag,
            cut,
            reconciled: Boolean(evidence),
            ...evidence,
            reason,
          });
          continue;
        }
      }
      record({
        product: row.product,
        status: live ? 'created' : 'would-create',
        tag: row.tag,
        cut,
      });
    } catch (error) {
      // Report an uncertain POST once; never retry a create/dispatch inside this run.
      record({ product: row.product, status: 'failed', reason: error.message.split('\n', 1)[0] });
    }
  }
  return { ...plan, mode: live ? 'live' : 'dry-run', actions };
}

/** Reconciled transport uncertainty is complete, so the tick must not see a failed cut run. */
export function cutExitCode(result) {
  return result.actions.some(
    (action) =>
      action.status === 'failed' || (action.status === 'dispatch-uncertain' && !action.reconciled)
  )
    ? 1
    : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 2) throw new Error('Usage: release-cut-live.mjs');
    const observe = (entry) => console.error(`Release cut: ${JSON.stringify(entry)}`);
    const client = createCutClient({
      repository: process.env.GITHUB_REPOSITORY,
      token: process.env.GH_TOKEN,
      ref: process.env.GITHUB_REF,
      observe,
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
      observe,
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
    process.exitCode = cutExitCode(result);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
