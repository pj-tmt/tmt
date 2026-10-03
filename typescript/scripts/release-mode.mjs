#!/usr/bin/env node
// Push cuts have hourly admission; schedule and owner dispatch retain their modes.
// Publication authorization remains in the native gates and the release skill.
import { appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { runPackedCommand } from './packed-command.mjs';

const MAIN = 'refs/heads/main';
const CADENCE_MS = 55 * 60_000;
const HISTORY_WINDOW_MS = 3 * 60 * 60_000;
const LIVE_ADMISSION = 'Admit live release cut';

function requireMain(ref) {
  if (ref !== MAIN) {
    throw new Error(
      `A live release run is only allowed on ${MAIN}, not on ${ref || 'an unknown ref'}.`
    );
  }
}

/** `dryRun` is the dispatch input; it is unset on push and schedule. */
export function releaseMode({ event, ref, dryRun }) {
  if (event === 'workflow_dispatch') {
    if (dryRun !== 'true' && dryRun !== 'false') {
      throw new Error('A manual release run needs dry_run to be true or false.');
    }
    if (dryRun === 'true') return { live: false, reason: 'a dry run was requested' };
    requireMain(ref);
    return { live: true, reason: 'a live run was requested' };
  }
  if (event === 'schedule' || event === 'push') {
    requireMain(ref);
    return {
      live: true,
      reason: event === 'schedule' ? 'hourly cuts and recovery' : 'push cadence elapsed',
    };
  }
  throw new Error(`A release run does not start on the ${event} event.`);
}

/** Only admitted pushes reset cadence. REST run titles distinguish dispatch dry/live inputs. */
export function pushCadence(
  { repository, token, runId, now = Date.now() },
  execute = runPackedCommand
) {
  try {
    if (!/^[\w.-]+\/[\w.-]+$/.test(repository ?? '') || !token || !/^[1-9]\d*$/.test(runId ?? ''))
      throw new Error('Repository, credentials and current run identity are required.');
    if (!Number.isFinite(now)) throw new Error('Invalid cadence clock.');
    let requests = 0;
    const deadline = Date.now() + 180_000;
    const api = (path) => {
      if (++requests > 180 || Date.now() >= deadline)
        throw new Error('REST history budget exceeded.');
      return JSON.parse(
        execute('gh', ['api', `repos/${repository}/${path}`, '--method', 'GET'], {
          cwd: fileURLToPath(new URL('../../', import.meta.url)),
          env: { ...process.env, GH_TOKEN: token },
          timeoutMs: Math.min(10_000, deadline - Date.now()),
        })
      );
    };
    const pages = (path, key) => {
      const rows = [];
      let total;
      for (let page = 1; ; page++) {
        const result = api(`${path}${path.includes('?') ? '&' : '?'}per_page=100&page=${page}`);
        if (
          !Number.isSafeInteger(result?.total_count) ||
          result.total_count < 0 ||
          !Array.isArray(result[key]) ||
          result[key].length > 100 ||
          (total !== undefined && total !== result.total_count)
        )
          throw new Error('Incomplete or changing REST history.');
        total = result.total_count;
        rows.push(...result[key]);
        if (rows.length === total) return rows;
        if (rows.length > total || result[key].length !== 100)
          throw new Error('Truncated REST history.');
      }
    };
    // Keep acquisition independent of accumulated history; cadence uses each run's start below.
    const since = encodeURIComponent(`>=${new Date(now - HISTORY_WINDOW_MS).toISOString()}`);
    const runs = pages(
      `actions/workflows/release.yml/runs?branch=main&created=${since}`,
      'workflow_runs'
    );
    const seen = new Set();
    for (const run of runs) {
      if (!Number.isSafeInteger(run?.id) || run.id <= 0 || seen.has(run.id))
        throw new Error('Invalid or duplicate workflow run identity.');
      seen.add(run.id);
      if (String(run.id) === runId) continue;
      if (run.head_branch !== 'main') throw new Error('Unexpected branch in main history.');
      if (!['push', 'schedule', 'workflow_dispatch'].includes(run.event)) continue;
      if (['queued', 'waiting', 'pending', 'requested'].includes(run.status)) continue;
      const started = Date.parse(run.run_started_at);
      if (!Number.isFinite(started)) throw new Error(`Run ${run.id} has no valid start time.`);
      if (now - started >= CADENCE_MS) continue;
      let live = run.event === 'schedule';
      if (run.event === 'workflow_dispatch') {
        if (run.display_title === 'Release cut (dry dispatch)') continue;
        if (run.display_title !== 'Release cut (live dispatch)')
          throw new Error(`Recent dispatch ${run.id} has no dry/live mode evidence.`);
        live = true;
      }
      if (run.event === 'push') {
        if (!Number.isSafeInteger(run.run_attempt) || run.run_attempt < 1)
          throw new Error(`Run ${run.id} has no attempt identity.`);
        const jobs = pages(`actions/runs/${run.id}/attempts/${run.run_attempt}/jobs`, 'jobs');
        const cut = jobs.filter((job) => job.name === 'Cut releases from main');
        if (cut.length !== 1 || !Array.isArray(cut[0].steps))
          throw new Error(`Run ${run.id} has no cut admission evidence.`);
        live = cut[0].steps.some(
          (step) => step.name === LIVE_ADMISSION && step.conclusion === 'success'
        );
      }
      if (live) return { live: false, reason: 'cadence' };
    }
    return { live: true, reason: 'push cadence elapsed' };
  } catch (error) {
    throw new Error(
      `Release cadence history unavailable; refusing live push cut: ${error.message}`,
      { cause: error }
    );
  }
}

function main(environment) {
  let mode = releaseMode({
    event: environment.EVENT,
    ref: environment.REF,
    dryRun: environment.DRY_RUN,
  });
  if (environment.EVENT === 'push') {
    mode = pushCadence({
      repository: environment.GITHUB_REPOSITORY,
      token: environment.GH_TOKEN,
      runId: environment.GITHUB_RUN_ID,
    });
  }
  const { live, reason } = mode;
  const line =
    reason === 'cadence'
      ? 'Release cut skipped: cadence (a live cut started less than 55 minutes ago).'
      : `${live ? 'Live' : 'Dry'} release run: ${reason}.`;
  process.stderr.write(`${line}\n`);
  if (environment.GITHUB_STEP_SUMMARY) appendFileSync(environment.GITHUB_STEP_SUMMARY, `${line}\n`);
  const output = `live=${live}\nreason=${reason}\n`;
  if (environment.GITHUB_OUTPUT) appendFileSync(environment.GITHUB_OUTPUT, output);
  else process.stdout.write(output);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.env);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
