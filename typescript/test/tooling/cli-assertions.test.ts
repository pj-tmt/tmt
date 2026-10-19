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

  it.each([
    ['a tmux pane address', {}],
    ['an agent session address', { address: 'claude:abc', driver: 'claude' }],
  ])('accepts a live row with %s and removes the address keys', (_name, patch) => {
    expect(rows(live(patch))).toEqual({
      id: 'id',
      presence: 'active',
      pane: '%1',
      target: 'my session:3.0',
    });
  });

  // Each case starts from a valid live row and changes only the address keys, so a
  // rejection can only come from the address check.
  const addressDefects: [string, string[], Record<string, unknown>][] = [
    ['both address keys missing', ['address', 'driver'], {}],
    ['the address key missing', ['address'], {}],
    ['the driver key missing', ['driver'], {}],
    ['an undefined address and driver', [], { address: undefined, driver: undefined }],
    ['an undefined address', [], { address: undefined }],
    ['an undefined driver', [], { driver: undefined }],
    ['a null address and driver', [], { address: null, driver: null }],
    ['a driver that is not the address prefix', [], { driver: 'codex' }],
  ];

  it.each(addressDefects)('rejects a live row with %s', (_name, drop, patch) => {
    const row = live(patch) as Record<string, unknown>;
    for (const key of drop) delete row[key];
    expect(() => rows(row)).toThrow();
  });

  // `tmt ls <name>` reports one identity with its pane as a document.
  const named = (patch: Record<string, unknown> = {}) => ({
    target: 'Name',
    identity: { id: 'id' },
    presence: 'active',
    pane: {
      id: '%1',
      target: 'my session:3.0',
      location: { host: 'tmux', session: 'my session', window: '3' },
    },
    address: 'tmux:%1',
    driver: 'tmux',
    ...patch,
  });
  const inspected = (json: unknown) => withoutAddress(result(json)).json;

  it.each([
    ['a tmux pane address', {}],
    ['an agent session address', { address: 'claude:abc', driver: 'claude' }],
  ])('accepts a live named row with %s and removes the address keys', (_name, patch) => {
    expect(inspected(named(patch))).toEqual({
      target: 'Name',
      identity: { id: 'id' },
      presence: 'active',
      pane: { id: '%1', target: 'my session:3.0' },
    });
  });

  it.each(addressDefects)('rejects a live named row with %s', (_name, drop, patch) => {
    const row = named(patch) as Record<string, unknown>;
    for (const key of drop) delete row[key];
    expect(() => inspected(row)).toThrow();
  });

  it('leaves an offline named row without an address alone', () => {
    const base = { target: 'Name', identity: { id: 'id' }, presence: 'offline', pane: null };
    expect(inspected({ ...base, address: null, driver: null })).toEqual(base);
  });

  it.each([
    ['an offline row', { id: 'id', presence: 'offline', pane: null }],
    ['an unknown row', { id: 'id', presence: 'unknown', pane: null }],
  ])('leaves %s without an address alone', (_name, base) => {
    const row = { ...base, address: null, driver: null };
    expect(rows(row)).toEqual(base);
  });

  it('accepts the remembered session address of an offline row', () => {
    const offline = {
      id: 'id',
      presence: 'offline',
      pane: null,
      address: 'codex:abc',
      driver: 'codex',
    };
    expect(rows(offline)).toEqual({ id: 'id', presence: 'offline', pane: null });
  });

  it.each([
    ['only the address key', { address: null }],
    ['only the driver key', { driver: null }],
    ['an undefined address and driver', { address: undefined, driver: undefined }],
    ['a non-string driver', { address: 'codex:abc', driver: 7 }],
  ])('rejects an offline row with %s', (_name, patch) => {
    expect(() => rows({ id: 'id', presence: 'offline', pane: null, ...patch })).toThrow();
  });
});
