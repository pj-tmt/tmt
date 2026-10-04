import { useEffect, useRef, useState } from 'react';
import { Listbox } from './components/listbox.js';
import type { AskBinding, PageAsk } from './ask-panel.js';
import type { AgentDestination } from './live-ask.js';
import type { ThreadBinding } from './thread-store.js';
import { conversationText, captureConversation } from './thread-store.js';
import type { DiscussionRef, QuoteSelector, ThreadView } from './thread-records.js';
import { formatAskMessage } from './ask-intent.js';

export function publishingDestination(agents: readonly AgentDestination[], publisher?: string) {
  const matches = agents.filter(
    (agent) =>
      agent.agentName === publisher && agent.online === 'online' && agent.presence !== 'offline',
  );
  return matches.length === 1 ? matches[0] : undefined;
}
function destinationKey(agent: AgentDestination) {
  return `${agent.machine}:${agent.agent}`;
}
export function mentionedDestination(
  value: string,
  agents: readonly AgentDestination[],
  selected?: string,
) {
  const matches = agents.filter(
    (agent) =>
      value.startsWith(`@${agent.agentName}`) &&
      /^\s/.test(value.slice(agent.agentName.length + 1)),
  );
  const longest = Math.max(0, ...matches.map((agent) => agent.agentName.length));
  const exact = matches.filter((agent) => agent.agentName.length === longest);
  return exact.length === 1 ? exact[0] : exact.find((agent) => destinationKey(agent) === selected);
}

