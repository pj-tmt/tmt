// Links to the old organization name (pj-tmt) in tracked files. Redirects keep old github.com links
// working, but github.io pages do not redirect, and new links kept spreading after the rename to
// tmt-ninja (#5137). A tracked file may keep only the links the ratchet lists for it, and may only
// lose them: `node scripts/previous-repository-links.mjs --write` lowers the ratchet after a
// cleanup and refuses to raise it. Bare `pj-tmt/tmt` repository constants that deliberately accept
// the old name are not URL-shaped, so they need no entry.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
export const RATCHET = 'typescript/test/fixtures/previous-repository-links.json';
const OWN = new Set([
  RATCHET,
  'typescript/scripts/previous-repository-links.mjs',
  'typescript/test/tooling/previous-repository-links.test.ts',
]);
const FILE_LIMIT = 4 * 1024 * 1024;
const LINK =
  /(?:(?:github|githubusercontent)\.com|ghcr\.io)\/pj-tmt(?![\w-])|\brepos\/pj-tmt(?![\w-])|pj-tmt\.github\.io/g;

/** The 1-based line of each link in `text`, once per link. */
export function linkLines(text) {
  return text
    .split('\n')
    .flatMap((line, index) => Array.from(line.matchAll(LINK), () => index + 1));
}

/** Every tracked text file that links to the old organization: `{ path, lines }`. */
export function scan(root = ROOT) {
  const files = execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8' })
    .split('\0')
    .filter((file) => file && !OWN.has(file));
  const found = [];
  for (const path of files) {
    let bytes;
    try {
      bytes = readFileSync(`${root}${path}`);
    } catch {
      continue; // listed but absent from the working tree (deleted, not yet committed)
    }
    if (bytes.length > FILE_LIMIT || bytes.subarray(0, 8192).includes(0)) continue;
    const lines = linkLines(bytes.toString('utf8'));
    if (lines.length > 0) found.push({ path, lines });
  }
  return found;
}

export function readRatchet(root = ROOT) {
  return JSON.parse(readFileSync(`${root}${RATCHET}`, 'utf8')).files;
}

/** One message per file that links to the old organization more often than the ratchet allows. */
export function violations(found, allowed) {
  return found
    .filter(({ path, lines }) => lines.length > (allowed[path] ?? 0))
    .map(
      ({ path, lines }) =>
        `${path} links to the old organization pj-tmt on line ${[...new Set(lines)].join(', ')}: ` +
        `use tmt-ninja (${lines.length} found, ${allowed[path] ?? 0} allowed by ${RATCHET})`
    );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  if (process.argv[2] !== '--write') {
    console.error('usage: node scripts/previous-repository-links.mjs --write');
    process.exit(2);
  }
  const found = scan();
  const problems = violations(found, readRatchet());
  if (problems.length > 0) {
    console.error(problems.join('\n'));
    process.exit(1);
  }
  const files = Object.fromEntries(
    found.map(({ path, lines }) => [path, lines.length]).sort(([a], [b]) => (a < b ? -1 : 1))
  );
  const comment =
    'Tracked files that may still link to the old organization pj-tmt, with how many links each may keep. Counts only go down; see scripts/previous-repository-links.mjs.';
  writeFileSync(`${ROOT}${RATCHET}`, `${JSON.stringify({ $comment: comment, files }, null, 2)}\n`);
}
