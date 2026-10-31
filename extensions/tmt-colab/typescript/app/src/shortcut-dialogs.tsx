import { text } from './strings.js';
import {
  BrowserAction,
  BrowserCountChip,
  BrowserDialog,
  BrowserField,
  BrowserIconAction,
} from '@tmt/browser-ui/react';
import { ChevronLeft, Funnel, GripVertical, Plus } from 'lucide-react';
import { useId, useRef, useState } from 'react';
import { listPages, type HomeSearch } from './page-list-filter.js';
import type { PageSummary } from './transport.js';
import { ShortcutStore } from './shortcut-store.js';
import {
  shortcutConditions,
  shortcutNameError,
  shortcutSearch,
  type Shortcut,
} from './shortcuts.js';
import './shortcut-dialogs.css';
const t = text.shortcuts;

export interface ShortcutDialogProps {
  mode: 'save' | 'edit' | 'manage';
  initial?: Shortcut;
  search: HomeSearch;
  store: ShortcutStore;
  items: readonly Shortcut[];
  pages: readonly PageSummary[];
  projects: readonly string[];
  error?: string;
  onClose(): void;
  focusFallback?(): HTMLElement | null;
  onSaved(item: Shortcut): void;
}
export function ShortcutDialog(props: ShortcutDialogProps) {
  const { mode, store, items, pages, projects, onClose, onSaved } = props;
  const [selected, setSelected] = useState(props.initial?.id);
  const [name, setName] = useState(props.initial?.name ?? '');
  const [search, setSearch] = useState(() =>
    shortcutSearch(props.mode === 'manage' ? (props.initial?.search ?? {}) : props.search),
  );
  const [error, setError] = useState<string>();
  const [drag, setDrag] = useState<string>();
  const [mobileView, setMobileView] = useState<'list' | 'editor'>('list');
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLElement>(null);
  const manage = mode === 'manage';
  const title = manage ? t.manage : mode === 'edit' ? t.edit : t.saveAs;
  const failure = () => setError(t.saveFailed);
  const run = (action: () => void) => {
    try {
      action();
      setError(undefined);
    } catch {
      failure();
    }
  };
  const choose = (item?: Shortcut) => {
    setSelected(item?.id);
    setName(item?.name ?? '');
    setSearch(item?.search ?? {});
    setError(undefined);
  };
  const save = () =>
    run(() => {
      const invalid = shortcutNameError(name.trim());
      if (invalid) {
        setError(t.nameErrors[invalid]);
        return;
      }
      const item = store.save({ id: selected, name, search });
      if (manage) {
        choose(item);
      } else onSaved(item);
    });
  const filterError = (search.project?.length ?? 0) > 8 ? t.projectLimit : undefined;
  const matching = listPages(pages, search);
  const patch = (next: Partial<HomeSearch>) => setSearch((previous) => ({ ...previous, ...next }));
  return (
    <BrowserDialog
      open
      title={title}
      size={manage ? 'wide' : 'regular'}
      className="shortcut-dialog"
      data-mode={mode}
      data-mobile-view={mobileView}
      onDismiss={onClose}
      initialFocus={() => input.current}
      focusFallback={props.focusFallback}
      actions={
        <>
          {selected && (
            <span className="shortcut-delete">
              <BrowserAction
                type="button"
                variant="ghost"
                label={text.pageList.remove}
                onActivate={() =>
                  run(() => {
                    store.delete(selected);
                    if (manage) choose();
                    else onClose();
                  })
                }
              />
            </span>
          )}
          <BrowserAction
            type="button"
            variant="secondary"
            label={text.pageList.cancel}
            onActivate={onClose}
          />
          <BrowserAction
            type="button"
            variant="primary"
            label={text.pageList.save}
            disabled={!!shortcutNameError(name.trim()) || !!filterError}
            onActivate={save}
          />
        </>
      }
    >
      <div className="shortcut-columns" data-manage={manage || undefined}>
        {manage && (
          <nav ref={list} aria-label={t.savedList}>
            <ul className="shortcut-list">
              {items.map((item) => (
                <li
                  key={item.id}
                  draggable
                  onDragStart={(event) => {
                    event.dataTransfer.setData('text/plain', item.id);
                    setDrag(item.id);
                  }}
                  onDragEnd={() => setDrag(undefined)}
                  onDragOver={(event) => {
                    if (drag) event.preventDefault();
                  }}
                  onDrop={(event) => {
                    event.preventDefault();
                    if (drag) run(() => store.reorder(drag, item.id));
                    setDrag(undefined);
                  }}
                >
                  <span role="img" aria-label={t.drag} title={t.drag}>
                    <GripVertical aria-hidden size={16} />
                  </span>
                  <button
                    type="button"
                    aria-current={selected === item.id ? 'true' : undefined}
                    title={item.name}
                    aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"
                    onClick={() => {
                      choose(item);
                      setMobileView('editor');
                      requestAnimationFrame(() => input.current?.focus());
                    }}
                    onKeyDown={(event) => {
                      if (
                        event.isTrusted &&
                        event.altKey &&
                        (event.key === 'ArrowUp' || event.key === 'ArrowDown')
                      ) {
                        event.preventDefault();
                        run(() => store.move(item.id, event.key === 'ArrowUp' ? -1 : 1));
                      }
                    }}
                  >
                    {item.name}
                  </button>
                  <span className="shortcut-count" data-match-count>
                    {listPages(pages, item.search).length}
                  </span>
                </li>
              ))}
            </ul>
            {!items.length && <p>{t.empty}</p>}
            <BrowserAction
              type="button"
              variant="ghost"
              label={t.create}
              leadingIcon={<Plus />}
              onActivate={() => {
                choose();
                setMobileView('editor');
                requestAnimationFrame(() => input.current?.focus());
              }}
            />
          </nav>
        )}
        <div className="shortcut-editor">
          {manage && (
            <div className="shortcut-mobile-back">
              <BrowserIconAction
                type="button"
                variant="text"
                label={t.backToList}
                icon={<ChevronLeft />}
                onActivate={() => {
                  setMobileView('list');
                  requestAnimationFrame(() =>
                    list.current
                      ?.querySelector<HTMLButtonElement>('button[aria-current], button')
                      ?.focus(),
                  );
                }}
              />
            </div>
          )}
          <BrowserField
            controlId={`${id}-name`}
            label={t.name}
            renderControl={(field) => (
              <input
                {...field}
                ref={input}
                value={name}
                autoComplete="off"
                onChange={(event) => setName(event.target.value)}
              />
            )}
          />
          {mode !== 'save' && (
            <>
              <BrowserField
                controlId={`${id}-projects`}
                label={text.pageList.projects}
                description={t.allProjects}
                descriptionId={`${id}-projects-description`}
                renderControl={(field) => (
                  <div {...field} role="group" className="shortcut-projects">
                    {[...new Set([...projects, ...(search.project ?? [])])].map((project) => (
                      <BrowserCountChip
                        key={project}
                        label={project}
                        pressed={search.project?.includes(project) ?? false}
                        tone={search.project?.includes(project) ? 'attention' : 'neutral'}
                        onActivate={() =>
                          patch({
                            project: search.project?.includes(project)
                              ? (search.project ?? []).filter((name) => name !== project)
                              : [...(search.project ?? []), project],
                          })
                        }
                      />
                    ))}
                  </div>
                )}
              />
              <BrowserField
                controlId={`${id}-state`}
                label={t.pageState}
                renderControl={(field) => (
                  <div {...field} role="group" className="shortcut-state">
                    {(['todo', 'in-progress', 'done'] as const).map((state) => {
                      const states = search.state
                        ? Array.isArray(search.state)
                          ? search.state
                          : [search.state]
                        : [];
                      const pressed = states.includes(state);
                      return (
                        <BrowserCountChip
                          key={state}
                          label={text.workBand.taskState[state]}
                          pressed={pressed}
                          tone={pressed ? 'attention' : 'neutral'}
                          onActivate={() => {
                            const next = pressed
                              ? states.filter((item) => item !== state)
                              : [...states, state];
                            patch({ state: next.length ? next : undefined });
                          }}
                        />
                      );
                    })}
                  </div>
                )}
              />
              <div className="shortcut-also">
                <div className="tmt-ui-field-label">{t.also}</div>
                <div className="shortcut-chips">
                  <BrowserCountChip
                    label={t.needsMe}
                    pressed={!!search.needs}
                    tone={search.needs ? 'attention' : 'neutral'}
                    onActivate={() => patch({ needs: search.needs ? undefined : true })}
                  />
                  <BrowserCountChip
                    label={t.includeArchived}
                    pressed={!!search.includeArchived}
                    tone={search.includeArchived ? 'attention' : 'neutral'}
                    onActivate={() =>
                      patch({ includeArchived: search.includeArchived ? undefined : true })
                    }
                  />
                </div>
              </div>
            </>
          )}
          {mode === 'save' && (
            <>
              <p className="shortcut-summary" role="status">
                <Funnel aria-hidden size={16} />
                <span>
                  {[...shortcutConditions(search), t.pageCount(matching.length)].join(' · ')}
                </span>
              </p>
              <p className="shortcut-extra">{t.saveNote}</p>
            </>
          )}
          {mode !== 'save' && (
            <p
              className="shortcut-summary"
              role="status"
              title={t.matchesPreview(matching.map((page) => page.title))}
            >
              <Funnel aria-hidden size={16} />
              <span>{t.matchesPreview(matching.map((page) => page.title))}</span>
            </p>
          )}
        </div>
      </div>
      {filterError && <p role="alert">{filterError}</p>}
      {(error || props.error) && <p role="alert">{error ?? props.error}</p>}
    </BrowserDialog>
  );
}
