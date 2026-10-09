import { BrowserField } from '@tmt/browser-ui/react';
import { useCallback, useEffect, useId, useRef, useState, type Ref } from 'react';
import { LexicalExtensionComposer } from '@lexical/react/LexicalExtensionComposer';
import { ReactExtension } from '@lexical/react/ReactExtension';
import { ContentEditable } from '@lexical/react/LexicalContentEditable';
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext';
import { PlainTextExtension } from '@lexical/plain-text';
import { HistoryExtension } from '@lexical/history';
import {
  $createTextNode,
  TextNode,
  $getSelection,
  $isRangeSelection,
  CLEAR_HISTORY_COMMAND,
  defineExtension,
  HISTORY_PUSH_TAG,
  KEY_DOWN_COMMAND,
  COMMAND_PRIORITY_HIGH,
} from 'lexical';
import { resolveMessageMentions } from '../message-recipient.js';
import { Listbox } from './listbox.js';
import type { MessageComposerProps } from './message-composer-edit.js';
import { MessageMentionContext } from './message-mention-chip.js';
import {
  $bindMessageMentions,
  $messageCaret,
  $messageMentions,
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

/** One plaintext editing boundary; the parent owns draft, recipients, admission and effects. */
export function MessageComposer(props: MessageComposerProps) {
  return (
    <MessageMentionContext value={{ candidates: props.candidates, disabled: props.disabled }}>
      <LexicalExtensionComposer extension={extension} contentEditable={null}>
        <MessageField {...props} />
      </LexicalExtensionComposer>
    </MessageMentionContext>
  );
}

function MessageField(props: MessageComposerProps) {
  const [editor] = useLexicalComposerContext();
  const controlId = useId();
  const current = useRef(props);
  current.current = props;
  const [{ query, open }, setSuggestions] = useState<{
    query?: ReturnType<typeof mentionQuery>;
    open: boolean;
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
  const candidates = fuzzyMessageCandidates(props.candidates ?? [], query?.query ?? '');
  const options = candidates.map((agent) => ({
    value: `${agent.machine}:${agent.agent}`,
    label: `@${agent.agentName} · ${agent.machineName}${agent.presence === 'offline' ? ' · offline' : ''}`,
    disabled: agent.online !== 'online',
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
      editor.update(() => $replaceMessage(props.edit.value, props.edit.mentions), {
        tag: 'parent-message-reset',
        discrete: true,
      });
      setOpen(false);
      editor.dispatchCommand(CLEAR_HISTORY_COMMAND, undefined);
    }
  }, [editor, props.edit.value, props.edit.mentions, props.resetKey, setOpen]);
  useEffect(() => {
    const chipKeys = editor.registerCommand(
      KEY_DOWN_COMMAND,
      (event) =>
        event.target instanceof HTMLElement && !!event.target.closest('.message-mention-chip'),
      COMMAND_PRIORITY_HIGH,
    );
    const bind = editor.registerNodeTransform(TextNode, () => {
      if (!editor.isComposing() && current.current.candidates)
        $bindMessageMentions(current.current.candidates);
    });
    const listener = editor.registerUpdateListener(
      ({ editorState, tags, dirtyElements, dirtyLeaves }) => {
        editorState.read(() => {
          const value = $messageText();
          const caret = $messageCaret();
          const mentions = $messageMentions();
          const ambiguous = resolveMessageMentions({ value, mentions }, current.current.candidates)
            .ambiguousTokens[0];
          const nextQuery =
            (caret === undefined ? undefined : mentionQuery(value, caret)) ??
            (ambiguous ? { ...ambiguous.range, query: ambiguous.name } : undefined);
          const previous = current.current.edit;
          // Selection/normalization updates are not parent draft edits.
          const changed =
            !tags.has('parent-message-reset') &&
            !!(dirtyElements.size || dirtyLeaves.size) &&
            (value !== previous.value ||
              JSON.stringify(mentions) !== JSON.stringify(previous.mentions ?? []));
          if (changed)
            current.current.onChange({
              value,
              mentions,
              edited: previous.edited,
            });
          // The IME owns its transient selection; keep the open list's query/highlight until commit.
          if (editor.isComposing()) return;
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
            return queryChanged || open !== previous.open ? { query: nextQuery, open } : previous;
          });
        });
      },
    );
    return () => {
      chipKeys();
      bind();
      listener();
    };
  }, [editor]);
  function choose(key: string) {
    const agent = candidates.find((candidate) => `${candidate.machine}:${candidate.agent}` === key);
    if (!agent || props.disabled) return;
    const chosen = { machine: agent.machine, agent: agent.agent };

    if (!query) return;
    editor.update(
      () => {
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
        value=""
        disabled={props.disabled}
        onChange={choose}
        inputTrigger={{
          open: open && options.length > 0 && !props.disabled,
          onOpenChange: setOpen,
          render: (trigger) => (
            <BrowserField
              controlId={controlId}
              label={props.label}
              describedByIds={props.describedBy ? [props.describedBy] : []}
              renderControl={(field) => (
                <ContentEditable
                  {...trigger}
                  {...field}
                  onKeyDown={undefined}
                  placeholder={null}
                  aria-placeholder={undefined}
                  className={`${field.className} message-composer-field`}
                  aria-multiline="true"
                  aria-disabled={props.disabled}
                  ref={trigger.ref as Ref<HTMLDivElement>}
                  data-placeholder={props.placeholder}
                  onKeyDownCapture={(event) => {
                    const native = event.nativeEvent;
                    // Chip actions keep their native keyboard activation; Enter here must not Send.
                    if ((event.target as HTMLElement).closest('.message-mention')) {
                      return;
                    }
                    if (
                      !native.isTrusted ||
                      native.isComposing ||
                      editor.isComposing() ||
                      native.keyCode === 229 ||
                      props.disabled
                    )
                      return;
                    if (trigger['aria-expanded'] === true) {
                      trigger.onKeyDown?.(event);
                      if (event.defaultPrevented) return;
                    }
                    if (event.key === 'Enter' && !event.shiftKey) {
                      let next = props.edit;
                      editor.update(
                        () => {
                          next = {
                            ...$bindMessageMentions(props.candidates ?? [], true),
                            edited: props.edit.edited,
                          };
                        },
                        { discrete: true },
                      );
                      const resolution = resolveMessageMentions(next, props.candidates);
                      if (resolution.ambiguous.length) {
                        const ambiguous = resolution.ambiguousTokens[0];
                        setSuggestions({
                          open: true,
                          query: { ...ambiguous.range, query: ambiguous.name },
                        });
                        event.preventDefault();
                        event.stopPropagation();
                        return;
                      }
                      if (props.onSubmit) {
                        event.preventDefault();
                        event.stopPropagation();
                        setOpen(false);
                        props.onSubmit(native, next);
                        return;
                      }
                    }
                    if (event.key === 'Escape' && props.onCancel) {
                      event.preventDefault();
                      event.stopPropagation();
                      props.onCancel(native);
                    }
                  }}
                />
              )}
            />
          ),
        }}
      />
    </>
  );
}

export type { ComposerEdit, MessageComposerProps, RecipientKey } from './message-composer-edit.js';
