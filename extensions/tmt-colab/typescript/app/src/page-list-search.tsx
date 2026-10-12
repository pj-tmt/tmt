import { BrowserIconAction } from '@tmt/browser-ui/react';
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext';
import {
  $createTextNode,
  $getNodeByKey,
  $getSelection,
  $isRangeSelection,
  $isTextNode,
  DecoratorNode,
  defineExtension,
  HISTORY_PUSH_TAG,
  type LexicalNode,
  type NodeKey,
  type SerializedLexicalNode,
  type Spread,
} from 'lexical';
import { X } from 'lucide-react';
import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { Listbox } from './components/listbox.js';
import {
  RICH_INPUT_RESET_TAG,
  RichInput,
  RichInputControl,
  richInputExtension,
  type RichInputDocument,
} from './components/rich-input.js';
import {
  $replaceRichInput,
  $richInputCaret,
  $richInputText,
  $richInputTokens,
  $selectRichInputRange,
  type RichInputRange,
} from './components/rich-input-document.js';
import { text } from './strings.js';
import './page-list-search.css';

/** The search draft: the editor's text and its label tokens, each token's text being `label:<name>`. */
export interface SearchEdit {
  value: string;
  tokens: { name: string; range: RichInputRange }[];
}
const TRIGGER = 'label:';

/** Label tokens first, then the free text; the same shape the editor reads back. */
export function editFromSearch(labels: readonly string[], q: string): SearchEdit {
  let value = '';
  const tokens: SearchEdit['tokens'] = [];
  for (const name of new Set(labels)) {
    const token = `${TRIGGER}${name}`;
    tokens.push({ name, range: { start: value.length, end: value.length + token.length } });
    value += `${token} `;
  }
  return { value: value + q, tokens };
}
/** What the draft asks for: its tokens' names and the text outside them, whitespace collapsed. */
export function searchFromEdit(edit: SearchEdit): { labels: string[]; q: string } {
  let free = '';
  let at = 0;
  for (const token of edit.tokens) {
    free += `${edit.value.slice(at, token.range.start)} `;
    at = token.range.end;
  }
  free += edit.value.slice(at);
  return {
    labels: [...new Set(edit.tokens.map((token) => token.name))],
    q: free.split(/\s+/).filter(Boolean).join(' '),
  };
}
/** Add the label as a token, or drop it when the draft already holds it; the free text stays. */
export function toggleLabel(edit: SearchEdit, name: string): SearchEdit {
  const { labels, q } = searchFromEdit(edit);
  return editFromSearch(
    labels.includes(name) ? labels.filter((label) => label !== name) : [...labels, name],
    q,
  );
}

type SerializedLabelFilter = Spread<{ name: string }, SerializedLexicalNode>;
/** An editing token only: the route search holds the applied names, so a stale one just matches nothing. */
export class LabelFilterNode extends DecoratorNode<ReactNode> {
  __name: string;
  static getType() {
    return 'label-filter';
  }
  static clone(node: LabelFilterNode) {
    return new LabelFilterNode(node.__name, node.__key);
  }
  constructor(name: string, key?: NodeKey) {
    super(key);
    this.__name = name;
  }
  static importJSON(serialized: SerializedLabelFilter) {
    return new LabelFilterNode(serialized.name).updateFromJSON(serialized);
  }
  exportJSON(): SerializedLabelFilter {
    return { ...super.exportJSON(), type: 'label-filter', name: this.__name };
  }
  createDOM() {
    const node = document.createElement('span');
    node.className = 'label-filter-host';
    return node;
  }
  updateDOM() {
    return false;
  }
  exportDOM() {
    return { element: document.createTextNode(this.getTextContent()) };
  }
  isInline() {
    return true;
  }
  getTextContent() {
    return `${TRIGGER}${this.__name}`;
  }
  decorate() {
    return createElement(LabelFilterChip, { nodeKey: this.getKey(), name: this.__name });
  }
}
const isLabelFilter = (node: LexicalNode): node is LabelFilterNode =>
  node instanceof LabelFilterNode;

/** Removing a token is a discrete choice, so it applies at once instead of waiting for Enter. */
const RemovedToken = createContext<(edit: SearchEdit) => void>(() => {});

function LabelFilterChip({ nodeKey, name }: { nodeKey: NodeKey; name: string }) {
  const [editor] = useLexicalComposerContext();
  const removed = useContext(RemovedToken);
  return (
    <span className="label-filter-chip" data-label-filter={name}>
      <span className="label-filter-name">
        {TRIGGER}
        {name}
      </span>
      <BrowserIconAction
        type="button"
        variant="text"
        label={text.pageList.removeFilter(name)}
        icon={<X />}
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
          removed(
            editor.getEditorState().read(() => searchDocument.read({ value: '', tokens: [] })),
          );
          editor.focus();
        }}
      />
    </span>
  );
}

const extension = defineExtension({
  name: 'Colab/PageListSearch',
  namespace: 'ColabPageListSearch',
  nodes: [LabelFilterNode],
  dependencies: [richInputExtension],
  $initialEditorState: () => $replaceRichInput(''),
  onError: (error: Error) => {
    throw error;
  },
});
const searchDocument: RichInputDocument<SearchEdit> = {
  read: () => ({
    value: $richInputText(),
    tokens: $richInputTokens(isLabelFilter).map(({ node, range }) => ({
      name: node.__name,
      range,
    })),
  }),
  replace: (edit) =>
    $replaceRichInput(
      edit.value,
      edit.tokens.map((token) => ({
        range: token.range,
        create: () => new LabelFilterNode(token.name),
      })),
    ),
  equal: (a, b) => a.value === b.value && JSON.stringify(a.tokens) === JSON.stringify(b.tokens),
};

