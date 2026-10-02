// Published-release evidence -> existing Project items. No release or issue mutation.
import { appendFileSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { productOfTag, archivePrefix } from './native-release-policy.mjs';
import { compareVersions, versionOfTag } from './release-versions.mjs';

export const PROJECT_ID = 'PVT_kwDOFBKkD84BlZ_A';
export const LIMITS = { graphql: 60, rest: 250, pages: 10, releases: 10, prs: 250, batch: 25 };
const replay = 'Replay the affected published tag with project-release.yml (dry-run first).';
const chunks = (rows, size = LIMITS.batch) =>
  Array.from({ length: Math.ceil(rows.length / size) }, (_, i) =>
    rows.slice(i * size, (i + 1) * size)
  );
const quote = JSON.stringify;

export function releaseIdentity(tag) {
  const product = productOfTag(tag);
  if (!product || (product === 'cli' && !tag.startsWith('v5.'))) return undefined;
  const version = versionOfTag(tag, product);
  try {
    compareVersions(version, version);
  } catch {
    return undefined;
  }
  return { product, version, label: `${archivePrefix(product)} ${version}` };
}

/** Only same-repository release-please links; /issues links may actually be PRs. */
export function noteReferences(body, repository) {
  const allowed = new Set([repository.toLowerCase()]);
  if (repository === 'pj-tmt/tmt') allowed.add('wkh237/tmt'); // Pre-transfer release notes.
  return [
    ...new Set(
      [...body.matchAll(/https:\/\/github\.com\/([^/\s)]+\/[^/\s)]+)\/(?:pull|issues)\/(\d+)\b/g)]
        .filter((match) => allowed.has(match[1].toLowerCase()))
        .map((match) => Number(match[2]))
    ),
  ].sort((a, b) => a - b);
}

/** A hard budget counts requests, including failed calls; no retries or polling. */
export function githubApi({ appToken, readToken, repository, spawn = spawnSync }) {
  if (!appToken)
    throw new Error(
      'RELEASE_APP_TOKEN is missing; mint the release App installation token before running the updater.'
    );
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) throw new Error('Invalid repository.');
  const counts = { graphql: 0, rest: 0 };
  const invoke = (kind, endpoint, input) => {
    if (++counts[kind] > LIMITS[kind])
      throw new Error(`${kind} request budget exceeded. ${replay}`);
    const args = ['api', endpoint, ...(input ? ['--input', '-'] : [])];
    const result = spawn('gh', args, {
      input: input ? JSON.stringify(input) : undefined,
      encoding: 'utf8',
      timeout: 30_000,
      maxBuffer: 16 * 1024 * 1024,
      env: {
        ...process.env,
        GH_TOKEN: kind === 'graphql' ? appToken : readToken || appToken,
      },
    });
    let data;
    try {
      data = JSON.parse(result.stdout);
    } catch {
      /* Transport errors may have no JSON body. */
    }
    if (data?.errors?.length) {
      throw new Error(
        `GitHub GraphQL rejected the request: ${data.errors.map((error) => error.message).join('; ')}. ${replay}`
      );
    }
    if (result.error || result.status !== 0 || data === undefined)
      throw new Error(
        `GitHub ${kind} request failed for ${endpoint}; no automatic retry. ${replay}`
      );
    return kind === 'graphql' ? data.data : data;
  };
  return {
    counts,
    rest: (path) => invoke('rest', `repos/${repository}/${path}`),
    graphql: (query) => invoke('graphql', 'graphql', { query }),
    reserve: (requests) => {
      if (counts.graphql + requests > LIMITS.graphql)
        throw new Error(`Insufficient GraphQL budget before writes. ${replay}`);
    },
  };
}

function nextCursor(connection) {
  if (!connection?.pageInfo || !Array.isArray(connection.nodes))
    throw new Error('Incomplete GraphQL connection.');
  if (!connection.pageInfo.hasNextPage) return null;
  if (!connection.pageInfo.endCursor) throw new Error('Missing pagination cursor.');
  return connection.pageInfo.endCursor;
}

