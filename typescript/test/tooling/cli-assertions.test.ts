import { describe, expect, it } from 'vite-plus/test';
import { expectJsonResult, withoutAddress } from '../e2e/cli-assertions.js';

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

describe('E2E ls row projection', () => {
  const live = (patch: Record<string, unknown> = {}) => ({
    id: 'id',
    presence: 'active',
    pane: '%1',
    target: 'my session:3.0',
    address: 'tmux:%1',
    driver: 'tmux',
    location: { host: 'tmux', session: 'my session', window: '3' },
    ...patch,
  });
  const result = (json: unknown) => ({ code: 0, stderr: '', stdout: '', json });
  const rows = (json: unknown) =>
    (withoutAddress(result({ identities: [json] })).json as { identities: unknown[] })
      .identities[0];

  it('checks a live pane location against its target, then removes it with the address keys', () => {
    expect(rows(live())).toEqual({
      id: 'id',
      presence: 'active',
      pane: '%1',
      target: 'my session:3.0',
    });
    const scoped = withoutAddress(
      result({
        target: 'x',
        pane: {
          id: '%1',
          target: 'a.b:12.1',
          location: { host: 'tmux', session: 'a.b', window: '12' },
        },
      })
    ).json;
    expect(scoped).toEqual({ target: 'x', pane: { id: '%1', target: 'a.b:12.1' } });
  });

  it.each([
    ['a missing location', { location: undefined }],
    ['another host', { location: { host: 'herdr', session: 'my session', window: '3' } }],
    [
      "a session that is not the target's",
      { location: { host: 'tmux', session: 'x', window: '3' } },
    ],
    [
      "a window that is not the target's",
      { location: { host: 'tmux', session: 'my session', window: '4' } },
    ],
    [
      'a non-numeric window',
      { target: 'my session:x.0', location: { host: 'tmux', session: 'my session', window: 'x' } },
    ],
  ])('rejects a live row with %s', (_name, patch) => {
    const row = live(patch);
    if (patch.location === undefined) delete (row as Record<string, unknown>).location;
    expect(() => rows(row)).toThrow();
  });

  it('leaves an offline row alone', () => {
    const offline = { id: 'id', presence: 'offline', pane: null, address: null, driver: null };
    expect(rows(offline)).toEqual({ id: 'id', presence: 'offline', pane: null });
  });
});
