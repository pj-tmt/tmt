import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { inScope, knownScope, readScope, scopeRows, writeScope } from '../src/project-scope.js';
import type { PageSummary } from '../src/transport.js';

const page = (id: string, projects: string[], waiting = false): PageSummary => ({
  id,
  title: id,
  sharing: 'private',
  projects,
  summary: { comments: 0, waiting, needsYou: waiting ? 1 : 0 },
});
function memoryStorage() {
  const items = new Map<string, string>();
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
    removeItem: (key: string) => void items.delete(key),
  };
}
afterEach(() => vi.unstubAllGlobals());

describe('project scope', () => {
  it('lists All pages first, then every project with its pages and the ones that wait', () => {
    const pages = [page('a', ['x', 'y'], true), page('b', ['x']), page('c', [], true)];
    expect(scopeRows(pages, ['empty', 'x', 'y'])).toEqual([
      { name: undefined, count: 3, waiting: 2 },
      { name: 'empty', count: 0, waiting: 0 },
      { name: 'x', count: 2, waiting: 1 },
      { name: 'y', count: 1, waiting: 1 },
    ]);
  });
  it('keeps the pages in the scope, or every page without one', () => {
    const pages = [page('a', ['x']), page('b', ['y']), page('c', [])];
    expect(inScope(pages, undefined)).toEqual(pages);
    expect(inScope(pages, 'x').map((p) => p.id)).toEqual(['a']);
    expect(inScope(pages, 'gone')).toEqual([]);
  });
  it('knows a scope only when the space still has that project', () => {
    expect(knownScope('x', ['x', 'y'])).toBe('x');
    expect(knownScope('gone', ['x'])).toBeUndefined();
    expect(knownScope(undefined, ['x'])).toBeUndefined();
    expect(knownScope('x', undefined)).toBeUndefined();
  });
  it('remembers the scope per space on this device and forgets it for All pages', () => {
    vi.stubGlobal('localStorage', memoryStorage());
    expect(readScope('s1')).toBeUndefined();
    writeScope('s1', 'design');
    writeScope('s2', 'ops');
    expect(readScope('s1')).toBe('design');
    expect(readScope('s2')).toBe('ops');
    writeScope('s1', undefined);
    expect(readScope('s1')).toBeUndefined();
    expect(readScope('s2')).toBe('ops');
  });
  it('ignores a stored name the grammar refuses, and a device without storage', () => {
    const storage = memoryStorage();
    vi.stubGlobal('localStorage', storage);
    storage.setItem('tmt-colab-project-scope-v1:s1', ' bad');
    expect(readScope('s1')).toBeUndefined();
    const refusing = {
      getItem: () => {
        throw new Error('denied');
      },
      setItem: () => {
        throw new Error('denied');
      },
      removeItem: () => {
        throw new Error('denied');
      },
    };
    vi.stubGlobal('localStorage', refusing);
    expect(readScope('s1')).toBeUndefined();
    expect(() => writeScope('s1', 'design')).not.toThrow();
    expect(() => writeScope('s1', undefined)).not.toThrow();
  });
});
