/** Injected host port exercises pending activation without rewriting Send owners. */
import { createRoot } from 'react-dom/client';
import { useLayoutEffect, useRef, useState } from 'react';
import { BrowserNotifications } from '@tmt/browser-ui/react';
import { ReplyNotificationObserver, type ReplySources } from '../src/reply-notifications.js';
import { useReplyNotifications } from '../src/use-reply-notifications.js';
import { id } from './ask-fixtures.js';
import { ReplyNotificationContext } from '../src/reply-notification-context.js';
import { useNotificationBlock } from '../src/use-notification-block.js';

let change: (action: string) => void;
const writer = id(4);
function sources(reply?: string): ReplySources {
  return {
    threads: [],
    asks: [
      {
        writer,
        operationId: id(21),
        requestId: `req_${id(22)}`,
        message: 'Own Ask',
        agent: id(6),
        agentName: 'Atlas',
        deviceName: 'You',
        machine: id(5),
        issuedAt: 1,
        state: 'accepted',
        canTrack: true,
        reply,
      },
    ],
  };
}
function Destination() {
  const block = useRef<HTMLElement>(null);
  useNotificationBlock('Chat', block, true);
  return (
    <section ref={block} tabIndex={-1} id="target">
      <p data-conversation-record={`reply:${writer}:${id(21)}`}>Target reply</p>
    </section>
  );
}
function Fixture() {
  const [pageKey, setPageKey] = useState('first');
  const [value, setValue] = useState<ReplySources>(() => sources());
  const [busy, setBusy] = useState(true);
  const [activations, setActivations] = useState(0);
  const observer = useRef(new ReplyNotificationObserver(sources()));
  const [showDestination, setShowDestination] = useState(false);
  useLayoutEffect(() => {
    change = (action) => {
      if (action === 'arrive') setValue(sources('Pending activation reply'));
      if (action === 'ready') setBusy(false);
      if (action === 'withdraw') setValue(sources());
      if (action === 'page') setPageKey('second');
      if (action === 'target') setShowDestination(true);
    };
  }, []);
  const notifications = useReplyNotifications({
    pageKey,
    observer: observer.current,
    sources: value,
    deviceId: writer,
    classification: { questionThreads: new Map() },
    blocked: false,
    activate: () => {
      if (busy) return false;
      setActivations((count) => count + 1);
      return true;
    },
  });
  return (
    <ReplyNotificationContext value={{ ...notifications.context, headerBottom: () => 0 }}>
      <BrowserNotifications
        pageKey={pageKey}
        events={notifications.events}
        suspended={false}
        isEligible={notifications.isEligible}
      />
      <output id="activations">{activations}</output>
      <output id="reveal">{notifications.context.reveal?.id ?? ''}</output>
      {showDestination && <Destination />}
    </ReplyNotificationContext>
  );
}
export function mount() {
  const node = document.createElement('div');
  document.body.replaceChildren(node);
  createRoot(node).render(<Fixture />);
}
export const control = (action: string) => change(action);
