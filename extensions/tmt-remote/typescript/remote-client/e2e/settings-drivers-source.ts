/** Explicit injected records for Settings presentation tests, never the product source. */
import type { TmtSettings } from '../src/settings-source.js';

export function driverFixture(mode: string, base: TmtSettings): TmtSettings {
  const reason = 'Permission refused for driver inventory.';
  return {
    ...base,
    sample: mode === 'injected',
    drivers:
      mode === 'refused'
        ? { rows: [], unavailable: reason }
        : {
            rows:
              mode === 'empty'
                ? []
                : [
                    { name: 'herdr', version: '0.2.0', state: 'ok' },
                    { name: 'zellij', version: '0.9.1', state: 'binary changed' },
                  ],
          },
    hooks:
      mode === 'refused'
        ? { rows: [], unavailable: reason }
        : {
            rows:
              mode === 'empty'
                ? []
                : [
                    { provider: 'claude', usage: true },
                    { provider: 'codex', usage: false },
                  ],
          },
  };
}
