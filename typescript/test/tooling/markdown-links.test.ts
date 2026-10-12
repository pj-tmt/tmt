import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

// Check the repository's link forms, not full CommonMark or remote URLs/anchors.
function destinations(markdown: string): string[] {
  let fence: string | undefined;
  const prose = markdown
    .replace(/<!--[\s\S]*?-->/g, '')
    .split('\n')
    .filter((line) => {
      const marker = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
      if (marker) {
        if (!fence) fence = marker[1];
        else if (marker[1][0] === fence[0] && marker[1].length >= fence.length && !marker[2].trim())
          fence = undefined;
        return false;
      }
      return !fence;
    })
    .join('\n')
    .replace(/(?<!`)(`+)(?!`)[\s\S]*?(?<!`)\1(?!`)/g, '');
  const inline =
    /(?<!\\)!?\[(?:\\.|[^\]\\])*\]\(\s*(?:<((?:\\.|[^>])*)>|((?:\\.|[^\s()\\]|\((?:\\.|[^\s()\\])*\))+))(?:\s+(?:"[^"]*"|'[^']*'|\([^)]*\)))?\s*\)/g;
  const reference = /^ {0,3}\[[^\]\n]+\]:[ \t]*(?:<([^>\n]+)>|([^\s]+))/gm;
  return [...prose.matchAll(inline), ...prose.matchAll(reference)].map(
    (match) => match[1] ?? match[2]
  );
}

function brokenLinks(root: string, source: string, markdown: string): string[] {
  return destinations(markdown).flatMap((destination) => {
    if (/^(?:[a-z][a-z0-9+.-]*:|\/|#)/i.test(destination)) return [];
    const file = destination.split(/[?#]/, 1)[0];
    if (!file) return [];
    let target: string;
    try {
      target = path.resolve(
        path.dirname(path.join(root, source)),
        decodeURIComponent(file.replace(/\\([\\()[\]<> ])/g, '$1'))
      );
    } catch {
      return [`${source}: ${destination} has an invalid URL escape`];
    }
    const relative = path.relative(root, target);
    return relative === '..' ||
      relative.startsWith(`..${path.sep}`) ||
      path.isAbsolute(relative) ||
      !fs.existsSync(target)
      ? [`${source}: ${destination} -> ${target} (outside repository or missing target)`]
      : [];
  });
}

const ROOT = fileURLToPath(new URL('../../../', import.meta.url));
describe('tracked Markdown relative links', () => {
  it('resolves tracked Markdown outside site without fetching external links', () => {
    const files = execFileSync('git', ['ls-files', '-z'], { cwd: ROOT, encoding: 'utf8' })
      .split('\0')
      .filter((file) => file.endsWith('.md') && !file.startsWith('site/'));
    expect(files.length).toBeGreaterThan(0);
    const failures = files.flatMap((file) =>
      brokenLinks(ROOT, file, fs.readFileSync(path.join(ROOT, file), 'utf8'))
    );
    expect(failures, failures.join('\n')).toEqual([]);
  });

  it('reads inline, image and reference destinations with angle brackets, escapes and titles', () => {
    expect(
      destinations(String.raw`
[plain](../guide.md#section "title")
![image](image.png 'caption')
[angle](<a file.md> (title))
[parentheses](a(b).md)
[escaped](a\(b\).md)
[reference][guide]
[guide]: ../guide.md "reference title"
[space]: <a file.md>
`)
    ).toEqual([
      '../guide.md#section',
      'image.png',
      'a file.md',
      'a(b).md',
      'a\\(b\\).md',
      '../guide.md',
      'a file.md',
    ]);
  });

  it('ignores backtick/tilde fences, inline code and HTML comments', () => {
    const markdown = [
      '```md',
      '[hidden](missing.md)',
      '````',
      '~~~~',
      '[hidden](missing.md)',
      '~~~',
      '[still hidden](missing.md)',
      '~~~~',
      '`[hidden](missing.md)` and ``[hidden](missing.md)``',
      '<!-- [hidden](missing.md)',
      '[also hidden](missing.md) -->',
      '[visible](ok.md)',
      String.raw`\[escaped](missing.md)`,
    ].join('\n');
    expect(destinations(markdown)).toEqual(['ok.md']);
  });

  it('checks actual local targets, strips fragments/queries, decodes escapes and refuses escapes or absence', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-markdown-links-'));
    try {
      fs.mkdirSync(path.join(root, 'docs'));
      fs.writeFileSync(path.join(root, 'a file.md'), 'target\n');
      fs.writeFileSync(path.join(root, 'a(b).md'), 'target\n');
      expect(
        brokenLinks(
          root,
          'docs/source.md',
          '[space](../a%20file.md?view=1#section) [escaped](../a\\(b\\).md) [directory](../docs) [web](https://example.invalid/no.md) [mail](mailto:a@example.invalid) [root](/relative) [fragment](#missing-anchor)'
        )
      ).toEqual([]);
      const missing = brokenLinks(root, 'docs/source.md', '[missing](absent.md)');
      expect(missing).toEqual([
        `docs/source.md: absent.md -> ${path.join(root, 'docs/absent.md')} (outside repository or missing target)`,
      ]);
      expect(brokenLinks(root, 'docs/source.md', '[escape](../../outside.md)')[0]).toContain(
        'outside repository'
      );
      expect(brokenLinks(root, 'docs/source.md', '[invalid](bad%XX.md)')).toEqual([
        'docs/source.md: bad%XX.md has an invalid URL escape',
      ]);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
});
