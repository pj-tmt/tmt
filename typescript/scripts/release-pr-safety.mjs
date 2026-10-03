// Read-only release PR gates. release-please retains ownership of changelog generation.
import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { parseComponentMap } from './ci-scope.mjs';
import { releasePolicy } from './native-release-policy.mjs';
import { compareVersions, publishedReleases } from './release-versions.mjs';
import { runPackedCommand } from './packed-command.mjs';
import { attributeReleaseConsumption } from './release-please-run.mjs';
import { loadReleasePleaseCommitRules } from './release-please-commits.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const PREFIX = 'release-please--branches--main--';
const SHA = /^[a-f0-9]{40}$/;
const PAGES = 10;
const REQUESTS = 60;
const QUEUE_COMMITS = 40;
const COVERAGE_COMMITS = 500;

/** Explicit workflow credentials, bounded REST pages and the existing process owner. */
export function createSafetyReader(
  { repository, token, cwd = ROOT, env = process.env },
  execute = runPackedCommand
) {
  if (!/^[A-Za-z0-9_-]+\/[\w.-]+$/.test(repository ?? ''))
    throw new Error('GITHUB_REPOSITORY must be owner/repo.');
  if (typeof token !== 'string' || !token) throw new Error('A workflow token is required.');
  let requests = 0;
  const get = (path) => {
    if (++requests > REQUESTS) throw new Error('Release PR REST request budget exceeded.');
    return JSON.parse(
      execute('gh', ['api', `repos/${repository}/${path}`], {
        cwd,
        env: { ...env, GH_TOKEN: token },
        timeoutMs: 30_000,
      })
    );
  };
  return {
    repository,
    get,
    list(path) {
      const result = [];
      for (let page = 1; page <= PAGES; page++) {
        const rows = get(`${path}${path.includes('?') ? '&' : '?'}per_page=100&page=${page}`);
        if (!Array.isArray(rows)) throw new Error(`Invalid REST list: ${path}.`);
        result.push(...rows);
        if (rows.length < 100) return result;
      }
      throw new Error(`Incomplete REST pagination: ${path}.`);
    },
    git(args) {
      return execute('git', args, { cwd, env, timeoutMs: 30_000 }).trim();
    },
  };
}

function releasesOf(reader) {
  const releases = reader.list('releases');
  for (const release of releases) {
    if (
      !release ||
      typeof release.tag_name !== 'string' ||
      typeof release.draft !== 'boolean' ||
      (!release.draft && !Number.isFinite(Date.parse(release.published_at)))
    ) {
      throw new Error('Invalid or incomplete release data.');
    }
  }
  return releases;
}

function componentOf(pr, components) {
  if (typeof pr?.head?.ref !== 'string') throw new Error('Missing PR branch data.');
  if (!pr.head.ref.startsWith(PREFIX)) return undefined;
  const component = components.find(
    (item) => pr.head.ref === `${PREFIX}components--${item.package}`
  );
  if (!component || component.release === false)
    throw new Error('Unknown or parked release PR component.');
  return component;
}

