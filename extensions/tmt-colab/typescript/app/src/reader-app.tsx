import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { Circle, Info, LoaderCircle, X } from 'lucide-react';
import { ColabHeader } from './colab-header.js';
import { NoticeCard } from './notice-card.js';
import { useEffect, useRef, useState } from 'react';
import { mountRenderer, type RenderState } from './renderer.js';
import { text } from './strings.js';
import type { PageView } from './transport.js';

export type ReaderState =
  | { kind: 'opening' }
  | { kind: 'ready'; view: PageView }
  | { kind: 'ended' }
  | { kind: 'invalid' }
  | { kind: 'failed' };

/** The read-only page: no editor, Ask, export or share controls exist in this entry. */
export function ReaderApp({ state }: { state: ReaderState }) {
  const source = state.kind === 'ready' ? state.view.source : null;
  const title = state.kind === 'ready' ? state.view.title : '';
  const [render, setRender] = useState<RenderState | 'loading'>('loading');
  const host = useRef<HTMLDivElement>(null);
  const bar = useRef<HTMLElement>(null);
  useEffect(() => {
    if (source === null) return;
    const controller = new AbortController();
    setRender('loading');
    void mountRenderer(host.current!, source, {
      signal: controller.signal,
      onState: setRender,
      viewportInset: () =>
        (bar.current?.offsetHeight ?? 0) + (bar.current?.getBoundingClientRect().top ?? 0),
    }).catch(() => {
      if (!controller.signal.aborted) setRender('failed');
    });
    return () => controller.abort();
  }, [source]);
  return (
    <>
      <ColabHeader
        headerRef={bar}
        title={
          state.kind === 'ready'
            ? title || text.unknownPageTitle
            : state.kind === 'opening'
              ? 'Opening page'
              : state.kind === 'ended'
                ? text.readerEnded
                : text.error
        }
        actions={
          <>
            {state.kind === 'ready' && (
              <>
                <span className="chip">{text.readerOnly}</span>
                <span
                  className={`status ${render === 'ready' ? 'live' : render === 'loading' ? 'waiting' : 'blocked'}`}
                  role="status"
                >
                  {render === 'ready' ? (
                    <Circle className="status-dot" fill="currentColor" aria-hidden />
                  ) : render === 'loading' ? (
                    <LoaderCircle aria-hidden />
                  ) : (
                    <X aria-hidden />
                  )}{' '}
                  {render === 'ready'
                    ? text.readerLive
                    : render === 'loading'
                      ? text.readerLoading
                      : text.readerStopped}
                </span>
              </>
            )}
            <details className="reader-information">
              <summary className={ui.action} data-variant="text" aria-label={text.readerInfo}>
                <Info aria-hidden />
              </summary>
              <div className="reader-information-panel">
                <p>{text.readerNote}</p>
                <p>{text.warning}</p>
              </div>
            </details>
          </>
        }
      />
      <main>
        {state.kind === 'ready' ? (
          <section className="page">
            <div className="workspace">
              <div className="canvas">
                <div className="frame-host" ref={host} />
                {(render === 'navigation' || render === 'failed') && (
                  <NoticeCard state="blocked" eyebrow={text.readerOnly} title={text.blocked}>
                    <p>{render === 'navigation' ? text.navigation : text.failed}</p>
                    <p>{text.limit}</p>
                  </NoticeCard>
                )}
              </div>
            </div>
          </section>
        ) : (
          <NoticeCard
            state={
              state.kind === 'opening' ? 'opening' : state.kind === 'ended' ? 'ended' : 'blocked'
            }
            eyebrow={text.readerOnly}
            title={
              state.kind === 'opening'
                ? text.readerOpening
                : state.kind === 'ended'
                  ? text.readerEnded
                  : text.error
            }
          >
            {state.kind === 'ended' && <p>{text.readerEndedNote}</p>}
            {state.kind === 'invalid' && <p>{text.readerInvalid}</p>}
            {state.kind === 'failed' && <p>{text.readerFailed}</p>}
          </NoticeCard>
        )}
      </main>
    </>
  );
}
