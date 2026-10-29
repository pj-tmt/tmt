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
    ? 'Manage shortcuts'
    : mode === 'edit'
      ? 'Edit shortcut'
      : 'Save as shortcut';
  const failure = () =>
    setError('Shortcut changes could not be saved on this machine. Your draft has been kept.');
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
        setError(invalid);
        return;
      }
      const item = store.save({ id: selected, name, search });
      if (manage) {
        choose(item);
      } else onSaved(item);
    });
  const extra = [
    search.q && `Search: ${search.q}`,
    search.by?.length && `Creators: ${search.by.join(', ')}`,
    search.sort && 'Sort: Title',
    search.stage && `Stage tab: ${search.stage}`,
    search.archived && 'Archived-only tab',
  ].filter(Boolean);
  const filterError =
    (search.project?.length ?? 0) > 8
      ? 'Save supports up to eight project filters. Reduce the project filters before saving.'
      : undefined;
  const matching = listPages(pages, search);
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
              label="Delete"
              onActivate={() =>
                run(() => {
                  store.delete(selected);
                  if (manage) choose();
                  else onClose();
                })
              }
            />
          )}
          <BrowserAction type="button" variant="secondary" label="Cancel" onActivate={onClose} />
          <BrowserAction
            type="button"
            variant="primary"
            label="Save"
            disabled={!!shortcutNameError(name.trim()) || !!filterError}
            onActivate={save}
          />
        </>
      }
    >
      <p className="shortcut-local">
        Saved on this machine only. Shortcuts are not synced or shared.
      </p>
      <div className="shortcut-columns" data-manage={manage || undefined}>
        {manage && (
          <nav aria-label="Saved shortcuts">
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
                    label={`Move ${item.name} up`}
                    icon={<ArrowUp />}
                    disabled={index === 0}
                    onActivate={() => run(() => store.move(item.id, -1))}
                  />
                  <BrowserIconAction
                    type="button"
                    variant="text"
                    label={`Move ${item.name} down`}
                    icon={<ArrowDown />}
                    disabled={index === items.length - 1}
                    onActivate={() => run(() => store.move(item.id, 1))}
                  />
                </li>
              ))}
            </ul>
            {!items.length && <p>No shortcuts yet.</p>}
            <BrowserAction
              type="button"
              variant="ghost"
              label="New shortcut"
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
            label="Name"
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
              <fieldset>
                <legend>Projects</legend>
                <p>None selected means all projects.</p>
                <div className="shortcut-projects">
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
              </fieldset>
              <BrowserField
                controlId={`${id}-state`}
                label="Page state"
                renderControl={(field) => (
                  <select
                    {...field}
                    value={search.state ?? ''}
                    onChange={(event) =>
                      patch({ state: (event.target.value || undefined) as HomeSearch['state'] })
                    }
                  >
                    <option value="">Any state</option>
                    <option value="todo">To do</option>
                    <option value="in-progress">In progress</option>
                    <option value="done">Done</option>
                  </select>
                )}
              />
              <label>
                <BrowserCheckbox
                  checked={!!search.needs}
                  onChange={(event) => patch({ needs: event.target.checked ? true : undefined })}
                />
                Needs me
              </label>
              <label>
                <BrowserCheckbox
                  checked={!!search.includeArchived}
                  onChange={(event) =>
                    patch({ includeArchived: event.target.checked ? true : undefined })
                  }
                />
                Include archived
              </label>
              {extra.length > 0 && (
                <p className="shortcut-extra">Also retained: {extra.join(' · ')}</p>
              )}
            </>
          )}
          {mode === 'save' && (
            <p className="shortcut-extra">
              {[
                ...(search.project ?? []).map((name) => `project: ${name}`),
                search.state && `State: ${search.state}`,
                search.needs && 'Needs me',
                search.includeArchived && 'Include archived',
                ...extra,
              ]
                .filter(Boolean)
                .join(' · ') || 'All active pages'}
            </p>
          )}
          <p role="status">{matching.length} matching pages</p>
          {mode !== 'save' && (
            <>
              <ul className="shortcut-preview" aria-label="Matching pages">
                {matching.slice(0, 5).map((page) => (
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
