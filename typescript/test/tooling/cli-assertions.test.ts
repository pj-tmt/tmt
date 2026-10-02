import { describe, expect, it } from 'vite-plus/test';
import { expectJsonResult } from '../e2e/cli-assertions.js';

describe('E2E JSON result assertion', () => {
  it('returns the original parsed value without copying or validating domain fields', () => {
    const payload = { identity: { id: 'scenario-owned-value' } };
    expect(
      expectJsonResult({ code: 0, stderr: '', stdout: JSON.stringify(payload), json: payload })
    ).toBe(payload);
  });

  it.each([null, false, 0, ''])('accepts defined JSON value %j', (json) => {
    expect(expectJsonResult({ code: 0, stderr: '', stdout: JSON.stringify(json), json })).toBe(
      json
    );
  });

  it.each([
    { name: 'nonzero exit', code: 3, stderr: '', stdout: '{}', json: {} },
    { name: 'unexpected stderr', code: 0, stderr: 'warning', stdout: '{}', json: {} },
    { name: 'absent parsed JSON', code: 0, stderr: '', stdout: 'not JSON', json: undefined },
  ])('rejects $name even when other envelope fields succeed', ({ name: _name, ...result }) => {
    expect(() => expectJsonResult(result)).toThrow();
  });
});
