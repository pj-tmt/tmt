#!/usr/bin/env node
// The gates a complete draft release passes before anything may publish it, in order:
//   channel       the release is an alpha (`X.Y.Z-alpha.N`); any other version is the owner's to publish
//   commit        the release's commit is on main and its pull request passed the required checks
//   immutability  the repository's newest published release is immutable, so the setting is on
//   monotonic     the release is newer than every published release of its product
//   migration     no breaking change since the last published release, and outside the alpha channel
//                 no new SQLite migration (an alpha publishes its migrations, which are forward-only)
//   upgrade       the upgrade from the last published release was proven (native-release-upgrade.yml)
// A failed gate holds the draft: `publication-held.json` on the draft says which gate, why and
// which run, and the draft is neither built again nor published until the owner publishes it by
// hand or releases the hold by dispatch, which skips only the gate named in the marker (never
// `channel`: a release that is not an alpha is published by hand, with the owner's explicit OK).
//   node publication-gates.mjs early --product P --tag TAG [--release-hold]
//   node publication-gates.mjs finish --product P --tag TAG --upgrade-result R --upgrade-outcome O \
//        [--upgrade-reason TEXT] [--skip GATE]
// Both run with a token that can see drafts, so they run this repository's main and only read the
// release commit's data through git and the API, never its code.
import { spawnSync } from 'node:child_process';
import { appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { readReleaseSourceAtRef } from './release-source-at-ref.mjs';
import { componentOfProduct } from './native-release-policy.mjs';
import {
  attributeCutCommits,
  parseReleaseCommits,
  readCutRange,
  releaseCutHistory,
} from './release-cut.mjs';
import { BUNDLE_ASSET, FAILURE_ASSET, HOLD_ASSET } from './plan-release-builds.mjs';
import { clearHold, ghApi, readHold, recordHold } from './release-draft-assets.mjs';
import {
  compareVersions,
  isAlphaVersion,
  publishedReleases,
  versionOfTag,
} from './release-versions.mjs';

/** The required contexts of `main`, which the pull request of the release commit must have passed. */
export const REQUIRED_CONTEXTS = [
  'Code quality',
  'Unit tests',
  'Docker E2E',
  'Native package matrix',
];
/** The gates that need no proof run, cheapest first. */
export const EARLY_GATES = ['channel', 'commit', 'immutability', 'monotonic', 'migration'];
/** The gate a hold release never skips: publishing a non-alpha release is the owner's act. */
export const UNSKIPPABLE_GATE = 'channel';
export const GATES = [...EARLY_GATES, 'upgrade'];

const pass = (reason = '') => ({ ok: true, reason });
const fail = (reason) => ({ ok: false, reason });
const short = (sha) => sha.slice(0, 8);

/**
 * The standing authorization covers alpha releases only. A stable version or another pre-release
 * label (beta, rc) passes every other gate and must still not publish without the owner.
 */
export function checkChannel({ product, tag }) {
  const version = versionOfTag(tag, product);
  return isAlphaVersion(version)
    ? pass(`${version} is an alpha release`)
    : fail(
        `${tag} is not an alpha release (X.Y.Z-alpha.N); only alpha releases publish automatically, any other the owner publishes by hand`
      );
}

/**
 * `pullRequest` is the merged pull request that produced the commit, with its head commit;
 * `checkRuns` are the check runs of that head. The latest completed run of a context counts, so
 * a re-run that passed replaces the failed one before it.
 */
export function checkCommit({ sha, onMain, pullRequest, checkRuns }) {
  if (!onMain) return fail(`commit ${short(sha)} is not on main`);
  if (!pullRequest) return fail(`no merged pull request produced commit ${short(sha)}`);
  for (const context of REQUIRED_CONTEXTS) {
    const latest = checkRuns
      .filter((run) => run.name === context && run.status === 'completed')
      .sort((left, right) => Date.parse(right.completed_at) - Date.parse(left.completed_at))[0];
    if (!latest) {
      return fail(`required check "${context}" did not complete on #${pullRequest.number}`);
    }
    if (latest.conclusion !== 'success') {
      return fail(`required check "${context}" ${latest.conclusion} on #${pullRequest.number}`);
    }
  }
  return pass(`#${pullRequest.number} passed ${REQUIRED_CONTEXTS.join(', ')}`);
}

/**
 * The setting itself is not readable by the workflow token, so the newest published release of
 * the repository stands for it: it shows that immutability was on when that release was made.
 */
export function checkImmutability({ releases }) {
  const [newest] = releases
    .filter((release) => release.draft !== true)
    .sort((left, right) => Date.parse(right.published_at) - Date.parse(left.published_at));
  if (!newest) return fail('no published release shows that release immutability is on');
  return newest.immutable === true
    ? pass(`${newest.tag_name} is immutable`)
    : fail(`the newest published release ${newest.tag_name} is not immutable`);
}

/** Stable latest remains monotonic; alpha tags are unique and converge latest separately. */
export function checkMonotonic({ releases, product, tag }) {
  if (isAlphaVersion(versionOfTag(tag, product))) {
    return releases.some((release) => release.draft !== true && release.tag_name === tag)
      ? fail(`${tag} is already published`)
      : pass('unique alpha tag; latest selection is verified independently');
  }
  const [newest] = publishedReleases(releases, product);
  if (!newest) return pass('no published release of the product yet');
  return compareVersions(versionOfTag(tag, product), versionOfTag(newest.tag_name, product)) > 0
    ? pass(`newer than ${newest.tag_name}`)
    : fail(`${newest.tag_name} is already published and is not older than ${tag}`);
}

/**
 * The number of entries of the `MIGRATIONS` list in a Rust source: the top-level elements of the
 * `&[ ... ]` it is assigned, whatever their shape, skipping strings and comments.
 */
export function countMigrations(source) {
  const start = /\bconst\s+MIGRATIONS\s*:[^=]*=\s*&\[/.exec(source);
  if (!start) return 0;
  let depth = 1;
  let count = 0;
  let pending = false;
  for (let index = start.index + start[0].length; index < source.length; index += 1) {
    const char = source[index];
    const next = source[index + 1];
    if (char === '/' && next === '/') {
      while (index < source.length && source[index] !== '\n') index += 1;
    } else if (char === '/' && next === '*') {
      index = source.indexOf('*/', index + 2);
      if (index < 0) throw new Error('An unterminated comment in the migration list.');
      index += 1;
    } else if (char === '"') {
      pending = true;
      for (index += 1; index < source.length && source[index] !== '"'; index += 1) {
        if (source[index] === '\\') index += 1;
      }
    } else if ('([{'.includes(char)) {
      pending = true;
      depth += 1;
    } else if (')]}'.includes(char)) {
      depth -= 1;
      if (depth === 0) return pending ? count + 1 : count;
    } else if (char === ',' && depth === 1) {
      if (pending) count += 1;
      pending = false;
    } else if (!/\s/.test(char)) {
      pending = true;
    }
  }
  throw new Error('The migration list is not closed.');
}

/** A conventional-commit `!` after the type or scope, or a `BREAKING CHANGE:` footer. */
export function isBreaking({ subject, body = '', breaking = false }) {
  return breaking || /^[a-z]+(\([^)]*\))?!:/.test(subject) || /^BREAKING[ -]CHANGE:/m.test(body);
}

/**
 * Holds a release that carries a breaking change, and outside the alpha channel one that carries
 * a new migration: `counts` are the entries of each of the component's migration files at the
 * candidate's commit and at its newest published ancestor release (`previous.counts`), `commits` the
 * release's own commits. An alpha publishes its migrations, which are forward-only, so it only
 * reports them.
 */
export function checkMigration({ files, counts, previous, commits, alpha = false }) {
  if (!previous) return pass('the first release of the product: nothing to compare with');
  const added = files.find((file) => counts[file] > (previous.counts[file] ?? 0));
  const growth = added
    ? `${added} has ${counts[added]} migrations, ${previous.counts[added] ?? 0} in ${previous.tag}`
    : '';
  if (added && !alpha) return fail(growth);
  const breaking = commits.find(isBreaking);
  if (breaking)
    return fail(`commit ${short(breaking.sha)} is a breaking change: ${breaking.subject}`);
  return pass(
    added
      ? `${growth}; an alpha publishes its migrations, and no commit is a breaking change`
      : `no new migration and no breaking change since ${previous.tag}`
  );
}

/**
 * The gate of the upgrade proof, from the result of `native-release-upgrade.yml`: it proved the
 * upgrade, found nothing to upgrade from, or could not run at this commit (`reason` then says why,
 * verbatim, so the owner sees it).
 */
export function checkUpgrade({ result, outcome, reason = '', url = '' }) {
  if (outcome === 'predates') return fail(reason);
  if (result === 'success' && outcome === 'nothing') {
    return pass('the first release of the product: nothing to upgrade from');
  }
  if (result === 'success' && outcome === 'proved') return pass('the upgrade proof passed');
  // `reason` is the cause the failed hosts' logs name, when there is one; the run is always cited.
  const detail = [reason, url && (reason ? `(${url})` : url)].filter(Boolean).join(' ');
  return fail(`the upgrade proof ${result}${detail ? `: ${detail}` : ''}`);
}

/**
 * Runs the named gates in order and stops at the first that fails. `skip` names the one gate a
 * hold release does not run. `checks` maps a gate to a function, so evidence is only gathered
 * for gates that run.
 */
export function runGates({ order, checks, skip = '' }) {
  const results = [];
  for (const gate of order) {
    if (gate === skip) {
      results.push({ gate, ok: true, skipped: true, reason: 'skipped by the hold release' });
      continue;
    }
    const outcome = checks[gate]();
    results.push({ gate, ...outcome });
    if (!outcome.ok) return { held: { gate, reason: outcome.reason }, results };
  }
  return { held: null, results };
}

/** Markdown for the run summary. */
export function renderGateSummary({ tag, results, held }) {
  const lines = [`### Publication gates for \`${tag}\``, ''];
  for (const { gate, ok, skipped, reason } of results) {
    lines.push(
      `- ${skipped ? 'skipped' : ok ? 'passed' : 'FAILED'} \`${gate}\`${reason ? `: ${reason}` : ''}`
    );
  }
  lines.push(
    '',
    held
      ? `**Held** at \`${held.gate}\`: ${held.reason}. It carries \`${HOLD_ASSET}\` until it is published by hand or the hold is released by dispatch.`
      : 'Every gate passed. The next job publishes the release.'
  );
  return `${lines.join('\n')}\n`;
}

// ---- evidence, gathered from git and the GitHub API --------------------------------------

function spawn(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: 'utf8',
    timeout: 120_000,
    maxBuffer: 64 * 1024 * 1024,
    ...options,
  });
  if (result.error) throw result.error;
  return result;
}
const git = (args) => {
  const result = spawn('git', args);
  if (result.status !== 0)
    throw new Error(`git ${args[0]} failed: ${result.stderr.trim()}`, {
      cause: { status: result.status },
    });
  return result.stdout;
};
const ghJson = (args) => {
  const result = spawn('gh', args);
  if (result.status !== 0) throw new Error(`gh ${args[0]} failed: ${result.stderr.trim()}`);
  return JSON.parse(result.stdout);
};

