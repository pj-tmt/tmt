#!/usr/bin/env node
// Publishes a complete draft release under its product's policy, then checks the published
// release. Publication cannot be undone once release immutability is on, so the two halves
// are separate and strict:
//   publish  re-reads the draft and refuses unless it is an alpha release of a released component
//            (`release: false` parks one) that carries the verified bundle and no hold or failure
//            marker; then one `gh release edit` makes it public with the policy's prerelease and
//            latest flags. The workflow calls it only after every gate passed.
//   verify   reads the published release back: it is public, immutable, carries the policy's flags,
//            its tag is on the release commit, and GitHub's release attestation covers the
//            release and every downloaded asset. A failure opens an issue and fails the run;
//            nothing is rolled back.
//   report   opens that same issue, or comments on it, for the failed checks the public install
//            smoke (`native-release-smoke.yml`) left as data in a directory.
//   node release-publish.mjs publish --product P --tag TAG [--components FILE]
//   node release-publish.mjs verify  --product P --tag TAG --directory DIR [--run-url URL] [--attempts N]
//   node release-publish.mjs report  --product P --tag TAG --directory DIR [--run-url URL]
// Both run with a token that can write, so they run this repository's main and never the
// release commit's code.
import { spawnSync } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import {
  checkLatestTag,
  productOfTag,
  publishFlags,
  releasePolicy,
} from './native-release-policy.mjs';
import { isReleased, parseComponentMap } from './ci-scope.mjs';
import { BUNDLE_ASSET, FAILURE_ASSET, HOLD_ASSET } from './plan-release-builds.mjs';
import { ghApi } from './release-draft-assets.mjs';
import { isAlphaVersion, versionOfTag } from './release-versions.mjs';

const COMMIT = /^[0-9a-f]{40}$/;
/** The component map of this repository, which decides what is released. */
const COMPONENTS = fileURLToPath(new URL('../../.github/components.json', import.meta.url));
/** GitHub makes the attestation and the immutable state shortly after publication, not atomically. */
const VERIFY_ATTEMPTS = 8;
const VERIFY_WAIT_MS = 15_000;

/**
 * Why this draft may not be published now, or an empty string. The workflow publishes only after
 * the gates passed, but publication cannot be undone, so the draft is read again here, and the
 * limits of the standing authorization are enforced here too: only a release of the alpha channel,
 * and only of a component that is released (`released` is false for `release: false`).
 */
export function publishBlocker({ release, product, tag, released = true }) {
  if (productOfTag(tag) !== product) return `${tag} is not a ${product} release tag`;
  if (!isAlphaVersion(versionOfTag(tag, product))) {
    return `${tag} is not an alpha release (X.Y.Z-alpha.N); only the owner publishes any other`;
  }
  if (!released) return `${product} is not released (release: false in .github/components.json)`;
  if (!release) return `there is no release ${tag}`;
  if (release.draft !== true) return `${tag} is already published`;
  const names = new Set((release.assets ?? []).map(({ name }) => name));
  if (!names.has(BUNDLE_ASSET)) return `${tag} has no verified bundle (${BUNDLE_ASSET})`;
  if (names.has(HOLD_ASSET)) return `${tag} is held (${HOLD_ASSET})`;
  if (names.has(FAILURE_ASSET)) return `${tag} carries a recorded failure (${FAILURE_ASSET})`;
  if (!COMMIT.test(release.target_commitish)) {
    return `the target ${release.target_commitish} of ${tag} is not a commit`;
  }
  return '';
}

/** Publishes the draft with the product's flags. Returns the flags it applied. */
export function publishDraft({ api, product, tag, released = true }) {
  const release = api.listReleases().find(({ tag_name: name }) => name === tag);
  const blocker = publishBlocker({ release, product, tag, released });
  if (blocker) throw new Error(`Not publishing: ${blocker}.`);
  const flags = publishFlags(product);
  api.publish(tag, flags);
  return { flags };
}

