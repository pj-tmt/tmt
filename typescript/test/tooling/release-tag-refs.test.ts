import { describe, expect, it } from 'vite-plus/test';
import { reconcilePublishedTags } from '../../scripts/release-tag-refs.mjs';

const commit = `${'a'.repeat(40)} commit`;
const missing = (tag: string) => `refs/tags/${tag}^{commit} missing`;
const fetchArgs = [
  'fetch',
  '--no-tags',
  '--no-recurse-submodules',
  '--no-write-fetch-head',
  '--quiet',
  '--stdin',
  'origin',
];

function fixture(batches: string[][], failFetch = false) {
  const calls: { args: string[]; input?: string }[] = [];
  const git = (args: string[], input?: string) => {
    calls.push({ args, input });
    if (args[0] === 'fetch') {
      if (failFetch) throw new Error('origin unavailable');
      return '';
    }
    const rows = batches.shift();
    if (!rows) throw new Error('Unexpected extra tag read');
    return rows.join('\n') + '\n';
  };
  return { calls, git };
}

describe('captured published tag reconciliation', () => {
  it('does no IO for an empty set and no fetch for already resolved tags', () => {
    const f = fixture([[commit]]);
    reconcilePublishedTags([], f.git);
    expect(f.calls).toEqual([]);
    reconcilePublishedTags(['v1', 'v1'], f.git);
    expect(f.calls).toEqual([
      {
        args: ['cat-file', '--batch-check=%(objectname) %(objecttype)'],
        input: 'refs/tags/v1^{commit}\n',
      },
    ]);
  });

  it('fetches only missing exact refs once without force, then revalidates the full set', () => {
    const f = fixture([
      [commit, missing('v2')],
      [commit, commit],
    ]);
    reconcilePublishedTags(['v1', 'v2'], f.git);
    expect(f.calls.map((c) => c.args[0])).toEqual(['cat-file', 'fetch', 'cat-file']);
    expect(f.calls[1]).toEqual({ args: fetchArgs, input: 'refs/tags/v2:refs/tags/v2\n' });
    expect(f.calls[0].input).toBe(f.calls[2].input);
  });

  it('keeps argv and git call count bounded independently of the tag count', () => {
    const tags = Array.from({ length: 5000 }, (_, i) => `v${i}`);
    const f = fixture([tags.map(missing), tags.map(() => commit)]);
    reconcilePublishedTags(tags, f.git);
    expect(f.calls).toHaveLength(3);
    expect(f.calls[1].args).toEqual(fetchArgs);
    expect(f.calls[1].input?.split('\n').filter(Boolean)).toHaveLength(tags.length);
  });

  it.each(['-force', 'v1:refs/heads/main', 'refs/../x', 'v1\nv2', '', 'v1.lock', 'v1.'])(
    'refuses unsafe ref input before git IO: %j',
    (tag) => {
      const f = fixture([]);
      expect(() => reconcilePublishedTags([tag], f.git)).toThrow('not fetched');
      expect(f.calls).toEqual([]);
    }
  );

  it('fails closed with the captured tag when origin is unavailable, without retrying', () => {
    const f = fixture([[missing('v2')]], true);
    expect(() => reconcilePublishedTags(['v2'], f.git)).toThrow(
      'published tag(s) do not resolve to a commit: v2 (origin unavailable)'
    );
    expect(f.calls.map((c) => c.args[0])).toEqual(['cat-file', 'fetch']);
  });

  it('rejects an unresolved or non-commit tag after the single fetch', () => {
    const f = fixture([[missing('v2')], [missing('v2')]]);
    expect(() => reconcilePublishedTags(['v2'], f.git)).toThrow(
      'published tag(s) do not resolve to a commit: v2'
    );
    expect(f.calls.filter((c) => c.args[0] === 'fetch')).toHaveLength(1);
  });

  it('refuses a changed previously resolved commit instead of accepting a new snapshot', () => {
    const f = fixture([
      [commit, missing('v2')],
      [`${'b'.repeat(40)} commit`, commit],
    ]);
    expect(() => reconcilePublishedTags(['v1', 'v2'], f.git)).toThrow(
      'existing published tag commit changed: v1'
    );
  });

  it('refuses an incomplete batch response before attempting a fetch', () => {
    const f = fixture([[commit]]);
    expect(() => reconcilePublishedTags(['v1', 'v2'], f.git)).toThrow(
      'unexpected tag check output'
    );
    expect(f.calls).toHaveLength(1);
  });
});
