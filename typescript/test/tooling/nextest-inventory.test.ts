import { describe, expect, it } from 'vitest';
import {
  cargoInventory,
  parseLibtestList,
  verifyNextestInventory,
} from '../../scripts/verify-nextest-inventory.mjs';

const cargo = [
  {
    id: ['pkg', 'lib', 'pkg'],
    tests: [
      { name: 'first', ignored: false },
      { name: 'second', ignored: false },
      { name: 'manual', ignored: true },
    ],
  },
  { id: ['pkg', 'test', 'empty'], tests: [] },
];
function listing(selected: string[] = ['first', 'second']) {
  return {
    'test-count': 3,
    'rust-suites': {
      pkg: {
        status: 'listed',
        'package-id': 'pkg',
        kind: 'lib',
        'binary-name': 'pkg',
        testcases: Object.fromEntries(
          ['first', 'second', 'manual'].map((name) => [
            name,
            {
              kind: 'test',
              ignored: name === 'manual',
              'filter-match': selected.includes(name)
                ? { status: 'matches' }
                : { status: 'mismatch', reason: name === 'manual' ? 'ignored' : 'partition' },
            },
          ])
        ),
      },
      empty: {
        status: 'listed',
        'package-id': 'pkg',
        kind: 'test',
        'binary-name': 'empty',
        testcases: {},
      },
    },
  };
}

describe('nextest inventory proof', () => {
  it('preserves names, ignored policy, empty harnesses and disjoint shard union', () => {
    expect(
      verifyNextestInventory(cargo, listing(), [listing(['first']), listing(['second'])])
    ).toMatchObject({ runnable: 2, ignored: 1, partitionCounts: [1, 1] });
  });
  it('collects Cargo executable harnesses and ignored names without running tests', () => {
    const calls: boolean[] = [];
    const artifact = {
      reason: 'compiler-artifact',
      profile: { test: true },
      executable: '/test',
      package_id: 'pkg',
      target: { kind: ['lib'], name: 'pkg' },
    };
    expect(
      cargoInventory([artifact], (_binary, ignored) => {
        calls.push(ignored);
        return ignored ? 'manual: test\n' : 'first: test\nmanual: test\n';
      })
    ).toEqual([
      {
        id: ['pkg', 'lib', 'pkg'],
        tests: [
          { name: 'first', ignored: false },
          { name: 'manual', ignored: true },
        ],
      },
    ]);
    expect(calls).toEqual([false, true]);
  });
  it.each(['extra', 'missing', 'ignored', 'empty-harness', 'summary', 'filtered'])(
    'rejects %s selection drift',
    (change) => {
      const full = listing();
      if (change === 'extra')
        full['rust-suites'].pkg.testcases.extra = {
          kind: 'test',
          ignored: false,
          'filter-match': { status: 'matches' },
        };
      if (change === 'missing') delete full['rust-suites'].pkg.testcases.first;
      if (change === 'ignored') full['rust-suites'].pkg.testcases.first.ignored = true;
      if (change === 'empty-harness')
        delete (full['rust-suites'] as Partial<(typeof full)['rust-suites']>).empty;
      if (change === 'summary') full['test-count'] = 0;
      if (change === 'filtered')
        full['rust-suites'].pkg.testcases.first['filter-match'] = {
          status: 'mismatch',
          reason: 'expression',
        };
      expect(() =>
        verifyNextestInventory(cargo, full, [listing(['first']), listing(['second'])])
      ).toThrow();
    }
  );
  it.each([
    [['first'], ['first']],
    [['first'], []],
    [['first', 'manual'], ['second']],
  ])('rejects invalid partition union %j', (first, second) => {
    expect(() =>
      verifyNextestInventory(cargo, listing(), [listing(first), listing(second)])
    ).toThrow();
  });
  it('rejects ambiguous Cargo output and duplicate names', () => {
    expect(() => parseLibtestList('not a test\n')).toThrow();
    expect(() => parseLibtestList('same: test\nsame: test\n')).toThrow();
    expect(() => cargoInventory([], () => '')).toThrow();
  });
});