const pass = (check, reason = '') => ({ check, ok: true, reason });
const fail = (check, reason) => ({ check, ok: false, reason });

/**
 * The checks of the release object itself. `latest` is the repository's latest release (or
 * null) and `tagCommit` the commit the published tag points at.
 */
export function checkPublishedRelease({ release, latest, tagCommit, product, tag }) {
  const policy = releasePolicy(product);
  const results = [];
  results.push(
    release.draft === false ? pass('published') : fail('published', `${tag} is still a draft`)
  );
  results.push(
    release.immutable === true
      ? pass('immutable')
      : fail('immutable', `${tag} is not immutable; release immutability may be off`)
  );

  const flags = [];
  if (release.prerelease !== policy.prerelease) {
    flags.push(`prerelease is ${release.prerelease}, the policy says ${policy.prerelease}`);
  }
  if (policy.latest) {
    if (latest?.tag_name !== tag) {
      flags.push(`the latest release is ${latest?.tag_name ?? 'missing'}, not ${tag}`);
    }
  } else if (latest?.tag_name === tag) {
    flags.push(`${tag} became the latest release, which only a CLI release may be`);
  } else if (!latest) {
    flags.push('the repository has no latest release, so install.sh would not resolve');
  } else {
    try {
      checkLatestTag(latest.tag_name);
    } catch (error) {
      flags.push(error.message);
    }
  }
  results.push(
    flags.length === 0
      ? pass('flags', `${policy.prerelease ? 'prerelease' : 'release'}, latest ${policy.latest}`)
      : fail('flags', flags.join('; '))
  );

  results.push(
    tagCommit === release.target_commitish
      ? pass('tag', `${tag} is on ${release.target_commitish.slice(0, 8)}`)
      : fail(
          'tag',
          `${tag} points at ${tagCommit ?? 'nothing'}, not at ${release.target_commitish}`
        )
  );
  results.push(
    (release.assets ?? []).some(({ name }) => name === BUNDLE_ASSET)
      ? pass('bundle', `${BUNDLE_ASSET} is public`)
      : fail('bundle', `${BUNDLE_ASSET} is missing from the published release`)
  );
  return results;
}

/** Runs `attempt` until it is ok, at most `attempts` times, waiting between tries. */
function settle(attempt, { attempts, wait }) {
  let result = attempt();
  for (let count = 1; !result.ok && count < attempts; count += 1) {
    wait();
    result = attempt();
  }
  return result;
}

/**
 * Reads the published release back and returns one result per check. `api` reads the release,
 * the latest release and the tag's commit, downloads the assets and runs GitHub's attestation
 * verification; immutability, `latest` and the release attestation are waited for because GitHub
 * finishes them shortly after publishing.
 */
export function verifyPublication({
  api,
  product,
  tag,
  directory,
  attempts = VERIFY_ATTEMPTS,
  sleep = () => {},
}) {
  const policy = releasePolicy(product);
  const wait = () => sleep(VERIFY_WAIT_MS);
  const retry = { attempts, wait };
  const release = settle(() => {
    const current = api.getRelease(tag);
    return { ok: current?.immutable === true, current };
  }, retry).current;
  if (!release) return [fail('published', `there is no published release ${tag}`)];

  // GitHub moves `latest` a moment after publishing, like the rest: wait for the state the policy
  // expects before judging it.
  const latest = settle(() => {
    const current = api.latestRelease();
    return { ok: (current?.tag_name === tag) === policy.latest, current };
  }, retry).current;
  const results = checkPublishedRelease({
    release,
    latest,
    tagCommit: api.tagCommit(tag),
    product,
    tag,
  });

  const attestation = settle(() => api.verifyRelease(tag), retry);
  results.push(
    attestation.ok
      ? pass('attestation', 'gh release verify passed')
      : fail('attestation', `gh release verify failed: ${attestation.output}`)
  );

  const downloaded = api.download(tag, directory);
  if (!downloaded.ok) {
    results.push(fail('assets', `the assets could not be downloaded: ${downloaded.output}`));
    return results;
  }
  const names = (release.assets ?? []).map(({ name }) => name);
  const failed = [];
  for (const name of names) {
    // Once the release attestation verifies, it covers every asset: one try each.
    const outcome = api.verifyAsset(tag, path.join(directory, name));
    if (!outcome.ok) failed.push(`${name}: ${outcome.output}`);
  }
  results.push(
    failed.length === 0
      ? pass('assets', `gh release verify-asset passed for ${names.length} assets`)
      : fail('assets', `gh release verify-asset failed for ${failed.join('; ')}`)
  );
  return results;
}

