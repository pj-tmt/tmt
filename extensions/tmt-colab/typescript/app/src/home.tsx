import { BrowserAction, BrowserList, BrowserListRow, BrowserMenu } from '@tmt/browser-ui/react';
import { getRouteApi, Link, useRouter } from '@tanstack/react-router';
import { Settings, Tag } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { LabelWriting, ManageLabelsDialog, NewLabelDialog } from './label-dialogs.js';
import {
  applyLabelChanges,
  filterPages,
  labelCounts,
  labelNameError,
  withLabel,
  type PageSort,
} from './labels.js';
import { ManagementError } from './management.js';
import { ManagePage } from './manage-page.js';
import { pageTitle, pageUpdate } from './page-index.js';
import {
  editFromSearch,
  PageListSearch,
  searchFromEdit,
  toggleLabel,
  type SearchEdit,
} from './page-list-search.js';
import { RetentionHint } from './retention-hint.js';
import { text } from './strings.js';
import type { PageSummary } from './transport.js';
import './page-list.css';

const t = text.pageList;
const rootRoute = getRouteApi('__root__');
const homeRoute = getRouteApi('/');
/** The query that may filter and order the list (and a copied URL), bounded and validated. */
export interface HomeSearch {
  q?: string;
  label?: string[];
  sort?: 'title';
  archived?: true;
}
export function validateHomeSearch(search: Record<string, unknown>): HomeSearch {
  const out: HomeSearch = {};
  if (typeof search.q === 'string' && search.q.trim()) out.q = search.q.trim().slice(0, 256);
  if (Array.isArray(search.label)) {
    const names = [
      ...new Set(
        search.label.filter(
          (name): name is string => typeof name === 'string' && labelNameError(name) === undefined,
        ),
      ),
    ].slice(0, 8);
    if (names.length) out.label = names;
  }
  if (search.sort === 'title') out.sort = 'title';
  if (search.archived === true || search.archived === 'true') out.archived = true;
  return out;
}

function PageListActions({ page }: { page: PageSummary }) {
  const port = rootRoute.useRouteContext().transport.management;
  const [manageOpen, setManageOpen] = useState(false);
  return (
    <>
      <BrowserMenu
        label={text.pageActionsFor(pageTitle(page))}
        description={
          <>
            <p>Page ID: {page.id}</p>
            <RetentionHint page={page} />
          </>
        }
        items={port ? [{ key: 'manage', label: text.managePage }] : []}
        onSelect={() => setManageOpen(true)}
      />
      {port && (
        <ManagePage
          pageId={page.id}
          title={page.title}
          open={manageOpen}
          close={() => setManageOpen(false)}
        />
      )}
    </>
  );
}

function PageLabelsMenu({
  page,
  names,
  toggle,
  create,
}: {
  page: PageSummary;
  names: readonly string[];
  toggle(page: PageSummary, name: string): void;
  create(page: PageSummary): void;
}) {
  const carried = new Set(page.labels ?? []);
  const full = carried.size >= 8;
  return (
    <BrowserMenu
      label={t.labelsFor(pageTitle(page))}
      icon={<Tag />}
      items={[
        ...names.map((name) => {
          const blocked = full && !carried.has(name);
          return {
            key: `label:${name}`,
            label: name,
            role: 'menuitemcheckbox' as const,
            checked: carried.has(name),
            ...(blocked ? { disabled: true, disabledReason: t.pageFull } : {}),
          };
        }),
        {
          key: 'new',
          label: t.newLabelItem,
          ...(full ? { disabled: true, disabledReason: t.pageFull } : {}),
        },
      ]}
      onSelect={(key) => (key === 'new' ? create(page) : toggle(page, key.slice('label:'.length)))}
    />
  );
}

