import { describe, expect, it } from 'vite-plus/test';
import {
  compareInventories,
  libtestCases,
  verifyExecution,
} from '../../scripts/workspace-nextest.mjs';

const cargo = () => [
  { executable: '/target/tests', tests: ['active', 'ignored'], ignored: ['ignored'] },
];
const nextest = () => ({
  'rust-suites': {
    suite: {
      'binary-path': '/target/tests',
      'binary-id': 'owner::suite',
      testcases: {
        active: { ignored: false, 'filter-match': { status: 'matches' } },
        ignored: { ignored: true, 'filter-match': { status: 'matches' } },
      },
    },
  },
});

describe('workspace nextest equivalence evidence', () => {
  it('reads libtest names, including panic decoration, and rejects duplicate identities', () => {
    expect(
      libtestCases('active: test\npanic - should panic: test\n2 tests, 0 benchmarks\n')
    ).toEqual(['active', 'panic']);
    expect(() => libtestCases('a: test\na: test\n')).toThrow('duplicate libtest case');
  });

  it('matches the independent binary, case and ignored inventories, including empty binaries', () => {
    const cases = compareInventories(cargo(), nextest());
    expect(cases).toEqual([
      { binary: 'owner::suite', name: 'active', ignored: false },
      { binary: 'owner::suite', name: 'ignored', ignored: true },
    ]);
    const empty = nextest();
    empty['rust-suites'].suite.testcases =
      {} as (typeof empty)['rust-suites']['suite']['testcases'];
    expect(
      compareInventories([{ executable: '/target/tests', tests: [], ignored: [] }], empty)
    ).toEqual([]);
  });

  it.each(['binary', 'case', 'ignored', 'filter', 'empty'] as const)(
    'refuses a %s mismatch',
    (kind) => {
      const input = nextest();
      if (kind === 'binary') input['rust-suites'].suite['binary-path'] = '/other';
      if (kind === 'case')
        delete (
          input['rust-suites'].suite.testcases as Partial<
            (typeof input)['rust-suites']['suite']['testcases']
          >
        ).active;
      if (kind === 'ignored') input['rust-suites'].suite.testcases.active.ignored = true;
      if (kind === 'filter')
        input['rust-suites'].suite.testcases.active['filter-match'].status = 'mismatch';
      if (kind === 'empty') input['rust-suites'] = {} as (typeof input)['rust-suites'];
      expect(() => compareInventories(cargo(), input)).toThrow();
    }
  );

  it('admits the default nextest ignored reason only for independently ignored cases', () => {
    const input = nextest();
    input['rust-suites'].suite.testcases.ignored['filter-match'] = {
      status: 'mismatch',
      reason: 'ignored',
    } as { status: string };
    expect(compareInventories(cargo(), input)).toHaveLength(2);
    input['rust-suites'].suite.testcases.active['filter-match'] = {
      status: 'mismatch',
      reason: 'ignored',
    } as { status: string };
    expect(() => compareInventories(cargo(), input)).toThrow('filtered test');
    input['rust-suites'].suite.testcases.active['filter-match'] = { status: 'matches' };
    input['rust-suites'].suite.testcases.ignored['filter-match'] = {
      status: 'mismatch',
      reason: 'expression',
    } as { status: string };
    expect(() => compareInventories(cargo(), input)).toThrow('filtered test');
  });

  it('pins real PASS identities, not a green summary or a matching count', () => {
    const cases = compareInventories(cargo(), nextest());
    const pass = '        PASS [  0.123s] (1/1) owner::suite active\n';
    expect(verifyExecution(cases, pass)).toEqual({ passed: 1, ignored: 1, total: 2 });
    for (const output of [
      '',
      'Summary: 1 test passed',
      pass.replace('active', 'other'),
      pass + pass,
      pass.replace('PASS', 'FAIL'),
      pass + pass.replace('active', 'ignored'),
    ]) {
      expect(() => verifyExecution(cases, output)).toThrow();
    }
  });
});