export function publishedWindow(releases, eventName, event, tag) {
  const published = releases
    .filter((r) => !r.draft && r.published_at && releaseIdentity(r.tag_name))
    .sort((a, b) => Date.parse(b.published_at) - Date.parse(a.published_at));
  if (eventName === 'workflow_dispatch') {
    const release = published.find((r) => r.tag_name === tag);
    if (!release) throw new Error('Manual replay requires an existing supported published tag.');
    return [release];
  }
  const selected = published.slice(0, LIMITS.releases);
  if (eventName === 'workflow_run') {
    const run = event.workflow_run;
    if (
      run?.name !== 'Native release artifacts' ||
      run.head_branch !== 'main' ||
      run.event !== 'workflow_dispatch'
    )
      throw new Error('Unexpected publishing workflow provenance.');
    const since = Date.parse(run.created_at);
    if (!Number.isFinite(since)) throw new Error('Publishing run has no valid start time.');
    if (published.slice(LIMITS.releases).some((r) => Date.parse(r.published_at) >= since))
      throw new Error(`Publishing run exceeds the ${LIMITS.releases}-release window. ${replay}`);
  } else if (eventName !== 'schedule') throw new Error('Unsupported updater event.');
  return selected;
}

export function readReleases(api) {
  const releases = [];
  for (let page = 1; page <= LIMITS.pages; page++) {
    const rows = api.rest(`releases?per_page=100&page=${page}`);
    releases.push(...rows);
    if (rows.length < 100) return releases;
  }
  throw new Error(`Release pagination cap reached. ${replay}`);
}

/** Resolve references in batches, including every closing-issue page. Plain issues are not PRs. */
export function resolveClosingIssues(api, repository, numbers) {
  const unique = [...new Set(numbers)];
  if (unique.length > LIMITS.prs) throw new Error(`PR reference cap reached. ${replay}`);
  const [owner, name] = repository.split('/');
  const resolved = new Map();
  for (const batch of chunks(unique)) {
    let pending = batch.map((number) => ({ number, cursor: null }));
    for (let page = 0; pending.length && page < LIMITS.pages; page++) {
      const fields = pending
        .map(
          ({ number, cursor }, i) =>
            `p${i}:issueOrPullRequest(number:${number}){__typename ... on PullRequest{number merged closingIssuesReferences(first:100,after:${quote(cursor)}){nodes{id number url} pageInfo{hasNextPage endCursor}}}}`
        )
        .join('\n');
      const data = api.graphql(
        `query{repository(owner:${quote(owner)},name:${quote(name)}){${fields}}}`
      );
      const more = [];
      pending.forEach(({ number }, i) => {
        const pr = data.repository?.[`p${i}`];
        if (!pr) throw new Error(`Reference #${number} could not be resolved.`);
        if (pr.__typename !== 'PullRequest' || !pr.merged) {
          resolved.set(number, null);
          return;
        }
        const issues = resolved.get(number) || new Map();
        const cursor = nextCursor(pr.closingIssuesReferences);
        for (const issue of pr.closingIssuesReferences.nodes) {
          if (!issue?.id) throw new Error(`Incomplete closing issue for PR #${number}.`);
          issues.set(issue.id, issue);
        }
        resolved.set(number, issues);
        if (cursor) more.push({ number, cursor });
      });
      pending = more;
    }
    if (pending.length) throw new Error(`Closing-issue pagination cap reached. ${replay}`);
  }
  return resolved;
}

function comparePrs(api, release, releases) {
  if (releases.length === 1) releases = readReleases(api);
  const identity = releaseIdentity(release.tag_name);
  const previous = releases
    .filter(
      (r) => !r.draft && r.published_at && releaseIdentity(r.tag_name)?.product === identity.product
    )
    .filter(
      (r) => compareVersions(versionOfTag(r.tag_name, identity.product), identity.version) < 0
    )
    .sort((a, b) =>
      compareVersions(
        versionOfTag(b.tag_name, identity.product),
        versionOfTag(a.tag_name, identity.product)
      )
    )[0];
  if (!previous)
    throw new Error(`No PR notes or previous release for ${release.tag_name}. ${replay}`);
  const commits = [];
  for (let page = 1; page <= LIMITS.pages; page++) {
    const data = api.rest(
      `compare/${encodeURIComponent(previous.tag_name)}...${encodeURIComponent(release.tag_name)}?per_page=100&page=${page}`
    );
    if (!['ahead', 'identical'].includes(data.status))
      throw new Error('Release comparison is not an ancestor range.');
    commits.push(...data.commits);
    if (commits.length === data.total_commits) break;
    if (page === LIMITS.pages || !data.commits.length)
      throw new Error(`Incomplete release compare range. ${replay}`);
  }
  const prs = new Set();
  for (const commit of commits) {
    for (let page = 1; page <= LIMITS.pages; page++) {
      const rows = api.rest(`commits/${commit.sha}/pulls?per_page=100&page=${page}`);
      for (const pr of rows)
        if (
          pr.merged_at &&
          pr.base?.repo?.full_name?.toLowerCase() === release.repository.toLowerCase()
        )
          prs.add(pr.number);
      if (rows.length < 100) break;
      if (page === LIMITS.pages) throw new Error(`Commit PR pagination cap reached. ${replay}`);
    }
  }
  return [...prs];
}

