// Plans release cuts without mutation. This module has no network/mutation interface.
import assert from 'node:assert/strict';
import { appendFileSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import parser from '@conventional-commits/parser';
import presetFactory from 'conventional-changelog-conventionalcommits';
import writer from 'conventional-changelog-writer';
import { ownerOf, releasedComponentsForPath } from './ci-scope.mjs';
import {
  componentOfProduct,
  productOfComponent,
  productOfTag,
  releasePolicy,
} from './native-release-policy.mjs';
import { compareVersions, publishedReleases, versionOfTag } from './release-versions.mjs';
import { readReleaseSourceAtRef } from './release-source-at-ref.mjs';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const SHA = /^[a-f0-9]{40}$/;
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
  const componentName = componentOfProduct(map, product).name;
  const selected = new Map();
  for (const commit of commits) {
    if (
      commit.files.some((path) => {
        const owner = ownerOf(path, map);
        return (
          releasedComponentsForPath(path, map, workspace).some(
            (component) => component.name === componentName
          ) || byName.get(owner)?.releaseConsumers.includes(componentName)
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

/** All drafts/tags reserve numbers; allocated ancestors delimit new work, published ancestors delimit notes. */
export function releaseCutHistory({ releases, product, cut, git, excludeTag = '' }) {
  const { tagPrefix } = releasePolicy(product);
  const tags = git(['tag', '--list', `${tagPrefix}*`])
    .trim()
    .split('\n')
    .filter(Boolean);
  const allocated = new Map();
  for (const tag of tags) {
    if (productOfTag(tag) === product && tag !== excludeTag)
      allocated.set(tag, { tag, tagged: true });
  }
  for (const release of releases) {
    if (productOfTag(release.tag_name) !== product || release.tag_name === excludeTag) continue;
    const existing = allocated.get(release.tag_name);
    allocated.set(release.tag_name, {
      tag: release.tag_name,
      tagged: existing?.tagged || !release.draft,
      sha: release.target_commitish,
      published: !release.draft,
    });
  }
  const ordered = [...allocated.values()].sort((a, b) =>
    compareVersions(versionOfTag(b.tag, product), versionOfTag(a.tag, product))
  );
  let previous = null;
  let previousAllocated = null;
  const isAncestor = (ancestor, descendant) => {
    try {
      git(['merge-base', '--is-ancestor', ancestor, descendant]);
      return true;
    } catch (error) {
      if (error.cause?.status !== 1) throw error;
      return false;
    }
  };
  for (const entry of ordered) {
    // Orphan tags reserve a number but do not establish an allocated release cut.
    if (entry.published === undefined) continue;
    const sha = entry.tagged
      ? git(['rev-parse', '--verify', `refs/tags/${entry.tag}^{commit}`]).trim()
      : entry.sha;
    // Old symbolic-target drafts reserve their numbers without inventing a source boundary.
    if (!SHA.test(sha ?? '')) {
      if (entry.tagged) throw new Error('Missing product tag commit.');
      continue;
    }
    if (!isAncestor(sha, cut)) continue;
    if (!previous && entry.published) previous = { tag: entry.tag, sha };
    if (
      !previousAllocated ||
      (previousAllocated.sha !== sha && isAncestor(previousAllocated.sha, sha))
    )
      previousAllocated = { tag: entry.tag, sha };
  }
  return {
    highestVersion: ordered[0] && versionOfTag(ordered[0].tag, product),
    previous,
    previousAllocated,
  };
}

export async function planReleaseCuts({ metadata, map, workspace, git, date, versions = {} }) {
  for (const [product, version] of Object.entries(versions)) {
    if (
      !componentOfProduct(map, product).package ||
      componentOfProduct(map, product).release === false
    )
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
      mode: 'plan',
      unavailable: metadata.evidenceError ?? 'Draft visibility unavailable.',
      components: [],
    };
  if (!Array.isArray(metadata.releases)) throw new Error('Incomplete shadow metadata.');
  git(['merge-base', '--is-ancestor', cut, 'refs/remotes/origin/main']);
  const components = [];
  for (const component of map.components) {
    if (component.release === false || !component.package) continue;
    const row = { product: component.name, cut, status: 'blocked' };
    try {
      const product = productOfComponent(component.name);
      row.product = product;
      const { tagPrefix } = releasePolicy(product);
      const history = releaseCutHistory({
        releases: metadata.releases,
        product,
        cut,
        git,
      });
      if (
        !versions[product] &&
        metadata.releases.some(
          (release) =>
            productOfTag(release.tag_name) === product && release.target_commitish === cut
        )
      ) {
        Object.assign(row, {
          status: 'already-cut',
          reason:
            'This component already has a release cut at X; later main cuts remain independent.',
        });
        components.push(row);
        continue;
      }
      if (!history.previous && (!component.bootstrapSha || !component.initialVersion))
        throw new Error('First cut needs bootstrapSha and an owner-approved initial version.');
      if (!history.previous && component.requiresCliSha) {
        const cli = publishedReleases(metadata.releases, 'cli')[0];
        const reason = `First ${product} cut requires a published supporting CLI containing registration ${component.requiresCliSha}.`;
        if (!cli) throw new Error(reason);
        const cliSha = git(['rev-parse', '--verify', `refs/tags/${cli.tag_name}^{commit}`]).trim();
        if (!SHA.test(cliSha)) throw new Error('Missing supporting CLI tag commit.');
        try {
          git(['merge-base', '--is-ancestor', component.requiresCliSha, cliSha]);
        } catch (error) {
          if (error.cause?.status !== 1) throw error;
          throw new Error(`${reason} Newest published CLI ${cli.tag_name} predates registration.`);
        }
      }
      const previous = history.previous?.sha ?? component.bootstrapSha;
      if (!SHA.test(previous)) throw new Error('Missing previous product cut commit.');
      const previousVersion = history.highestVersion;
      if (
        versions[product] &&
        previousVersion &&
        compareVersions(versions[product], previousVersion) <= 0
      )
        throw new Error('Explicit cut version must advance every allocated product version.');
      const allocatedPrevious = history.previousAllocated?.sha ?? previous;
      const pendingCommits = attributeCutCommits(
        readCutRange(git, allocatedPrevious, cut),
        map,
        product,
        workspace
      );
      // Use the same renderer visibility contract before choosing a new version.
      const pending = await renderCutNotes({
        commits: pendingCommits,
        repository,
        version: previousVersion ?? component.initialVersion,
        previousTag: history.previous?.tag,
        tag: `${tagPrefix}${previousVersion ?? component.initialVersion}`,
        date,
      });
      if (!pending.commits.length) {
        Object.assign(row, {
          previous,
          previousTag: history.previous?.tag,
          status: 'no-releasable-commits',
          reason: 'No releasable commits since the newest allocated ancestor cut.',
        });
        components.push(row);
        continue;
      }
      const version =
        versions[product] ??
        (previousVersion ? nextAlphaVersion(previousVersion) : component.initialVersion);
      if (!previousVersion) nextAlphaVersion(version);
      const tag = `${tagPrefix}${version}`;
      const notesInput = {
        repository,
        version,
        previousTag: history.previous?.tag,
        tag,
        date,
      };
      const commits =
        allocatedPrevious === previous
          ? pendingCommits
          : attributeCutCommits(readCutRange(git, previous, cut), map, product, workspace);
      const notes = await renderCutNotes({
        ...notesInput,
        commits,
      });
      Object.assign(row, notes, {
        previous,
        previousTag: history.previous?.tag,
        version,
        tag,
        status: notes.commits.length ? 'proposed' : 'no-releasable-commits',
        authorization: notes.breaking || versions[product] ? 'owner-required' : 'alpha',
      });
    } catch (error) {
      row.reason = error.message;
    }
    components.push(row);
  }
  return { cut, repository, mode: 'plan', mapDigest: map.digest, components };
}

export function renderCutSummary(plan) {
  const lines = [
    '## Release cut (read only)',
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
        `Next tag: \`${row.tag}\`; release boundary: \`${row.previous}\`; authorization: ${row.authorization}.`,
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
      throw new Error('Usage: release-cut.mjs <metadata.json> (read only)');
    const metadata = JSON.parse(readFileSync(file, 'utf8'));
    const git = (args) =>
      runPackedCommand('git', args, { cwd: ROOT, env: process.env, timeoutMs: 10_000 }).trimEnd();
    // All release decisions use the map from the captured cut, never a later main checkout.
    const { map, workspace } = readReleaseSourceAtRef(metadata.cut, { warm: true });
    const plan = await planReleaseCuts({
      metadata,
      map,
      workspace,
      git,
      date: metadata.capturedAt?.slice(0, 10),
    });
    console.log(JSON.stringify(plan, null, 2));
    if (process.env.GITHUB_STEP_SUMMARY)
      appendFileSync(process.env.GITHUB_STEP_SUMMARY, renderCutSummary(plan));
    if (plan.unavailable) process.exitCode = 1;
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