function fileCount(sha, file) {
  const result = spawn('git', ['show', `${sha}:${file}`]);
  return result.status === 0 ? countMigrations(result.stdout) : 0;
}

/** Publication sees the same cut range, additive consumers and nested breaks as the notes. */
export function releaseCommits({ from, to, product, map, workspace }, readGit = git) {
  return attributeCutCommits(readCutRange(readGit, from, to), map, product, workspace).map(
    (commit) => ({
      sha: commit.sha,
      subject: commit.message.split('\n')[0],
      body: commit.message,
      breaking: parseReleaseCommits([commit]).some((parsed) => parsed.notes.length > 0),
    })
  );
}

function earlyChecks({ product, tag, release, releases, repository }) {
  const sha = release.target_commitish;
  return {
    channel: () => checkChannel({ product, tag }),
    commit: () => {
      const onMain = spawn('git', ['merge-base', '--is-ancestor', sha, 'HEAD']).status === 0;
      const pulls = onMain ? ghJson(['api', `repos/${repository}/commits/${sha}/pulls`]) : [];
      const pullRequest = pulls.find((pull) => pull.merged_at && pull.merge_commit_sha === sha);
      const checkRuns = pullRequest
        ? ghJson([
            'api',
            '--paginate',
            '--slurp',
            `repos/${repository}/commits/${pullRequest.head.sha}/check-runs`,
          ]).flatMap((page) => page.check_runs)
        : [];
      return checkCommit({
        sha,
        onMain,
        pullRequest: pullRequest && { number: pullRequest.number, head: pullRequest.head.sha },
        checkRuns,
      });
    },
    immutability: () => checkImmutability({ releases }),
    monotonic: () => checkMonotonic({ releases, product, tag }),
    migration: () => {
      const { map, workspace } = readReleaseSourceAtRef(sha, { root: process.cwd(), warm: true });
      const component = componentOfProduct(map, product);
      const alpha = isAlphaVersion(versionOfTag(tag, product));
      const previousCut = releaseCutHistory({
        releases,
        product,
        cut: sha,
        git,
        excludeTag: tag,
      }).previous;
      const count = (at) =>
        Object.fromEntries(component.migrations.map((file) => [file, fileCount(at, file)]));
      if (!previousCut) {
        return checkMigration({
          files: component.migrations,
          counts: {},
          previous: null,
          commits: [],
          alpha,
        });
      }
      const previousSha = previousCut.sha;
      return checkMigration({
        files: component.migrations,
        counts: count(sha),
        previous: { tag: previousCut.tag, counts: count(previousSha) },
        commits: releaseCommits({
          from: previousSha,
          to: sha,
          product,
          map,
          workspace,
        }),
        alpha,
      });
    },
  };
}

