import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { useCallback, useEffect, useRef, useState, type Ref } from 'react';
import { LexicalExtensionComposer } from '@lexical/react/LexicalExtensionComposer';
import { ReactExtension } from '@lexical/react/ReactExtension';
import { ContentEditable } from '@lexical/react/LexicalContentEditable';
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext';
import { PlainTextExtension } from '@lexical/plain-text';
import { HistoryExtension } from '@lexical/history';
import {
  $createTextNode,
  $getSelection,
  $isRangeSelection,
  CLEAR_HISTORY_COMMAND,
  defineExtension,
  HISTORY_PUSH_TAG,
} from 'lexical';
import { Listbox } from './listbox.js';
import type { MessageComposerProps } from './message-composer-edit.js';
import {
  $clearMessageMentions,
  $messageCaret,
  $messageMention,
  $messageText,
  $replaceMessage,
  $selectMessageRange,
  fuzzyMessageCandidates,
  mentionQuery,
  MessageMentionNode,
} from './message-composer-document.js';
import './message-composer.css';

// Module identity stays stable; each mounted composer receives its own editor/history.
const extension = defineExtension({
  name: 'Colab/MessageComposer',
  namespace: 'ColabMessage',
  nodes: [MessageMentionNode],
  dependencies: [PlainTextExtension, HistoryExtension, ReactExtension],
  $initialEditorState: () => $replaceMessage(''),
  onError: (error: Error) => {
    throw error;
  },
});

/** One plaintext editing boundary; the parent owns draft, recipient, admission and effects. */
export function MessageComposer(props: MessageComposerProps) {
  return (
    <LexicalExtensionComposer extension={extension} contentEditable={null}>
      <MessageField {...props} />
    </LexicalExtensionComposer>
  );
}

