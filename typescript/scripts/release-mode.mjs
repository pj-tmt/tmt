#!/usr/bin/env node
// Decides whether a release run changes anything: opens and merges release pull requests,
// creates draft releases and starts the per-product builds. Until the owner has added the
// release App secrets every push run is a dry run, so adding the workflow changes nothing by
// itself; a dispatch chooses explicitly and never falls back silently.
//   EVENT=push|workflow_dispatch DRY_RUN=true|false APP_ID=... APP_KEY=... node release-mode.mjs
import { appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

/** `dryRun` is the dispatch input; it is unset on a push. */
export function releaseMode({ event, dryRun, hasSecrets }) {
  if (event === 'workflow_dispatch') {
    if (dryRun !== 'true' && dryRun !== 'false') {
      throw new Error('A manual release run needs dry_run to be true or false.');
    }
    if (dryRun === 'true') return { live: false, reason: 'a dry run was requested' };
    if (!hasSecrets) {
      throw new Error(
        'A live release run needs the RELEASE_APP_ID and RELEASE_APP_PRIVATE_KEY secrets, and they are not both set.'
      );
    }
    return { live: true, reason: 'a live run was requested' };
  }
  if (event === 'push') {
    return hasSecrets
      ? { live: true, reason: 'the release App secrets are configured' }
      : { live: false, reason: 'the release App secrets are not configured, so this is a dry run' };
  }
  throw new Error(`A release run does not start on the ${event} event.`);
}

function main(environment) {
  const { live, reason } = releaseMode({
    event: environment.EVENT,
    dryRun: environment.DRY_RUN,
    hasSecrets: Boolean(environment.APP_ID) && Boolean(environment.APP_KEY),
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
