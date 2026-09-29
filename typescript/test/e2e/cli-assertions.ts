import { expect } from 'vitest';
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
 * A `tmt ls`/`tmt list` JSON result without the additive `address` and
 * `driver` keys (#434), after checking them, so scenarios keep asserting the
 * pre-existing contract exactly. Both keys come together; a live pane always
 * has an address, which is its tmux pane unless an agent session is known.
 */
export function withoutAddress<T>(result: CliResult<T>): CliResult<T> {
  const strip = (row: Row): Row => {
    if (!('address' in row) && !('driver' in row)) return row;
    const { address, driver, ...rest } = row;
    expect(address === null, 'address and driver come together').toBe(driver === null);
    if (typeof address === 'string') expect(address.startsWith(`${String(driver)}:`)).toBe(true);
    if (typeof rest.pane === 'string' && rest.presence === 'active') {
      expect(address, 'a live pane always has an address').not.toBeNull();
    }
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
