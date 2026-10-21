import { expect, it } from 'vite-plus/test';
import { readFileSync, readdirSync } from 'node:fs';
import { covered } from '../gallery/covered';

const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');

/** Value exports of the React entry; `export type` lines and inline `type` names are skipped. */
function reactValueExports(): string[] {
  const source = read('../src/react.ts');
  const names: string[] = [];
  for (const match of source.matchAll(/^export\s+(?!type\b)\{([^}]*)\}/gm))
    for (const part of match[1]!.split(',')) {
      const name = part.trim();
      if (!name || name.startsWith('type ')) continue;
      names.push(name.split(/\s+as\s+/).at(-1)!);
    }
  for (const match of source.matchAll(/^export\s+(?:const|function|class)\s+(\w+)/gm))
    names.push(match[1]!);
  return names.sort();
}

it('keeps the dev gallery registry equal to the React entry value exports', () => {
  const exported = reactValueExports();
  expect(exported.length).toBeGreaterThan(20);
  expect(Object.keys(covered).sort()).toEqual(exported);
});

it('renders or calls every React entry value export in the gallery source', () => {
  const directory = new URL('../gallery/', import.meta.url);
  const source = readdirSync(directory)
    .filter((name) => name.endsWith('.tsx'))
    .map((name) => readFileSync(new URL(name, directory), 'utf8'))
    .join('\n')
    // Imports name an export without using it.
    .replace(/^import[\s\S]*?from\s+'[^']+';$/gm, '');
  const missing = Object.keys(covered).filter((name) =>
    /^[A-Z]/.test(name)
      ? !new RegExp(`<${name}[\\s/>]`).test(source)
      : !new RegExp(`\\b${name}\\b`).test(source),
  );
  expect(missing).toEqual([]);
});
