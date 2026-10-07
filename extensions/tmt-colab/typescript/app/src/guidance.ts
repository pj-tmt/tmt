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
  try {
    const recovered = await recoverSession({
      mount,
      storage: sessionStorage,
      async reopen() {
        const path = '/sdk/remote-v1.js';
        const sdk = await import(/* @vite-ignore */ path);
        await sdk.reopenSession();
      },
      reload: () => location.reload(),
    });
    if (!recovered) showGuidance();
  } catch {
    showGuidance();
  }
}
void start();
