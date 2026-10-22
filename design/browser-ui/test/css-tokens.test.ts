import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';

const root = new URL('../', import.meta.url);
const read = (path: string) => readFileSync(new URL(path, root), 'utf8');

/**
 * Selectors that give a rounded box (radius above 0) a lone edge: a side-specific border (also an
 * override of a full `border`, which leaves unequal sides) or an inset box-shadow stripe. Rules
 * with the same selector are merged in order, so a later `.x { border-left: ... }` still meets the
 * radius an earlier `.x { ... }` set.
 */
function oneSidedRounded(css: string): string[] {
  const side =
    /^border-(top|right|bottom|left|block-start|block-end|inline-start|inline-end)(-width|-style|-color)?$/;
  const plain = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const merged = new Map<string, Map<string, string>>();
  for (const match of plain.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    for (const selector of match[1]!.split(',').map((part) => part.trim().replace(/\s+/g, ' '))) {
      const declarations = merged.get(selector) ?? new Map<string, string>();
      for (const declaration of match[2]!.split(';')) {
        const at = declaration.indexOf(':');
        if (at < 0) continue;
        declarations.set(declaration.slice(0, at).trim(), declaration.slice(at + 1).trim());
      }
      merged.set(selector, declarations);
    }
  }
  return [...merged].flatMap(([selector, declarations]) => {
    const radius = declarations.get('border-radius');
    const rounded = radius !== undefined && !/^0(px)?$/.test(radius);
    const lone = [...declarations].some(
      ([name, value]) =>
        (side.test(name) && !/^(0|none)\b/.test(value)) ||
        (name === 'box-shadow' && /\binset\b/.test(value)),
    );
    return rounded && lone ? [selector] : [];
  });
}

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

  it('never gives a rounded box a one-sided border (gui-style: no hook at the corner)', () => {
    const files = [
      ...readdirSync(new URL('src/', root)).map((name) => `src/${name}`),
      ...readdirSync(new URL('gallery/', root)).map((name) => `gallery/${name}`),
    ];
    const offenders = files
      .filter((name) => name.endsWith('.css'))
      .flatMap((file) => oneSidedRounded(read(file)).map((selector) => `${file}: ${selector}`));
    expect(offenders).toEqual([]);
    // Inline styles in components and the gallery are rules too: no single-side border there.
    const inline = files
      .filter((name) => name.endsWith('.tsx'))
      .filter((file) => /\bborder(Top|Right|Bottom|Left|Inline\w*|Block\w*)\s*:/.test(read(file)));
    expect(inline).toEqual([]);
  });

  it('finds a lone edge on a rounded box, across rules of one selector, and nothing else', () => {
    expect(oneSidedRounded('.a { border-radius: 8px; border-bottom: 1px solid red; }')).toEqual([
      '.a',
    ]);
    expect(oneSidedRounded('.a { border-top-color: red; border-radius: var(--r); }')).toEqual([
      '.a',
    ]);
    // The gallery swatch: a full border and a radius, then a later override of one side.
    expect(
      oneSidedRounded(
        '.a, .b { border: 1px solid red; border-radius: 8px; } .b { border-left: 3px solid red; }',
      ),
    ).toEqual(['.b']);
    expect(oneSidedRounded('.a { border-radius: 8px; box-shadow: inset 3px 0 0 red; }')).toEqual([
      '.a',
    ]);
    // Square, all four sides, a fill and a zeroed side are all allowed.
    expect(oneSidedRounded('.a { border-radius: 0; border-bottom: 1px solid red; }')).toEqual([]);
    expect(oneSidedRounded('.a { border-radius: 8px; border: 1px solid red; }')).toEqual([]);
    expect(oneSidedRounded('.a { border-radius: 8px; border-left: 0; background: red; }')).toEqual(
      [],
    );
  });

  it('draws the menu description as the README says: 12px in the secondary colour', () => {
    const rule = read('src/menu.css').match(/\.tmt-ui-menu-description \{([^}]*)\}/)?.[1] ?? '';
    expect(rule).toContain('color: var(--tmt-ui-color-secondary);');
    expect(rule).toContain('font-size: 12px;');
  });
});