function MessageField(props: MessageComposerProps) {
  const [editor] = useLexicalComposerContext();
  const current = useRef(props);
  current.current = props;
  const recipient = useRef(props.edit.recipient);
  recipient.current = props.edit.recipient;
  const [{ query, open, explicit }, setSuggestions] = useState<{
    query?: ReturnType<typeof mentionQuery>;
    open: boolean;
    explicit?: boolean;
  }>({ open: false });
  const setOpen = useCallback((open: boolean) => {
    setSuggestions((previous) => (previous.open === open ? previous : { ...previous, open }));
  }, []);
  const reset = useRef(props.resetKey);
  useEffect(() => {
    // Lexical sets the caret; DOM focus must also leave the sandboxed renderer.
    if (props.autoFocus)
      editor.focus(() => editor.getRootElement()?.focus({ preventScroll: true }));
  }, [editor, props.autoFocus]);
  const candidates = fuzzyMessageCandidates(
    props.candidates ?? [],
    explicit ? '' : (query?.query ?? ''),
  );
  const options = candidates.map((agent) => ({
    value: `${agent.machine}:${agent.agent}`,
    label: `@${agent.agentName} · ${agent.machineName}`,
    disabled: agent.online !== 'online' || agent.presence === 'offline',
  }));
  const wasDisabled = useRef(props.disabled);
  useEffect(() => {
    const completed = wasDisabled.current && !props.disabled;
    wasDisabled.current = props.disabled;
    editor.setEditable(!props.disabled);
    if (
      completed &&
      props.autoFocus &&
      editor.getRootElement()?.closest('dialog[open]') &&
      document.activeElement === document.body
    )
      editor.focus();
  }, [editor, props.disabled, props.autoFocus]);
  useEffect(() => {
    const explicit = reset.current !== props.resetKey;
    reset.current = props.resetKey;
    const value = editor.getEditorState().read($messageText);
    if (explicit || value !== props.edit.value) {
      editor.update(() => $replaceMessage(props.edit.value, props.edit.mention), {
        tag: 'parent-message-reset',
        discrete: true,
      });
      setOpen(false);
      editor.dispatchCommand(CLEAR_HISTORY_COMMAND, undefined);
    }
  }, [editor, props.edit.value, props.edit.mention, props.resetKey, setOpen]);
  useEffect(() => {
    const transform = editor.registerNodeTransform(MessageMentionNode, (node) => {
      if (node.getTextContent() !== node.__token)
        node.replace($createTextNode(node.getTextContent()));
    });
    const listener = editor.registerUpdateListener(
      ({ editorState, tags, dirtyElements, dirtyLeaves }) => {
        editorState.read(() => {
          const value = $messageText();
          const caret = $messageCaret();
          const nextQuery = caret === undefined ? undefined : mentionQuery(value, caret);
          const mention = $messageMention();
          const previous = current.current.edit;
          // Selection/normalization updates are not parent draft edits.
          const changed =
            !tags.has('parent-message-reset') &&
            !!(dirtyElements.size || dirtyLeaves.size) &&
            (value !== previous.value ||
              recipient.current?.machine !== previous.recipient?.machine ||
              recipient.current?.agent !== previous.recipient?.agent ||
              mention?.key.machine !== previous.mention?.key.machine ||
              mention?.key.agent !== previous.mention?.key.agent ||
              mention?.range.start !== previous.mention?.range.start ||
              mention?.range.end !== previous.mention?.range.end);
          if (changed)
            current.current.onChange({
              value,
              recipient: recipient.current,
              ...(mention ? { mention } : {}),
            });
          setSuggestions((previous) => {
            const queryChanged =
              nextQuery?.start !== previous.query?.start ||
              nextQuery?.end !== previous.query?.end ||
              nextQuery?.query !== previous.query?.query;
            // DOM input can commit text before its caret. A newly admitted query opens
            // once selection arrives; Escape stays closed while that query is unchanged.
            const open = tags.has('parent-message-reset')
              ? false
              : queryChanged || changed
                ? nextQuery !== undefined
                : previous.open;
            return queryChanged || open !== previous.open || (changed && previous.explicit)
              ? { query: nextQuery, open, explicit: changed ? false : previous.explicit }
              : previous;
          });
        });
      },
    );
    return () => {
      transform();
      listener();
    };
  }, [editor]);
  function choose(key: string) {
    const agent = candidates.find((candidate) => `${candidate.machine}:${candidate.agent}` === key);
    if (!agent || props.disabled) return;
    const chosen = { machine: agent.machine, agent: agent.agent };
    recipient.current = chosen;
    // A picker without an @ query changes only the explicit recipient.
    if (explicit || !query) {
      props.onChange({ ...props.edit, recipient: chosen });
      setOpen(false);
      return;
    }
    editor.update(
      () => {
        $clearMessageMentions();
        const selection = $selectMessageRange(query.start, query.end);
        const token = `@${agent.agentName}`;
        const mention = new MessageMentionNode(token, chosen);
        selection.insertNodes([mention, $createTextNode(' ')]);
        const active = $getSelection();
        if ($isRangeSelection(active)) active.format = 0;
      },
      { tag: HISTORY_PUSH_TAG, discrete: true },
    );
    setOpen(false);
  }
  return (
    <>
      <Listbox
        label={props.label}
        options={options}
        value={
          props.edit.recipient
            ? `${props.edit.recipient.machine}:${props.edit.recipient.agent}`
            : ''
        }
        disabled={props.disabled}
        onChange={choose}
        inputTrigger={{
          open: open && options.length > 0 && !props.disabled,
          onOpenChange: setOpen,
          render: (trigger) => (
            <ContentEditable
              {...trigger}
              onKeyDown={undefined}
              placeholder={null}
              aria-placeholder={undefined}
              className="message-composer-field"
              aria-multiline="true"
              aria-label={props.label}
              ref={trigger.ref as Ref<HTMLDivElement>}
              data-placeholder={props.placeholder}
              onKeyDownCapture={(event) => {
                const native = event.nativeEvent;
                if (
                  !native.isTrusted ||
                  native.isComposing ||
                  editor.isComposing() ||
                  native.keyCode === 229 ||
                  props.disabled
                )
                  return;
                trigger.onKeyDown?.(event);
                if (event.defaultPrevented) return;
                if (event.key === 'Enter' && !event.shiftKey && props.onSubmit) {
                  event.preventDefault();
                  event.stopPropagation();
                  props.onSubmit(native);
                } else if (event.key === 'Escape' && props.onCancel) {
                  event.preventDefault();
                  event.stopPropagation();
                  props.onCancel(native);
                }
              }}
            />
          ),
        }}
      />
      {props.recipientPickerLabel && (
        <button
          className={ui.action}
          type="button"
          disabled={
            props.disabled ||
            !props.candidates?.some(
              (candidate) => candidate.online === 'online' && candidate.presence !== 'offline',
            )
          }
          onClick={(event) => {
            if (!event.isTrusted) return;
            editor.focus();
            setSuggestions((previous) => ({ ...previous, open: true, explicit: true }));
          }}
        >
          {props.recipientPickerLabel}
        </button>
      )}
    </>
  );
}

export type { ComposerEdit, MessageComposerProps, RecipientKey } from './message-composer-edit.js';
