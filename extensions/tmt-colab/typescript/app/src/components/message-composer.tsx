import { useCallback, useEffect, useRef, useState } from 'react';
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext';
import {
  $createTextNode,
  TextNode,
  $getSelection,
  $isRangeSelection,
  defineExtension,
  HISTORY_PUSH_TAG,
  KEY_DOWN_COMMAND,
  COMMAND_PRIORITY_HIGH,
} from 'lexical';
import { resolveMessageMentions } from '../message-recipient.js';
import { Listbox } from './listbox.js';
import type { MessageComposerProps } from './message-composer-edit.js';
import { MessageMentionContext } from './message-mention-chip.js';
import { MentionChip } from './mention-chip.js';
import {
  $bindMessageMentions,
  $messageCaret,
  $messageMentions,
  $messageSelection,
  $prependMessageMention,
  $messageText,
  $replaceMessage,
  $setDefaultCreator,
  $selectMessageRange,
  fuzzyMessageCandidates,
  mentionQuery,
  MessageMentionNode,
} from './message-composer-document.js';
import {
  RichInput,
  RichInputControl,
  richInputExtension,
  RICH_INPUT_RESET_TAG,
} from './rich-input.js';
import type { ComposerEdit } from './message-composer-edit.js';
import './message-composer.css';

// Module identity stays stable; each mounted composer receives its own editor/history.
const extension = defineExtension({
  name: 'Colab/MessageComposer',
  namespace: 'ColabMessage',
  nodes: [MessageMentionNode],
  dependencies: [richInputExtension],
  $initialEditorState: () => $replaceMessage(''),
  onError: (error: Error) => {
    throw error;
  },
});

/** One plaintext editing boundary; the parent owns draft, recipients, admission and effects. */
export function MessageComposer(props: MessageComposerProps) {
  return (
    <MessageMentionContext value={{ candidates: props.candidates, disabled: props.disabled }}>
      <RichInput {...props} extension={extension} document={messageDocument}>
        <MessageField {...props} />
      </RichInput>
    </MessageMentionContext>
  );
}

const messageDocument = {
  read(previous: ComposerEdit): ComposerEdit {
    return { value: $messageText(), mentions: $messageMentions(), edited: previous.edited };
  },
  replace(edit: ComposerEdit) {
    $replaceMessage(edit.value, edit.mentions);
  },
  equal(a: ComposerEdit, b: ComposerEdit) {
    return (
      a.value === b.value && JSON.stringify(a.mentions ?? []) === JSON.stringify(b.mentions ?? [])
    );
  },
};

