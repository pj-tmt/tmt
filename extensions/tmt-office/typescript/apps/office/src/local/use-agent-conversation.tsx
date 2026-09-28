import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import type { LocalRuntime } from './local-runtime.js';
import { AgentConversation } from './agent-conversation.js';
import type { ConversationTarget } from './agent-conversation.js';
import './agent-hud.css';

const targetKey = (target: ConversationTarget) => `${target.id}:${target.room?.id ?? ''}`;
type Tab = 'chat' | 'info';
interface Contents {
  info(target: ConversationTarget): ReactNode;
  portrait(target: ConversationTarget): ReactNode;
  closed(): void;
  suspended?: boolean;
  initial?: ConversationTarget;
  refreshInfo?(): void;
  refreshingInfo?: boolean;
}

/** One retained inspector session; hiding pauses observation without discarding its draft. */
export function useAgentConversation(
  runtime: Pick<LocalRuntime, 'dispatch' | 'requests'>,
  contents: Contents
) {
  const [target, setTarget] = useState<ConversationTarget | undefined>(contents.initial);
  const [nextTarget, setNextTarget] = useState<ConversationTarget>();
  const [mode, setMode] = useState<'closed' | 'expanded'>(contents.initial ? 'expanded' : 'closed');
  const [tab, setTab] = useState<Tab>(contents.initial ? 'info' : 'chat');
  const [dirty, setDirty] = useState(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const [refreshHost, setRefreshHost] = useState<HTMLSpanElement | null>(null);
  const key = target && targetKey(target);
  const visible = mode === 'expanded' && !contents.suspended;
  useEffect(() => {
    if (visible) heading.current?.focus({ preventScroll: true });
  }, [visible, key]);
  const open = useCallback(
    (identity: ConversationTarget, nextTab: Tab = 'chat') => {
      if (target && targetKey(target) !== targetKey(identity) && dirty) setNextTarget(identity);
      else {
        setTarget(identity);
        setNextTarget(undefined);
        setTab(nextTab);
      }
      setMode('expanded');
    },
    [dirty, target]
  );
  function close() {
    setMode('closed');
    contents.closed();
  }
  function render() {
    if (!target) return null;
    return (
      <div
        className="agent-hud-host"
        hidden={!visible}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.stopPropagation();
            close();
          }
        }}
      >
        <aside aria-label="Agent details" hidden={mode !== 'expanded'}>
          <section
            className="agent-hud"
            role="dialog"
            aria-label="Agent conversation"
            aria-modal="false"
          >
            <header className="agent-hud-header">
              {contents.portrait(target)}
              <div className="agent-hud-name">
                <h2 ref={heading} tabIndex={-1}>
                  {target.name}
                </h2>
                <small>{target.room?.name ?? 'Direct conversation'}</small>
              </div>
              <div className="agent-hud-tabs" role="tablist" aria-label="Agent view">
                <button
                  role="tab"
                  aria-label="Chat"
                  title="Chat"
                  aria-selected={tab === 'chat'}
                  onClick={() => setTab('chat')}
                >
                  <svg aria-hidden="true" viewBox="0 0 24 24">
                    <path d="M5 4h14v12H10l-5 4Z" />
                  </svg>
                </button>
                <button
                  role="tab"
                  aria-label="Info"
                  title="Info"
                  aria-selected={tab === 'info'}
                  onClick={() => setTab('info')}
                >
                  <svg aria-hidden="true" viewBox="0 0 24 24">
                    <circle cx="12" cy="12" r="9" />
                    <path d="M12 10v7m0-11v1" />
                  </svg>
                </button>
              </div>
              <span ref={setRefreshHost} hidden={tab !== 'chat' || Boolean(nextTarget)} />
              {tab === 'info' && contents.refreshInfo && (
                <button
                  aria-label="Refresh agent information"
                  title="Refresh agent information"
                  disabled={contents.refreshingInfo}
                  onClick={contents.refreshInfo}
                >
                  ↻
                </button>
              )}
              <button aria-label="Close agent conversation" title="Close" onClick={close}>
                ×
              </button>
            </header>
            {nextTarget && (
              <section className="agent-hud-confirm" aria-label="Switch conversation confirmation">
                <h3>
                  Open conversation with {nextTarget.name}
                  {nextTarget.room ? ` in ${nextTarget.room.name}` : ''}?
                </h3>
                <p>
                  The unsent draft here will be discarded. Submitted work is not cancelled;
                  unconfirmed sends remain recoverable when you return.
                </p>
                <button
                  onClick={() => {
                    setTarget(nextTarget);
                    setNextTarget(undefined);
                    setDirty(false);
                    setTab('chat');
                  }}
                >
                  Switch conversation
                </button>
                <button onClick={() => setNextTarget(undefined)}>Keep this conversation</button>
              </section>
            )}
            <div role="tabpanel" aria-label="Chat" hidden={tab !== 'chat' || Boolean(nextTarget)}>
              <AgentConversation
                key={targetKey(target)}
                target={target}
                runtime={runtime}
                compact
                refreshHost={refreshHost}
                active={visible && tab === 'chat' && !nextTarget}
                reading={visible && tab === 'chat' && !nextTarget}
                onDirtyChange={setDirty}
              />
            </div>
            <div role="tabpanel" aria-label="Info" hidden={tab !== 'info' || Boolean(nextTarget)}>
              {contents.info(target)}
              <button className="agent-info-message" onClick={() => setTab('chat')}>
                Message {target.name}
              </button>
            </div>
          </section>
        </aside>
      </div>
    );
  }
  return { open, render, visible, identityId: visible ? target?.id : undefined };
}