/** The `label:` text before the caret in the text run it sits in; tokens are never queries. */
export function labelQuery(run: string, caret: number) {
  const match = /(?:^|\s)label:([^\n]*)$/i.exec(run);
  return match
    ? { start: caret - match[1].length - TRIGGER.length, end: caret, query: match[1] }
    : undefined;
}
/** Names holding every word of the query, ignoring case, that the draft has not already taken. */
export function labelCandidates(
  labels: readonly { name: string; count: number }[],
  query: string,
  taken: readonly string[],
) {
  const words = query.toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return labels.filter(
    ({ name }) =>
      !taken.includes(name) && words.every((word) => name.toLocaleLowerCase().includes(word)),
  );
}

export interface PageListSearchProps {
  edit: SearchEdit;
  onChange(edit: SearchEdit): void;
  /** An intentional replacement from outside the editor (chip click, Back, clear). */
  resetKey: number;
  labels: readonly { name: string; count: number }[];
  /** Enter with no suggestion list open. */
  onApply(edit: SearchEdit): void;
  /** Escape with no suggestion list open. */
  onClear(): void;
}

/** The shared rich input with a `label:` token; the page list owns what the draft means. */
export function PageListSearch(props: PageListSearchProps) {
  return (
    <RemovedToken value={props.onApply}>
      <RichInput
        edit={props.edit}
        onChange={props.onChange}
        resetKey={props.resetKey}
        document={searchDocument}
        extension={extension}
        disabled={false}
        label={text.pageList.searchLabel}
      >
        <SearchField {...props} />
      </RichInput>
    </RemovedToken>
  );
}

function SearchField(props: PageListSearchProps) {
  const [editor] = useLexicalComposerContext();
  const current = useRef(props);
  current.current = props;
  const [{ query, open }, setSuggestions] = useState<{
    query?: ReturnType<typeof labelQuery>;
    open: boolean;
  }>({ open: false });
  const setOpen = useCallback((open: boolean) => {
    setSuggestions((previous) => (previous.open === open ? previous : { ...previous, open }));
  }, []);
  useEffect(
    () =>
      editor.registerUpdateListener(({ editorState, tags, dirtyElements, dirtyLeaves }) => {
        if (editor.isComposing()) return;
        editorState.read(() => {
          const selection = $getSelection();
          const caret = $richInputCaret();
          let next: ReturnType<typeof labelQuery>;
          if (
            caret !== undefined &&
            $isRangeSelection(selection) &&
            selection.anchor.type === 'text'
          ) {
            const node = selection.anchor.getNode();
            if ($isTextNode(node))
              next = labelQuery(node.getTextContent().slice(0, selection.anchor.offset), caret);
          }
          const changed = !!(dirtyElements.size || dirtyLeaves.size);
          setSuggestions((previous) => {
            const queryChanged =
              next?.start !== previous.query?.start ||
              next?.end !== previous.query?.end ||
              next?.query !== previous.query?.query;
            const open = tags.has(RICH_INPUT_RESET_TAG)
              ? false
              : queryChanged || changed
                ? next !== undefined
                : previous.open;
            return queryChanged || open !== previous.open ? { query: next, open } : previous;
          });
        });
      }),
    [editor],
  );
  const taken = props.edit.tokens.map((token) => token.name);
  const candidates = labelCandidates(props.labels, query?.query ?? '', taken);
  const options = candidates.map(({ name, count }) => ({
    value: name,
    label: `${name} (${count})`,
  }));
  function choose(name: string) {
    if (!query) return;
    editor.update(
      () => {
        const selection = $selectRichInputRange(query.start, query.end);
        selection.insertNodes([new LabelFilterNode(name), $createTextNode(' ')]);
        const active = $getSelection();
        if ($isRangeSelection(active)) active.format = 0;
      },
      { tag: HISTORY_PUSH_TAG, discrete: true },
    );
    setOpen(false);
  }
  return (
    <Listbox
      label={text.pageList.searchLabel}
      options={options}
      value=""
      onChange={choose}
      inputTrigger={{
        open: open && options.length > 0,
        onOpenChange: setOpen,
        render: (trigger) => (
          <RichInputControl
            label={text.pageList.searchLabel}
            placeholder={text.pageList.searchPlaceholder}
            disabled={false}
            className="page-list-search-field"
            trigger={trigger}
            onKeyDownCapture={(event) => {
              if ((event.target as HTMLElement).closest('.label-filter-chip')) return;
              if (trigger['aria-expanded'] === true) {
                trigger.onKeyDown?.(event);
                if (event.defaultPrevented) return;
              }
              if (event.key === 'Enter') {
                // One line: Enter applies the draft, never breaks it.
                event.preventDefault();
                event.stopPropagation();
                setOpen(false);
                current.current.onApply(current.current.edit);
              } else if (event.key === 'Escape') {
                event.preventDefault();
                event.stopPropagation();
                current.current.onClear();
              }
            }}
          />
        ),
      }}
    />
  );
}