/** One trusted input. Enter is the explicit effect; disclosure never prepares or sends an intent. */
export function AnnotationInput({
  binding,
  discussion,
  anchor,
  thread,
  asks,
  title,
  publisher,
  blocked,
  cancel,
  committed,
  chat = false,
}: {
  binding?: AskBinding;
  discussion?: ThreadBinding;
  anchor: QuoteSelector | null;
  thread?: ThreadView;
  asks: readonly PageAsk[];
  title: string;
  publisher?: string;
  blocked: boolean;
  cancel(): void;
  committed(ref: DiscussionRef): void;
  chat?: boolean;
}) {
  const inputElement = useRef<HTMLTextAreaElement | null>(null);
  const [agents, setAgents] = useState<AgentDestination[]>([]);
  const [value, setValue] = useState('');
  const [selected, setSelected] = useState<string>();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const sending = useRef(false);
  const dirty = useRef(false);
  const wasBusy = useRef(false);
  const [error, setError] = useState<string>();
  const [delivered, setDelivered] = useState<string>();
  const [recorded, setRecorded] = useState<DiscussionRef>();
  useEffect(() => {
    let active = true;
    if (!binding) return;
    void binding.destinations().then(
      (destinations) => {
        if (!active) return;
        setAgents(destinations);
        const target = publishingDestination(destinations, publisher);
        if (target && !dirty.current) {
          setSelected(destinationKey(target));
          setValue(`@${target.agentName} `);
        }
      },
      () => {
        if (active) setError('Agents are unavailable.');
      },
    );
    return () => {
      active = false;
    };
  }, [binding, publisher]);
  useEffect(() => {
    if (!dirty.current && inputElement.current) {
      inputElement.current.setSelectionRange(value.length, value.length);
    }
  }, [value]);
  useEffect(() => {
    const completed = wasBusy.current && !busy;
    wasBusy.current = busy;
    const input = inputElement.current;
    if (
      chat &&
      completed &&
      !blocked &&
      !recorded &&
      input &&
      input.closest('dialog[open]') &&
      document.activeElement === document.body
    )
      input.focus({ preventScroll: true });
  }, [busy, chat, blocked, recorded]);
  const destination = mentionedDestination(value, agents, selected);
  const quote = thread?.anchor?.exact ?? anchor?.exact ?? '';
  const comment = thread ? conversationText(thread.comments, thread.threadId, value, asks) : value;
  const disclosure =
    delivered ??
    (destination
      ? `[remote: ${destination.deviceName}]\n${formatAskMessage({ title, url: location.href, quote, comment })}`
      : 'Choose a recipient by typing @.');
  const query = /^@([^\n]*)$/.exec(value)?.[1] ?? '';
  const options = agents
    .filter((agent) => agent.agentName.toLowerCase().startsWith(query.toLowerCase()))
    .map((agent) => ({
      value: destinationKey(agent),
      label: `@${agent.agentName} · ${agent.machineName}`,
      disabled: agent.online === 'offline' || agent.presence === 'offline',
    }));
  async function send() {
    if (sending.current || recorded || blocked || !binding || !discussion) return;
    if (!destination || !value.slice(destination.agentName.length + 1).trim()) {
      setError('Start with @agent and write a message.');
      return;
    }
    const captured = {
      value,
      destination: structuredClone(destination),
      anchor: structuredClone(anchor),
      thread: thread?.ref,
      threadRevision: thread?.revision,
      title,
      conversation: captureConversation(thread, asks),
      url: location.href,
    };
    sending.current = true;
    setBusy(true);
    setError(undefined);
    let origin: Awaited<ReturnType<ThreadBinding['create']>> | undefined;
    try {
      origin = captured.thread
        ? await discussion.reply(captured.thread, captured.value, captured.threadRevision)
        : chat
          ? await discussion.createChat(captured.value)
          : await discussion.create(captured.value, captured.anchor);
      const attempt = await binding.prepare({
        quote,
        comment: captured.value,
        title: captured.title,
        url: captured.url,
        destination: captured.destination,
        context: { ...origin, conversation: captured.conversation },
      });
      setDelivered(attempt.preview.view.deliveredMessage);
      const outcome = await attempt.send();
      if (!['accepted', 'held', 'uncertain'].includes(outcome.state))
        setError(`Send ${outcome.state}. The recorded turn was kept.`);
      setValue(chat ? `@${captured.destination.agentName} ` : '');
      committed(origin.thread);
    } catch {
      setError(
        origin
          ? 'The turn was recorded, but delivery is unavailable or uncertain. Check the thread before sending again.'
          : 'The turn could not be recorded.',
      );
      if (origin) {
        setValue('');
        setRecorded(origin.thread);
      }
    } finally {
      sending.current = false;
      setBusy(false);
    }
  }
  return (
    <section className="annotation-compose" data-testid="annotation-compose">
      <Listbox
        label="Message to agent"
        options={options}
        value={selected ?? ''}
        disabled={busy || blocked || !!recorded}
        onChange={(key) => {
          const agent = agents.find((candidate) => destinationKey(candidate) === key);
          if (agent) {
            dirty.current = true;
            setSelected(key);
            setValue(`@${agent.agentName} `);
            setOpen(false);
          }
        }}
        inputTrigger={{
          open: open && options.length > 0 && !busy && !blocked && !recorded,
          onOpenChange: setOpen,
          render: (props) => (
            <textarea
              {...props}
              ref={(node) => {
                inputElement.current = node;
                if (typeof props.ref === 'function') props.ref(node);
              }}
              autoFocus
              rows={3}
              value={value}
              disabled={busy || blocked || !!recorded || !binding || !discussion}
              placeholder="@agent Write a message…"
              onChange={(event) => {
                dirty.current = true;
                setValue(event.target.value);
                setDelivered(undefined);
                setOpen(/^@[^\s]*$/.test(event.target.value));
              }}
              onKeyDown={(event) => {
                if (!event.isTrusted || event.nativeEvent.isComposing) return;
                props.onKeyDown?.(event);
                if (event.defaultPrevented) return;
                if (event.key === 'Enter' && !event.shiftKey) {
                  event.preventDefault();
                  void send();
                } else if (event.key === 'Escape' && !sending.current) {
                  event.preventDefault();
                  event.stopPropagation();
                  cancel();
                }
              }}
            />
          ),
        }}
      />
      <p className="annotation-hint">
        {busy ? 'Sending…' : 'Enter sends · Shift+Enter adds a line · Esc cancels'}
      </p>
      <details>
        <summary>Show exactly what is sent</summary>
        <pre data-testid="annotation-exact-bytes">{disclosure}</pre>
      </details>
      {error && <p role="alert">{error}</p>}
      {recorded && (chat || !thread) && (
        <button
          type="button"
          onClick={(event) => {
            if (event.isTrusted) {
              if (chat) {
                setRecorded(undefined);
                setError(undefined);
                setValue(destination ? `@${destination.agentName} ` : '');
              } else committed(recorded);
            }
          }}
        >
          {chat ? 'Write another message' : 'Open recorded thread'}
        </button>
      )}
    </section>
  );
}
