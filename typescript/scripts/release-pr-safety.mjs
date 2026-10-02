// Read-only release PR gates. release-please retains ownership of changelog generation.
import { appendFileSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { parseComponentMap } from './ci-scope.mjs';
import { releasePolicy } from './native-release-policy.mjs';
import { compareVersions, publishedReleases } from './release-versions.mjs';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const PREFIX = 'release-please--branches--main--';
const SHA = /^[a-f0-9]{40}$/;
const PAGES = 10;
const REQUESTS = 60;
const QUEUE_COMMITS = 40;

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

/** Compare anchor and linked SHA membership only; no conventional-commit or entry counting rules. */
export function checkReleaseNotes({ pr, base, components, reader, releases }) {
  const component = componentOf(pr, components);
  if (!component) return null;
  if (!SHA.test(base ?? '')) throw new Error('Missing candidate base SHA.');
  if (typeof pr.body !== 'string') throw new Error('Missing release PR notes.');
  const [published] = publishedReleases(releases ?? releasesOf(reader), component.name);
  if (!published) throw new Error(`No published ${component.name} anchor.`);
  const headers = [...pr.body.matchAll(/^## \[[^\]\r\n]+\]\((https:\/\/github\.com\/[^\s)]+)\)/gm)];
  if (headers.length !== 1) throw new Error('Missing or ambiguous release compare header.');
  const url = new URL(headers[0][1]);
  const prefix = `/${reader.repository}/compare/`;
  const tags = url.pathname.startsWith(prefix)
    ? url.pathname.slice(prefix.length).split('...')
    : [];
  if (
    url.origin !== 'https://github.com' ||
    tags.length !== 2 ||
    !tags[1] ||
    decodeURIComponent(tags[0]) !== published.tag_name
  ) {
    throw new Error(`Release compare base must equal published tag ${published.tag_name}.`);
  }
  const anchor = reader.git(['rev-parse', '--verify', `refs/tags/${published.tag_name}^{commit}`]);
  if (!SHA.test(anchor)) throw new Error('Missing published tag commit.');
  reader.git(['merge-base', '--is-ancestor', anchor, base]);
  const text = reader.git(['rev-list', '--ancestry-path', `${anchor}..${base}`]);
  const range = text ? text.split('\n') : [];
  if (range.some((sha) => !SHA.test(sha))) throw new Error('Incomplete candidate commit range.');
  const allowed = new Set(range);
  const links = releaseNoteLinks(pr.body);
  for (const { repository, sha } of links) {
    if (repository !== reader.repository || !SHA.test(sha) || !allowed.has(sha)) {
      throw new Error(
        `Release note commit ${sha} is outside (${published.tag_name}, candidate base].`
      );
    }
  }
  return { tag: published.tag_name, linkedCommits: links.length };
}

/** Pending squash commits do not yet have REST /commits/:sha/pulls associations. */
export function verifyReleasePrNotes({ eventName, event, components, reader }) {
  if (eventName === 'pull_request') {
    return checkReleaseNotes({
      pr: event.pull_request,
      base: event.pull_request?.base?.sha,
      components,
      reader,
    })
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
    checkReleaseNotes({ pr, base: parent, components, reader, releases });
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
      const checked = verifyReleasePrNotes({
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
