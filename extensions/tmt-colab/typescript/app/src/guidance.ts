import { entryMount, publicEntry } from './entry.js';
import { sessionLimit } from './session-limit-error.js';
import { recoverSession } from './session-recovery.js';
import { text } from './strings.js';

/** Public boot entry: the SDK alone inspects its paired key and door identity.
 * No registration, page data or agent capability is available here. */
async function start() {
  const guidance = document.getElementById('colab-guidance'),
    status = document.getElementById('colab-recovery-status');
  const showGuidance = (error?: unknown) => {
    // Only the SDK's typed attempt distinguishes an absent key or verified signed refusal.
    // Plain messages, storage failures and missing owner context are never pairing evidence.
    const reason =
      error instanceof Error && error.name === 'ReopenSessionError'
        ? (error as Error & { reason?: unknown }).reason
        : undefined;
    const pairingLost = reason === 'unpaired' || reason === 'revoked' || reason === 'expired';
    const blocked = pairingLost || reason === 'mismatch';
    const sentence =
      reason === 'mismatch'
        ? text.failureAccessEnded
        : sessionLimit(error)
          ? text.failureSessionLimit
          : reason === 'revoked'
            ? text.failurePairingRemoved
            : reason === 'expired'
              ? text.failurePairingExpired
              : reason === 'unpaired'
                ? text.failureUnpaired
                : text.reconnectFailed;
    if (guidance) {
      const paragraph = document.createElement('p');
      paragraph.textContent = sentence;
      guidance.replaceChildren(paragraph);
      if (pairingLost) {
        const command = document.createElement('div');
        command.className = 'tmt-ui-command';
        const code = document.createElement('code');
        code.className = 'tmt-ui-command-text';
        code.textContent = 'tmt remote pair';
        command.append(code);
        guidance.append(command);
      }
      guidance.hidden = false;
    }
    const card = document.querySelector<HTMLElement>('.guidance-card');
    if (card) card.dataset.tone = blocked ? 'blocked' : 'muted';
    const label = document.querySelector('.tmt-ui-notice-mark > span:last-child');
    if (label) label.textContent = blocked ? 'failed' : 'not connected';
    const mark = document.querySelector('.guidance-mark');
    if (mark) {
      const shape = document.createElementNS(
        'http://www.w3.org/2000/svg',
        blocked ? 'path' : 'circle',
      );
      if (blocked) shape.setAttribute('d', 'M18 6 6 18M6 6l12 12');
      else
        for (const [key, value] of Object.entries({ cx: '12', cy: '12', r: '10' }))
          shape.setAttribute(key, value);
      mark.replaceChildren(shape);
    }
    const heading = document.querySelector('.tmt-ui-notice-heading');
    if (heading)
      heading.textContent = pairingLost ? 'Pair this browser first' : 'Could not reconnect';
    const title = document.querySelector('.tmt-ui-title');
    if (title) title.textContent = pairingLost ? 'Pair browser' : 'Open Colab';
    if (status) status.hidden = true;
  };
  // A signal selects one typed SDK attempt, without adding automatic retries.
  const reopen = async () => {
    const path = '/sdk/remote-v1.js';
    const sdk = await import(/* @vite-ignore */ path);
    return sdk.reopenSession(undefined, { signal: AbortSignal.timeout(10_000) });
  };
  if (publicEntry()) {
    try {
      const mount = entryMount();
      try {
        await reopen();
      } catch (error) {
        // A paired device at its session limit still loads the app: its own open shows the limit
        // card with the fixes, where guidance would wrongly say the browser is not paired.
        if (!sessionLimit(error)) throw error;
      }
      const response = await fetch(new URL('index.html', mount), {
        signal: AbortSignal.timeout(10_000),
      });
      if (!response.ok) throw new Error('App unavailable');
      const reader = response.body!.getReader();
      const parts: Uint8Array[] = [];
      let size = 0;
      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          size += value.length;
          if (size > 64 * 1024) throw new Error('App entry exceeds limit');
          parts.push(value);
        }
      } finally {
        await reader.cancel();
      }
      const raw = new Uint8Array(size);
      let offset = 0;
      for (const part of parts) {
        raw.set(part, offset);
        offset += part.length;
      }
      const html = new DOMParser().parseFromString(new TextDecoder().decode(raw), 'text/html');
      const asset = (value: string) => {
        const url = new URL(value, mount);
        if (
          url.origin !== mount.origin ||
          !url.pathname.startsWith(`${mount.pathname}assets/`) ||
          url.search ||
          url.hash
        )
          throw new Error('Invalid app asset');
        return url.href;
      };
      const scripts = [...html.querySelectorAll<HTMLScriptElement>('script[type="module"][src]')];
      if (scripts.length !== 1) throw new Error('Invalid app entry');
      const script = document.createElement('script');
      script.type = 'module';
      script.src = asset(scripts[0].getAttribute('src')!);
      const styles = [...html.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]')].map(
        (link) => {
          const style = document.createElement('link');
          style.rel = 'stylesheet';
          style.href = asset(link.getAttribute('href')!);
          return style;
        },
      );
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => {
          script.remove();
          reject(new Error('App unavailable'));
        }, 10_000);
        script.onload = () => {
          clearTimeout(timer);
          resolve();
        };
        script.onerror = () => {
          clearTimeout(timer);
          reject(new Error('App unavailable'));
        };
        document.head.append(...styles, script);
      });
      return;
    } catch (error) {
      showGuidance(error);
      return;
    }
  }
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
      reopen,
      failed: showGuidance,
      reload: () => location.reload(),
    });
    if (!recovered && guidance?.hidden) showGuidance();
  } catch {
    showGuidance();
  }
}
void start();
