/** The settings page's one map of product copy for the names it knows. Any other name shows no
 * description. These are words about the product, never values read from this machine. */
import type { createIcon } from '@tmt/browser-ui/glyph';

export type SettingsIcon = Parameters<typeof createIcon>[0];

export const MODULE_COPY: Readonly<Record<string, { icon: SettingsIcon; description: string }>> = {
  colab: { icon: 'file-text', description: 'Shared pages and agent collaboration.' },
  remote: { icon: 'monitor-smartphone', description: 'Browser access to your machine.' },
  ops: { icon: 'users', description: 'Squads, agent state and timers.' },
  digest: { icon: 'chart-column', description: 'Usage and message batching.' },
};

/** Host drivers by name: only herdr, the one driver the product ships. */
export const DRIVER_COPY: Readonly<Record<string, string>> = {
  herdr: 'Host driver for agent windows.',
};

const USAGE_HOOK = 'Usage collection records agent usage for Digest.';
/** Agent hooks by provider name. */
export const HOOK_COPY: Readonly<Record<string, string>> = {
  claude: USAGE_HOOK,
  codex: USAGE_HOOK,
};

export const displayName = (name: string) => name.charAt(0).toUpperCase() + name.slice(1);
