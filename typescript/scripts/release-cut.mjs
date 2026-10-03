// Plans release cuts in shadow mode. This module has no network/mutation interface.
import assert from 'node:assert/strict';
import { appendFileSync, readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import { readCargoWorkspace } from './cargo-workspace.mjs';
import parser from '@conventional-commits/parser';
import presetFactory from 'conventional-changelog-conventionalcommits';
import writer from 'conventional-changelog-writer';
import { ownerOf, parseComponentMap, releasedComponentsForPath } from './ci-scope.mjs';
import { releasePolicy } from './native-release-policy.mjs';
import { compareVersions, publishedReleases, versionOfTag } from './release-versions.mjs';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const SHA = /^[a-f0-9]{40}$/;
const ACTIVE = new Set(['queued', 'in_progress', 'requested', 'waiting', 'pending']);
const nodes = (node) => [node, ...(node.children ?? []).flatMap(nodes)];
const content = (message, node) =>
  message.slice(node.position.start.offset, node.position.end.offset);

/** AST-backed conventional parsing, including nested messages; no release-please API. */
export function parseReleaseCommits(commits) {
  const parsed = [];
  for (const commit of commits) {
    if (!SHA.test(commit.sha) || typeof commit.message !== 'string' || !Array.isArray(commit.files))
      throw new Error('Invalid cut commit.');
    const parts = commit.message.split(/BEGIN_NESTED_COMMIT\r?\n/);
    const messages = [parts.shift()];
    for (const part of parts) {
      const end = part.indexOf('END_NESTED_COMMIT');
      if (end < 0) throw new Error('Unclosed nested release message.');
      messages.push(part.slice(0, end));
      messages[0] += part.slice(end + 'END_NESTED_COMMIT'.length);
    }
    const parse = (message) => {
      let ast;
      try {
        ast = parser.parser(message.trim());
      } catch {
        return;
      }
      message = message.trim();
      const summary = ast.children.find((node) => node.type === 'summary');
      const fields = summary.children;
      if (!fields.some((node) => node.type === 'separator' && node.value === ':')) return;
      const subject = fields.find((node) => node.type === 'text')?.value;
      const notes = [];
      const references = [];
      const footers = ast.children.filter((node) => node.type === 'footer');
      for (const footer of footers) {
        const leaves = nodes(footer);
        const text = leaves
          .filter((node) => node.type === 'text' || node.type === 'newline')
          .map((node) => node.value)
          .join('')
          .trim();
        if (
          leaves.some((node) => node.type === 'breaking-change' && node.value.includes('BREAKING'))
        )
          notes.push({ title: 'BREAKING CHANGE', text });
        const token = leaves.find((node) => node.type === 'type')?.value;
        const issue = /^#?(\d+)$/.exec(text);
        if (
          issue &&
          /^(?:refs?|fix(?:es)?|clos(?:es|e[sd])|resolv(?:es|e[sd]))$/i.test(token ?? '')
        )
          references.push({ action: token, prefix: '#', issue: issue[1] });
        // Conventional footer messages share their source SHA; issue/author footers do not parse.
        if (token && token.toLowerCase() !== 'release-as') parse(content(message, footer));
      }
      if (!notes.length && fields.some((node) => node.type === 'breaking-change'))
        notes.push({ title: 'BREAKING CHANGE', text: subject });
      const body = ast.children.find((node) => node.type === 'body');
      if (body) {
        const match = /(?:^|\n)BREAKING[ -]CHANGE:\s*([^\n]+(?:\n(?!\n)[^\n]+)*)/.exec(
          content(message, body)
        );
        if (match) notes.push({ title: 'BREAKING CHANGE', text: match[1].trim() });
      }
      parsed.push({
        hash: commit.sha,
        header: content(message, summary),
        subject,
        type: fields.find((node) => node.type === 'type').value,
        scope: fields.find((node) => node.type === 'scope')?.value ?? null,
        notes,
        references,
        body: '',
        footer: '',
        merge: null,
        revert: null,
        mentions: [],
      });
    };
    for (const message of messages) parse(message);
  }
  return parsed;
}

/** Ownership is the component map's responsibility, including private-leaf consumers. */
export function attributeCutCommits(commits, map, product, workspace) {
  const byName = new Map(map.components.map((c) => [c.name, c]));
  const selected = new Map();
  for (const commit of commits) {
    if (
      commit.files.some((path) => {
        const owner = ownerOf(path, map);
        return (
          owner === product ||
          releasedComponentsForPath(path, map, workspace).some(
            (component) => component.name === product
          ) ||
          byName.get(owner)?.releaseConsumers.includes(product)
        );
      })
    )
      selected.set(commit.sha, commit);
  }
  return [...selected.values()];
}

export function nextAlphaVersion(version) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-alpha\.(0|[1-9]\d*)$/.exec(version);
  if (
    !match ||
    !Number.isSafeInteger(Number(match[4])) ||
    Number(match[4]) >= Number.MAX_SAFE_INTEGER
  )
    throw new Error(
      'A non-alpha or major/minor change requires an owner-dispatched explicit version.'
    );
  return `${match.slice(1, 4).join('.')}-alpha.${Number(match[4]) + 1}`;
}

