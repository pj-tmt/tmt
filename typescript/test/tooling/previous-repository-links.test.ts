import { describe, expect, it } from 'vite-plus/test';
import {
  linkLines,
  readRatchet,
  scan,
  violations,
} from '../../scripts/previous-repository-links.mjs';

// The organization was renamed to tmt-ninja (#5137), and #5242 still added three links to the old
// name in a shipped skill with every check green. A tracked file may keep only the links the
// ratchet lists for it; lower it with `node scripts/previous-repository-links.mjs --write`.
const OLD = 'pj-tmt';

describe('links to the old organization name', () => {
  it('recognizes URL-shaped uses only', () => {
    const text = [
      `see https://github.com/${OLD}/tmt/blob/main/AGENTS.md`,
      `curl https://raw.githubusercontent.com/${OLD}/tmt/release-index/x.json`,
      `gh api repos/${OLD}/tmt/actions/runs`,
      `site https://${OLD}.github.io/tmt/`,
      `image ghcr.io/${OLD}/tmt:latest`,
      `two on one line github.com/${OLD}/a github.com/${OLD}/b`,
      `a bare repository constant: ${OLD}/tmt`,
      `prose: never create anything named ${OLD}`,
      `another organization: https://github.com/${OLD}-labs/tmt`,
      `the new one: https://github.com/tmt-ninja/tmt`,
    ].join('\n');
    expect(linkLines(text)).toEqual([1, 2, 3, 4, 5, 6, 6]);
  });

  it('lists only counted, sorted files in the ratchet', () => {
    const files = Object.entries(readRatchet());
    expect(files.length).toBeGreaterThan(0);
    expect(files.map(([path]) => path)).toEqual(files.map(([path]) => path).sort());
    expect(files.every(([, count]) => Number.isSafeInteger(count) && count > 0)).toBe(true);
  });

  it('names the file, the line and the replacement when a link is added or the count rises', () => {
    const allowed = { 'skills/tmt/SKILL.md': 2 };
    expect(violations([{ path: 'skills/tmt/SKILL.md', lines: [4, 9] }], allowed)).toEqual([]);
    expect(violations([{ path: 'skills/tmt/SKILL.md', lines: [4] }], allowed)).toEqual([]);
    const added = violations(
      [
        { path: 'skills/tmt/SKILL.md', lines: [4, 9, 12] },
        { path: 'skills/new/SKILL.md', lines: [3] },
      ],
      allowed
    );
    expect(added).toHaveLength(2);
    expect(added[0]).toContain('skills/tmt/SKILL.md');
    expect(added[0]).toContain('4, 9, 12');
    expect(added[0]).toContain('3 found, 2 allowed');
    expect(added[1]).toContain('skills/new/SKILL.md');
    expect(added[1]).toContain('use tmt-ninja');
    expect(added[1]).toContain('1 found, 0 allowed');
  });

  it('finds no tracked file with more links to the old name than the ratchet allows', () => {
    const found = violations(scan(), readRatchet());
    expect(found, found.join('\n')).toEqual([]);
  });
});