function assetText({ repository }) {
  return (asset) => {
    const result = spawn('gh', [
      'api',
      '-H',
      'Accept: application/octet-stream',
      `repos/${repository}/releases/assets/${asset.id}`,
    ]);
    if (result.status !== 0) throw new Error(`gh could not read ${asset.name}: ${result.stderr}`);
    return result.stdout;
  };
}

function output(environment, values) {
  const text = Object.entries(values)
    .map(([key, value]) => `${key}=${value}\n`)
    .join('');
  if (environment.GITHUB_OUTPUT) appendFileSync(environment.GITHUB_OUTPUT, text);
  else process.stdout.write(text);
}

function report(environment, text) {
  process.stderr.write(text);
  if (environment.GITHUB_STEP_SUMMARY) appendFileSync(environment.GITHUB_STEP_SUMMARY, text);
}

function main(argv, environment) {
  const [command, ...rest] = argv;
  const { values } = parseArgs({
    args: rest,
    options: {
      product: { type: 'string' },
      tag: { type: 'string' },
      rerun: { type: 'boolean', default: false },
      'rerun-gate': { type: 'string', default: '' },
      'release-hold': { type: 'boolean', default: false },
      'upgrade-result': { type: 'string', default: '' },
      'upgrade-outcome': { type: 'string', default: '' },
      'upgrade-reason': { type: 'string', default: '' },
      skip: { type: 'string', default: '' },
    },
  });
  for (const name of ['product', 'tag']) {
    if (!values[name]) throw new Error(`--${name} is required.`);
  }
  const repository = environment.GITHUB_REPOSITORY;
  if (!repository) throw new Error('GITHUB_REPOSITORY is not set.');
  const runUrl = `${environment.GITHUB_SERVER_URL ?? 'https://github.com'}/${repository}/actions/runs/${environment.GITHUB_RUN_ID ?? ''}`;
  const api = ghApi({ repository });
  const releases = api.listReleases();
  const release = releases.find((candidate) => candidate.tag_name === values.tag);
  if (!release) throw new Error(`There is no release ${values.tag}.`);
  if (release.draft !== true) throw new Error(`Release ${values.tag} is already published.`);
  const sha = release.target_commitish;
  // Re-read the durable hold at each job boundary; never clear a different release or gate.
  const rerunHold = (gate = '') => {
    const hold = readHold({ api, tag: values.tag, download: assetText({ repository }) });
    const names = new Set((release.assets ?? []).map(({ name }) => name));
    if (
      !names.has(BUNDLE_ASSET) ||
      names.has(FAILURE_ASSET) ||
      !hold ||
      hold.tag !== values.tag ||
      hold.sha !== sha ||
      !GATES.includes(hold.gate) ||
      (gate && hold.gate !== gate)
    ) {
      throw new Error(
        'Rerun gate or marker mismatch; requires a matching bundled hold with a known gate, tag and commit. The hold is unchanged.'
      );
    }
    return hold;
  };

  if (command === 'early') {
    if (values.rerun && values['release-hold'])
      throw new Error('Rerun and release-hold are separate runs.');
    let skip = '';
    let rerunGate = '';
    if (values.rerun) {
      rerunGate = rerunHold().gate;
    }
    if (values['release-hold']) {
      const hold = readHold({ api, tag: values.tag, download: assetText({ repository }) });
      if (!hold) throw new Error(`Draft ${values.tag} carries no ${HOLD_ASSET} to release.`);
      if (hold.gate === UNSKIPPABLE_GATE) {
        throw new Error(
          `Draft ${values.tag} is held by the ${UNSKIPPABLE_GATE} gate: a release that is not an alpha is published by hand, with the owner's explicit OK, never by releasing the hold.`
        );
      }
      skip = hold.gate;
    }
    const { held, results } = runGates({
      order: EARLY_GATES,
      checks: earlyChecks({
        product: values.product,
        tag: values.tag,
        release,
        releases,
        repository,
      }),
      skip: EARLY_GATES.includes(skip) ? skip : '',
    });
    if (held && !values.rerun) recordHold({ api, tag: values.tag, hold: { sha, ...held, runUrl } });
    report(environment, renderGateSummary({ tag: values.tag, results, held }));
    output(environment, {
      held: held?.gate ?? '',
      skip,
      ...(values.rerun ? { rerun_gate: rerunGate } : {}),
    });
  } else if (command === 'finish') {
    const rerunGate = values['rerun-gate'];
    if (rerunGate) {
      if (values.skip) throw new Error('Rerun cannot skip a gate; the hold is unchanged.');
      rerunHold(rerunGate);
    }
    const skipped = values.skip === 'upgrade';
    const outcome = skipped
      ? pass()
      : checkUpgrade({
          result: values['upgrade-result'],
          outcome: values['upgrade-outcome'],
          reason: values['upgrade-reason'],
          url: runUrl,
        });
    const held = outcome.ok ? null : { gate: 'upgrade', reason: outcome.reason };
    if (held) {
      if (!rerunGate) recordHold({ api, tag: values.tag, hold: { sha, ...held, runUrl } });
    } else clearHold({ api, tag: values.tag });
    report(
      environment,
      renderGateSummary({
        tag: values.tag,
        held,
        results: [
          {
            gate: 'upgrade',
            ok: outcome.ok,
            skipped,
            reason: skipped ? 'skipped by the hold release' : outcome.reason,
          },
        ],
      })
    );
    output(environment, { held: held?.gate ?? '' });
  } else {
    throw new Error('Usage: publication-gates.mjs early|finish --product P --tag TAG ...');
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
