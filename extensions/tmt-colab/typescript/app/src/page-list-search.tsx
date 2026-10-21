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
  type HTMLAttributes,
  type Ref,
  type KeyboardEvent,
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

/** The two filters the search takes as tokens: a project the page is in, a creator it has. */
export type FilterKind = 'project' | 'by';
const TRIGGERS: Record<FilterKind, string> = { project: 'project:', by: 'by:' };

/** The search draft: the editor's text and its tokens, each token's text being `project:<name>` or `by:<name>`. */
export interface SearchEdit {
  value: string;
  tokens: { kind: FilterKind; name: string; range: RichInputRange }[];
}

/** Project tokens, then creator tokens, then the free text; the same shape the editor reads back. */
export function editFromSearch(
  projects: readonly string[],
  q: string,
  authors: readonly string[] = [],
): SearchEdit {
  let value = '';
  const tokens: SearchEdit['tokens'] = [];
  for (const [kind, names] of [
    ['project', projects],
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
  projects: string[];
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
    projects: names('project'),
    authors: names('by'),
    q: free.split(/\s+/).filter(Boolean).join(' '),
  };
}
/** Add the project as a token, or drop it when the draft already holds it; the rest stays. */
export function toggleProject(edit: SearchEdit, name: string): SearchEdit {
  const { projects, authors, q } = searchFromEdit(edit);
  return editFromSearch(
    projects.includes(name) ? projects.filter((project) => project !== name) : [...projects, name],
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
    node.className = 'project-filter-host';
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
    <span {...(kind === 'project' ? { 'data-project-filter': name } : { 'data-by-filter': name })}>
      <BrowserChip
        label={`${TRIGGERS[kind]}${name}`}
        removeLabel={
          kind === 'project'
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

/** What a suggestion list completes: a project or a creator becomes a token, a stage selects the tab. */
export type SuggestKind = FilterKind | 'stage';
/** The `project:`, `by:` or `stage:` text before the caret in the text run it sits in; tokens are never queries. */
export function filterQuery(run: string, caret: number) {
  const match = /(?:^|\s)(project|by|stage):([^\n]*)$/i.exec(run);
  if (!match) return undefined;
  const kind = match[1].toLowerCase() as SuggestKind;
  return {
    kind,
    start: caret - match[2].length - kind.length - 1,
    end: caret,
    query: match[2],
  };
}
/** Names holding every word of the query, ignoring case, that the draft has not already taken. */
export function nameCandidates(
  projects: readonly { name: string; count: number }[],
  query: string,
  taken: readonly string[],
) {
  const words = query.toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return projects.filter(
    ({ name }) =>
      !taken.includes(name) && words.every((word) => name.toLocaleLowerCase().includes(word)),
  );
}

export interface PageListSearchProps {
  edit: SearchEdit;
  onChange(edit: SearchEdit): void;
  /** An intentional replacement from outside the editor (chip click, Back, clear). */
  resetKey: number;
  projects: readonly { name: string; count: number }[];
  /** Creators of the pages the list could show, for `by:`. */
  authors: readonly { name: string; count: number }[];
  /** The stage names `stage:` completes, with their page counts; a host without stage tabs gives none. */
  stages?: readonly { name: string; count: number }[];
  /** A stage picked from the suggestions: the page list selects that tab. */
  onStage?(stage: string): void;
  /** Enter with no suggestion list open. */
  onApply(edit: SearchEdit): void;
  /** Escape with no suggestion list open. */
  onClear(): void;
  /** A host result list takes over only while filter suggestions are closed. */
  resultTrigger?: Pick<
    HTMLAttributes<HTMLElement>,
    'aria-controls' | 'aria-expanded' | 'aria-activedescendant' | 'aria-haspopup'
  > & { ref: Ref<HTMLElement> };
  /** Suggestions retain first refusal; consuming this original event skips list defaults. */
  onUnhandledKeyDown?(event: KeyboardEvent<HTMLDivElement>): void;
  placeholder?: string;
}

/** The shared rich input with `project:` and `by:` tokens; the page list owns what the draft means. */
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
  const kind = query?.kind ?? 'project';
  // A stage is one state with the tabs, never a token: nothing is taken, and picking one selects it.
  const taken =
    kind === 'stage'
      ? []
      : props.edit.tokens.filter((token) => token.kind === kind).map((token) => token.name);
  const candidates = nameCandidates(
    kind === 'project' ? props.projects : kind === 'by' ? props.authors : (props.stages ?? []),
    query?.query ?? '',
    taken,
  );
  const options = candidates.map(({ name, count }) => ({
    value: name,
    label: `${name} (${count})`,
  }));
  function choose(name: string) {
    if (!query) return;
    const { kind: chosen } = query;
    editor.update(
      () => {
        const selection = $selectRichInputRange(query.start, query.end);
        if (chosen === 'stage') {
          selection.removeText();
          return;
        }
        selection.insertNodes([new FilterTokenNode(chosen, name), $createTextNode(' ')]);
        const active = $getSelection();
        if ($isRangeSelection(active)) active.format = 0;
      },
      { tag: HISTORY_PUSH_TAG, discrete: true },
    );
    if (chosen === 'stage') current.current.onStage?.(name);
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
          <SearchControl
            label={text.pageList.searchLabel}
            placeholder={props.placeholder ?? text.pageList.searchPlaceholder}
            disabled={false}
            className="page-list-search-field"
            trigger={trigger}
            resultTrigger={props.resultTrigger}
            onKeyDownCapture={(event) => {
              if (trigger['aria-expanded'] === true) {
                trigger.onKeyDown?.(event);
                if (event.defaultPrevented) return;
              }
              current.current.onUnhandledKeyDown?.(event);
              if (event.defaultPrevented) return;
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

/** Both list owners observe one mounted editable; only the visible suggestion level owns ARIA. */
function SearchControl({
  resultTrigger,
  trigger,
  ...props
}: Parameters<typeof RichInputControl>[0] & {
  resultTrigger?: PageListSearchProps['resultTrigger'];
}) {
  const suggestionRef = trigger?.ref;
  const resultRef = resultTrigger?.ref;
  const ref = useCallback(
    (node: HTMLElement | null) => {
      for (const owner of [suggestionRef, resultRef]) {
        if (typeof owner === 'function') owner(node);
        else if (owner) owner.current = node;
      }
    },
    [suggestionRef, resultRef],
  );
  const attributes =
    trigger?.['aria-expanded'] === true ? trigger : { ...trigger, ...resultTrigger };
  return <RichInputControl {...props} trigger={{ ...attributes, ref }} />;
}
