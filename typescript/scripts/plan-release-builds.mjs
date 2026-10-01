#!/usr/bin/env node
// Plans which draft releases of one product the release run has to work on, oldest first:
// every draft that has no verified bundle and no recorded failure (it is built and verified),
// and every complete draft that carries no hold (it waits for its gates and its publication).
// The run plans from these durable markers on the draft itself, so a run that was replaced
// while it waited for its concurrency group, or one that stopped before it published, loses
// nothing: the run that follows plans the same drafts.
//   gh api --paginate --slurp repos/OWNER/REPO/releases \
//     | node plan-release-builds.mjs --product cli [--retry TAG | --hold TAG] [--components FILE]
import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { isReleased, parseComponentMap } from './ci-scope.mjs';
import { productOfTag } from './native-release-policy.mjs';

/** Uploaded last, after every verify job passed: a draft that has it carries a complete bundle. */
export const BUNDLE_ASSET = 'release-publication.json';
/** Uploaded when a job of a draft's pipeline failed: later runs skip the draft until a retry. */
export const FAILURE_ASSET = 'verification-failed.json';
/**
 * Uploaded when a publication gate failed for a complete bundle: which gate, why and which run. A
 * held draft is not published until the owner publishes it or releases the hold by dispatch.
 */
export const HOLD_ASSET = 'publication-held.json';

const COMMIT = /^[0-9a-f]{40}$/;
/** The component map of this repository, which decides what is released. */
const COMPONENTS = fileURLToPath(new URL('../../.github/components.json', import.meta.url));

const oldestFirst = (left, right) =>
  Date.parse(left.createdAt) - Date.parse(right.createdAt) || left.tag.localeCompare(right.tag);

/**
 * `releases` are GitHub release objects. Returns the drafts of `product` to build, the complete
 * drafts that await their publication, the drafts that are parked and the drafts that are held,
 * each oldest first. A `retry` tag plans exactly that draft, ignoring its failure marker, and
 * refuses a tag that is not an unbundled draft of the product.
 */
export function planReleaseBuilds({ releases, product, retry = '', hold = '', released = true }) {
  if (retry !== '' && hold !== '') {
    throw new Error('A retry and a released hold are separate runs; give one of them.');
  }
  const drafts = releases.filter(
    (release) => release.draft === true && productOfTag(release.tag_name) === product
  );
  if (!released) {
    if (retry !== '' || hold !== '') {
      throw new Error(`${product} is not released (release: false in .github/components.json).`);
    }
    return {
      builds: [],
      awaiting: [],
      blocked: [],
      held: [],
      unreleased: drafts.map((release) => ({ tag: release.tag_name })),
    };
  }
  const named = (release) => ({
    tag: release.tag_name,
    sha: release.target_commitish,
    createdAt: release.created_at,
  });
  const assetNames = (release) => new Set((release.assets ?? []).map(({ name }) => name));

  if (retry !== '') {
    const release = drafts.find(({ tag_name: tag }) => tag === retry);
    if (!release) {
      throw new Error(
        `Cannot retry ${retry}: it is not a draft release of the ${product} product.`
      );
    }
    if (assetNames(release).has(BUNDLE_ASSET)) {
      throw new Error(`Cannot retry ${retry}: it already carries a verified bundle.`);
    }
    if (!COMMIT.test(release.target_commitish)) {
      throw new Error(
        `Cannot retry ${retry}: its target ${release.target_commitish} is not a commit.`
      );
    }
    return { builds: [named(release)], awaiting: [], blocked: [], held: [] };
  }

  if (hold !== '') {
    const release = drafts.find(({ tag_name: tag }) => tag === hold);
    if (!release) {
      throw new Error(
        `Cannot release the hold of ${hold}: it is not a draft release of the ${product} product.`
      );
    }
    const assets = assetNames(release);
    if (!assets.has(BUNDLE_ASSET) || !assets.has(HOLD_ASSET)) {
      throw new Error(
        `Cannot release the hold of ${hold}: it has no bundle held by ${HOLD_ASSET}.`
      );
    }
    if (!COMMIT.test(release.target_commitish)) {
      throw new Error(
        `Cannot release the hold of ${hold}: its target ${release.target_commitish} is not a commit.`
      );
    }
    return { builds: [named(release)], awaiting: [], blocked: [], held: [] };
  }

  const builds = [];
  const awaiting = [];
  const blocked = [];
  const held = [];
  for (const release of drafts) {
    const assets = assetNames(release);
    if (assets.has(BUNDLE_ASSET)) {
      if (assets.has(HOLD_ASSET)) held.push({ tag: release.tag_name });
      else if (!COMMIT.test(release.target_commitish)) {
        blocked.push({
          tag: release.tag_name,
          reason: `its target ${release.target_commitish} is not a commit SHA, so the commit to publish is unknown`,
        });
      } else awaiting.push(named(release));
      continue;
    }
    if (!COMMIT.test(release.target_commitish)) {
      blocked.push({
        tag: release.tag_name,
        reason: `its target ${release.target_commitish} is not a commit SHA, so the commit to build is unknown`,
      });
    } else if (assets.has(FAILURE_ASSET)) {
      blocked.push({
        tag: release.tag_name,
        reason: 'its verification failed earlier; retry it by dispatch or delete the draft',
      });
    } else {
      builds.push(named(release));
    }
  }
  return {
    builds: builds.sort(oldestFirst),
    awaiting: awaiting.sort(oldestFirst),
    blocked,
    held: held.sort(oldestFirst),
  };
}

