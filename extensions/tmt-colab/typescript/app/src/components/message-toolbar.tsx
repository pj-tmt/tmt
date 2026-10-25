import { BrowserIconAction } from '@tmt/browser-ui/react';
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext';
import { HISTORY_PUSH_TAG } from 'lexical';
import { AtSign, Check, Sparkles } from 'lucide-react';
import { useState, type ReactNode, type Ref } from 'react';
import type { AgentDestination } from '../live-ask.js';
import { text } from '../strings.js';
import { AgentChip } from './agent-chip.js';
import { Listbox } from './listbox.js';
import { $insertMentionTrigger, $toggleMessageMention } from './message-composer-document.js';
import type { RecipientKey } from './message-composer-edit.js';
import './message-toolbar.css';

/** The agents this message asks; the pill shows them and the agent list changes them. */
export type AskedAgent = RecipientKey & { agentName: string };
const CREATOR_ROW = 'creator-start';
const key = (agent: RecipientKey) => `${agent.machine}:${agent.agent}`;

/**
 * The row inside the field under the text: Attach and Mention at the start; at the end who is
 * asked, then Send. Rendered inside the editor's composer, so a choice here edits the same
 * mention tokens the text holds and the pill never keeps a second list.
 */
export function MessageToolbar({
  attach,
  send,
  asked,
  candidates,
  creator,
  disabled,
}: {
  attach?: ReactNode;
  send: ReactNode;
  asked: readonly AskedAgent[];
  candidates?: readonly AgentDestination[];
  /** The page creator, when the composer starts a fresh message with their mention. */
  creator?: { name: string; enabled: boolean; toggle(): void };
  disabled: boolean;
}) {
  const [editor] = useLexicalComposerContext();
  const [open, setOpen] = useState(false);
  const askedKeys = new Set(asked.map(key));
  const pill = asked.length
    ? text.recipientAsks(asked.map((agent) => agent.agentName))
    : text.recipientCommentOnly;
  const options = [
    ...(candidates ?? []).map((agent) => ({
      value: key(agent),
      label: `@${agent.agentName}`,
      // An agent that cannot be asked can still be taken off.
      disabled: agent.online !== 'online' && !askedKeys.has(key(agent)),
      selected: askedKeys.has(key(agent)),
    })),
    ...(creator
      ? [
          {
            value: CREATOR_ROW,
            label: text.recipientCreatorStart(creator.name),
            selected: creator.enabled,
          },
        ]
      : []),
  ];
  const choose = (value: string) => {
    if (disabled) return;
    // The list hands focus back to its trigger; the writer carries on typing.
    queueMicrotask(() => editor.focus());
    if (value === CREATOR_ROW) {
      creator?.toggle();
      return;
    }
    const agent = candidates?.find((candidate) => key(candidate) === value);
    if (!agent) return;
    editor.update(
      () => $toggleMessageMention({ machine: agent.machine, agent: agent.agent }, agent.agentName),
      {
        tag: HISTORY_PUSH_TAG,
        discrete: true,
      },
    );
  };
  return (
    <div className="message-toolbar">
      <div className="message-toolbar-tools">
        {attach}
        <BrowserIconAction
          type="button"
          variant="text"
          label={text.messageMention}
          icon={<AtSign />}
          disabled={disabled}
          onActivate={(event) => {
            if (!event.isTrusted) return;
            editor.update($insertMentionTrigger, { tag: HISTORY_PUSH_TAG, discrete: true });
            editor.focus();
          }}
        />
      </div>
      <div className="message-toolbar-end">
        <Listbox
          label={text.recipientMenu}
          options={options}
          value=""
          multiple
          disabled={disabled || options.length === 0}
          onChange={choose}
          popup={{ width: 280, align: 'end', className: 'recipient-list' }}
          renderOption={(option) => {
            const agent = candidates?.find((candidate) => key(candidate) === option.value);
            return agent ? (
              <span className="recipient-row">
                <span
                  className="recipient-dot"
                  data-online={agent.online === 'online' || undefined}
                  aria-hidden
                />
                <AgentChip name={`@${agent.agentName}`} mark={false} machine={agent.machineName} />
                <span className="recipient-state">
                  {agent.online === 'online'
                    ? text.messageMentionState.online
                    : text.messageMentionState.offline}
                </span>
                {askedKeys.has(key(agent)) && <Check className="recipient-check" aria-hidden />}
              </span>
            ) : (
              <span
                className="recipient-row recipient-creator"
                data-on={creator?.enabled || undefined}
              >
                <span>{option.label}</span>
                {creator?.enabled && <Check className="recipient-check" aria-hidden />}
              </span>
            );
          }}
          inputTrigger={{
            open,
            onOpenChange: setOpen,
            render: (trigger) => (
              <button
                {...trigger}
                ref={trigger.ref as Ref<HTMLButtonElement>}
                role={undefined}
                type="button"
                className="recipient-pill"
                data-asking={asked.length > 0 || undefined}
                aria-label={`${text.recipientMenu}: ${pill}`}
                disabled={disabled || options.length === 0}
                onClick={(event) => {
                  if (event.isTrusted) setOpen(!open);
                }}
              >
                {asked.length > 0 && <Sparkles aria-hidden />}
                <span>{pill}</span>
              </button>
            ),
          }}
        />
        {send}
      </div>
    </div>
  );
}