export function releaseIssues(api, repository, selected, releases) {
  const references = new Map(
    selected.map((r) => [r.tag_name, noteReferences(r.body || '', repository)])
  );
  let resolved = resolveClosingIssues(api, repository, [...references.values()].flat());
  const fallbacks = new Map();
  for (const release of selected) {
    if (!references.get(release.tag_name).some((number) => resolved.get(number))) {
      fallbacks.set(release.tag_name, comparePrs(api, { ...release, repository }, releases));
    }
  }
  if (fallbacks.size) {
    const extra = resolveClosingIssues(api, repository, [...fallbacks.values()].flat());
    resolved = new Map([...resolved, ...extra]);
  }
  const issues = new Map();
  const sources = [];
  for (const release of selected) {
    const numbers = fallbacks.get(release.tag_name) || references.get(release.tag_name);
    const label = releaseIdentity(release.tag_name).label;
    const prs = numbers.filter((number) => resolved.get(number));
    sources.push({
      tag: release.tag_name,
      method: fallbacks.has(release.tag_name) ? 'compare' : 'notes',
      prs,
    });
    for (const number of prs)
      for (const issue of resolved.get(number).values()) {
        const row = issues.get(issue.id) || { ...issue, labels: new Set() };
        row.labels.add(label);
        issues.set(issue.id, row);
      }
  }
  return { issues, sources };
}

export function readProject(api, projectId = PROJECT_ID) {
  let cursor = null;
  const items = new Map();
  let fields;
  for (let page = 0; page < LIMITS.pages; page++) {
    const data = api.graphql(
      `query{node(id:${quote(projectId)}){... on ProjectV2{id fields(first:100){nodes{... on ProjectV2Field{id name dataType} ... on ProjectV2SingleSelectField{id name options{id name}}} pageInfo{hasNextPage endCursor}} items(first:100,after:${quote(cursor)}){nodes{id content{... on Issue{id number url}} status:fieldValueByName(name:"Status"){... on ProjectV2ItemFieldSingleSelectValue{name}} released:fieldValueByName(name:"Released in"){... on ProjectV2ItemFieldTextValue{text}}} pageInfo{hasNextPage endCursor}}}}}`
    );
    if (!data.node || nextCursor(data.node.fields))
      throw new Error('Missing project or incomplete field schema.');
    fields = data.node.fields.nodes;
    const connection = data.node.items;
    cursor = nextCursor(connection);
    for (const row of connection.nodes) if (row.content?.id) items.set(row.content.id, row);
    if (!cursor) {
      const status = fields.find((f) => f.name === 'Status' && f.options);
      const released = fields.find((f) => f.name === 'Released in' && f.dataType === 'TEXT');
      const option = status?.options.find((o) => o.name === 'Released');
      if (!status || !released || !option)
        throw new Error('Project requires Status=Released and a Released in text field.');
      return {
        projectId,
        items,
        statusId: status.id,
        releasedId: released.id,
        optionId: option.id,
        pages: page + 1,
      };
    }
  }
  throw new Error(`Project pagination cap reached. ${replay}`);
}

