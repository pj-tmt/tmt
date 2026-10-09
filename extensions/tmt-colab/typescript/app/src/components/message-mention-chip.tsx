import { BrowserIconAction } from '@tmt/browser-ui/react';
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext';
import { $getNodeByKey, HISTORY_PUSH_TAG, type NodeKey } from 'lexical';
import { X } from 'lucide-react';
import { createContext, useContext } from 'react';
import type { AgentDestination } from '../live-ask.js';
import { text } from '../strings.js';
import type { RecipientKey } from './message-composer-edit.js';

/** Presentation follows this composer's current directory; node identity keeps routing UUIDs. */
export const MessageMentionContext = createContext<{
  candidates?: readonly AgentDestination[];
  disabled: boolean;
}>({ disabled: false });

export function MessageMentionChip({
  nodeKey,
  token,
  recipient,
}: {
  nodeKey: NodeKey;
  token: string;
  recipient: RecipientKey;
}) {
  const [editor] = useLexicalComposerContext();
  const { candidates, disabled } = useContext(MessageMentionContext);
  const matches = candidates?.filter(
    (agent) => agent.machine === recipient.machine && agent.agent === recipient.agent,
  );
  const agent = matches?.length === 1 ? matches[0] : undefined;
  const state = !candidates
    ? 'checking'
    : !agent
      ? 'not-found'
      : agent.online !== 'online' || agent.presence === 'offline'
        ? 'offline'
        : agent.presence === 'active'
          ? 'online'
          : 'unknown';
  const sameName =
    agent && candidates!.filter((other) => other.agentName === agent.agentName).length > 1;
  const description = text.messageMentionDescription(
    agent ? `@${agent.agentName}` : token,
    agent?.machineName,
    text.messageMentionState[state],
  );
  return (
    <span className="message-mention message-mention-chip" data-state={state} title={description}>
      <span className="message-mention-dot" aria-hidden="true" />
      <span className="message-mention-name">{token}</span>
      {sameName && <span className="message-mention-machine"> · {agent.machineName}</span>}
      <BrowserIconAction
        type="button"
        variant="text"
        label={text.messageMentionRemove(description)}
        icon={<X />}
        disabled={disabled}
        onActivate={(event) => {
          if (!event.isTrusted || !editor.isEditable()) return;
          editor.update(
            () => {
              const node = $getNodeByKey(nodeKey);
              if (!node) return;
              node.selectPrevious();
              node.remove();
            },
            { tag: HISTORY_PUSH_TAG, discrete: true },
          );
          editor.focus();
        }}
      />
    </span>
  );
}