/** Markdown for the run summary. */
export function renderPlanSummary({
  product,
  builds,
  awaiting = [],
  blocked,
  held = [],
  unreleased = [],
  retry = '',
  hold = '',
}) {
  const lines = [`### Release run for ${product}`, ''];
  if (retry !== '') lines.push(`Retrying ${retry} by dispatch.`, '');
  if (hold !== '') lines.push(`Releasing the hold of ${hold} by dispatch.`, '');
  lines.push(
    builds.length === 0
      ? 'No draft release needs a build.'
      : `Drafts to build and verify, oldest first: ${builds.map(({ tag }) => `\`${tag}\``).join(', ')}.`
  );
  if (awaiting.length > 0) {
    lines.push(
      '',
      `Complete drafts that await their gates and publication, oldest first: ${awaiting.map(({ tag }) => `\`${tag}\``).join(', ')}.`
    );
  }
  if (unreleased.length > 0) {
    lines.push(
      '',
      `**Left alone** (${product} is not released, \`release: false\` in the component map):`
    );
    for (const { tag } of unreleased) lines.push(`- \`${tag}\``);
  }
  if (held.length > 0) {
    lines.push(
      '',
      `**Held drafts** (complete, not published; each carries \`${HOLD_ASSET}\` with the gate and the reason):`
    );
    for (const { tag } of held) lines.push(`- \`${tag}\``);
  }
  if (blocked.length > 0) {
    lines.push('', '**Parked drafts** (not built until retried by dispatch or deleted):');
    for (const { tag, reason } of blocked) lines.push(`- \`${tag}\`: ${reason}`);
  }
  return `${lines.join('\n')}\n`;
}

/** The pages `gh api --paginate --slurp` prints, or a plain list of releases, as one list. */
export function releasesFrom(parsed) {
  return parsed.flat();
}

function main(argv, stdin) {
  const { values } = parseArgs({
    args: argv,
    options: {
      product: { type: 'string' },
      retry: { type: 'string', default: '' },
      hold: { type: 'string', default: '' },
      components: { type: 'string', default: COMPONENTS },
    },
  });
  if (!values.product) {
    throw new Error(
      'Usage: plan-release-builds.mjs --product <product> [--retry <tag> | --hold <tag>]'
    );
  }
  const releases = releasesFrom(JSON.parse(stdin));
  const map = parseComponentMap(readFileSync(values.components, 'utf8'));
  const plan = planReleaseBuilds({
    releases,
    product: values.product,
    retry: values.retry,
    hold: values.hold,
    released: isReleased(map, values.product),
  });
  const summary = renderPlanSummary({
    product: values.product,
    retry: values.retry,
    hold: values.hold,
    ...plan,
  });
  process.stderr.write(summary);
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
  // One pipeline per draft, oldest first: a build and a waiting draft differ only in which jobs
  // of the pipeline have work left, which its own first job decides from the draft.
  const work = [...plan.builds, ...plan.awaiting].sort(oldestFirst);
  const outputs = [
    `matrix=${JSON.stringify({ include: work.map(({ tag, sha }) => ({ tag, sha })) })}`,
    `any=${work.length > 0}`,
  ].join('\n');
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${outputs}\n`);
  else process.stdout.write(`${outputs}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2), readFileSync(0, 'utf8'));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
