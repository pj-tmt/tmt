import { BrowserChip } from '@tmt/browser-ui/react';
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

/** The two filters the search takes as tokens: a label the page carries, a creator it has. */
export type FilterKind = 'label' | 'by';
const TRIGGERS: Record<FilterKind, string> = { label: 'label:', by: 'by:' };

/** The search draft: the editor's text and its tokens, each token's text being `label:<name>` or `by:<name>`. */
export interface SearchEdit {
  value: string;
  tokens: { kind: FilterKind; name: string; range: RichInputRange }[];
}

/** Label tokens, then creator tokens, then the free text; the same shape the editor reads back. */
export function editFromSearch(
  labels: readonly string[],
  q: string,
  authors: readonly string[] = [],
): SearchEdit {
  let value = '';
  const tokens: SearchEdit['tokens'] = [];
  for (const [kind, names] of [
    ['label', labels],
    ['by', authors],
  ] as const)
    for (const name of new Set(names)) {
      const token = `${TRIGGERS[kind]}${name}`;
      tokens.push({
        kind,
        name,
        range: { start: value.length, end: value.length + token.length },
      });
      value += `${token} `;
    }
  return { value: value + q, tokens };
}
/** What the draft asks for: its tokens' names and the text outside them, whitespace collapsed. */
export function searchFromEdit(edit: SearchEdit): {
  labels: string[];
  authors: string[];
  q: string;
} {
  let free = '';
  let at = 0;
  for (const token of edit.tokens) {
    free += `${edit.value.slice(at, token.range.start)} `;
    at = token.range.end;
  }
  free += edit.value.slice(at);
  const names = (kind: FilterKind) => [
    ...new Set(edit.tokens.filter((token) => token.kind === kind).map((token) => token.name)),
  ];
  return {
    labels: names('label'),
    authors: names('by'),
    q: free.split(/\s+/).filter(Boolean).join(' '),
  };
}
/** Add the label as a token, or drop it when the draft already holds it; the rest stays. */
export function toggleLabel(edit: SearchEdit, name: string): SearchEdit {
  const { labels, authors, q } = searchFromEdit(edit);
  return editFromSearch(
    labels.includes(name) ? labels.filter((label) => label !== name) : [...labels, name],
    q,
    authors,
  );
}

type SerializedFilterToken = Spread<{ kind: FilterKind; name: string }, SerializedLexicalNode>;
/** An editing token only: the route search holds the applied names, so a stale one just matches nothing. */
export class FilterTokenNode extends DecoratorNode<ReactNode> {
  __kind: FilterKind;
  __name: string;
  static getType() {
    return 'filter-token';
  }
  static clone(node: FilterTokenNode) {
    return new FilterTokenNode(node.__kind, node.__name, node.__key);
  }
  constructor(kind: FilterKind, name: string, key?: NodeKey) {
    super(key);
    this.__kind = kind;
    this.__name = name;
  }
  static importJSON(serialized: SerializedFilterToken) {
    return new FilterTokenNode(serialized.kind, serialized.name).updateFromJSON(serialized);
  }
  exportJSON(): SerializedFilterToken {
    return { ...super.exportJSON(), type: 'filter-token', kind: this.__kind, name: this.__name };
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
    return `${TRIGGERS[this.__kind]}${this.__name}`;
  }
  decorate() {
    return createElement(FilterTokenChip, {
      nodeKey: this.getKey(),
      kind: this.__kind,
      name: this.__name,
    });
  }
}
const isFilterToken = (node: LexicalNode): node is FilterTokenNode =>
  node instanceof FilterTokenNode;

/** Removing a token is a discrete choice, so it applies at once instead of waiting for Enter. */
const RemovedToken = createContext<(edit: SearchEdit) => void>(() => {});

function FilterTokenChip({
  nodeKey,
  kind,
  name,
}: {
  nodeKey: NodeKey;
  kind: FilterKind;
  name: string;
}) {
  const [editor] = useLexicalComposerContext();
  const removed = useContext(RemovedToken);
  return (
    <span {...(kind === 'label' ? { 'data-label-filter': name } : { 'data-by-filter': name })}>
      <BrowserChip
        label={`${TRIGGERS[kind]}${name}`}
        removeLabel={
          kind === 'label'
            ? text.pageList.removeFilter(name)
            : text.pageList.removeAuthorFilter(name)
        }
        onRemove={(event) => {
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
  nodes: [FilterTokenNode],
  dependencies: [richInputExtension],
  $initialEditorState: () => $replaceRichInput(''),
  onError: (error: Error) => {
    throw error;
  },
});
const searchDocument: RichInputDocument<SearchEdit> = {
  read: () => ({
    value: $richInputText(),
    tokens: $richInputTokens(isFilterToken).map(({ node, range }) => ({
      kind: node.__kind,
      name: node.__name,
      range,
    })),
  }),
  replace: (edit) =>
    $replaceRichInput(
      edit.value,
      edit.tokens.map((token) => ({
        range: token.range,
        create: () => new FilterTokenNode(token.kind, token.name),
      })),
    ),
  equal: (a, b) => a.value === b.value && JSON.stringify(a.tokens) === JSON.stringify(b.tokens),
};

/** The `label:` or `by:` text before the caret in the text run it sits in; tokens are never queries. */
export function filterQuery(run: string, caret: number) {
  const match = /(?:^|\s)(label|by):([^\n]*)$/i.exec(run);
  if (!match) return undefined;
  const kind = match[1].toLowerCase() as FilterKind;
  return {
    kind,
    start: caret - match[2].length - TRIGGERS[kind].length,
    end: caret,
    query: match[2],
  };
}
/** Names holding every word of the query, ignoring case, that the draft has not already taken. */
export function nameCandidates(
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
  /** Creators of the pages the list could show, for `by:`. */
  authors: readonly { name: string; count: number }[];
  /** Enter with no suggestion list open. */
  onApply(edit: SearchEdit): void;
  /** Escape with no suggestion list open. */
  onClear(): void;
}

/** The shared rich input with `label:` and `by:` tokens; the page list owns what the draft means. */
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
    query?: ReturnType<typeof filterQuery>;
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
          let next: ReturnType<typeof filterQuery>;
          if (
            caret !== undefined &&
            $isRangeSelection(selection) &&
            selection.anchor.type === 'text'
          ) {
            const node = selection.anchor.getNode();
            if ($isTextNode(node))
              next = filterQuery(node.getTextContent().slice(0, selection.anchor.offset), caret);
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
  const kind = query?.kind ?? 'label';
  const taken = props.edit.tokens.filter((token) => token.kind === kind).map((token) => token.name);
  const candidates = nameCandidates(
    kind === 'label' ? props.labels : props.authors,
    query?.query ?? '',
    taken,
  );
  const options = candidates.map(({ name, count }) => ({
    value: name,
    label: `${name} (${count})`,
  }));
  function choose(name: string) {
    if (!query) return;
    editor.update(
      () => {
        const selection = $selectRichInputRange(query.start, query.end);
        selection.insertNodes([new FilterTokenNode(query.kind, name), $createTextNode(' ')]);
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
              if ((event.target as HTMLElement).closest('[data-label-filter],[data-by-filter]')) {
                // Native remove-button activation must not become an editor Enter/Space command.
                if (event.key === 'Enter' || event.key === ' ') event.stopPropagation();
                return;
              }
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
