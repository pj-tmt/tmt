/** Real Settings view with an explicit test source and the real shared shell. */
import { mountShell, shellRail } from '@tmt/browser-ui/shell';
import { applyStoredTheme } from '@tmt/browser-ui/theme';
import { setRouteTitle, type RemoteShell } from '../src/remote-shell.js';
import { settingsSource } from '../src/settings-source.js';
import { tmtSettingsPage } from '../src/tmt-settings-page.js';
import { driverFixture } from './settings-drivers-source.js';

declare global {
  interface Window {
    driverSourceReads: number;
  }
}
window.driverSourceReads = 0;
applyStoredTheme();
const controller = new AbortController();
const rail = shellRail(undefined, {
  origin: location.origin,
  href: `${location.origin}/settings/`,
});
const shell = mountShell(document.querySelector('.tmt-ui-shell')!, { rail });
shell.setCommonControls({ openSearch() {}, remote: { kind: 'unconfirmed', href: '/remote/' } });
const mode = new URL(location.href).searchParams.get('state') ?? 'injected';
tmtSettingsPage(
  {
    signal: controller.signal,
    setTitle: (title, ancestors) =>
      setRouteTitle(
        {
          ...shell,
          setTitle: (text) => {
            document.title = `${text} · TMT`;
          },
        },
        title,
        ancestors,
      ),
    setMenu(menu) {
      shell.update({ rail, menu });
    },
    described(listener) {
      listener(undefined);
    },
  } as RemoteShell,
  {
    async read() {
      window.driverSourceReads++;
      if (mode === 'failed') throw new Error('Injected read failure');
      const base = await settingsSource.read();
      return mode === 'unavailable' ? base : driverFixture(mode, base);
    },
  },
);