/** Structural links only; release-please remains the changelog and releasability owner. */
export function releaseNoteLinks(body) {
  return [...body.matchAll(/https:\/\/github\.com\/([^\s/]+\/[^\s/]+)\/commit\/([^\s)#?]*)/g)].map(
    ([, repository, sha]) => ({ repository, sha })
  );
}

/** Project linked SHAs with the pinned release-please rules, never a second type/path parser. */
async function listedReleaseCommits({ component, components, reader, anchor, base }) {
  const config = JSON.parse(reader.git(['show', `${base}:release-please-config.json`]));
  const entries = Object.entries(config.packages ?? {}).filter(
    ([, entry]) => entry.component === component.package
  );
  if (entries.length !== 1) throw new Error('Missing or ambiguous release package config.');
  const [packagePath, entry] = entries[0];
  const options = { ...config, ...entry };
  if ((options['changelog-type'] ?? 'default') !== 'default')
    throw new Error('Release notes coverage requires the pinned default changelog renderer.');
  const { CommitSplit, CommitExclude, parseConventionalCommits, DefaultChangelogNotes } =
    loadReleasePleaseCommitRules();
  const raw = reader.git([
    'log',
    `--max-count=${COVERAGE_COMMITS + 1}`,
    '--format=%H%x00%B%x00',
    `${anchor}..${base}`,
  ]);
  const fields = raw ? raw.split('\0') : [];
  if (
    (raw && (fields.length < 3 || fields.length % 2 !== 1 || fields.at(-1).trim())) ||
    (!raw && anchor !== base)
  )
    throw new Error('Incomplete release coverage history.');
  if ((fields.length - 1) / 2 > COVERAGE_COMMITS)
    throw new Error(
      'Release coverage history exceeds 500 commits; regenerate or investigate the range.'
    );
  const commits = [];
  for (let index = 0; index + 1 < fields.length; index += 2) {
    const sha = fields[index].trim();
    if (!SHA.test(sha)) throw new Error('Invalid release coverage commit.');
    const names = reader.git([
      'diff-tree',
      '--root',
      '--no-commit-id',
      '--name-only',
      '-r',
      '-m',
      '--no-renames',
      '-z',
      sha,
    ]);
    const files = names ? names.split('\0').filter(Boolean) : [];
    commits.push({ sha, message: fields[index + 1], files });
  }
  // Reuse the workflow's private-leaf attribution before the pinned splitter/exclusions.
  const github = attributeReleaseConsumption(
    {
      async *mergeCommitIterator(_branch) {
        yield* commits;
      },
    },
    components
  );
  const attributed = [];
  for await (const commit of github.mergeCommitIterator('main')) attributed.push(commit);
  const split = new CommitSplit({
    includeEmpty: true,
    packagePaths: Object.keys(config.packages),
  }).split(attributed);
  const selected = packagePath === '.' ? attributed : (split[packagePath] ?? []);
  const filtered = new CommitExclude({
    [packagePath]: { excludePaths: options['exclude-paths'] },
  }).excludeCommits({ [packagePath]: selected })[packagePath];
  const [owner, repository] = reader.repository.split('/');
  // Config omission deliberately uses the pinned renderer's defaults. It also owns
  // breaking/nested commits, hidden sections, scopes and revert-pair suppression.
  const body = await new DefaultChangelogNotes().buildNotes(parseConventionalCommits(filtered), {
    owner,
    repository,
    version: 'coverage',
    currentTag: 'coverage',
    targetBranch: 'main',
    changelogSections: options['changelog-sections'],
  });
  return new Set(releaseNoteLinks(body).map(({ sha }) => sha));
}

/** Proven invalid or incomplete notes require regeneration, rather than an unsafe queue skip. */
export class ReleaseNotesRefreshRequiredError extends Error {}

/** Anchor, linked membership and coverage; release-please owns which commits are listed. */
export async function checkReleaseNotes({ pr, base, components, reader, releases }) {
  const component = componentOf(pr, components);
  if (!component) return null;
  if (!SHA.test(base ?? '')) throw new Error('Missing candidate base SHA.');
  if (typeof pr.body !== 'string') throw new Error('Missing release PR notes.');
  const [published] = publishedReleases(releases ?? releasesOf(reader), component.name);
  let anchor;
  let anchorName;
  const headers = [
    ...pr.body.matchAll(/^## \[([^\]\r\n]+)\]\((https:\/\/github\.com\/[^\s)]+)\)/gm),
  ];
  if (published) {
    if (headers.length !== 1)
      throw new ReleaseNotesRefreshRequiredError('Missing or ambiguous release compare header.');
    const url = new URL(headers[0][2]);
    const prefix = `/${reader.repository}/compare/`;
    const tags = url.pathname.startsWith(prefix)
      ? url.pathname.slice(prefix.length).split('...')
      : [];
    if (
      url.origin !== 'https://github.com' ||
      tags.length !== 2 ||
      !tags[1] ||
      decodeURIComponent(tags[0]) !== published.tag_name
    )
      throw new ReleaseNotesRefreshRequiredError(
        `Release compare base must equal published tag ${published.tag_name}.`
      );
    anchor = reader.git(['rev-parse', '--verify', `refs/tags/${published.tag_name}^{commit}`]);
    if (!SHA.test(anchor)) throw new Error('Missing published tag commit.');
    anchorName = published.tag_name;
  } else {
    const config = JSON.parse(reader.git(['show', `${base}:release-please-config.json`]));
    const entries = Object.entries(config.packages ?? {}).filter(
      ([, entry]) => entry.component === component.package
    );
    if (entries.length !== 1) throw new Error('Missing or ambiguous release package config.');
    const [packagePath, entry] = entries[0];
    anchor = entry['bootstrap-sha'];
    if (!anchor) throw new Error(`No published ${component.name} anchor.`);
    if (typeof anchor !== 'string' || !SHA.test(anchor)) throw new Error('Invalid bootstrap SHA.');
    anchorName = anchor;
    // The pinned Manifest seeds a previous tag from a nonzero manifest version.
    // With a zero seed, DefaultChangelogNotes renders a plain version heading.
    const manifest = JSON.parse(reader.git(['show', `${base}:.release-please-manifest.json`]));
    const seed = manifest[packagePath];
    const prefix = releasePolicy(component.name).tagPrefix;
    if (headers.length) {
      try {
        compareVersions(headers[0][1], headers[0][1]);
      } catch {
        throw new ReleaseNotesRefreshRequiredError('Invalid first-release version header.');
      }
      const url = new URL(headers[0][2]);
      const compare = `/${reader.repository}/compare/`;
      const tags = url.pathname.startsWith(compare)
        ? url.pathname.slice(compare.length).split('...')
        : [];
      if (
        headers.length !== 1 ||
        url.origin !== 'https://github.com' ||
        url.search ||
        url.hash ||
        seed === '0.0.0' ||
        !seed ||
        tags.length !== 2 ||
        decodeURIComponent(tags[0]) !== `${prefix}${seed}` ||
        decodeURIComponent(tags[1]) !== `${prefix}${headers[0][1]}`
      )
        throw new ReleaseNotesRefreshRequiredError('Invalid first-release compare header.');
    } else if (
      !/^## \d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?: \(\d{4}-\d{2}-\d{2}\))?$/m.test(pr.body) ||
      (seed && seed !== '0.0.0')
    ) {
      throw new ReleaseNotesRefreshRequiredError('Missing first-release version header.');
    }
    reader.git(['rev-parse', '--verify', `${anchor}^{commit}`]);
  }
  reader.git(['merge-base', '--is-ancestor', anchor, base]);
  const text = reader.git(['rev-list', '--ancestry-path', `${anchor}..${base}`]);
  const range = text ? text.split('\n') : [];
  if (range.some((sha) => !SHA.test(sha))) throw new Error('Incomplete candidate commit range.');
  const allowed = new Set(range);
  const links = releaseNoteLinks(pr.body);
  for (const { repository, sha } of links) {
    if (repository !== reader.repository || !SHA.test(sha) || !allowed.has(sha)) {
      throw new ReleaseNotesRefreshRequiredError(
        `Release note commit ${sha} is outside (${anchorName}, candidate base].`
      );
    }
  }
  const listed = await listedReleaseCommits({ component, components, reader, anchor, base });
  const linked = new Set(links.map(({ sha }) => sha));
  const missing = [...listed].filter((sha) => !linked.has(sha));
  if (missing.length)
    throw new ReleaseNotesRefreshRequiredError(
      `Release notes COVERAGE missing commit(s): ${missing.join(', ')}. Regenerate the release PR with release-please.`
    );
  return {
    tag: published?.tag_name ?? null,
    ...(published ? {} : { bootstrapSha: anchor }),
    linkedCommits: links.length,
  };
}

/** Pending squash commits do not yet have REST /commits/:sha/pulls associations. */
export async function verifyReleasePrNotes({ eventName, event, components, reader }) {
  if (eventName === 'pull_request') {
    return (await checkReleaseNotes({
      pr: event.pull_request,
      base: event.pull_request?.base?.sha,
      components,
      reader,
    }))
      ? 1
      : 0;
  }
  if (eventName !== 'merge_group') throw new Error('Missing merge-group event data.');
  const commits = pendingQueueSubjects({ event, reader });
  let checked = 0;
  let releases;
  for (const { sha, title, number } of commits) {
    const pr = reader.get(`pulls/${number}`);
    if (!componentOf(pr, components)) continue;
    if (
      pr.number !== number ||
      pr.title !== title ||
      pr.base?.ref !== 'main' ||
      pr.base?.repo?.full_name !== reader.repository
    ) {
      throw new Error('Pending queue PR data does not match its squash commit.');
    }
    releases ??= releasesOf(reader);
    const parent = reader.git(['rev-parse', '--verify', `${sha}^`]);
    await checkReleaseNotes({ pr, base: parent, components, reader, releases });
    checked++;
  }
  return checked;
}

/** Shared bounded queue evidence for notes and the title report; no REST PR-title comparison. */
export function pendingQueueSubjects({ event, reader }) {
  if (!SHA.test(event?.merge_group?.head_sha ?? ''))
    throw new Error('Missing merge-group event data.');
  const head = event.merge_group.head_sha;
  const base = reader.git(['merge-base', '--all', 'refs/remotes/origin/main', head]);
  if (!SHA.test(base)) throw new Error('Missing or ambiguous cumulative queue base.');
  const log = reader.git(['log', '--reverse', '--format=%H%x09%s', `${base}..${head}`]);
  const commits = log ? log.split('\n') : [];
  if (!commits.length || commits.length > QUEUE_COMMITS)
    throw new Error('Missing or oversized pending queue range.');
  return commits.map((line) => {
    const sha = line.slice(0, 40);
    const subject = line.slice(41);
    if (!SHA.test(sha) || line[40] !== '\t') throw new Error('Invalid pending queue commit.');
    // GitHub's squash queue appends the PR number; only remove that final suffix.
    const match = / \(#(\d+)\)$/.exec(subject);
    if (!match) throw new Error('Pending queue commit has no PR number.');
    return { sha, title: subject.slice(0, match.index), number: Number(match[1]) };
  });
}

/** Syntax feedback only. release-please still owns release attribution and changelog rules. */
export function conventionalPrTitle(title) {
  return (
    typeof title === 'string' && /^[a-z][a-z0-9-]*(?:\([^()\r\n]+\))?!?: \S[^\r\n]*$/.test(title)
  );
}

export function checkQueueTitles({ event, reader }) {
  const commits = pendingQueueSubjects({ event, reader });
  return {
    checked: commits.length,
    findings: commits.filter(({ title }) => !conventionalPrTitle(title)),
  };
}

function summaryText(text) {
  return String(text).replace(
    /[&<>"']/g,
    (char) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]
  );
}

// The observation phase must not turn missing evidence or summary I/O into a queue gate.
function writeTitleReport(text) {
  process.stdout.write(text);
  try {
    if (!process.env.GITHUB_STEP_SUMMARY) throw new Error('GITHUB_STEP_SUMMARY is required.');
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  } catch (error) {
    process.stderr.write(`Title report summary unavailable: ${error.message}\n`);
  }
}

/** A matching draft blocks release-pr only until its actual git tag exists. */
export function inspectManifestDrafts({ manifest, components, reader }) {
  if (
    !manifest ||
    typeof manifest !== 'object' ||
    Array.isArray(manifest) ||
    !Object.keys(manifest).length
  ) {
    throw new Error('Missing release manifest data.');
  }
  const paths = components
    .filter((component) => component.release !== false)
    .map((component) => {
      if (component.owns.length !== 1) throw new Error('Invalid release component root.');
      return component.owns[0];
    });
  if (JSON.stringify(Object.keys(manifest).sort()) !== JSON.stringify(paths.sort())) {
    throw new Error('Incomplete release manifest components.');
  }
  const releases = releasesOf(reader);
  const heldPaths = [];
  const drafts = [];
  for (const [path, version] of Object.entries(manifest)) {
    const component = components.find((item) => item.owns.length === 1 && item.owns[0] === path);
    if (!component || component.release === false || typeof version !== 'string') {
      throw new Error(`Invalid manifest component/version: ${path}.`);
    }
    compareVersions(version, version);
    const tag = `${releasePolicy(component.name).tagPrefix}${version}`;
    const matching = releases.filter((release) => release.draft && release.tag_name === tag);
    if (!matching.length) continue;
    // Pass metadata only; draft body/assets and the push-capable token remain in this job.
    drafts.push(
      ...matching.map(({ id, tag_name, created_at }) => ({ path, id, tag_name, created_at }))
    );

    const refs = reader.list(`git/matching-refs/tags/${encodeURIComponent(tag)}`);
    if (
      refs.some((ref) => !ref || typeof ref.ref !== 'string' || !SHA.test(ref.object?.sha ?? ''))
    ) {
      throw new Error(`Invalid git tag data for ${tag}.`);
    }
    if (!refs.some((ref) => ref.ref === `refs/tags/${tag}`)) heldPaths.push(path);
  }
  return { heldPaths, drafts };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const command = process.argv[2];
  try {
    if (process.argv.length !== 3 || !['notes', 'draft', 'titles-report'].includes(command)) {
      throw new Error('Usage: release-pr-safety.mjs notes | draft | titles-report');
    }
    const components = parseComponentMap(
      readFileSync(`${ROOT}.github/components.json`, 'utf8')
    ).components;
    const reader = createSafetyReader({
      repository: process.env.GITHUB_REPOSITORY,
      token: command === 'draft' ? process.env.RELEASE_TOKEN : process.env.GITHUB_TOKEN,
    });
    if (command === 'titles-report') {
      if (process.env.GITHUB_EVENT_NAME !== 'merge_group')
        throw new Error('The title report runs only on merge groups.');
      const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
      const { checked, findings } = checkQueueTitles({ event, reader });
      const details = findings
        .map(
          ({ sha, title, number }) =>
            `- PR #${number}, commit <code>${sha}</code>: <code>${summaryText(title)}</code>\n`
        )
        .join('');
      writeTitleReport(
        `### Conventional PR titles (report-only)\n\nChecked ${checked} queued title(s); ${findings.length} finding(s). ` +
          'Expected type(scope)?: subject, with an optional breaking-change marker. Findings do not fail this merge group.\n\n' +
          details
      );
    } else if (command === 'notes') {
      const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
      const checked = await verifyReleasePrNotes({
        eventName: process.env.GITHUB_EVENT_NAME,
        event,
        components,
        reader,
      });
      process.stdout.write(`Checked ${checked} release PR(s).\n`);
    } else {
      const manifest = JSON.parse(readFileSync(`${ROOT}.release-please-manifest.json`, 'utf8'));
      const { heldPaths: held, drafts } = inspectManifestDrafts({ manifest, components, reader });
      if (!process.env.GITHUB_OUTPUT || !process.env.GITHUB_STEP_SUMMARY)
        throw new Error('Workflow output/summary paths are required.');
      appendFileSync(
        process.env.GITHUB_OUTPUT,
        `skip=${held.length === Object.keys(manifest).length}\nheld_paths=${JSON.stringify(held)}\ndrafts=${JSON.stringify(drafts)}\n`
      );
      if (held.length)
        appendFileSync(
          process.env.GITHUB_STEP_SUMMARY,
          `Tagless manifest path(s): ${held.join(', ')}; holding only those release PR candidates. github-release still runs.\n`
        );
      process.stdout.write(held.length === Object.keys(manifest).length ? 'skip\n' : 'run\n');
    }
  } catch (error) {
    if (command === 'titles-report') {
      writeTitleReport(
        `### Conventional PR titles (report-only)\n\nTitle evidence unavailable: <code>${summaryText(error.message)}</code>. This report does not fail the merge group.\n`
      );
    } else {
      process.stderr.write(`${error.message}\n`);
      process.exitCode = 1;
    }
  }
}