/** The monitor still recognizes historical anonymous rate-limit issues. */
export function postPublicationIssueTitles(tag) {
  return {
    failure: `Release ${tag} failed its post-publication checks`,
    rateLimit: `Release ${tag} public install blocked by GitHub API rate limit`,
  };
}

/** The title and body of the issue a failed check opens. */
export function renderFailureIssue({ tag, results, runUrl }) {
  const failed = results.filter(({ ok }) => !ok);
  const titles = postPublicationIssueTitles(tag);
  return {
    title: titles.failure,
    body: [
      `The release pipeline published \`${tag}\`, and these checks of the published release failed:`,
      '',
      ...failed.map(({ check, reason }) => `- \`${check}\`: ${reason}`),
      '',
      ...(runUrl ? [`Run: ${runUrl}`, ''] : []),
      'Nothing was rolled back: a published release cannot be undone and, with immutability on, its assets and tag cannot be changed. The owner decides whether it stays as it is or a new reviewed version repairs it. Later drafts of the product publish only while their own gates pass, and the immutability gate stops them if the repository setting is off.',
      '',
      'Opened by `typescript/scripts/release-publish.mjs` (the post-publication checks and the public install smoke).',
    ].join('\n'),
  };
}

/** Opens the failure issue, or comments on the one already open for this release. */
export function reportFailure({ api, tag, results, runUrl }) {
  const { title, body } = renderFailureIssue({ tag, results, runUrl });
  const open = api.openIssue(title);
  if (open) {
    api.commentIssue(open, body);
    return { issue: open, created: false };
  }
  return { issue: api.createIssue(title, body), created: true };
}

/**
 * The failed checks the smoke legs left in `directory`, one result each, named with its host. The
 * legs wrote them, from what the installed release printed, so they are data: bounded strings
 * on one line, never trusted beyond the issue text. No file at all (the download of the
 * artifacts failed) still reports a failure, since the run says a leg failed.
 */
export function readSmokeFailures(
  directory,
  { expectedResults = 0, artifactPrefix = 'smoke-failures-' } = {}
) {
  let observed = 0;
  const line = (text) =>
    String(text)
      .replace(/\p{Cc}+/gu, ' ')
      .trim()
      .slice(0, 500);
  const results = [];
  for (const name of existsSync(directory) ? readdirSync(directory).sort() : []) {
    if (!name.startsWith(artifactPrefix)) continue;
    observed += 1;
    const target = name.slice(artifactPrefix.length);
    try {
      const { failed } = JSON.parse(
        readFileSync(path.join(directory, name, 'smoke-result.json'), 'utf8')
      );
      if (!Array.isArray(failed) || failed.length > 10) throw new Error('Invalid smoke failures');
      for (const { check, reason } of failed) {
        if (typeof check !== 'string' || typeof reason !== 'string')
          throw new Error('Invalid smoke failure');
        results.push({
          check: `${line(check)} (${line(target)})`,
          ok: false,
          reason: line(reason),
        });
      }
    } catch {
      results.push({
        check: `public install (${line(target)})`,
        ok: false,
        reason: 'its result file could not be read; see the run',
      });
    }
  }
  if (expectedResults && observed !== expectedResults) {
    results.push({
      check: 'public install evidence',
      ok: false,
      reason: `expected ${expectedResults} host results, found ${observed}; complete smoke evidence is required`,
    });
  }
  return results.length > 0
    ? results
    : [
        {
          check: 'public install',
          ok: false,
          reason: 'a host failed and no details were kept; see the run',
        },
      ];
}

