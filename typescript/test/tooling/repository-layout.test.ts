import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { ownerOf, parseComponentMap } from '../../scripts/ci-scope.mjs';
const { runPackedCommand } = await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
);

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const components = parseComponentMap(
  readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
);
const layout = JSON.parse(
  readFileSync(new URL('../../../.github/repository-layout.json', import.meta.url), 'utf8')
) as {
  permanent: string[];
  languageExceptions?: Record<string, string>;
  temporary: Record<string, { removeIn: string[] }>;
};
const I18N_ROOT = 'site/src/i18n/';
const entries = [...layout.permanent, ...Object.keys(layout.temporary)];
const allowed = new Set(entries);

function layoutViolations(
  files: readonly string[],
  map = components,
  topLevel = allowed
): string[] {
  const unowned = files
    .filter((file) => ownerOf(file, map) === 'unowned')
    .map((file) => `${file}: assign a component owner in .github/components.json`);
  const unexpected = [...new Set(files.map((file) => file.split('/')[0]))]
    .filter((entry) => !topLevel.has(entry))
    .map((entry) => `${entry}: use an existing home or propose a layout allowlist entry to infra`);
  return [...unowned, ...unexpected];
}

const LANGUAGE_TAGS: Record<string, string> = {
  'site/src/i18n/ja': 'ja',
  'site/src/i18n/zh': 'zh-Hant',
};

function languageViolations(
  files: readonly string[],
  listed: Readonly<Record<string, string>>
): string[] {
  return files
    .filter((file) => file.startsWith(I18N_ROOT))
    .filter((file) => !Object.keys(listed).some((directory) => file.startsWith(`${directory}/`)))
    .map((file) => `${file}: add its directory to languageExceptions or move the file`);
}

function languageEntryProblems(
  listed: Readonly<Record<string, string>>,
  files: readonly string[]
): string[] {
  return Object.entries(listed).flatMap(([directory, tag]) => {
    if (!(directory in LANGUAGE_TAGS)) {
      return [`${directory}: not an allowed language (ja, zh)`];
    }
    return [
      ...(tag === LANGUAGE_TAGS[directory]
        ? []
        : [`${directory}: expected the language tag ${LANGUAGE_TAGS[directory]}`]),
      ...(files.some((file) => file.startsWith(`${directory}/`))
        ? []
        : [`${directory}: track a translated page in it or remove the entry`]),
    ];
  });
}

function staleExceptions(files: readonly string[], exceptions: readonly string[]): string[] {
  const roots = new Set(files.map((file) => file.split('/')[0]));
  return exceptions
    .filter((entry) => !roots.has(entry))
    .map((entry) => `${entry}: remove the stale temporary layout exception`);
}

describe('repository layout', () => {
  it('gives every tracked file a component owner and an allowed top-level home', () => {
    const files = runPackedCommand('git', ['ls-files', '-z'], {
      cwd: repository,
      env: process.env,
    })
      .split('\0')
      .filter(Boolean);
    expect(files.length).toBeGreaterThan(0);
    expect(layoutViolations(files)).toEqual([]);
    expect(staleExceptions(files, Object.keys(layout.temporary))).toEqual([]);
    const listed = layout.languageExceptions ?? {};
    expect(languageViolations(files, listed)).toEqual([]);
    expect(languageEntryProblems(listed, files)).toEqual([]);
  });

  it('allows non-English handbook prose only under a listed language directory', () => {
    const listed = { 'site/src/i18n/ja': 'ja', 'site/src/i18n/zh': 'zh-Hant' };
    const directories = Object.keys(listed);
    expect(
      languageViolations(['site/src/i18n/ja/start.mdx', 'site/src/i18n/zh/a.mdx'], listed)
    ).toEqual([]);
    expect(
      languageViolations(
        ['site/src/i18n/fr/start.mdx', 'site/src/i18n/start.mdx', 'site/src/i18n/jaish/a.mdx'],
        listed
      )
    ).toEqual([
      'site/src/i18n/fr/start.mdx: add its directory to languageExceptions or move the file',
      'site/src/i18n/start.mdx: add its directory to languageExceptions or move the file',
      'site/src/i18n/jaish/a.mdx: add its directory to languageExceptions or move the file',
    ]);
    expect(languageViolations(['site/src/i18n/ja/start.mdx'], {})).toEqual([
      'site/src/i18n/ja/start.mdx: add its directory to languageExceptions or move the file',
    ]);
    expect(languageViolations(['site/src/main.tsx', 'rust/cli.rs'], {})).toEqual([]);
    expect(directories).toHaveLength(2);
  });

  it('keeps the language set closed and requires tracked content in each listed directory', () => {
    const listed = { 'site/src/i18n/ja': 'ja', 'site/src/i18n/zh': 'zh-Hant' };
    expect(
      languageEntryProblems(listed, ['site/src/i18n/ja/a.mdx', 'site/src/i18n/zh/a.mdx'])
    ).toEqual([]);
    expect(
      languageEntryProblems({ 'site/src/i18n/ja': 'ja', 'site/src/i18n/zh': 'zh' }, [
        'site/src/i18n/ja/a.mdx',
      ])
    ).toEqual([
      'site/src/i18n/zh: expected the language tag zh-Hant',
      'site/src/i18n/zh: track a translated page in it or remove the entry',
    ]);
    expect(languageEntryProblems({ 'site/src/i18n/fr': 'fr' }, ['site/src/i18n/fr/a.mdx'])).toEqual(
      ['site/src/i18n/fr: not an allowed language (ja, zh)']
    );
  });

  it('reports an unowned file even when its top-level home is allowed', () => {
    const map = parseComponentMap(JSON.stringify({ components: { cli: { owns: ['rust'] } } }));
    expect(layoutViolations(['retired/cli.rs'], map, new Set(['retired']))).toEqual([
      'retired/cli.rs: assign a component owner in .github/components.json',
    ]);
  });

  it('reports an unapproved home even when the component map owns it', () => {
    expect(layoutViolations(['unexpected/entry.ts'])).toEqual([
      'unexpected: use an existing home or propose a layout allowlist entry to infra',
    ]);
  });

  it('reports a temporary exception without a tracked top-level entry', () => {
    expect(staleExceptions(['rust/cli.rs'], ['rust', 'retired'])).toEqual([
      'retired: remove the stale temporary layout exception',
    ]);
  });

  it('keeps permanent and temporary entries distinct and names each exception removal', () => {
    expect(new Set(entries).size).toBe(entries.length);
    for (const [entry, exception] of Object.entries(layout.temporary)) {
      expect(exception.removeIn.length, `${entry}: name the removal issue or PR`).toBeGreaterThan(
        0
      );
      for (const reference of exception.removeIn) expect(reference).toMatch(/^#[1-9]\d*$/);
    }
  });
});
