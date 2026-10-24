/** Explicit injected records for Settings presentation tests, never the product source. */
import type { TmtSettings } from '../src/settings-source.js';

export const longIdentity = 'guest-session-with-a-long-identity-name';
export function identityFixture(mode: string, base: TmtSettings): TmtSettings {
  return {
    ...base,
    sample: mode === 'injected',
    identities:
      mode === 'refused'
        ? { rows: [], unavailable: 'Permission refused for identity inventory.' }
        : {
            rows:
              mode === 'empty'
                ? []
                : [
                    { name: 'tmt-lead', lifetime: 'saved', provider: 'claude', online: true },
                    { name: 'core-1', lifetime: 'saved', provider: 'codex', online: true },
                    { name: 'ux-1', lifetime: 'saved', online: false },
                    { name: longIdentity, lifetime: 'temporary', provider: 'codex', online: false },
                  ],
          },
  };
}
