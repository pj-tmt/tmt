/**
 * What the TMT settings page (`/settings`) reads: this machine's identities, installed modules,
 * host drivers, agent hooks and a few general settings. The page depends only on `SettingsSource`.
 * The source below is a MOCK until Core exposes a read API; the dev preview swaps in reads of the
 * read-only CLI JSON (`tmt ls`, `tmt extension ls`, `tmt driver ls`, `tmt setup --status`,
 * `tmt config show`), which hold the same facts on the terminal side.
 */
import { needs } from './not-available.js';

/** One part of the settings; a part the source cannot read is empty and says why. */
export interface Part<Row> {
  rows: readonly Row[];
  unavailable?: string;
}

export interface IdentityRow {
  name: string;
  lifetime: 'saved' | 'temporary';
  /** Whether the agent is in a live pane now. */
  online: boolean;
  /** The agent provider, such as claude or codex, when known. */
  provider?: string;
}
export interface ModuleRow {
  /** The extension name, such as colab. */
  name: string;
  installed: boolean;
  version?: string;
  channel?: string;
}
export interface DriverRow {
  name: string;
  version: string;
  /** `ok`, or the reason the driver no longer runs as approved. */
  state: string;
}
export interface HookRow {
  provider: string;
  /** Whether context and usage collection is on for this provider. */
  usage: boolean;
}
export interface GeneralRow {
  name: string;
  value: string;
  description: string;
}

export interface TmtSettings {
  general: Part<GeneralRow>;
  identities: Part<IdentityRow>;
  modules: Part<ModuleRow>;
  drivers: Part<DriverRow>;
  hooks: Part<HookRow>;
  /** Whether these values are sample data, not this machine's. */
  sample: boolean;
}

export interface SettingsSource {
  read(): Promise<TmtSettings>;
}

// MOCK: fixed sample values, so the page can be built and reviewed before the Core read API
// exists. Replace with the API-backed source; nothing else in the page changes.
export const mockSettingsSource: SettingsSource = {
  async read() {
    return {
      sample: true,
      general: {
        rows: [
          { name: 'Upgrade channel', value: 'alpha', description: 'Where `tmt upgrade` looks' },
          { name: 'Message batching', value: '5 s', description: 'Replies arrive in one batch' },
        ],
      },
      identities: {
        rows: [
          { name: 'ben', lifetime: 'saved', online: false },
          { name: 'ux-1', lifetime: 'saved', online: true, provider: 'claude' },
          { name: 'night-1', lifetime: 'temporary', online: false, provider: 'codex' },
        ],
      },
      modules: {
        rows: [
          { name: 'colab', installed: true, version: '0.1.0-alpha.231', channel: 'alpha' },
          { name: 'remote', installed: true, version: '0.1.0-alpha.135', channel: 'alpha' },
          { name: 'ops', installed: true, version: '0.1.0-alpha.158', channel: 'alpha' },
          { name: 'office', installed: false },
        ],
      },
      drivers: { rows: [{ name: 'herdr', version: '0.1.0-alpha.0', state: 'ok' }] },
      hooks: {
        rows: [
          { provider: 'claude', usage: true },
          { provider: 'codex', usage: true },
        ],
      },
    };
  },
};

/** The source the page reads by default. */
export const settingsSource: SettingsSource = {
  async read() {
    const none = { rows: [], unavailable: needs('a Core read of this machine’s settings') };
    return {
      general: none,
      identities: none,
      modules: none,
      drivers: none,
      hooks: none,
      sample: false,
    };
  },
};
