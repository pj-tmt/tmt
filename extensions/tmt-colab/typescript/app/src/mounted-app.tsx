import { useEffect, useState } from 'react';
import { RouterProvider } from '@tanstack/react-router';
import { ActiveTab, InactiveTabError } from './active-tab.js';
import { mountUrl } from './bootstrap.js';
import { AppHeader, createAppRouter } from './router.js';
import { mountedTransport } from './mounted.js';
import { text } from './strings.js';
import { NoticeCard } from './notice-card.js';

/** One mounted app owns the lease and all page bindings until takeover/unload. */
export function MountedApp() {
  const [state, setState] = useState<'loading' | 'inactive' | 'failed' | 'ready'>('loading');
  const [router, setRouter] = useState<ReturnType<typeof createAppRouter> | null>(null);
  const [useHere, setUseHere] = useState<(() => void) | null>(null);
  useEffect(() => {
    let disposed = false;
    let generation = 0;
    let mounted: Awaited<ReturnType<typeof mountedTransport>> | null = null;
    let key: string;
    try {
      key = mountUrl().pathname;
    } catch {
      setState('failed');
      return;
    }
    const tabs = new ActiveTab(key, () => {
      generation++;
      mounted?.close();
      mounted = null;
      if (!disposed) {
        setRouter(null);
        setState('inactive');
      }
    });
    async function activate() {
      if (disposed) return;
      const current = ++generation;
      let acquired = false;
      setState('loading');
      try {
        await tabs.takeover();
        acquired = true;
        if (disposed || current !== generation || !tabs.active) return;
        const next = await tabs.run(() => mountedTransport(tabs));
        if (disposed || current !== generation || !tabs.active) {
          next.close();
          return;
        }
        mounted = next;
        setRouter(createAppRouter(next.transport, next.space));
        setState('ready');
      } catch (error) {
        if (
          !disposed &&
          current === generation &&
          (!acquired || tabs.active) &&
          !(error instanceof InactiveTabError) &&
          !(error instanceof DOMException && error.name === 'AbortError')
        )
          setState('failed');
      }
    }
    setUseHere(() => () => {
      void activate();
    });
    const close = () => {
      mounted?.close();
      tabs.close();
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
  if (state === 'ready' && router) return <RouterProvider router={router} />;
  return <MountedNotice state={state} useHere={useHere} />;
}

/** Presentation shared by the mounted lifecycle and the screen layout fixture. */
export function MountedNotice({
  state,
  useHere,
}: {
  state: 'loading' | 'inactive' | 'failed' | 'ready';
  useHere: (() => void) | null;
}) {
  return (
    <>
      <AppHeader
        linked={false}
        title={
          state === 'inactive' ? 'Another tab' : state === 'failed' ? text.error : 'Opening space'
        }
      />
      <main>
        <NoticeCard
          state={state === 'inactive' ? 'inactive' : state === 'failed' ? 'blocked' : 'opening'}
          eyebrow="Paired space"
          title={
            state === 'inactive'
              ? text.otherTab
              : state === 'failed'
                ? text.error
                : text.registering
          }
          testId={state === 'inactive' ? 'colab-inactive' : undefined}
          actions={
            state === 'inactive' ? (
              <button
                data-testid="colab-use-here"
                onClick={(event) => {
                  if (event.isTrusted) useHere?.();
                }}
              >
                {text.useHere}
              </button>
            ) : state === 'failed' ? (
              <button onClick={() => location.reload()}>{text.reload}</button>
            ) : undefined
          }
        >
          {state === 'inactive' && <p>{text.oneTab}</p>}
          {state === 'failed' && <p>{text.registrationFailed}</p>}
        </NoticeCard>
      </main>
    </>
  );
}