/** Markdown for the run summary. */
export function renderVerifySummary({ tag, results }) {
  const lines = [`### Published release \`${tag}\``, ''];
  for (const { check, ok, reason } of results) {
    lines.push(`- ${ok ? 'passed' : 'FAILED'} \`${check}\`${reason ? `: ${reason}` : ''}`);
  }
  return `${lines.join('\n')}\n`;
}

// ---- GitHub, through gh ------------------------------------------------------------------

/** `gh` for one repository: the draft API of `release-draft-assets.mjs` plus what publishing needs. */
export function ghPublishApi({ repository, env = process.env, spawn = spawnSync }) {
  const run = (args, timeout = 120_000) => {
    const result = spawn('gh', args, {
      env,
      encoding: 'utf8',
      timeout,
      maxBuffer: 64 * 1024 * 1024,
    });
    if (result.error) throw result.error;
    return result;
  };
  const text = (result) => `${result.stdout ?? ''}${result.stderr ?? ''}`.trim();
  const json = (args) => {
    const result = run(args);
    if (result.status !== 0) throw new Error(`gh ${args[0]} failed: ${text(result)}`);
    return JSON.parse(result.stdout);
  };
  const outcome = (result) => ({ ok: result.status === 0, output: text(result) });
  return {
    ...ghApi({ repository, env, spawn }),
    publish: (tag, flags) => {
      const result = run(['release', 'edit', tag, '--repo', repository, ...flags]);
      if (result.status !== 0) throw new Error(`gh release edit failed: ${text(result)}`);
    },
    getRelease: (tag) => {
      const result = run(['api', `repos/${repository}/releases/tags/${tag}`]);
      return result.status === 0 ? JSON.parse(result.stdout) : null;
    },
    latestRelease: () => {
      const result = run(['api', `repos/${repository}/releases/latest`]);
      return result.status === 0 ? JSON.parse(result.stdout) : null;
    },
    tagCommit: (tag) => {
      const result = run(['api', `repos/${repository}/commits/${tag}`]);
      return result.status === 0 ? JSON.parse(result.stdout).sha : null;
    },
    download: (tag, directory) =>
      outcome(run(['release', 'download', tag, '--repo', repository, '--dir', directory], 600_000)),
    verifyRelease: (tag) => outcome(run(['release', 'verify', tag, '--repo', repository])),
    verifyAsset: (tag, file) =>
      outcome(run(['release', 'verify-asset', tag, file, '--repo', repository])),
    openIssue: (title) => {
      const matches = [];
      for (let page = 1; page <= 10; page += 1) {
        const issues = json([
          'api',
          `repos/${repository}/issues?state=open&per_page=100&page=${page}`,
        ]);
        matches.push(...issues.filter((issue) => !issue.pull_request && issue.title === title));
        if (issues.length < 100) {
          if (matches.length > 1) throw new Error(`Multiple open issues have title ${title}.`);
          return matches[0]?.number ?? null;
        }
      }
      throw new Error('Open-issue discovery exceeded 10 pages.');
    },
    createIssue: (title, body) =>
      json([
        'api',
        `repos/${repository}/issues`,
        '--method',
        'POST',
        '-f',
        `title=${title}`,
        '-f',
        `body=${body}`,
      ]).number,
    commentIssue: (number, body) =>
      json([
        'api',
        `repos/${repository}/issues/${number}/comments`,
        '--method',
        'POST',
        '-f',
        `body=${body}`,
      ]),
  };
}

