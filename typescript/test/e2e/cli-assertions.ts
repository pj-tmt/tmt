import { expect } from 'vite-plus/test';
import type { CliResult } from './harness.js';

/** Assert the E2E JSON envelope; each scenario still owns its payload assertions. */
export function expectJsonResult<T>(result: CliResult<T>): T {
  expect(result.code, result.stderr || result.stdout).toBe(0);
  expect(result.stderr).toBe('');
  expect(result.json, result.stdout).toBeDefined();
  return result.json as T;
}

type Row = Record<string, unknown>;

/**
 * Core's additive `location` object, checked against the `target` tmux formats beside
 * it and then removed (from a row or its `pane` document), so scenarios keep asserting
 * the pre-existing row exactly. A live pane always has one: the tmux host plus the
 * session and window index as the host reports them.
 */
function withoutLocation(row: Row): Row {
  const check = (holder: Row): Row => {
    if (!('location' in holder)) return holder;
    const { location, ...rest } = holder;
    const place = location as { host?: string; session?: string; window?: string };
    expect(place.host, 'a tmux pane reports the tmux host').toBe('tmux');
    expect(typeof place.session).toBe('string');
    expect(place.window).toMatch(/^\d+$/);
    expect(String(rest.target), 'the fields agree with the displayed target').toMatch(
      new RegExp(`^${place.session!.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}:${place.window}\\.`)
    );
    return rest;
  };
  if (typeof row.pane === 'string' && row.presence === 'active') {
    expect('location' in row, 'a live pane always has a location').toBe(true);
  }
  const stripped = check(row);
  return stripped.pane !== null && typeof stripped.pane === 'object'
    ? { ...stripped, pane: check(stripped.pane as Row) }
    : stripped;
}

/**
 * A `tmt ls` JSON result without the additive `address` and
 * `driver` keys (#434) and the pane `location`, after checking them, so scenarios
 * keep asserting the pre-existing contract exactly. A row carries both keys or
 * neither (a direct-pane document has neither); each is null or a string, and the two
 * are null together. A live pane always has a string address, which is its tmux pane
 * unless an agent session is known.
 */
export function withoutAddress<T>(result: CliResult<T>): CliResult<T> {
  const strip = (original: Row): Row => {
    const row = withoutLocation(original);
    const live = typeof row.pane === 'string' && row.presence === 'active';
    expect('address' in row, 'address and driver come together').toBe('driver' in row);
    if (!('address' in row)) {
      expect(live, 'a live pane always has an address').toBe(false);
      return row;
    }
    const { address, driver, ...rest } = row;
    for (const value of [address, driver]) {
      expect(
        value === null || typeof value === 'string',
        'address and driver are null or text'
      ).toBe(true);
    }
    expect(address === null, 'address and driver come together').toBe(driver === null);
    if (typeof address === 'string') expect(address.startsWith(`${String(driver)}:`)).toBe(true);
    if (live) expect(address, 'a live pane always has an address').not.toBeNull();
    return rest;
  };
  const value = result.json as unknown;
  if (value === null || typeof value !== 'object') return result;
  const document = value as Row;
  const json = Array.isArray(document.identities)
    ? { ...document, identities: (document.identities as Row[]).map(strip) }
    : strip(document);
  return { ...result, json: json as T };
}

/** No admitted provider event or authoritative process-end observation. */
export const unknownActivity = {
  state: 'unknown',
  sinceMs: null,
  lastActivityMs: null,
  providers: {},
};
