import type { Root } from 'react-dom/client';
import { probeCapabilities } from '@tmt/colab-client';
import { UnsupportedBrowserNotice } from './notice-card.js';
import { HostedApp } from './hosted-app.js';
import { HostedEntry } from './hosted-entry.js';
import { captureHostedLink } from './hosted-location.js';

/** Origin bootstrap is distinct from the mounted door and never probes its API. */
export async function startHosted(root: Root): Promise<() => Promise<void>> {
  // Handle rejection immediately while the capability probe runs; no raw fragment enters React.
  const selection = captureHostedLink().then(
    (value) => ({ value }),
    (failure) => ({ failure }),
  );
  root.render(<HostedApp state={{ phase: 'loading' }} signIn={() => {}} />);
  try {
    await probeCapabilities();
  } catch {
    const parsed = await selection;
    if ('value' in parsed) parsed.value?.seed.fill(0);
    root.render(<UnsupportedBrowserNotice />);
    return async () => {};
  }
  const entry = new HostedEntry({
    visible: document.visibilityState !== 'hidden',
    state: (state) =>
      root.render(
        <HostedApp
          state={state}
          signIn={() => {
            void entry.signIn();
          }}
        />,
      ),
  });
  let closing: Promise<void> | undefined;
  const changed = () => {
    if (!closing) void entry.select(captureHostedLink());
  };
  const visible = () => {
    if (!closing) void entry.visible(document.visibilityState !== 'hidden');
  };
  const close = () => {
    closing ??= entry.close();
    return closing;
  };
  const pagehide = () => {
    void close();
  };
  const pageshow = (event: PageTransitionEvent) => {
    if (event.persisted) location.reload();
  };
  window.addEventListener('hashchange', changed);
  document.addEventListener('visibilitychange', visible);
  window.addEventListener('pagehide', pagehide);
  window.addEventListener('pageshow', pageshow);
  await entry.select(
    selection.then((parsed) => {
      if ('failure' in parsed) throw parsed.failure;
      return parsed.value;
    }),
  );
  return async () => {
    window.removeEventListener('hashchange', changed);
    document.removeEventListener('visibilitychange', visible);
    window.removeEventListener('pagehide', pagehide);
    window.removeEventListener('pageshow', pageshow);
    await close();
  };
}
