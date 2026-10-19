import { entryMount, publicEntry } from './entry.js';
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
  if (publicEntry()) {
    try {
      const mount = entryMount();
      const path = '/sdk/remote-v1.js';
      const sdk = await import(/* @vite-ignore */ path);
      await sdk.reopenSession();
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
    } catch {
      showGuidance();
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
