/** Real Settings view with an explicit test source and the real shared shell. */
import { mountShell, shellRail } from '@tmt/browser-ui/shell';
import { applyStoredTheme } from '@tmt/browser-ui/theme';
import type { RemoteShell } from '../src/remote-shell.js';
import { settingsSource } from '../src/settings-source.js';
import { tmtSettingsPage } from '../src/tmt-settings-page.js';
import { describedModules, moduleFixture } from './settings-modules-source.js';

declare global {
  interface Window {
    moduleSourceReads: number;
  }
}
window.moduleSourceReads = 0;
applyStoredTheme();
const controller = new AbortController();
const rail = shellRail(undefined, {
  origin: location.origin,
  href: `${location.origin}/settings/`,
});
const shell = mountShell(document.querySelector('.tmt-ui-shell')!, { rail });
const mode = new URL(location.href).searchParams.get('state') ?? 'injected';
tmtSettingsPage(
  {
    signal: controller.signal,
    setMenu(menu) {
      shell.update({ rail, menu });
    },
    // The shell answers late in `late`, so the actions are proved to follow the description.
    described(listener) {
      if (mode === 'undescribed') listener(undefined);
      else if (mode === 'late') setTimeout(() => listener(describedModules), 500);
      else listener(describedModules);
    },
  } as RemoteShell,
  {
    async read() {
      window.moduleSourceReads++;
      if (mode === 'failed') throw new Error('Injected read failure');
      const base = await settingsSource.read();
      return mode === 'unavailable' ? base : moduleFixture(mode, base);
    },
  },
);
