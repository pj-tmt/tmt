// Repository state -> existing Project items. No release, issue or membership mutation.
import { appendFileSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { productOfTag, archivePrefix, releasePolicy } from './native-release-policy.mjs';
import { compareVersions, versionOfTag } from './release-versions.mjs';
import { ownerOf, parseComponentMap, releasedComponentsForPath } from './ci-scope.mjs';
import { readCargoWorkspace } from './cargo-workspace.mjs';

export const PROJECT_ID = 'PVT_kwDOFBKkD84BlZ_A';
export const LIMITS = { graphql: 200, rest: 20, pages: 20, prs: 2000, batch: 25 };
const recovery =
  'Run a full project-release.yml dry run; resolve incomplete evidence before retrying.';
const chunks = (rows, size = LIMITS.batch) =>
  Array.from({ length: Math.ceil(rows.length / size) }, (_, i) =>
    rows.slice(i * size, (i + 1) * size)
  );
const quote = JSON.stringify;

export function releaseIdentity(tag) {
  const product = productOfTag(tag);
  if (!product) return undefined;
  const version = versionOfTag(tag, product);
  try {
    compareVersions(version, version);
    if (product === 'cli' && compareVersions(version, '5.0.0-alpha.0') < 0) return undefined;
  } catch {
    return undefined;
  }
  return { product, version, label: `${archivePrefix(product)} ${version}` };
}

/** A hard budget counts requests, including failed calls; no retries or polling. */
export function githubApi({ appToken, readToken, repository, spawn = spawnSync }) {
  if (!appToken)
    throw new Error(
      'RELEASE_APP_TOKEN is missing; mint the release App installation token before running the updater.'
    );
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) throw new Error('Invalid repository.');
  const counts = { graphql: 0, rest: 0 };
  const points = { cost: 0, remaining: null };
  const invoke = (kind, endpoint, input) => {
    if (counts[kind] >= LIMITS[kind])
      throw new Error(`${kind} request budget exceeded. ${recovery}`);
    counts[kind]++;
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
    const rateLimit = kind === 'graphql' ? data?.data?.rateLimit : undefined;
    const validRateLimit =
      Number.isSafeInteger(rateLimit?.cost) &&
      rateLimit.cost >= 0 &&
      Number.isSafeInteger(rateLimit?.remaining) &&
      rateLimit.remaining >= 0;
    // Preserve reported cost even when GitHub also returns a partial-query error.
    if (validRateLimit) {
      points.cost += rateLimit.cost;
      points.remaining = rateLimit.remaining;
    }
    if (data?.errors?.length) {
      throw new Error(
        `GitHub GraphQL rejected the request: ${data.errors.map((error) => error.message).join('; ')}. ${recovery}`
      );
    }
    if (result.error || result.status !== 0 || data === undefined)
      throw new Error(
        `GitHub ${kind} request failed for ${endpoint}; no automatic retry. ${recovery}`
      );
    if (kind === 'graphql' && input.query.startsWith('query') && !validRateLimit)
      throw new Error('GraphQL read omitted valid rate-limit cost/remaining evidence.');
    return kind === 'graphql' ? data.data : data;
  };
  return {
    counts,
    points,
    rest: (path) => invoke('rest', `repos/${repository}/${path}`),
    graphql: (query) => invoke('graphql', 'graphql', { query }),
    reserve: (requests) => {
      if (counts.graphql + requests > LIMITS.graphql)
        throw new Error(`Insufficient GraphQL budget before writes. ${recovery}`);
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

/** Read every published release, without a recency or notes-based selection window. */
export function readReleases(api) {
  const releases = [];
  for (let page = 1; page <= LIMITS.pages; page++) {
    const rows = api.rest(`releases?per_page=100&page=${page}`);
    if (!Array.isArray(rows)) throw new Error('Incomplete releases response.');
    for (const row of rows) {
      if (row.draft || !releaseIdentity(row.tag_name)) continue;
      if (!row.published_at || !Number.isFinite(Date.parse(row.published_at)))
        throw new Error(`Missing publication time for ${row.tag_name}.`);
      releases.push(row);
    }
    if (rows.length < 100)
      return releases.sort(
        (a, b) =>
          Date.parse(a.published_at) - Date.parse(b.published_at) ||
          a.tag_name.localeCompare(b.tag_name)
      );
  }
  throw new Error(`Release pagination cap reached. ${recovery}`);
}

export function readProject(api, projectId = PROJECT_ID) {
  let cursor = null;
  const items = new Map();
  let fields;
  for (let page = 0; page < LIMITS.pages; page++) {
    const data = api.graphql(
      `query{rateLimit{cost remaining} node(id:${quote(projectId)}){... on ProjectV2{id fields(first:100){nodes{... on ProjectV2Field{id name dataType} ... on ProjectV2SingleSelectField{id name options{id name}}} pageInfo{hasNextPage endCursor}} items(first:100,after:${quote(cursor)}){nodes{id content{__typename ... on Issue{id number url state repository{nameWithOwner} labels(first:20){nodes{name} pageInfo{hasNextPage endCursor}}}} status:fieldValueByName(name:"Status"){... on ProjectV2ItemFieldSingleSelectValue{name}} released:fieldValueByName(name:"Released in"){... on ProjectV2ItemFieldTextValue{text}}} pageInfo{hasNextPage endCursor}}}}}`
    );
    if (!data.node || nextCursor(data.node.fields))
      throw new Error('Missing project or incomplete field schema.');
    fields = data.node.fields.nodes;
    const connection = data.node.items;
    cursor = nextCursor(connection);
    for (const row of connection.nodes) {
      if (!row?.id) throw new Error('Incomplete Project item.');
      if (row.content?.__typename === 'Issue') {
        if (
          !row.content.id ||
          !row.content.url ||
          !row.content.repository?.nameWithOwner ||
          !['OPEN', 'CLOSED'].includes(row.content.state)
        )
          throw new Error('Incomplete Project issue.');
        if (
          nextCursor(row.content.labels) ||
          row.content.labels.nodes.some((label) => typeof label?.name !== 'string')
        )
          throw new Error('Incomplete issue labels; refusing to risk updating an epic tracker.');
        items.set(row.content.id, row);
      }
    }
    if (!cursor) {
      const status = fields.find((f) => f.name === 'Status' && f.options);
      const released = fields.find((f) => f.name === 'Released in' && f.dataType === 'TEXT');
      const options = Object.fromEntries((status?.options || []).map((o) => [o.name, o.id]));
      if (!status || !released || ['Merged', 'Released', 'Done'].some((name) => !options[name]))
        throw new Error(
          'Project requires Status= Merged, Released, Done and a Released in text field.'
        );
      return {
        projectId,
        items,
        statusId: status.id,
        releasedId: released.id,
        options,
        pages: page + 1,
      };
    }
  }
  throw new Error(`Project pagination cap reached. ${recovery}`);
}

/** GitHub's closing relationship is authoritative, not notes, titles or issue text. */
export function readClosingPrs(api, items, repository) {
  const issues = new Map();
  const prs = new Map();
  for (const batch of chunks(items)) {
    let pending = batch.map((item) => ({ id: item.content.id, cursor: null }));
    for (let page = 0; pending.length && page < LIMITS.pages; page++) {
      const fields = pending
        .map(
          ({ id, cursor }, i) =>
            `i${i}:node(id:${quote(id)}){... on Issue{id state closedByPullRequestsReferences(first:10,after:${quote(cursor)},includeClosedPrs:true){nodes{id number merged mergeCommit{oid} repository{nameWithOwner}} pageInfo{hasNextPage endCursor}}}}`
        )
        .join('\n');
      const data = api.graphql(`query{rateLimit{cost remaining} ${fields}}`);
      const more = [];
      pending.forEach(({ id }, i) => {
        const issue = data[`i${i}`];
        if (issue?.id !== id || issue.state !== 'CLOSED')
          throw new Error('Issue disappeared or reopened during discovery; rerun the sweep.');
        const connection = issue.closedByPullRequestsReferences;
        const cursor = nextCursor(connection);
        const closing = issues.get(id) || new Set();
        for (const pr of connection.nodes) {
          if (!pr?.id || typeof pr.merged !== 'boolean') throw new Error('Incomplete closing PR.');
          if (!pr.merged) continue;
          if (
            pr.repository?.nameWithOwner.toLowerCase() !== repository.toLowerCase() ||
            !/^[a-f0-9]{40}$/.test(pr.mergeCommit?.oid || '')
          )
            throw new Error(`Missing same-repository merge evidence for closing PR #${pr.number}.`);
          prs.set(pr.id, pr);
          closing.add(pr.id);
        }
        issues.set(id, closing);
        if (cursor) more.push({ id, cursor });
      });
      if (prs.size > LIMITS.prs) throw new Error(`Closing PR cap reached. ${recovery}`);
      pending = more;
    }
    if (pending.length) throw new Error(`Closing PR pagination cap reached. ${recovery}`);
  }
  return { issues, prs };
}

/** Local full-history git supplies merged paths and ancestry; no per-commit API fan-out. */
export function gitEvidence({ cwd = process.cwd(), spawn = spawnSync } = {}) {
  const git = (args, input) => {
    const result = spawn('git', args, {
      cwd,
      input,
      encoding: 'utf8',
      timeout: 30_000,
      maxBuffer: 32 * 1024 * 1024,
    });
    if (result.error || result.status !== 0)
      throw new Error(`Git evidence failed: git ${args.join(' ')}`);
    return result.stdout;
  };
  if (git(['rev-parse', '--is-shallow-repository']).trim() !== 'false')
    throw new Error('Release tracking requires full git history and tags.');
  return {
    validateTags: (tags) => {
      if (!tags.length) return;
      const objects = git(
        ['cat-file', '--batch-check=%(objectname) %(objecttype)'],
        tags.map((tag) => `refs/tags/${tag}^{commit}\n`).join('')
      )
        .trim()
        .split('\n');
      if (
        objects.length !== tags.length ||
        objects.some((object) => !/^[a-f0-9]{40} commit$/.test(object))
      )
        throw new Error('Git evidence failed: a published tag does not resolve to a commit.');
    },
    paths: (sha) => {
      if (!/^[a-f0-9]{40}$/.test(sha)) throw new Error('Invalid merge commit.');
      // First-parent delta is the change actually merged, including both sides of renames.
      return git(['diff', '--name-only', '--no-renames', '-z', `${sha}^1`, sha, '--'])
        .split('\0')
        .filter(Boolean);
    },
    containingTags: (sha) => {
      if (!/^[a-f0-9]{40}$/.test(sha)) throw new Error('Invalid merge commit.');
      return new Set(git(['tag', '--contains', sha]).split('\n').filter(Boolean));
    },
  };
}

export function affectedProducts(paths, map, workspace) {
  const products = new Set(),
    unpublished = new Set();
  for (const path of paths) {
    const name = ownerOf(path, map);
    const component = map.components.find((entry) => entry.name === name);
    if (!component) throw new Error(`No component owns ${path}.`);
    if (component.releaseStatus === 'never') continue;
    // Consumers replace only their private leaf, while Cargo/root membership remains additive.
    const owners = new Set([
      ...(component.releaseConsumers.length ? component.releaseConsumers : [name]),
      ...releasedComponentsForPath(path, map, workspace).map((entry) => entry.name),
    ]);
    for (const owner of owners) {
      try {
        releasePolicy(owner);
        products.add(owner);
      } catch {
        unpublished.add(owner);
      }
    }
  }
  return { products: [...products].sort(), unpublished: [...unpublished].sort() };
}

export function deriveEvidence(items, closing, releases, git, map, workspace) {
  git.validateTags(releases.map((r) => r.tag_name));
  const merged = new Map();
  for (const [id, pr] of closing.prs) {
    const sha = pr.mergeCommit.oid;
    const paths = git.paths(sha);
    merged.set(id, {
      ...affectedProducts(paths, map, workspace),
      empty: !paths.length,
      tags: git.containingTags(sha),
    });
  }
  const evidence = new Map();
  for (const item of items) {
    const ids = [...closing.issues.get(item.content.id)];
    const requirements = new Map(),
      waiting = new Set();
    for (const id of ids) {
      const pr = merged.get(id);
      if (pr.empty) waiting.add('Merge has no changed paths');
      for (const owner of pr.unpublished) waiting.add(`No publication policy: ${owner}`);
      for (const product of pr.products) {
        const tags = requirements.get(product) || [];
        tags.push(pr.tags);
        requirements.set(product, tags);
      }
    }
    const labels = [];
    for (const [product, contains] of [...requirements].sort(([a], [b]) => a.localeCompare(b))) {
      const first = releases.find(
        (r) =>
          releaseIdentity(r.tag_name).product === product &&
          contains.every((tags) => tags.has(r.tag_name))
      );
      if (first) labels.push(releaseIdentity(first.tag_name).label);
      else waiting.add(`Awaiting ${product}`);
    }
    const parked = map.components.filter(
      (component) =>
        component.releaseStatus === 'parked' &&
        (waiting.has(`Awaiting ${component.name}`) ||
          waiting.has(`No publication policy: ${component.name}`))
    );
    const parkedReasons = new Set(
      parked.flatMap((component) => [
        `Awaiting ${component.name}`,
        `No publication policy: ${component.name}`,
      ])
    );
    const onlyParked =
      waiting.size > 0 && [...waiting].every((reason) => parkedReasons.has(reason));
    const parkedNotes = onlyParked
      ? parked
          .sort((a, b) => a.name.localeCompare(b.name))
          .map((component) => {
            const product = component.name
              .replace(/^tmt-/, '')
              .replace(/^./, (letter) => letter.toUpperCase());
            return `ships with the first ${product} release`;
          })
      : [];
    const noRelease = requirements.size === 0 && waiting.size === 0;
    evidence.set(item.content.id, {
      status:
        !ids.length || noRelease || onlyParked ? 'Done' : waiting.size ? 'Merged' : 'Released',
      text: [...labels, ...parkedNotes].join('\n'),
      prs: ids.map((id) => closing.prs.get(id).number),
      waiting: [...waiting].sort(),
    });
  }
  return evidence;
}

/** Recompute both owned fields, including incorrect existing terminal values. */
export function planUpdates(evidence, project) {
  const rows = [];
  for (const [id, planned] of evidence) {
    const item = project.items.get(id);
    if (!item) throw new Error(`Project item disappeared: ${id}.`);
    const current = { status: item.status?.name || '', text: item.released?.text || '' };
    if (current.status && !project.options[current.status])
      throw new Error(`Unknown status for ${item.content.url}.`);
    if (planned.text.length > 10_000) throw new Error('Released in text limit reached.');
    rows.push({
      itemId: item.id,
      issue: item.content.url,
      current,
      ...planned,
      writeText: current.text !== planned.text,
      writeStatus: current.status !== planned.status,
    });
  }
  return { rows, changes: rows.filter((row) => row.writeText || row.writeStatus) };
}

export function applyUpdates(api, project, plan, dryRun) {
  const status = plan.changes.filter((row) => row.writeStatus);
  const batches = [
    // Correct false Released claims before replacing their evidence.
    ...chunks(status.filter((row) => row.status !== 'Released')).map((rows) => ({
      rows,
      field: 'status',
    })),
    ...chunks(plan.changes.filter((row) => row.writeText)).map((rows) => ({ rows, field: 'text' })),
    ...chunks(status.filter((row) => row.status === 'Released')).map((rows) => ({
      rows,
      field: 'status',
    })),
  ];
  api.reserve(batches.length + (plan.changes.length ? LIMITS.pages : 0));
  if (dryRun) return;
  for (const { rows, field } of batches) {
    const mutations = rows
      .map((row, i) => {
        const common = `projectId:${quote(project.projectId)},itemId:${quote(row.itemId)},fieldId:${quote(field === 'text' ? project.releasedId : project.statusId)}`;
        if (field === 'text' && !row.text)
          return `u${i}:clearProjectV2ItemFieldValue(input:{${common}}){projectV2Item{id}}`;
        const value =
          field === 'text'
            ? `text:${quote(row.text)}`
            : `singleSelectOptionId:${quote(project.options[row.status])}`;
        return `u${i}:updateProjectV2ItemFieldValue(input:{${common},value:{${value}}}){projectV2Item{id}}`;
      })
      .join('\n');
    const result = api.graphql(`mutation{${mutations}}`);
    rows.forEach((row, i) => {
      if (result[`u${i}`]?.projectV2Item?.id !== row.itemId)
        throw new Error('Incomplete Project mutation response.');
    });
  }
}

export function reconcile({
  api,
  repository,
  dryRun,
  projectId = PROJECT_ID,
  workspace = readCargoWorkspace(fileURLToPath(new URL('../../', import.meta.url))),
  git = gitEvidence(),
  map = parseComponentMap(
    readFileSync(new URL('../../.github/components.json', import.meta.url), 'utf8')
  ),
}) {
  const releases = readReleases(api);
  const project = readProject(api, projectId);
  const closed = [...project.items.values()].filter(
    (item) =>
      item.content.state === 'CLOSED' &&
      item.content.repository.nameWithOwner.toLowerCase() === repository.toLowerCase()
  );
  const epics = closed.filter((item) =>
    item.content.labels.nodes.some(({ name }) => name.toLowerCase() === 'epic')
  );
  const epicIds = new Set(epics.map((item) => item.id));
  const items = closed.filter((item) => !epicIds.has(item.id));
  const closing = readClosingPrs(api, items, repository);
  const evidence = deriveEvidence(items, closing, releases, git, map, workspace);
  const plan = planUpdates(evidence, project);
  applyUpdates(api, project, plan, dryRun);
  if (!dryRun && plan.changes.length) {
    const readback = readProject(api, projectId);
    for (const item of items)
      if (readback.items.get(item.content.id)?.content.state !== 'CLOSED')
        throw new Error('Issue reopened during reconciliation; inspect the readback.');
    if (planUpdates(evidence, readback).changes.length)
      throw new Error(`Project readback did not match the plan. ${recovery}`);
  }
  return {
    dryRun,
    releases: releases.map((r) => r.tag_name),
    issues: closed.length,
    rows: [
      ...plan.rows,
      ...epics.map((item) => {
        const current = { status: item.status?.name || '', text: item.released?.text || '' };
        return {
          itemId: item.id,
          issue: item.content.url,
          current,
          ...current,
          prs: [],
          waiting: ['skipped: epic tracker'],
          writeText: false,
          writeStatus: false,
        };
      }),
    ],
    changed: plan.changes,
    requests: { ...api.counts },
    points: { ...api.points },
  };
}

function renderPoints(points) {
  return `GraphQL points: ${points.cost} (reported reads); last remaining: ${points.remaining ?? 'unavailable'}.`;
}

export function renderFailure(error, api) {
  return `Project release tracking failed: ${error.message}\nRequests: ${JSON.stringify(api.counts)}\n${renderPoints(api.points)}\n`;
}

export function renderSummary(result) {
  const cell = (value) =>
    String(value)
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;')
      .replaceAll('|', '&#124;')
      .replace(/\r?\n/g, '<br>');
  return [
    `### Project release tracking${result.dryRun ? ' (dry run)' : ''}`,
    '',
    `${result.issues} closed issues; ${result.changed.length} changes; REST ${result.requests.rest}/${LIMITS.rest}; GraphQL ${result.requests.graphql}/${LIMITS.graphql}.`,
    '',
    renderPoints(result.points),
    '',
    '| Item | Status: current → planned | Released in: current → planned | Waiting |',
    '| --- | --- | --- | --- |',
    ...result.rows.map(
      (row) =>
        `| ${cell(row.issue)} | ${cell(row.current.status)} → ${cell(row.status)} | ${cell(row.current.text || '—')} → ${cell(row.text || '—')} | ${cell(row.waiting.join('; '))} |`
    ),
    '',
    'The full sweep is authoritative for eligible issues; epic tracker fields remain owner-managed. Built-in close/merge events cannot cause permanent drift: the next full sweep repairs it.',
    '',
  ].join('\n');
}

export function main(env = process.env) {
  if (!['workflow_dispatch', 'schedule'].includes(env.GITHUB_EVENT_NAME))
    throw new Error('Unsupported updater event.');
  const api = githubApi({
    appToken: env.RELEASE_APP_TOKEN,
    readToken: env.GITHUB_TOKEN,
    repository: env.GITHUB_REPOSITORY,
  });
  try {
    const result = reconcile({
      api,
      repository: env.GITHUB_REPOSITORY,
      dryRun: env.GITHUB_EVENT_NAME === 'workflow_dispatch' ? env.DRY_RUN !== 'false' : false,
    });
    const summary = renderSummary(result);
    process.stdout.write(summary);
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, summary);
  } catch (error) {
    const message = renderFailure(error, api);
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, message);
    throw new Error(message);
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