export function planUpdates(issues, project) {
  const changes = [],
    skipped = [];
  for (const issue of issues.values()) {
    const item = project.items.get(issue.id);
    if (!item) {
      skipped.push(issue.url);
      continue;
    }
    if (
      item.status?.name &&
      !['Todo', 'In Progress', 'In Review', 'Merged', 'Released'].includes(item.status.name)
    ) {
      throw new Error(`Unknown status for ${issue.url}; refusing to replace it.`);
    }
    const old = item.released?.text || '';
    const lines = new Set(
      old
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter(Boolean)
    );
    const added = [...issue.labels].filter((label) => !lines.has(label)).sort();
    const text = added.length
      ? old + (old && !old.endsWith('\n') ? '\n' : '') + added.join('\n')
      : old;
    if (text.length > 10_000) throw new Error(`Released in text limit reached for ${issue.url}.`);
    if (added.length || item.status?.name !== 'Released')
      changes.push({
        itemId: item.id,
        issue: issue.url,
        text,
        writeText: !!added.length,
        writeStatus: item.status?.name !== 'Released',
      });
  }
  return { changes, skipped };
}

export function applyUpdates(api, project, plan, dryRun) {
  const text = plan.changes.filter((c) => c.writeText);
  const status = plan.changes.filter((c) => c.writeStatus);
  const batches = [
    ...chunks(text).map((rows) => ({ rows, fieldId: project.releasedId, field: 'text' })),
    ...chunks(status).map((rows) => ({
      rows,
      fieldId: project.statusId,
      field: 'singleSelectOptionId',
    })),
  ];
  api.reserve(batches.length + (plan.changes.length ? project.pages : 0)); // Include one bounded verification readback.
  if (dryRun) return;
  // Publish evidence first. A partial failure leaves retryable text, never a false terminal state.
  for (const { rows, fieldId, field } of batches) {
    const mutations = rows
      .map(
        (row, i) =>
          `u${i}:updateProjectV2ItemFieldValue(input:{projectId:${quote(project.projectId)},itemId:${quote(row.itemId)},fieldId:${quote(fieldId)},value:{${field}:${quote(field === 'text' ? row.text : project.optionId)}}}){projectV2Item{id}}`
      )
      .join('\n');
    api.graphql(`mutation{${mutations}}`);
  }
}

export function reconcile({
  api,
  repository,
  eventName,
  event,
  tag,
  dryRun,
  projectId = PROJECT_ID,
}) {
  const releases =
    eventName === 'workflow_dispatch'
      ? [api.rest(`releases/tags/${encodeURIComponent(tag || '')}`)]
      : readReleases(api);
  const selected = publishedWindow(releases, eventName, event, tag);
  const { issues, sources } = releaseIssues(api, repository, selected, releases);
  if (!issues.size) {
    return {
      dryRun,
      releases: sources,
      issues: 0,
      changed: [],
      outsideProject: [],
      requests: api.counts,
    };
  }
  const project = readProject(api, projectId);
  const plan = planUpdates(issues, project);
  applyUpdates(api, project, plan, dryRun);
  if (!dryRun && plan.changes.length) {
    const remaining = planUpdates(issues, readProject(api, projectId));
    if (remaining.changes.length)
      throw new Error(`Project readback did not match the release plan. ${replay}`);
  }
  return {
    dryRun,
    releases: sources,
    issues: issues.size,
    changed: plan.changes,
    outsideProject: plan.skipped,
    requests: api.counts,
  };
}

export function main(env = process.env) {
  const api = githubApi({
    appToken: env.RELEASE_APP_TOKEN,
    readToken: env.GITHUB_TOKEN,
    repository: env.GITHUB_REPOSITORY,
  });
  const event = JSON.parse(readFileSync(env.GITHUB_EVENT_PATH, 'utf8'));
  const dryRun = env.GITHUB_EVENT_NAME === 'workflow_dispatch' ? env.DRY_RUN !== 'false' : false;
  const result = reconcile({
    api,
    repository: env.GITHUB_REPOSITORY,
    eventName: env.GITHUB_EVENT_NAME,
    event,
    tag: env.RELEASE_TAG || '',
    dryRun,
  });
  const summary = `### Project release tracking${dryRun ? ' (dry run)' : ''}\n\n\`\`\`json\n${JSON.stringify(result, null, 2)}\n\`\`\`\n`;
  process.stdout.write(summary);
  if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, summary);
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    const message = `Project release tracking failed: ${error.message}\n`;
    process.stderr.write(message);
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, message);
    process.exitCode = 1;
  }
}