function MessageField(props: MessageComposerProps) {
  const [editor] = useLexicalComposerContext();
  const current = useRef(props);
  current.current = props;
  const [{ query, open }, setSuggestions] = useState<{
    query?: ReturnType<typeof mentionQuery>;
    open: boolean;
  }>({ open: false });
  const setOpen = useCallback((open: boolean) => {
    setSuggestions((previous) => (previous.open === open ? previous : { ...previous, open }));
  }, []);
  const activated = useRef<string | undefined>(undefined);
  const lastSelection = useRef<ReturnType<typeof $messageSelection>>(undefined);
  const mentionRequest = useRef<string | undefined>(undefined);
  useEffect(() => {
    const request = props.prependMention;
    if (!request || mentionRequest.current === request.id) return;
    mentionRequest.current = request.id;
    const matches = props.candidates?.filter(
      (candidate) =>
        candidate.machine === request.key.machine && candidate.agent === request.key.agent,
    );
    // A stale click never waits to apply after editing or recipient admission returns.
    if (!props.disabled && !editor.isComposing() && matches?.length === 1) {
      editor.update(
        () => {
          const selection = lastSelection.current;
          if (!$messageSelection() && selection)
            $selectMessageRange(selection.anchor, selection.focus);
          $prependMessageMention(request.key, request.label);
        },
        { tag: HISTORY_PUSH_TAG, discrete: true },
      );
      editor.getRootElement()?.focus({ preventScroll: true });
    }
    request.consumed();
  }, [editor, props.prependMention, props.candidates, props.disabled]);
  const activationId = props.activation?.id;
  useEffect(() => {
    if (!activationId || activated.current === activationId) return;
    let frame: number | undefined;
    let cancelled = false;
    const dispose = editor.registerRootListener((root) => {
      if (!root) return;
      if (frame !== undefined) cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        // Wait for the existing drawer's native open and focus steps.
        const admitted = () =>
          !cancelled &&
          root.isConnected &&
          !!root.closest('dialog[open]') &&
          !current.current.disabled &&
          current.current.activation?.id === activationId;
        if (!admitted()) return;
        editor.focus(() => {
          if (!admitted() || activated.current === activationId) return;
          const request = current.current.activation!;
          activated.current = activationId;
          if (request.insertMention)
            editor.update(
              () => {
                let selection = $getSelection();
                if (!$isRangeSelection(selection)) {
                  $selectMessageRange($messageText().length, $messageText().length);
                  selection = $getSelection();
                }
                if ($isRangeSelection(selection)) selection.insertText('@');
              },
              { tag: HISTORY_PUSH_TAG, discrete: true },
            );
          root.focus({ preventScroll: true });
          request.consumed();
        });
      });
    });
    return () => {
      cancelled = true;
      dispose();
      if (frame !== undefined) cancelAnimationFrame(frame);
    };
  }, [editor, activationId]);
  const preferenceRevision = useRef(props.creatorDefault?.revision ?? 0);
  useEffect(() => {
    const preference = props.creatorDefault;
    if (!preference || props.disabled || preferenceRevision.current === preference.revision) return;
    preferenceRevision.current = preference.revision;
    editor.update(() => $setDefaultCreator(preference.key, preference.label, preference.enabled), {
      tag: HISTORY_PUSH_TAG,
      discrete: true,
    });
  }, [editor, props.creatorDefault, props.disabled]);
  const candidates = fuzzyMessageCandidates(props.candidates ?? [], query?.query ?? '');
  const options = candidates.map((agent) => ({
    value: `${agent.machine}:${agent.agent}`,
    label: `@${agent.agentName} · ${agent.machineName}${agent.presence === 'offline' ? ' · offline' : ''}`,
    disabled: agent.online !== 'online',
  }));
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
          if (tags.has(RICH_INPUT_RESET_TAG)) lastSelection.current = undefined;
          const selection = $messageSelection();
          if (selection) lastSelection.current = selection;
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
            !tags.has(RICH_INPUT_RESET_TAG) &&
            !!(dirtyElements.size || dirtyLeaves.size) &&
            !messageDocument.equal({ value, mentions }, previous);
          // The IME owns its transient selection; keep the open list's query/highlight until commit.
          if (editor.isComposing()) return;
          setSuggestions((previous) => {
            const queryChanged =
              nextQuery?.start !== previous.query?.start ||
              nextQuery?.end !== previous.query?.end ||
              nextQuery?.query !== previous.query?.query;
            // DOM input can commit text before its caret. A newly admitted query opens
            // once selection arrives; Escape stays closed while that query is unchanged.
            const open = tags.has(RICH_INPUT_RESET_TAG)
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
        renderOption={(option) => {
          const agent = candidates.find(
            (candidate) => `${candidate.machine}:${candidate.agent}` === option.value,
          );
          return agent ? (
            <>
              <MentionChip token={`@${agent.agentName}`} />
              <span>
                {' '}
                · {agent.machineName}
                {agent.presence === 'offline' ? ' · offline' : ''}
              </span>
            </>
          ) : (
            option.label
          );
        }}
        value=""
        disabled={props.disabled}
        onChange={choose}
        inputTrigger={{
          open: open && options.length > 0 && !props.disabled,
          onOpenChange: setOpen,
          render: (trigger) => (
            <RichInputControl
              {...props}
              className="message-composer-field"
              trigger={trigger}
              onKeyDownCapture={(event) => {
                const native = event.nativeEvent;
                // Chip actions keep their native keyboard activation; Enter here must not Send.
                if ((event.target as HTMLElement).closest('.message-mention')) {
                  return;
                }
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
          ),
        }}
      />
    </>
  );
}

export type {
  ComposerActivation,
  ComposerEdit,
  MessageComposerProps,
  RecipientKey,
} from './message-composer-edit.js';