export async function renderCutNotes({ commits, repository, version, previousTag, tag, date }) {
  const [owner, repo] = repository.split('/');
  const preset = await presetFactory({});
  preset.writerOpts.commitPartial = preset.writerOpts.commitPartial.replace(
    /,\s*closes/g,
    ', refs'
  );
  const context = {
    host: 'https://github.com',
    owner,
    repository: repo,
    version,
    previousTag,
    currentTag: tag,
    linkCompare: !!previousTag,
    date,
  };
  const parsed = parseReleaseCommits(commits);
  for (const commit of parsed) {
    commit.subject = commit.subject.replace(/``[^`].*[^`]``|`[^`]*`|<|>/g, (match) =>
      match.length > 1 ? match : match === '<' ? '&lt;' : '&gt;'
    );
    for (const note of commit.notes)
      note.text = note.text.replace(
        /\(#(\d+)\)/g,
        `([#$1](https://github.com/${repository}/issues/$1))`
      );
  }
  const expected = [
    ...new Set(
      parsed
        .filter((commit) => preset.writerOpts.transform(structuredClone(commit), context))
        .map((commit) => commit.hash)
    ),
  ].sort();
  const notes = writer.parseArray(parsed, context, preset.writerOpts).trim();
  const linked = [
    ...new Set([...notes.matchAll(/\/commit\/([a-f0-9]{40})\)/g)].map((match) => match[1])),
  ].sort();
  assert.deepEqual(linked, expected, 'Cut notes must list exactly the releasable source SHAs.');
  return { notes, commits: expected, breaking: parsed.some((commit) => commit.notes.length > 0) };
}

/** Immutable local acquisition. Merge diffs use the first parent, not every parent's files. */
export function readCutRange(git, previous, cut) {
  if (!SHA.test(previous) || !SHA.test(cut)) throw new Error('Cut range requires exact SHAs.');
  git(['merge-base', '--is-ancestor', previous, cut]);
  const raw = git([
    'log',
    '--first-parent',
    '--max-count=501',
    '--format=%H%x00%B%x00',
    `${previous}..${cut}`,
  ]);
  if (!raw) return [];
  const fields = raw.split('\0');
  if (fields.length % 2 !== 1 || fields.at(-1).trim() || (fields.length - 1) / 2 > 500)
    throw new Error('Incomplete or oversized cut history.');
  const commits = [];
  for (let i = 0; i + 1 < fields.length; i += 2) {
    const sha = fields[i].trim();
    if (!SHA.test(sha)) throw new Error('Invalid cut history SHA.');
    const parents = git(['show', '-s', '--format=%P', sha]).trim().split(' ').filter(Boolean);
    const files = parents.length
      ? git(['diff', '--name-only', '--no-renames', '-z', parents[0], sha])
      : git([
          'diff-tree',
          '--root',
          '--no-commit-id',
          '-r',
          '--name-only',
          '--no-renames',
          '-z',
          sha,
        ]);
    commits.push({ sha, message: fields[i + 1], files: files.split('\0').filter(Boolean) });
  }
  return commits;
}

