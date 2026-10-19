import { useCallback, useState } from 'react';
import {
  BrowserDrawer,
  BrowserMenu,
  BrowserNotifications,
  type BrowserNotification,
} from '../../src/react.js';

const replies = [
  { destination: 'Chat', firstLine: 'The revised layout is ready for your review.' },
  {
    destination: 'Thread',
    firstLine: 'I checked the proposed wording and added one example.',
    quote:
      'The discussion stays attached to the quoted passage, including this complete long quotation for the accessible text.',
  },
  { destination: 'Answer', firstLine: 'Your answer has a follow-up question.' },
  { destination: 'Proposal', firstLine: 'The proposal review is complete.' },
  { destination: 'Chat', firstLine: 'The last reply is waiting in the queue.' },
] as const;

/** Fake admission only: no product records, navigation, counts or bridge. */
export function Notifications() {
  const [events, setEvents] = useState<readonly BrowserNotification[]>([]);
  const [pageKey, setPageKey] = useState(0);
  const [drawer, setDrawer] = useState(false);
  const [menu, setMenu] = useState(false);
  const [mounted, setMounted] = useState(true);
  const [excluded, setExcluded] = useState<readonly string[]>([]);
  const [activations, setActivations] = useState<readonly string[]>([]);
  const [transitions, setTransitions] = useState<readonly boolean[]>([]);
  const [outerEscapes, setOuterEscapes] = useState(0);
  const eligible = useCallback(
    (event: BrowserNotification) => !excluded.includes(event.id),
    [excluded],
  );
  function admit(count: number) {
    setEvents(
      replies.slice(0, count).map((reply, index) => ({
        ...reply,
        id: String(index + 1),
        onActivate: (event) =>
          setActivations((previous) => [...previous, `${index + 1}:${event.isTrusted}`]),
      })),
    );
  }
  const report = useCallback((open: boolean) => {
    setMenu(open);
    setTransitions((previous) => [...previous, open]);
  }, []);
  return (
    <div
      data-testid="notification-fixture"
      onKeyDown={(event) => {
        if (event.key === 'Escape') setOuterEscapes((value) => value + 1);
      }}
    >
      <header className="tmt-ui-header" style={{ position: 'sticky', top: 0, zIndex: 2147483647 }}>
        <strong>Reply notifications</strong>
        <button type="button" onClick={() => setDrawer(true)}>
          Open drawer
        </button>
        <BrowserMenu
          label="Header menu"
          items={[{ key: 'one', label: 'One' }]}
          onSelect={() => {}}
          onOpenChange={report}
        />
      </header>
      <div style={{ padding: 12, maxWidth: 640 }}>
        <p>Fake reply sources</p>
        <button type="button" onClick={() => admit(1)}>
          Admit one
        </button>
        <button type="button" onClick={() => admit(3)}>
          Admit three
        </button>
        <button type="button" onClick={() => admit(5)}>
          Admit five
        </button>
        <button type="button" onClick={() => setExcluded(['1', '4'])}>
          Withdraw one and four
        </button>
        <button type="button" onClick={() => setExcluded(['2'])}>
          Withdraw two
        </button>
        <button
          type="button"
          onClick={() => {
            setPageKey((value) => value + 1);
            setExcluded([]);
          }}
        >
          New page
        </button>
        <button type="button" onClick={() => setMounted((value) => !value)}>
          Toggle notifications
        </button>
        <input aria-label="Retained focus" defaultValue="Keep typing here" />
        <p>
          Activations: <output data-testid="activations">{activations.join(',')}</output>
        </p>
        <p>
          Menu transitions:{' '}
          <output data-testid="transitions">{transitions.map(String).join(',')}</output>
        </p>
        <p>
          Outer Escapes: <output data-testid="outer-escapes">{outerEscapes}</output>
        </p>
      </div>
      <iframe
        title="Author content"
        srcDoc='<body style="margin:0;background:#ececec"><button onclick="document.body.dataset.presses = String(Number(document.body.dataset.presses || 0) + 1)" style="width:100%;height:600px">Author button</button></body>'
        style={{ width: '100%', height: 620, border: 0 }}
      />
      {mounted && (
        <BrowserNotifications
          pageKey={String(pageKey)}
          events={events}
          suspended={drawer || menu}
          isEligible={eligible}
        />
      )}
      <BrowserDrawer
        title="Notification suspension"
        panel="fixture"
        open={drawer}
        onDismiss={() => setDrawer(false)}
      >
        <p>Replies queue silently while this modal is open.</p>
        <button type="button" onClick={() => admit(5)}>
          Admit five
        </button>
        <button type="button" onClick={() => setExcluded(['1', '4'])}>
          Withdraw one and four
        </button>
        <BrowserMenu
          label="Drawer menu"
          items={[{ key: 'one', label: 'One' }]}
          onSelect={() => {}}
          onOpenChange={report}
        />
      </BrowserDrawer>
    </div>
  );
}
