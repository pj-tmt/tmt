import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';

const root = new URL('../', import.meta.url);
const read = (path: string) => readFileSync(new URL(path, root), 'utf8');

describe('browser-ui CSS tokens', () => {
  it('defines every shared token its source CSS reads; only host tokens come from the host', () => {
    const defined = new Set(
      [...read('generated/static.css').matchAll(/(--tmt-ui-[a-z0-9-]+)\s*:/g)].map(
        (match) => match[1],
      ),
    );
    const undefinedTokens = new Map<string, string[]>();
    for (const file of readdirSync(new URL('src/', root)).filter((name) => name.endsWith('.css'))) {
      for (const [, token] of read(`src/${file}`).matchAll(/var\((--tmt-ui-[a-z0-9-]+)/g)) {
        // `--tmt-ui-host-*` is the host's contract (the README names them); the rest is shared.
        if (token.startsWith('--tmt-ui-host-') || defined.has(token)) continue;
        undefinedTokens.set(token, [...(undefinedTokens.get(token) ?? []), file]);
      }
    }
    // An undefined token makes its declaration invalid at computed-value time, so the text or
    // surface silently inherits instead of taking the intended role.
    expect(Object.fromEntries(undefinedTokens)).toEqual({});
  });

  it('draws the menu description as the README says: 12px in the secondary colour', () => {
    const rule = read('src/menu.css').match(/\.tmt-ui-menu-description \{([^}]*)\}/)?.[1] ?? '';
    expect(rule).toContain('color: var(--tmt-ui-color-secondary);');
    expect(rule).toContain('font-size: 12px;');
  });
});