export async function planReleaseCuts({
  metadata,
  map,
  git,
  date,
  workspace,
  initialVersions = {},
  versions = {},
}) {
  for (const [product, version] of Object.entries(versions)) {
    if (!map.components.some((c) => c.name === product && c.package && c.release !== false))
      throw new Error(`Explicit version names an unreleased component ${product}.`);
    if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-alpha\.(0|[1-9]\d*))?$/.test(version))
      throw new Error('An explicit cut version must be a canonical stable or alpha version.');
  }
  if (
    metadata.schema !== 1 ||
    !SHA.test(metadata.cut ?? '') ||
    !/^[\w.-]+\/[\w.-]+$/.test(metadata.repository ?? '')
  )
    throw new Error('Invalid shadow snapshot.');
  const { cut, repository } = metadata;
  if (metadata.evidenceError || metadata.draftVisibility !== 'trusted')
    return {
      cut,
      repository,
      mode: 'shadow',
      unavailable: metadata.evidenceError ?? 'Draft visibility unavailable.',
      components: [],
    };
  if (!Array.isArray(metadata.releases) || !Array.isArray(metadata.runs))
    throw new Error('Incomplete shadow metadata.');
  git(['merge-base', '--is-ancestor', cut, 'refs/remotes/origin/main']);
  const components = [];
  for (const component of map.components) {
    if (component.release === false || !component.package) continue;
    const row = { product: component.name, cut, status: 'blocked' };
    try {
      const { tagPrefix } = releasePolicy(component.name);
      const active = metadata.runs.filter((run) => ACTIVE.has(run.status));
      const unknown = active.some((run) => {
        const product = /^Native release: ([\w-]+)$/.exec(run.display_title ?? '')?.[1];
        return !map.components.some((component) => component.name === product);
      });
      if (
        unknown ||
        active.some((run) => run.display_title === `Native release: ${component.name}`)
      )
        throw new Error(
          unknown
            ? 'Active native release has no reliable product identity.'
            : 'Native release is queued/running for this component.'
        );
      if (
        metadata.releases.some(
          (release) =>
            release.draft === true &&
            release.tag_name.startsWith(tagPrefix) &&
            /^\d/.test(release.tag_name.slice(tagPrefix.length))
        )
      )
        throw new Error(
          'Component draft is in flight; tagged/unexpected drafts also require investigation.'
        );
      const previousRelease = publishedReleases(metadata.releases, component.name)[0];
      if (!previousRelease && (!component.bootstrapSha || !initialVersions[component.name]))
        throw new Error('First cut needs bootstrapSha and an owner-approved initial version.');
      const previous = previousRelease
        ? git(['rev-parse', '--verify', `refs/tags/${previousRelease.tag_name}^{commit}`]).trim()
        : component.bootstrapSha;
      if (!SHA.test(previous)) throw new Error('Missing published product tag commit.');
      const previousVersion =
        previousRelease && versionOfTag(previousRelease.tag_name, component.name);
      const version =
        versions[component.name] ??
        (previousRelease ? nextAlphaVersion(previousVersion) : initialVersions[component.name]);
      if (previousRelease && compareVersions(version, previousVersion) <= 0)
        throw new Error('Explicit cut version must advance the latest published product version.');
      if (!previousRelease) nextAlphaVersion(version); // validate the explicitly supplied alpha seed
      const tag = `${tagPrefix}${version}`;
      const commits = attributeCutCommits(
        readCutRange(git, previous, cut),
        map,
        component.name,
        workspace
      );
      const notes = await renderCutNotes({
        commits,
        repository,
        version,
        previousTag: previousRelease?.tag_name,
        tag,
        date,
      });
      Object.assign(row, notes, {
        previous,
        previousTag: previousRelease?.tag_name,
        version,
        tag,
        status: notes.commits.length ? 'proposed' : 'no-releasable-commits',
        authorization: notes.breaking || versions[component.name] ? 'owner-required' : 'alpha',
      });
    } catch (error) {
      row.reason = error.message;
    }
    components.push(row);
  }
  return { cut, repository, mode: 'shadow', mapDigest: map.digest, components };
}

export function renderCutSummary(plan) {
  const lines = [
    '## Release cut (shadow only)',
    '',
    `Cut: \`${plan.cut}\``,
    '',
    'Creates no drafts, tags or dispatches.',
    '',
  ];
  if (plan.unavailable) lines.push(`Evidence unavailable: ${plan.unavailable}`, '');
  for (const row of plan.components) {
    lines.push(
      `### ${row.product}: ${row.status}`,
      '',
      row.reason ??
        `Next tag: \`${row.tag}\`; previous cut: \`${row.previous}\`; authorization: ${row.authorization}.`,
      ''
    );
    if (row.notes)
      lines.push(row.notes, '', `Releasable SHAs: ${(row.commits ?? []).join(', ') || 'none'}`, '');
  }
  return `${lines.join('\n')}\n`;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [file, ...extra] = process.argv.slice(2);
    if (!file || extra.length)
      throw new Error('Usage: release-cut.mjs <metadata.json> (shadow only)');
    const metadata = JSON.parse(readFileSync(file, 'utf8'));
    const git = (args) =>
      runPackedCommand('git', args, { cwd: ROOT, env: process.env, timeoutMs: 10_000 }).trimEnd();
    // All release decisions use the map from the captured cut, never a later main checkout.
    const map = parseComponentMap(git(['show', `${metadata.cut}:.github/components.json`]));
    // Cargo must read the same immutable cut as the map, not the running checkout.
    const directory = mkdtempSync(join(tmpdir(), 'tmt-cut-workspace-'));
    let plan;
    try {
      const archive = join(directory, 'source.tar');
      git(['archive', '--format=tar', `--output=${archive}`, metadata.cut]);
      runPackedCommand('tar', ['-xf', archive, '-C', directory], { cwd: ROOT, env: process.env });
      plan = await planReleaseCuts({
        metadata,
        map,
        git,
        workspace: readCargoWorkspace(directory),
        date: metadata.capturedAt?.slice(0, 10),
      });
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
    console.log(JSON.stringify(plan, null, 2));
    if (process.env.GITHUB_STEP_SUMMARY)
      appendFileSync(process.env.GITHUB_STEP_SUMMARY, renderCutSummary(plan));
    if (plan.unavailable) process.exitCode = 1;
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
