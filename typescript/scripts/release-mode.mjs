#!/usr/bin/env node
// Main pushes and daily recovery are live; manual runs choose dry/live explicitly.
// Publication authorization remains in the native gates and the release skill.
//   EVENT=push|schedule|workflow_dispatch REF=refs/heads/main DRY_RUN=true|false
//   node release-mode.mjs
import { appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const MAIN = 'refs/heads/main';

function requireMain(ref) {
  if (ref !== MAIN) {
    throw new Error(
      `A live release run is only allowed on ${MAIN}, not on ${ref || 'an unknown ref'}.`
    );
  }
}

/** `dryRun` is the dispatch input; it is unset on a push. */
export function releaseMode({ event, ref, dryRun }) {
  if (event === 'workflow_dispatch') {
    if (dryRun !== 'true' && dryRun !== 'false') {
      throw new Error('A manual release run needs dry_run to be true or false.');
    }
    if (dryRun === 'true') return { live: false, reason: 'a dry run was requested' };
    requireMain(ref);
    return { live: true, reason: 'a live run was requested' };
  }
  if (event === 'push' || event === 'schedule') {
    requireMain(ref);
    return { live: true, reason: event === 'push' ? 'main advanced' : 'daily recovery' };
  }
  throw new Error(`A release run does not start on the ${event} event.`);
}

function main(environment) {
  const { live, reason } = releaseMode({
    event: environment.EVENT,
    ref: environment.REF,
    dryRun: environment.DRY_RUN,
  });
  const line = `${live ? 'Live' : 'Dry'} release run: ${reason}.`;
  process.stderr.write(`${line}\n`);
  if (environment.GITHUB_STEP_SUMMARY) appendFileSync(environment.GITHUB_STEP_SUMMARY, `${line}\n`);
  const output = `live=${live}\n`;
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
