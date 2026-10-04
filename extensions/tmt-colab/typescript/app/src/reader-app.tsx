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
        (bar.current?.offsetHeight ?? 56) + (bar.current?.getBoundingClientRect().top ?? 0),
    }).catch(() => {
      if (!controller.signal.aborted) setRender('failed');
    });
    return () => controller.abort();
  }, [source]);
  return (
    <>
      <header className="reader-bar" ref={bar}>
        <span className="reader-brand">
          {text.product}
          <span>tmt</span>
        </span>
        {state.kind === 'ready' && (
          <>
            <h1 className="reader-title">{title}</h1>
            <span className="chip">{text.readerOnly}</span>
            <span
              className={`status ${render === 'ready' ? 'live' : render === 'loading' ? 'waiting' : 'blocked'}`}
              role="status"
            >
              <span aria-hidden>{render === 'ready' ? '●' : render === 'loading' ? '○' : '✗'}</span>{' '}
              {render === 'ready'
                ? text.readerLive
                : render === 'loading'
                  ? text.readerLoading
                  : text.readerStopped}
            </span>
          </>
        )}
        <details className="reader-information">
          <summary>{text.readerInfo}</summary>
          <div className="reader-information-panel">
            <p>{text.readerNote}</p>
            <p>{text.warning}</p>
          </div>
        </details>
      </header>
      <main>
        {state.kind === 'ready' ? (
          <section className="page">
            <div className="workspace">
              <div className="canvas">
                <div className="frame-host" ref={host} />
                {(render === 'navigation' || render === 'failed') && (
                  <div className="notice blocked" role="alert">
                    <span className="notice-mark" aria-hidden>
                      ✗
                    </span>
                    <h2>{text.blocked}</h2>
                    <p>{render === 'navigation' ? text.navigation : text.failed}</p>
                    <p>{text.limit}</p>
                  </div>
                )}
              </div>
            </div>
          </section>
        ) : (
          <section
            className={`notice ${state.kind === 'opening' ? 'waiting' : 'blocked'}`}
            role={state.kind === 'opening' ? 'status' : 'alert'}
          >
            <span className="notice-mark" aria-hidden>
              {state.kind === 'opening' ? '○' : '✗'}
            </span>
            <p className="notice-eyebrow">{text.readerOnly}</p>
            <h1>
              {state.kind === 'opening'
                ? text.readerOpening
                : state.kind === 'ended'
                  ? text.readerEnded
                  : text.error}
            </h1>
            {state.kind === 'ended' && <p>{text.readerEndedNote}</p>}
            {state.kind === 'invalid' && <p>{text.readerInvalid}</p>}
            {state.kind === 'failed' && <p>{text.readerFailed}</p>}
          </section>
        )}
      </main>
    </>
  );
}
