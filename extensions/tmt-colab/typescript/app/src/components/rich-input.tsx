import { BrowserField } from '@tmt/browser-ui/react';
import {
  useEffect,
  useId,
  useRef,
  type ComponentProps,
  type HTMLAttributes,
  type KeyboardEvent,
  type ReactNode,
  type Ref,
} from 'react';
import { LexicalExtensionComposer } from '@lexical/react/LexicalExtensionComposer';
import { ReactExtension } from '@lexical/react/ReactExtension';
import { ContentEditable } from '@lexical/react/LexicalContentEditable';
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext';
import { PlainTextExtension } from '@lexical/plain-text';
import { HistoryExtension } from '@lexical/history';
import { CLEAR_HISTORY_COMMAND, defineExtension, SKIP_DOM_SELECTION_TAG } from 'lexical';
import { $richInputText, $replaceRichInput } from './rich-input-document.js';
import './rich-input.css';

export const RICH_INPUT_RESET_TAG = 'parent-rich-input-reset';

export type RichInputEdit = { value: string };
export type RichInputDocument<Edit extends RichInputEdit> = {
  /** Runs inside an editor-state read; caller tokens remain caller-owned. */
  read(previous: Edit): Edit;
  replace(edit: Edit): void;
  equal(a: Edit, b: Edit): boolean;
};
export type RichInputControlProps = {
  label: string;
  className?: string;
  placeholder?: string;
  disabled: boolean;
  describedBy?: string;
  /** Presentation only, inside the input below text; grants no editor or send authority. */
  accessory?: ReactNode;
  trigger?: HTMLAttributes<HTMLElement> & { ref: Ref<HTMLElement> };
  onKeyDownCapture?(event: KeyboardEvent<HTMLDivElement>): void;
};
export type RichInputProps<Edit extends RichInputEdit> = RichInputControlProps & {
  edit: Edit;
  onChange(edit: Edit): void;
  autoFocus?: boolean;
  resetKey?: string | number;
  document: RichInputDocument<Edit>;
  extension?: ComponentProps<typeof LexicalExtensionComposer>['extension'];
  children?: ReactNode;
};

export const plainRichInputDocument: RichInputDocument<RichInputEdit> = {
  read: () => ({ value: $richInputText() }),
  replace: (edit) => $replaceRichInput(edit.value),
  equal: (a, b) => a.value === b.value,
};

// Stable module identity; every mount owns its own editor and undo history.
export const richInputExtension = defineExtension({
  name: 'Colab/RichInput',
  namespace: 'ColabRichInput',
  dependencies: [PlainTextExtension, HistoryExtension, ReactExtension],
  $initialEditorState: () => $replaceRichInput(''),
  onError: (error: Error) => {
    throw error;
  },
});

/** The shared editing lifetime. Callers own token plugins and every effect outside editing. */
export function RichInput<Edit extends RichInputEdit>(props: RichInputProps<Edit>) {
  return (
    <LexicalExtensionComposer
      extension={props.extension ?? richInputExtension}
      contentEditable={null}
    >
      <EditingLifetime {...props} />
      {props.children ?? <RichInputControl {...props} />}
    </LexicalExtensionComposer>
  );
}
function EditingLifetime<Edit extends RichInputEdit>(props: RichInputProps<Edit>) {
  const [editor] = useLexicalComposerContext();
  const current = useRef(props);
  current.current = props;
  const reset = useRef(props.resetKey);
  useEffect(() => {
    if (!props.autoFocus) return;
    // Inline editors can mount before Lexical attaches the editable root.
    return editor.registerRootListener((root) => {
      if (root) editor.focus(() => root.focus({ preventScroll: true }));
    });
  }, [editor, props.autoFocus]);
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
    const value = editor.getEditorState().read(() => props.document.read(props.edit));
    if (explicit || value.value !== props.edit.value) {
      editor.update(() => props.document.replace(props.edit), {
        // A background editor replacement must not move the active editor's caret.
        tag: editor.getRootElement()?.contains(document.activeElement)
          ? RICH_INPUT_RESET_TAG
          : [RICH_INPUT_RESET_TAG, SKIP_DOM_SELECTION_TAG],
        discrete: true,
      });
      editor.dispatchCommand(CLEAR_HISTORY_COMMAND, undefined);
    }
  }, [editor, props.edit, props.document, props.resetKey]);
  useEffect(
    () =>
      editor.registerUpdateListener(({ editorState, tags, dirtyElements, dirtyLeaves }) => {
        if (tags.has(RICH_INPUT_RESET_TAG) || !(dirtyElements.size || dirtyLeaves.size)) return;
        const props = current.current;
        const next = editorState.read(() => props.document.read(props.edit));
        if (!props.document.equal(next, props.edit)) props.onChange(next);
      }),
    [editor],
  );
  return null;
}

/** One editable surface and composition/trust guard, reused by each caller's keyboard policy. */
export function RichInputControl(props: RichInputControlProps) {
  const [editor] = useLexicalComposerContext();
  const controlId = useId();
  return (
    <BrowserField
      controlId={controlId}
      label={props.label}
      describedByIds={props.describedBy ? [props.describedBy] : []}
      renderControl={(field) => {
        const editable = (
          <ContentEditable
            {...props.trigger}
            {...field}
            onKeyDown={undefined}
            placeholder={null}
            aria-placeholder={undefined}
            className={`${field.className} rich-input-field ${props.className ?? ''}`}
            aria-multiline="true"
            aria-disabled={props.disabled}
            ref={props.trigger?.ref as Ref<HTMLDivElement>}
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
              props.onKeyDownCapture?.(event);
            }}
          />
        );
        // The surface never changes the editable's parent: adding/removing a row must
        // keep the mounted root, focus and DOM selection, including during typing.
        return (
          <div
            className="rich-input-surface"
            data-accessory={Boolean(props.accessory) || undefined}
            data-disabled={props.disabled || undefined}
          >
            {editable}
            {props.accessory && <div className="rich-input-accessory">{props.accessory}</div>}
          </div>
        );
      }}
    />
  );
}
