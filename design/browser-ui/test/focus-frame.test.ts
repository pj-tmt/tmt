import { expect, it } from 'vite-plus/test';
import { readFileSync, readdirSync } from 'node:fs';

/**
 * One focus frame (#3998): a control with its own border shows focus by recolouring that border
 * and draws no outline; a borderless control draws one inset outline. No focus outline ever sits
 * outside the box with a gap.
 */
function rules(css: string): { selector: string; body: string }[] {
  const plain = css.replace(/\/\*[\s\S]*?\*\//g, '');
  return [...plain.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((match) => ({
    selector: match[1]!.trim(),
    body: match[2]!,
  }));
}
const value = (body: string, property: string) =>
  new RegExp(`(?:^|;|\\s)${property}\\s*:\\s*([^;]+)`).exec(body)?.[1]?.trim();

/** `0`, a negative length, `calc(-1 * w)` or `calc(w * -1)`. */
const inset = (offset: string) =>
  /^0\b|^-/.test(offset) || /^calc\(\s*-/.test(offset) || /\*\s*-1\s*\)$/.test(offset);

export function focusProblems(css: string): string[] {
  const all = rules(css);
  const bordered = (base: string) =>
    all.some(({ selector, body }) => {
      if (!selector.split(',').some((part) => part.trim() === base)) return false;
      const border = value(body, 'border') ?? value(body, 'border-width');
      return border !== undefined && !/^(0|none)\b/.test(border);
    });
  const problems: string[] = [];
  for (const { selector, body } of all) {
    if (!selector.includes(':focus-visible')) continue;
    const offset = value(body, 'outline-offset');
    if (offset && !inset(offset))
      problems.push(`${selector}: outline-offset ${offset} sits outside the box`);
    const outline = value(body, 'outline');
    if (!outline || /^(0|none)\b/.test(outline)) continue;
    for (const part of selector.split(',')) {
      const base = part
        .replace(/:focus-visible/g, '')
        .replace(/:not\([^)]*\)/g, '')
        .trim();
      if (base && bordered(base))
        problems.push(`${part.trim()}: an outline on a bordered control (recolour its border)`);
    }
  }
  return problems;
}

const sheets = (directory: string) => {
  const url = new URL(directory, import.meta.url);
  return readdirSync(url)
    .filter((name) => name.endsWith('.css'))
    .map((name) => [`${directory}${name}`, readFileSync(new URL(name, url), 'utf8')] as const);
};

it('flags a positive offset and an outline on a bordered control', () => {
  expect(
    focusProblems(`.a { border: 1px solid red; } .a:focus-visible { outline: 2px solid; }
      .b { border: 0; } .b:focus-visible { outline: 2px solid; outline-offset: 2px; }
      .c { border: 0; } .c:focus-visible { outline: 2px solid; outline-offset: calc(-1 * 2px); }`),
  ).toEqual([
    '.a:focus-visible: an outline on a bordered control (recolour its border)',
    '.b:focus-visible: outline-offset 2px sits outside the box',
  ]);
});

it('draws one focus frame in the design system, the gallery and the Colab and Remote pages', () => {
  for (const [name, css] of [
    ...sheets('../src/'),
    ...sheets('../gallery/'),
    ...sheets('../../../extensions/tmt-colab/typescript/app/src/'),
    ...sheets('../../../extensions/tmt-remote/rust/tmt-remote/assets/'),
  ])
    expect(focusProblems(css), name).toEqual([]);
});

it('gives each React control one focus frame: its border ring, or one inset outline', () => {
  const directory = new URL('../src/ui/', import.meta.url);
  for (const name of readdirSync(directory).filter((file) => file.endsWith('.tsx'))) {
    const source = readFileSync(new URL(name, directory), 'utf8');
    const outline = /focus-visible:outline-solid/.test(source);
    expect(outline && /focus-visible:border-ring/.test(source), name).toBe(false);
    if (outline) expect(source, name).toMatch(/focus-visible:-outline-offset-/);
  }
});
