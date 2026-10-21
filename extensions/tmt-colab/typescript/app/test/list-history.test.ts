import { describe, expect, it } from 'vite-plus/test';
import type { RouterHistory } from '@tanstack/react-router';
import { isListPath, keepListSearch, storedListSearch } from '../src/list-history.js';

type Call = { kind: 'push' | 'replace'; path: string; state: unknown; options: unknown };
function recorder() {
  const calls: Call[] = [];
  const history = {
    push: (path: string, state: unknown, options: unknown) =>
      calls.push({ kind: 'push', path, state, options }),
    replace: (path: string, state: unknown, options: unknown) =>
      calls.push({ kind: 'replace', path, state, options }),
  } as unknown as RouterHistory;
  return { calls, history: keepListSearch(history) };
}

describe('isListPath', () => {
  it('is the bare list and the list with a search, nothing else', () => {
    expect(isListPath('/')).toBe(true);
    expect(isListPath('/?stage=all')).toBe(true);
    expect(isListPath('/pages/10000000-0000-4000-8000-000000000001')).toBe(false);
    expect(isListPath('/all')).toBe(false);
    expect(isListPath('/all?stage=all')).toBe(false);
  });
});

describe('keepListSearch', () => {
  it('stores the list search in the entry state, on push and on replace', () => {
    const { calls, history } = recorder();
    const options = { ignoreBlocker: true };
    history.push('/?stage=all&q=notes', { key: 'a' }, options);
    history.replace('/?archived=true', undefined);
    expect(calls).toEqual([
      {
        kind: 'push',
        path: '/?stage=all&q=notes',
        state: { key: 'a', tmtColabList: '?stage=all&q=notes' },
        options,
      },
      {
        kind: 'replace',
        path: '/?archived=true',
        state: { tmtColabList: '?archived=true' },
        options: undefined,
      },
    ]);
  });

  it('leaves the bare list and every other route as it is', () => {
    const { calls, history } = recorder();
    const state = { key: 'a' };
    history.push('/', state);
    history.push('/?', state);
    history.replace('/pages/10000000-0000-4000-8000-000000000001', state);
    expect(calls.map((call) => call.state)).toEqual([state, state, state]);
  });
});

describe('storedListSearch', () => {
  it('reads the search a list entry kept', () => {
    expect(storedListSearch({ tmtColabList: '?stage=done' })).toBe('?stage=done');
  });

  it('is no search when the entry kept none or something that is not plain bounded text', () => {
    expect(storedListSearch(null)).toBe('');
    expect(storedListSearch(undefined)).toBe('');
    expect(storedListSearch({})).toBe('');
    expect(storedListSearch({ tmtColabList: 7 })).toBe('');
    expect(storedListSearch({ tmtColabList: '' })).toBe('');
    expect(storedListSearch({ tmtColabList: '?' })).toBe('');
    expect(storedListSearch({ tmtColabList: 'stage=all' })).toBe('');
    expect(storedListSearch({ tmtColabList: '?stage=all#frag' })).toBe('');
    expect(storedListSearch({ tmtColabList: `?q=${'x'.repeat(2048)}` })).toBe('');
  });
});
