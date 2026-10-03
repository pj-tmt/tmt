// The shadow's only network boundary: bounded REST GETs, never a publishing client.
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runPackedCommand } from './packed-command.mjs';

const SHA = /^[a-f0-9]{40}$/;

export function readCutMetadata(
  { repository, cut, token, draftVisibility },
  execute = runPackedCommand
) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository ?? '') || !SHA.test(cut ?? ''))
    throw new Error('Shadow metadata needs a repository and exact main cut SHA.');
  if (!token || draftVisibility !== 'trusted')
    throw new Error('Draft visibility requires the trusted contents-write metadata reader.');
  let requests = 0;
  const deadline = Date.now() + 90_000;
  const get = (path) => {
    if (++requests > 60 || Date.now() >= deadline) throw new Error('Shadow REST budget exceeded.');
    return JSON.parse(
      execute('gh', ['api', `repos/${repository}/${path}`, '--method', 'GET'], {
        cwd: process.cwd(),
        env: { ...process.env, GH_TOKEN: token },
        timeoutMs: Math.min(10_000, deadline - Date.now()),
      })
    );
  };
  const list = (path) => {
    const rows = [];
    for (let page = 1; page <= 10; page += 1) {
      const result = get(`${path}${path.includes('?') ? '&' : '?'}per_page=100&page=${page}`);
      const batch = result;
      if (!Array.isArray(batch) || batch.length > 100) throw new Error('Invalid shadow REST page.');
      rows.push(...batch);
      if (batch.length < 100) {
        const ids = rows.map((row) => row.id);
        if (ids.some((id) => !Number.isSafeInteger(id)) || new Set(ids).size !== ids.length)
          throw new Error('Missing or duplicate shadow REST records.');
        return rows;
      }
    }
    throw new Error('Incomplete shadow REST pagination.');
  };
  const releases = list('releases').map(({ id, tag_name, draft, body, target_commitish }) => ({
    id,
    tag_name,
    draft,
    body,
    target_commitish,
  }));
  if (releases.some((r) => typeof r.draft !== 'boolean' || typeof r.tag_name !== 'string'))
    throw new Error('Invalid release metadata.');
  return {
    schema: 1,
    repository,
    cut,
    draftVisibility: 'trusted',
    capturedAt: new Date().toISOString(),
    releases,
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [output, ...extra] = process.argv.slice(2);
  if (!output || extra.length) throw new Error('Usage: release-cut-read.mjs <metadata.json>');
  const input = {
    repository: process.env.GITHUB_REPOSITORY,
    cut: process.env.CUT_SHA,
    token: process.env.GH_TOKEN,
    draftVisibility: process.env.DRAFT_VISIBILITY,
  };
  let result;
  try {
    result = readCutMetadata(input);
  } catch (error) {
    // An unavailable snapshot is visible, never interpreted as an empty draft/run list.
    result = {
      schema: 1,
      repository: input.repository,
      cut: input.cut,
      evidenceError: error.message,
    };
  }
  writeFileSync(output, `${JSON.stringify(result, null, 2)}\n`);
}
