import { PageStateMenu } from './page-state-menu.js';
import { text } from './strings.js';
import {
  BrowserAction,
  BrowserCheckbox,
  BrowserDialog,
  BrowserField,
  BrowserIconAction,
} from '@tmt/browser-ui/react';
import { ArrowDown, ArrowUp, GripVertical, Plus } from 'lucide-react';
import { useId, useRef, useState } from 'react';
import { listPages, type HomeSearch } from './page-list-filter.js';
import type { PageSummary } from './transport.js';
import { ShortcutStore } from './shortcut-store.js';
import { shortcutNameError, shortcutSearch, type Shortcut } from './shortcuts.js';
import './shortcut-dialogs.css';
const t = text.shortcuts;

export interface ShortcutDialogProps {
  mode: 'save' | 'edit' | 'manage' | 'create';
  initial?: Shortcut;
  search: HomeSearch;
  store: ShortcutStore;
  items: readonly Shortcut[];
  pages?: readonly PageSummary[];
  projects: readonly string[];
  error?: string;
  onClose(): void;
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
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const manage = mode === 'manage';
  const title = manage
    ? t.manage
    : mode === 'edit'
      ? t.edit
      : mode === 'create'
        ? t.create
        : t.saveAs;
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
  const extra = [
    search.q && t.search(search.q),
    search.by?.length && t.creators(search.by.join(', ')),
    search.sort && t.sort(text.pageList.sortOptions.title),
    search.stage && t.stage(search.stage),
    search.archived && t.archivedOnly,
  ].filter(Boolean);
  const filterError = (search.project?.length ?? 0) > 8 ? t.projectLimit : undefined;
  const matching = pages ? listPages(pages, search) : undefined;
  const patch = (next: Partial<HomeSearch>) => setSearch((previous) => ({ ...previous, ...next }));
  return (
    <BrowserDialog
      open
      title={title}
      size={manage ? 'wide' : 'regular'}
      className="shortcut-dialog"
      data-mode={mode}
      onDismiss={onClose}
      initialFocus={() => input.current}
      actions={
        <>
          {selected && (
            <BrowserAction
              type="button"
              variant="destructive"
              label={text.pageList.remove}
              onActivate={() =>
                run(() => {
                  store.delete(selected);
                  if (manage) choose();
                  else onClose();
                })
              }
            />
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
      <p className="shortcut-local">{t.localOnly}</p>
      <div className="shortcut-columns" data-manage={manage || undefined}>
        {manage && (
          <nav aria-label={t.savedList}>
            <ul className="shortcut-list">
              {items.map((item, index) => (
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
                  <GripVertical aria-hidden size={14} />
                  <button
                    type="button"
                    aria-current={selected === item.id ? 'true' : undefined}
                    onClick={() => choose(item)}
                  >
                    {item.name}
                  </button>
                  <BrowserIconAction
                    type="button"
                    variant="text"
                    label={t.moveUp(item.name)}
                    icon={<ArrowUp />}
                    disabled={index === 0}
                    onActivate={() => run(() => store.move(item.id, -1))}
                  />
                  <BrowserIconAction
                    type="button"
                    variant="text"
                    label={t.moveDown(item.name)}
                    icon={<ArrowDown />}
                    disabled={index === items.length - 1}
                    onActivate={() => run(() => store.move(item.id, 1))}
                  />
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
                input.current?.focus();
              }}
            />
          </nav>
        )}
        <div className="shortcut-editor">
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
                      <label key={project}>
                        <BrowserCheckbox
                          checked={search.project?.includes(project) ?? false}
                          onChange={(event) =>
                            patch({
                              project: event.target.checked
                                ? [...(search.project ?? []), project]
                                : (search.project ?? []).filter((name) => name !== project),
                            })
                          }
                        />
                        {project}
                      </label>
                    ))}
                  </div>
                )}
              />
              <BrowserField
                controlId={`${id}-state`}
                label={t.pageState}
                renderControl={(field) => (
                  <div {...field} role="group" className="shortcut-state">
                    <PageStateMenu state={search.state} onChange={(state) => patch({ state })} />
                  </div>
                )}
              />
              <label>
                <BrowserCheckbox
                  checked={!!search.needs}
                  onChange={(event) => patch({ needs: event.target.checked ? true : undefined })}
                />
                {t.needsMe}
              </label>
              <label>
                <BrowserCheckbox
                  checked={!!search.includeArchived}
                  onChange={(event) =>
                    patch({ includeArchived: event.target.checked ? true : undefined })
                  }
                />
                {t.includeArchived}
              </label>
              {extra.length > 0 && (
                <p className="shortcut-extra">{t.alsoRetained(extra.join(' · '))}</p>
              )}
            </>
          )}
          {mode === 'save' && (
            <p className="shortcut-extra">
              {[
                ...(search.project ?? []).map((name) => t.project(name)),
                search.state && t.state(text.workBand.taskState[search.state]),
                search.needs && t.needsMe,
                search.includeArchived && t.includeArchived,
                ...extra,
              ]
                .filter(Boolean)
                .join(' · ') || t.allActive}
            </p>
          )}
          <p role="status">{matching ? t.matchingPages(matching.length) : t.countUnavailable}</p>
          {mode !== 'save' && (
            <>
              <ul className="shortcut-preview" aria-label={t.matchingLabel}>
                {matching?.slice(0, 5).map((page) => (
                  <li key={page.id}>{page.title}</li>
                ))}
              </ul>
            </>
          )}
        </div>
      </div>
      {filterError && <p role="alert">{filterError}</p>}
      {(error || props.error) && <p role="alert">{error ?? props.error}</p>}
    </BrowserDialog>
  );
}