function output(environment, values) {
  const lines = Object.entries(values)
    .map(([key, value]) => `${key}=${value}\n`)
    .join('');
  if (environment.GITHUB_OUTPUT) appendFileSync(environment.GITHUB_OUTPUT, lines);
  else process.stdout.write(lines);
}

function report(environment, markdown) {
  process.stderr.write(markdown);
  if (environment.GITHUB_STEP_SUMMARY) appendFileSync(environment.GITHUB_STEP_SUMMARY, markdown);
}

function main(argv, environment) {
  const [command, ...rest] = argv;
  const { values } = parseArgs({
    args: rest,
    options: {
      product: { type: 'string' },
      tag: { type: 'string' },
      directory: { type: 'string' },
      'run-url': { type: 'string', default: '' },
      attempts: { type: 'string', default: String(VERIFY_ATTEMPTS) },
      'expected-results': { type: 'string', default: '0' },
      components: { type: 'string', default: COMPONENTS },
    },
  });
  for (const name of ['product', 'tag']) {
    if (!values[name]) throw new Error(`--${name} is required.`);
  }
  const repository = environment.GITHUB_REPOSITORY;
  if (!repository) throw new Error('GITHUB_REPOSITORY is not set.');
  const api = ghPublishApi({ repository });

  if (command === 'publish') {
    const map = parseComponentMap(readFileSync(values.components, 'utf8'));
    const { flags } = publishDraft({
      api,
      product: values.product,
      tag: values.tag,
      released: isReleased(map, values.product),
    });
    report(
      environment,
      `### Published \`${values.tag}\`\n\nEvery gate passed; it was published with \`${flags.join(' ')}\`. The next job checks the published release.\n`
    );
    output(environment, { published: true });
  } else if (command === 'verify') {
    if (!values.directory) throw new Error('verify needs --directory.');
    const attempts = Number(values.attempts);
    if (!Number.isInteger(attempts) || attempts < 1) {
      throw new Error('--attempts must be a positive whole number.');
    }
    mkdirSync(values.directory, { recursive: true });
    const results = verifyPublication({
      api,
      product: values.product,
      tag: values.tag,
      directory: values.directory,
      attempts,
      sleep: (milliseconds) =>
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds),
    });
    report(environment, renderVerifySummary({ tag: values.tag, results }));
    if (results.some(({ ok }) => !ok)) {
      try {
        const { issue, created } = reportFailure({
          api,
          tag: values.tag,
          results,
          runUrl: values['run-url'],
        });
        process.stderr.write(`${created ? 'Opened' : 'Commented on'} issue #${issue}.\n`);
      } catch (error) {
        process.stderr.write(`The failure could not be reported as an issue: ${error.message}\n`);
      }
      process.exitCode = 1;
    }
  } else if (command === 'report') {
    if (!values.directory) throw new Error('report needs --directory.');
    const expectedResults = Number(values['expected-results']);
    if (!Number.isSafeInteger(expectedResults) || expectedResults < 0 || expectedResults > 10)
      throw new Error('--expected-results must be a whole number from 0 to 10.');
    const results = readSmokeFailures(values.directory, {
      expectedResults,
      artifactPrefix: `smoke-failures-${values.product}-${values.tag}-`,
    });
    report(environment, renderVerifySummary({ tag: values.tag, results }));
    try {
      const { issue, created } = reportFailure({
        api,
        tag: values.tag,
        results,
        runUrl: values['run-url'],
      });
      process.stderr.write(`${created ? 'Opened' : 'Commented on'} issue #${issue}.\n`);
    } catch (error) {
      process.stderr.write(`The failure could not be reported as an issue: ${error.message}\n`);
      process.exitCode = 1;
    }
  } else {
    throw new Error('Usage: release-publish.mjs publish|verify|report --product P --tag TAG ...');
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2), process.env);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
