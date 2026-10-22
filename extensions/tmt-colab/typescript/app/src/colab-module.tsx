/**
 * Colab as a route of the one TMT web app (#3937): the host imports this from Colab's own mount
 * and mounts it into its shell at `/colab/...`. The host owns the rail; Colab renders its header
 * into the host's slot and gives the host its module menu. Unmounting closes the Remote session,
 * the page bindings and the React root.
 */
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { probeCapabilities } from '@tmt/colab-client';
import { moduleMount } from '@tmt/browser-ui/shell';
import type { ShellSlot } from '@tmt/browser-ui/shell-host';
import { ColabModuleBoundary } from './colab-module-boundary.js';
import '@tmt/browser-ui/static.css';
import './style.css';
import { ShellHostSlot } from './colab-shell.js';
import { hostInShell } from './entry.js';
import { previewTransport } from './local-pages.js';
import { MountedApp } from './mounted-app.js';
import { UnsupportedBrowserNotice } from './notice-card.js';
import { createAppRouter, shellLocation } from './router.js';

/** The paired mount the composed shell names for Colab, or null for the local preview. */
function pairedMount(slot: ShellSlot): URL | null {
  const colab = slot.answer()?.modules.find((module) => module.extension === 'colab');
  if (!colab) return null;
  const mount = new URL(colab.rail.path, location.href);
  return mount.origin === location.origin && moduleMount(mount.pathname) === 'colab' ? mount : null;
}

/**
 * Link the built module's stylesheet, beside it under the mount (the dev server injects styles),
 * and return what removes it again: nothing when the page already had it before this mount.
 */
function stylesheet(): () => void {
  if (import.meta.env.DEV) return () => {};
  const href = new URL('./colab-module.css', import.meta.url).href;
  if (document.querySelector(`link[href="${href}"]`)) return () => {};
  const link = document.head.appendChild(
    Object.assign(document.createElement('link'), { rel: 'stylesheet', href }),
  );
  return () => link.remove();
}

export async function mount(slot: ShellSlot): Promise<() => void> {
  const unstyle = stylesheet();
  const host = document.createElement('div');
  host.className = 'colab-route';
  slot.main.replaceChildren(host);
  const root = createRoot(host);
  try {
    await probeCapabilities();
  } catch {
    root.render(<UnsupportedBrowserNotice />);
    return () => {
      root.unmount();
      unstyle();
    };
  }
  const paired = pairedMount(slot);
  hostInShell(paired);
  shellLocation();
  slot.setTitle('Colab');
  root.render(
    <StrictMode>
      <ColabModuleBoundary slot={slot}>
        <ShellHostSlot value={slot}>
          {paired ? <MountedApp /> : <RouterProvider router={createAppRouter(previewTransport)} />}
        </ShellHostSlot>
      </ColabModuleBoundary>
    </StrictMode>,
  );
  // MountedApp closes its session and bindings when it unmounts.
  return () => {
    root.unmount();
    unstyle();
  };
}
