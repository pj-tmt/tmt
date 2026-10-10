import { BrowserAction } from '@tmt/browser-ui/react';
import { useEffect, useState } from 'react';
import { RouterProvider } from '@tanstack/react-router';
import { mountUrl } from './bootstrap.js';
import { AppHeader, createAppRouter } from './router.js';
import { mountedTransport } from './mounted.js';
import { text } from './strings.js';
import { NoticeCard } from './notice-card.js';
import { buildWatch } from './app-build.js';
import { UpdateNotice } from './update-notice.js';

/** Each tab owns its Remote session and mounted page bindings until unload. */
export function MountedApp() {
  const [state, setState] = useState<'loading' | 'failed' | 'ready'>('loading');
  const [router, setRouter] = useState<ReturnType<typeof createAppRouter> | null>(null);
  useEffect(() => {
    let disposed = false;
    const controller = new AbortController();
    let mounted: Awaited<ReturnType<typeof mountedTransport>> | null = null;
    try {
      mountUrl();
    } catch {
      setState('failed');
      return;
    }
    async function activate() {
      try {
        const next = await mountedTransport(controller.signal);
        if (disposed) {
          next.close();
          return;
        }
        mounted = next;
        setRouter(createAppRouter(next.transport, next.space, next.pageIds));
        setState('ready');
      } catch {
        if (!disposed) setState('failed');
        void buildWatch.check();
      }
    }
    const close = () => {
      controller.abort();
      mounted?.close();
    };
    const restore = (event: PageTransitionEvent) => {
      if (event.persisted) location.reload();
    };
    window.addEventListener('pagehide', close);
    window.addEventListener('pageshow', restore);
    void activate();
    return () => {
      disposed = true;
      window.removeEventListener('pagehide', close);
      window.removeEventListener('pageshow', restore);
      close();
    };
  }, []);
  return (
    <>
      {state === 'ready' && router ? (
        <RouterProvider router={router} />
      ) : (
        <MountedNotice state={state} />
      )}
      <UpdateNotice />
    </>
  );
}

/** Presentation shared by the mounted lifecycle and the screen layout fixture. */
export function MountedNotice({ state }: { state: 'loading' | 'failed' | 'ready' }) {
  return (
    <>
      <AppHeader linked={false} title={state === 'failed' ? text.error : 'Opening space'} />
      <main>
        <NoticeCard
          state={state === 'failed' ? 'blocked' : 'opening'}
          eyebrow="Paired space"
          title={state === 'failed' ? text.error : text.registering}
          actions={
            state === 'failed' ? (
              <BrowserAction
                type="button"
                variant="text"
                label={text.reload}
                onActivate={() => location.reload()}
              />
            ) : undefined
          }
        >
          {state === 'failed' && <p>{text.registrationFailed}</p>}
        </NoticeCard>
      </main>
    </>
  );
}