export function Home() {
  const space = homeRoute.useLoaderData();
  const { transport } = rootRoute.useRouteContext();
  const search = homeRoute.useSearch();
  const navigate = homeRoute.useNavigate();
  const router = useRouter();
  const port = transport.management;
  const archived = !!port && search.archived === true;
  const sort: PageSort = search.sort ?? 'updated';
  const filter = { labels: search.label ?? [], text: search.q ?? '' };
  const shown = space.pages.filter((p) => Boolean(p.archived) === archived);
  const pages = filterPages(shown, filter, sort);
  const counts = labelCounts(shown);
  const allNames = labelCounts(space.pages).map((label) => label.name);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => setNow(Date.now()), [space.pages, archived]);
  useEffect(() => {
    document.title =
      space.title === text.product ? text.product : `${space.title} · ${text.product}`;
  }, [space.title]);

  function setSearch(patch: Partial<HomeSearch>) {
    void navigate({ search: (previous) => ({ ...previous, ...patch }), replace: true });
  }
  // The route search holds what is applied; the editor holds the draft until Enter applies it.
  const applied = JSON.stringify([filter.labels, filter.text]);
  const lastApplied = useRef(applied);
  const [edit, setEdit] = useState(() => editFromSearch(filter.labels, filter.text));
  const [resetKey, setResetKey] = useState(0);
  useEffect(() => {
    if (applied === lastApplied.current) return;
    lastApplied.current = applied;
    setEdit(editFromSearch(filter.labels, filter.text));
    setResetKey((key) => key + 1);
    // `applied` is the whole dependency: it is the serialized filter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [applied]);
  function apply(next: SearchEdit) {
    const { labels, q } = searchFromEdit(next);
    lastApplied.current = JSON.stringify([labels, q]);
    setSearch({ label: labels.length ? labels : undefined, q: q || undefined });
  }
  function replaceDraft(next: SearchEdit) {
    setEdit(next);
    setResetKey((key) => key + 1);
    apply(next);
  }

  const [status, setStatus] = useState<{ text: string; failed: boolean }>();
  const [dialog, setDialog] = useState<
    { kind: 'new'; initial: string[] } | { kind: 'manage' } | undefined
  >();
  const writing: LabelWriting = {
    async save(changes) {
      const outcome = await applyLabelChanges(port!, changes);
      if (outcome.updated) await router.invalidate();
      return outcome;
    },
    failureText(failure) {
      const code = failure instanceof ManagementError ? failure.code : 'UNKNOWN';
      return t.failures[code as keyof typeof t.failures] ?? t.failures.UNKNOWN;
    },
  };
  async function toggleOnPage(page: PageSummary, name: string) {
    const change = withLabel(page, name, !page.labels?.includes(name));
    if (change === 'capacity')
      return setStatus({ text: `${pageTitle(page)}: ${t.pageFull}`, failed: true });
    const outcome = await writing.save([change]);
    setStatus(
      outcome.failure === undefined
        ? { text: t.saved, failed: false }
        : { text: writing.failureText(outcome.failure), failed: true },
    );
  }
  const sortLabel = t.sortOptions[sort];
  const filtered = filter.labels.length > 0 || filter.text !== '';
  return (
    <section className="home">
      <p className="eyebrow">{text.pages}</p>
      <h1>{space.title}</h1>
      <p className="intro">{text.intro}</p>
      <div className="page-list-controls">
        <div className="page-list-search">
          <PageListSearch
            edit={edit}
            onChange={setEdit}
            resetKey={resetKey}
            labels={counts}
            onApply={apply}
            onClear={() => replaceDraft(editFromSearch([], ''))}
          />
        </div>
        <BrowserMenu
          label={t.sort}
          valueLabel={sortLabel}
          items={(['updated', 'title'] as const).map((key) => ({
            key,
            label: t.sortOptions[key],
            role: 'menuitemradio' as const,
            checked: sort === key,
          }))}
          onSelect={(key) => setSearch({ sort: key === 'title' ? 'title' : undefined })}
        />
        {port && (
          <BrowserMenu
            label={t.listOptions}
            icon={<Settings />}
            items={[
              {
                key: 'archived',
                label: text.showArchived,
                role: 'menuitemcheckbox' as const,
                checked: archived,
              },
              { key: 'labels', label: `${t.manageLabels}…` },
            ]}
            onSelect={(key) =>
              key === 'archived'
                ? setSearch({ archived: archived ? undefined : true })
                : setDialog({ kind: 'manage' })
            }
          />
        )}
      </div>
      {(counts.length > 0 || port) && (
        <ul className="label-filters" aria-label={t.labels}>
          {counts.map(({ name, count }) => (
            <li key={name}>
              <button
                type="button"
                className="label-filter-button"
                aria-pressed={filter.labels.includes(name)}
                aria-label={t.labelFilter(name, count)}
                onClick={(event) => {
                  if (event.isTrusted) replaceDraft(toggleLabel(edit, name));
                }}
              >
                <span>{name}</span>
                <span className="label-count" aria-hidden="true">
                  {count}
                </span>
              </button>
            </li>
          ))}
          {port && (
            <li>
              <BrowserAction
                type="button"
                variant="text"
                label={`+ ${t.newLabel}`}
                onActivate={() => setDialog({ kind: 'new', initial: [] })}
              />
            </li>
          )}
        </ul>
      )}
      <p
        className="page-list-status"
        role={status?.failed ? 'alert' : 'status'}
        data-tone={status?.failed ? 'error' : undefined}
      >
        {[archived ? t.archivedView : '', status?.text].filter(Boolean).join(' · ')}
      </p>
      {pages.length ? (
        <BrowserList label={text.pages} className="pages">
          {pages.map((p) => {
            const title = pageTitle(p);
            const updated = pageUpdate(p.lastUpdateAtMs, now);
            return (
              <BrowserListRow
                key={p.id}
                data-page-id={p.id}
                title={
                  <>
                    <Link to="/pages/$pageId" params={{ pageId: p.id }} title={title}>
                      {title}
                    </Link>
                    {p.labels?.length ? (
                      <ul className="page-labels" aria-label={t.labels}>
                        {p.labels.map((name) => (
                          <li key={name}>
                            <button
                              type="button"
                              className="page-label"
                              aria-pressed={filter.labels.includes(name)}
                              onClick={(event) => {
                                if (event.isTrusted) replaceDraft(toggleLabel(edit, name));
                              }}
                            >
                              {name}
                            </button>
                          </li>
                        ))}
                      </ul>
                    ) : null}
                  </>
                }
                metadata={
                  updated.dateTime ? (
                    <time
                      dateTime={updated.dateTime}
                      title={updated.absolute}
                      aria-label={`${text.lastUpdated}: ${updated.label}`}
                    >
                      {updated.label}
                    </time>
                  ) : (
                    updated.label
                  )
                }
                state={
                  p.archived ? text.archived : p.sharing === 'private' ? text.private : text.shared
                }
                actions={
                  port ? (
                    <>
                      <PageLabelsMenu
                        page={p}
                        names={allNames}
                        toggle={(page, name) => void toggleOnPage(page, name)}
                        create={(page) => setDialog({ kind: 'new', initial: [page.id] })}
                      />
                      <PageListActions page={p} />
                    </>
                  ) : undefined
                }
              />
            );
          })}
        </BrowserList>
      ) : (
        <div className="page-list-empty">
          {filtered && shown.length ? (
            <>
              <p>{t.noMatch}</p>
              <BrowserAction
                type="button"
                variant="text"
                label={t.clearFilters}
                onActivate={() => replaceDraft(editFromSearch([], ''))}
              />
            </>
          ) : archived ? (
            <p>{t.noArchived}</p>
          ) : space.pages.length ? (
            <p>{t.noActive}</p>
          ) : (
            <p>
              {text.empty}
              <code className="tmt-ui-code">tmt colab page create --title "Notes"</code>.
            </p>
          )}
        </div>
      )}
      {port && (
        <>
          <NewLabelDialog
            open={dialog?.kind === 'new'}
            pages={space.pages}
            initial={dialog?.kind === 'new' ? dialog.initial : []}
            writing={writing}
            onClose={() => setDialog(undefined)}
          />
          <ManageLabelsDialog
            open={dialog?.kind === 'manage'}
            pages={space.pages}
            labels={labelCounts(space.pages)}
            readPages={async () => (await transport.spaceHome()).pages}
            writing={writing}
            onClose={() => setDialog(undefined)}
          />
        </>
      )}
    </section>
  );
}
