// Advisory release monitoring; release-please alone owns releasability and candidate notes.
import { appendFileSync, readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { runPackedCommand } from './packed-command.mjs';
import { parseComponentMap } from './ci-scope.mjs';
import { releasePolicy } from './native-release-policy.mjs';
import { releaseNoteLinks } from './release-pr-safety.mjs';
import {
  attributeReleaseConsumption,
  assertReleasePleaseApi,
  loadPinnedReleasePlease,
} from './release-please-run.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const TITLE = 'Release stalled';
const SHA = /^[a-f0-9]{40}$/;
const PREFIX = 'release-please--branches--main--components--';
const MAX_COMMITS = 500;
const MAX_PAGES = 10;
const MAX_REQUESTS = 60;
const DEADLINE_MS = 90_000;

/** Independent bounded REST budget. Issue writes use github.token, draft reads use the App token. */
export function createStallClient(
  { repository, readToken, issueToken, cwd = ROOT, env = process.env },
  execute = runPackedCommand
) {
  if (!/^[A-Za-z0-9_-]+\/[\w.-]+$/.test(repository ?? '') || !readToken)
    throw new Error('Release monitor needs repository and read token.');
  let requests = 0;
  const deadline = Date.now() + DEADLINE_MS;
  const options = (token) => {
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error('Release monitor deadline exceeded.');
    return { cwd, env: { ...env, GH_TOKEN: token }, timeoutMs: Math.min(10_000, remaining) };
  };
  const request = (endpoint, method = 'GET', data, issueRead = false) => {
    if (++requests > MAX_REQUESTS) throw new Error('Release monitor REST budget exceeded.');
    const args = ['api', endpoint, '--method', method];
    if (data) for (const [key, value] of Object.entries(data)) args.push('-f', `${key}=${value}`);
    const token = method === 'GET' && !issueRead ? readToken : issueToken;
    if (!token) throw new Error('Release monitor issue token is required for writes.');
    return JSON.parse(execute('gh', args, options(token)));
  };
  return {
    repository,
    get: (path) => request(`repos/${repository}/${path}`),
    write: (path, method, data) => request(`repos/${repository}/${path}`, method, data),
    search() {
      const query = encodeURIComponent(`repo:${repository} is:issue in:title "${TITLE}"`);
      const result = request(`search/issues?q=${query}&per_page=100`, 'GET', undefined, true);
      if (
        result.incomplete_results !== false ||
        !Number.isSafeInteger(result.total_count) ||
        result.total_count > 100 ||
        !Array.isArray(result.items) ||
        result.items.length !== result.total_count
      )
        throw new Error('Incomplete release stall issue discovery.');
      let exact = result.items.filter((issue) => issue.title === TITLE && !issue.pull_request);
      if (!exact.length) {
        const recent = request(
          `repos/${repository}/issues?state=all&sort=created&direction=desc&per_page=100`,
          'GET',
          undefined,
          true
        );
        if (!Array.isArray(recent) || recent.length > 100)
          throw new Error('Invalid recent release stall issue discovery.');
        exact = recent.filter((issue) => issue.title === TITLE && !issue.pull_request);
      }
      if (exact.length > 1) throw new Error('Multiple Release stalled issues need reconciliation.');
      return exact[0];
    },
    list(path) {
      const rows = [];
      for (let page = 1; page <= MAX_PAGES; page++) {
        const response = request(
          `repos/${repository}/${path}${path.includes('?') ? '&' : '?'}per_page=100&page=${page}`,
          'GET',
          undefined,
          path.startsWith('issues/')
        );
        if (!Array.isArray(response) || response.length > 100)
          throw new Error('Invalid release monitor REST page.');
        rows.push(...response);
        if (response.length < 100) return rows;
      }
      throw new Error('Incomplete release monitor REST pagination.');
    },
    git: (args) => execute('git', args, options(readToken)).trimEnd(),
  };
}

/** Local immutable history supplies SCM data, while the real pinned manifest plans releases. */
export async function planReleaseCommits({ client, components, releases }) {
  const api = loadPinnedReleasePlease();
  assertReleasePleaseApi(api);
  const [owner, repo] = client.repository.split('/');
  const github = await api.GitHub.create({ owner, repo, defaultBranch: 'main' });
  // No unconfigured REST or GraphQL acquisition may escape the local planning adapter.
  github.getGitHubApi().octokit.hook.before('request', () => {
    throw new Error('Unexpected release monitor SCM request.');
  });
  github.graphqlRequest = async () => {
    throw new Error('Release monitor cannot use GraphQL.');
  };
  const raw = client.git([
    'log',
    `--max-count=${MAX_COMMITS + 1}`,
    '--format=%H%x00%ct%x00%B%x00',
    'HEAD',
  ]);
  const fields = raw.split('\0');
  if (!raw || fields.length % 3 !== 1)
    throw new Error('Missing or incomplete local release history.');
  const commits = [];
  for (let index = 0; index + 2 < fields.length; index += 3) {
    const sha = fields[index].trim();
    const time = Number(fields[index + 1]) * 1000;
    if (!SHA.test(sha) || !Number.isFinite(time) || time <= 0)
      throw new Error('Invalid local release commit data.');
    commits.push({ sha, time, message: fields[index + 2] });
  }
  const tags = new Map();
  const tagText = client.git([
    'for-each-ref',
    '--format=%(refname:strip=2)%00%(objectname)%00%(*objectname)',
    'refs/tags',
  ]);
  for (const row of tagText ? tagText.split('\n') : []) {
    const [name, object, peeled] = row.split('\0');
    const sha = peeled || object;
    if (!name || !SHA.test(sha)) throw new Error('Invalid local tag data.');
    tags.set(name, sha);
  }
  github.getFileContentsOnBranch = async (path) => {
    const parsedContent = client.git(['show', `HEAD:${path}`]);
    return {
      parsedContent,
      content: Buffer.from(parsedContent).toString('base64'),
      sha: 'local',
      mode: '100644',
    };
  };
  github.releaseIterator = async function* () {
    for (const release of releases) {
      if (release.draft) continue;
      if (!tags.has(release.tag_name))
        throw new Error('Published release tag absent from checkout.');
      yield {
        tagName: release.tag_name,
        sha: tags.get(release.tag_name),
        notes: release.body ?? '',
        name: release.name ?? release.tag_name,
      };
    }
  };
  github.tagIterator = async function* () {
    for (const [name, sha] of tags) yield { name, sha };
  };
  github.mergeCommitIterator = async function* (_branch, _options = {}) {
    for (const commit of commits.slice(0, MAX_COMMITS)) {
      const names = client.git([
        'diff-tree',
        '--root',
        '--no-commit-id',
        '--name-only',
        '-r',
        '-m',
        '--no-renames',
        commit.sha,
      ]);
      yield {
        sha: commit.sha,
        message: commit.message,
        files: names ? [...new Set(names.split('\n'))] : [],
      };
    }
    if (commits.length > MAX_COMMITS)
      throw new Error('Release planning history exceeds monitor bound.');
  };
  attributeReleaseConsumption(github, components);
  const manifest = await api.Manifest.fromManifest(github, 'main');
  const candidates = await manifest.buildPullRequests();
  return candidates.map((candidate) => {
    const links = releaseNoteLinks(candidate.body.toString());
    if (
      !links.length ||
      links.some(({ repository, sha }) => repository !== client.repository || !SHA.test(sha))
    )
      throw new Error('Candidate is missing valid releasable commit links.');
    const shas = new Set(links.map(({ sha }) => sha));
    if ([...shas].some((sha) => !commits.some((commit) => commit.sha === sha)))
      throw new Error('Candidate linked commit absent from bounded history.');
    const newest = commits.find((commit) => shas.has(commit.sha));
    return { branch: candidate.headRefName, sha: newest.sha, time: newest.time };
  });
}

function timestamp(value) {
  const result = Date.parse(value);
  if (!Number.isFinite(result)) throw new Error('Missing release monitor timestamp.');
  return result;
}
function occurrence(key, message) {
  return { key: createHash('sha256').update(key).digest('hex'), message };
}

/** Thresholds are strict; complete evidence is required before declaring healthy. */
export async function detectReleaseStalls({
  client,
  manifest,
  components,
  heldPaths,
  queueSkipped,
  now = Date.now(),
  plan = planReleaseCommits,
}) {
  if (
    !Array.isArray(heldPaths) ||
    heldPaths.some((path) => !Object.hasOwn(manifest, path)) ||
    typeof queueSkipped !== 'boolean' ||
    !Number.isFinite(now)
  )
    throw new Error('Invalid release guard evidence.');
  const releases = client.list('releases');
  if (
    releases.some(
      (release) => typeof release.tag_name !== 'string' || typeof release.draft !== 'boolean'
    )
  )
    throw new Error('Invalid release discovery data.');
  const findings = [];
  for (const component of components.filter((item) => item.release !== false)) {
    const path = component.owns[0];
    const version = manifest[path];
    if (component.owns.length !== 1 || typeof version !== 'string')
      throw new Error('Missing manifest component evidence.');
    if (!queueSkipped && !heldPaths.includes(path)) continue;
    const tag = `${releasePolicy(component.name).tagPrefix}${version}`;
    for (const draft of releases.filter((release) => release.draft && release.tag_name === tag)) {
      if (!Number.isSafeInteger(draft.id) || draft.id < 1)
        throw new Error('Invalid held draft ID.');
      if (now - timestamp(draft.created_at) > 30 * 60_000)
        findings.push(
          occurrence(
            `draft:${draft.id}`,
            `${path}: held draft ${tag} is older than 30 minutes (${queueSkipped ? 'queue skip' : 'TAGLESS_DRAFT'}).`
          )
        );
    }
  }
  const pulls = client.list('pulls?state=open&base=main');
  const releasePulls = pulls.filter(
    (pr) => pr.head?.ref?.startsWith(PREFIX) && pr.head?.repo?.full_name === client.repository
  );
  if (!releasePulls.length) return { findings };
  try {
    const candidates = await plan({ client, components, releases });
    for (const pr of releasePulls) {
      if (!Number.isSafeInteger(pr.number) || !SHA.test(pr.head.sha) || pr.base?.ref !== 'main')
        throw new Error('Invalid open release PR data.');
      if (
        !components.some(
          (component) =>
            component.release !== false && pr.head.ref === `${PREFIX}${component.package}`
        )
      )
        throw new Error('Unknown release PR component.');
      const newest = candidates.find((candidate) => candidate.branch === pr.head.ref);
      if (!newest) continue;
      if (!SHA.test(newest.sha) || !Number.isFinite(newest.time))
        throw new Error('Invalid newest release commit evidence.');
      const head = client.get(`commits/${pr.head.sha}`);
      if (head.sha !== pr.head.sha) throw new Error('Release PR head changed during discovery.');
      if (newest.time - timestamp(head.commit?.committer?.date) <= 60 * 60_000) continue;
      const comparison = client.get(`compare/${newest.sha}...${pr.head.sha}`);
      if (!['ahead', 'behind', 'identical', 'diverged'].includes(comparison.status))
        throw new Error('Invalid release head ancestry evidence.');
      if (comparison.status === 'ahead' || comparison.status === 'identical') continue;
      findings.push(
        occurrence(
          `pr:${pr.number}:${pr.head.sha}:${newest.sha}`,
          `${pr.head.ref}: PR #${pr.number} head ${pr.head.sha} is more than one hour older than missing releasable commit ${newest.sha}.`
        )
      );
    }
    return { findings };
  } catch (error) {
    return { findings, warning: error.message };
  }
}

/** A fixed title identifies one durable issue; comment markers deduplicate retried occurrences. */
export function reconcileStallIssue(client, findings) {
  let issue = client.search();
  if (issue && (!Number.isSafeInteger(issue.number) || !['open', 'closed'].includes(issue.state)))
    throw new Error('Invalid Release stalled issue.');
  if (!findings.length) {
    if (issue?.state === 'open')
      client.write(`issues/${issue.number}`, 'PATCH', {
        state: 'closed',
        body: 'Release monitoring is healthy; all bounded discovery completed without a stall.',
      });
    return;
  }
  const body = `Advisory release monitor findings:\n\n${findings.map(({ message }) => `- ${message}`).join('\n')}\n\nThe monitor never fails the release job. Reconcile the held draft or regenerate the stale component PR; publication remains governed by its existing gates.`;
  if (!issue) {
    issue = client.write('issues', 'POST', { title: TITLE, body });
    if (!Number.isSafeInteger(issue.number))
      throw new Error('Invalid created Release stalled issue.');
  } else if (issue.body !== body || issue.state !== 'open')
    client.write(`issues/${issue.number}`, 'PATCH', { state: 'open', body });
  const comments = client.list(`issues/${issue.number}/comments`);
  if (comments.some((comment) => typeof comment.body !== 'string'))
    throw new Error('Invalid release stall comments.');
  const fresh = findings.filter(
    ({ key }) => !comments.some(({ body }) => body.includes(`<!-- release-stall:${key} -->`))
  );
  if (fresh.length)
    client.write(`issues/${issue.number}/comments`, 'POST', {
      body: fresh
        .map(({ key, message }) => `${message}\n<!-- release-stall:${key} -->`)
        .join('\n\n'),
    });
}

/** Every error is advisory, including summary or issue failures; uncertainty never closes an issue. */
export async function monitorReleaseStalls(options, { summarize, warn = console.warn } = {}) {
  try {
    const { findings, warning } = await detectReleaseStalls(options);
    const summary = findings.length
      ? `### Release stalled\n\n${findings.map(({ message }) => `- ${message}`).join('\n')}\n`
      : warning
        ? 'Release stall monitor: incomplete evidence.\n'
        : 'Release stall monitor: healthy.\n';
    summarize?.(summary);
    if (warning) summarize?.(`Release stall monitor warning: ${warning}\n`);
    if (options.live && (findings.length || !warning))
      reconcileStallIssue(options.client, findings);
    return { findings, healthy: !warning && !findings.length, warning };
  } catch (error) {
    const warning = `Release stall monitor warning: ${error.message}`;
    try {
      warn(warning);
    } catch {
      /* Advisory reporting must not fail the job. */
    }
    try {
      summarize?.(`### Release stall monitor warning\n\n${warning}\n`);
    } catch {
      /* stdout remains available if the summary cannot be written. */
    }
    return { findings: [], healthy: false, warning };
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    if (!['true', 'false'].includes(process.env.QUEUE_SKIPPED))
      throw new Error('Missing queue guard result.');
    const client = createStallClient({
      repository: process.env.GITHUB_REPOSITORY,
      readToken: process.env.RELEASE_TOKEN,
      issueToken: process.env.ISSUE_TOKEN,
    });
    await monitorReleaseStalls(
      {
        client,
        manifest: JSON.parse(readFileSync(`${ROOT}.release-please-manifest.json`, 'utf8')),
        components: parseComponentMap(readFileSync(`${ROOT}.github/components.json`, 'utf8'))
          .components,
        heldPaths: JSON.parse(process.env.TAGLESS_DRAFT_PATHS ?? 'null'),
        queueSkipped: process.env.QUEUE_SKIPPED === 'true',
        live: process.env.LIVE === 'true',
      },
      {
        summarize: (text) => {
          process.stdout.write(text);
          if (process.env.GITHUB_STEP_SUMMARY)
            appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
        },
      }
    );
  } catch (error) {
    console.warn(`Release stall monitor warning: ${error.message}`);
    try {
      if (process.env.GITHUB_STEP_SUMMARY)
        appendFileSync(
          process.env.GITHUB_STEP_SUMMARY,
          `Release stall monitor warning: ${error.message}\n`
        );
    } catch {
      /* Always advisory. */
    }
  }
}
