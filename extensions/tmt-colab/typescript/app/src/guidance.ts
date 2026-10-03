import { ActiveTab, InactiveTabError } from './active-tab.js';
import { recoverSession } from './session-recovery.js';

/** Public boot entry: the SDK alone inspects its paired key and door identity.
 * No registration, page data or agent capability is available here. */
async function start() {
  const guidance = document.getElementById('colab-guidance'),
    status = document.getElementById('colab-recovery-status');
  const showGuidance = () => {
    if (guidance) guidance.hidden = false;
    if (status) status.hidden = true;
  };
  const mount = new URL(location.href);
  mount.hash = '';
  if (!/^\/r\/[a-z0-9]+\/x\/colab\/$/.test(mount.pathname) || mount.search) {
    showGuidance();
    return;
  }
  let tabs: ActiveTab | undefined;
  let reloading = false;
  try {
    tabs = new ActiveTab(mount.pathname, () => {
      if (!reloading) showGuidance();
    });
    const owner = tabs;
    const recovered = await recoverSession({
      mount,
      storage: sessionStorage,
      async reopen() {
        await owner.takeover();
        const path = '/sdk/remote-v1.js';
        const sdk = await import(/* @vite-ignore */ path);
        await owner.run(() => sdk.reopenSession());
      },
      reload: () => {
        if (!owner.active) throw new InactiveTabError();
        reloading = true;
        location.reload();
      },
    });
    if (!recovered) showGuidance();
  } catch {
    showGuidance();
  } finally {
    tabs?.close();
  }
}
void start();
