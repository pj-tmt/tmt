import { HomeTitleLink } from './home-titles.js';
import { useHomeSpace, useSpaceTitle } from './home-space.js';
import {
  BrowserAction,
  BrowserIconAction,
  BrowserList,
  BrowserListRow,
  BrowserMenu,
  BrowserTabs,
} from '@tmt/browser-ui/react';
import { getRouteApi, useRouter } from '@tanstack/react-router';
import { Archive, ExternalLink } from 'lucide-react';
import { useContext, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import {
  ProjectWriting,
  ManageProjectsDialog,
  NewProjectDialog,
  projectFailureText,
} from './project-dialogs.js';
import { PageProjectsMenu } from './page-projects.js';
import {
  applyProjectChanges,
  authorCounts,
  waitingCount,
  filterPages,
  projectCounts,
  projectNameError,
  withProject,
  type PageSort,
  type ProjectChange,
} from './projects.js';
import { AUTHOR_BYTES } from './bootstrap.js';
import { HeaderSlot } from './header-slot.js';
import { ProjectScopeMenu } from './project-scope-menu.js';
import { inScope, knownScope, scopeRows, writeScope } from './project-scope.js';
import { ManagePage } from './manage-page.js';
import { PageOperationDialog } from './page-operation-dialog.js';
import { runPageOperations, type PageOperation } from './page-operations.js';
import { operationStatus } from './page-operation-status.js';
import { usePageListKeys } from './page-list-keys.js';
import { BulkBar } from './bulk-bar.js';
import { pageTitle, pageUpdate } from './page-index.js';
import {
  editFromSearch,
  PageListSearch,
  searchFromEdit,
  toggleProject,
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
  project?: string[];
  /** The project the list is scoped to: the base the filters narrow; absent is All pages. */
  scope?: string;
  by?: string[];
  sort?: 'title';
  archived?: true;
  /** Only the pages with an item waiting on the viewer. */
  needs?: true;
}
/** The route merges what this returns over the raw query, so every key is named here: one it
 * cannot read is set to `undefined` and replaces the raw value instead of reaching the list. */
export function validateHomeSearch(search: Record<string, unknown>): HomeSearch {
  const out: HomeSearch = {
    q: undefined,
    project: undefined,
    scope: undefined,
    by: undefined,
    sort: undefined,
    archived: undefined,
    needs: undefined,
  };
  if (typeof search.q === 'string' && search.q.trim()) out.q = search.q.trim().slice(0, 256);
  if (Array.isArray(search.project)) {
    const names = [
      ...new Set(
        search.project.filter(
          (name): name is string =>
            typeof name === 'string' && projectNameError(name) === undefined,
        ),
      ),
    ].slice(0, 8);
    if (names.length) out.project = names;
  }
  if (typeof search.scope === 'string' && projectNameError(search.scope) === undefined)
    out.scope = search.scope;
  if (Array.isArray(search.by)) {
    const names = [
      ...new Set(
        search.by.filter(
          (name): name is string =>
            typeof name === 'string' &&
            name.length > 0 &&
            new TextEncoder().encode(name).length <= AUTHOR_BYTES &&
            !/\p{Cc}/u.test(name),
        ),
      ),
    ].slice(0, 8);
    if (names.length) out.by = names;
  }
  if (search.sort === 'title') out.sort = 'title';
  if (search.archived === true || search.archived === 'true') out.archived = true;
  if (search.needs === true || search.needs === 'true') out.needs = true;
  return out;
}

/** The ⋯ menu: management and the actions a phone has no room for beside the row. */
function PageListActions({
  page,
  operate,
}: {
  page: PageSummary;
  operate(operation: PageOperation, page: PageSummary): void;
}) {
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
        items={
          port
            ? [
                { key: 'manage', label: text.managePage },
                ...(page.archived ? [] : [{ key: 'archive', label: t.archiveItem }]),
                { key: 'delete', label: t.deleteItem, separated: true },
              ]
            : []
        }
        onSelect={(key) =>
          key === 'manage'
            ? setManageOpen(true)
            : operate(key === 'archive' ? 'page.archive' : 'page.delete', page)
        }
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

/** Open the page in another tab: the row's link does it with a modifier, a row action without one. */
function OpenInNewTabAction({ page }: { page: PageSummary }) {
  const router = useRouter();
  return (
    <span className="page-open-tab">
      <BrowserIconAction
        type="button"
        variant="text"
        label={t.openInNewTab(pageTitle(page))}
        icon={<ExternalLink />}
        onActivate={() => {
          const location = router.buildLocation({
            to: '/pages/$pageId',
            params: { pageId: page.id },
          });
          window.open(router.history.createHref(location.href), '_blank', 'noopener');
        }}
      />
    </span>
  );
}

/** A phone has the same item in the ⋯ menu. */
function ArchiveAction({ page, operate }: { page: PageSummary; operate(): void }) {
  return (
    <span className="page-archive-action">
      <BrowserIconAction
        type="button"
        variant="text"
        label={t.archivePage(pageTitle(page))}
        icon={<Archive />}
        hasPopup="dialog"
        onActivate={operate}
      />
    </span>
  );
}

/** `by <creator> · updated · N comments`: each part only when the page has it. */
function PageMetadata({
  author,
  comments,
  updated,
}: {
  author?: string;
  comments?: number;
  updated: ReturnType<typeof pageUpdate>;
}) {
  const parts = [
    author ? (
      <span key="author" className="page-author">
        {t.byAuthor(author)}
      </span>
    ) : null,
    updated.dateTime ? (
      <time
        key="updated"
        dateTime={updated.dateTime}
        title={updated.absolute}
        aria-label={`${text.lastUpdated}: ${updated.label}`}
      >
        {updated.label}
      </time>
    ) : (
      <span key="updated">{updated.label}</span>
    ),
    comments === undefined ? null : (
      <span key="comments" className="page-comments">
        {t.commentCount(comments)}
      </span>
    ),
  ].filter((part) => part !== null);
  return <span className="page-metadata">{parts}</span>;
}

export function Home() {
  const space = homeRoute.useLoaderData();
  const { transport, space: spaceId } = rootRoute.useRouteContext();
  const slot = useContext(HeaderSlot);
  const search = homeRoute.useSearch();
  const navigate = homeRoute.useNavigate();
  const router = useRouter();
  const { pages: homePages, hints: titleHints } = useHomeSpace(space);
  const port = transport.management;
  const archived = !!port && search.archived === true;
  const sort: PageSort = search.sort ?? 'updated';
  const filter = {
    projects: search.project ?? [],
    authors: search.by ?? [],
    text: search.q ?? '',
    needsYou: search.needs === true,
  };
  const shown = homePages.filter((p) => Boolean(p.archived) === archived);
  const projectNames = projectCounts(space.pages, space.projects).map((project) => project.name);
  // The scope is the base: filters, counts and the needs-you chip see only the pages in it.
  const scope = knownScope(search.scope, projectNames);
  const scoped = inScope(shown, scope);
  // The tabs count what the scope holds: the Archived tab says how many archived pages wait there.
  const archivedInScope = inScope(
    homePages.filter((p) => p.archived),
    scope,
  ).length;
  const viewTabs = [
    { id: 'all', tabId: 'page-list-tab-all', panelId: 'page-list-panel-all', label: t.tabAll },
    {
      id: 'archived',
      tabId: 'page-list-tab-archived',
      panelId: 'page-list-panel-archived',
      label: t.tabArchived(archivedInScope),
    },
  ];
  const pages = filterPages(scoped, filter, sort);
  const counts = projectCounts(scoped);
  const waiting = waitingCount(scoped);
  const showNeeds = waiting > 0 || filter.needsYou;
  const authors = authorCounts(shown);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => setNow(Date.now()), [space.pages, archived]);
  useSpaceTitle(space.title, text.product);

  function setSearch(patch: Partial<HomeSearch>) {
    void navigate({ search: (previous) => ({ ...previous, ...patch }), replace: true });
  }
  /** The choice is the default the next visit opens on; All pages removes it. */
  function chooseScope(name: string | undefined) {
    writeScope(spaceId, name);
    setSearch({ scope: name });
  }
  // The route search holds what is applied; the editor holds the draft until Enter applies it.
  const applied = JSON.stringify([filter.projects, filter.authors, filter.text]);
  const lastApplied = useRef(applied);
  const [edit, setEdit] = useState(() =>
    editFromSearch(filter.projects, filter.text, filter.authors),
  );
  const [resetKey, setResetKey] = useState(0);
  useEffect(() => {
    if (applied === lastApplied.current) return;
    lastApplied.current = applied;
    setEdit(editFromSearch(filter.projects, filter.text, filter.authors));
    setResetKey((key) => key + 1);
    // `applied` is the whole dependency: it is the serialized filter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [applied]);
  function apply(next: SearchEdit) {
    const { projects, authors, q } = searchFromEdit(next);
    lastApplied.current = JSON.stringify([projects, authors, q]);
    setSearch({
      project: projects.length ? projects : undefined,
      by: authors.length ? authors : undefined,
      q: q || undefined,
    });
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
  const [operation, setOperation] = useState<{ kind: PageOperation; pages: PageSummary[] }>();
  const [operating, setOperating] = useState(false);
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(new Set());
  // Only pages the list shows count: a filter, a tab or a refresh drops the rest from the selection.
  const chosen = pages.filter((page) => selectedIds.has(page.id));
  function toggleSelected(pageId: string) {
    setSelectedIds((previous) => {
      const next = new Set(previous);
      if (!next.delete(pageId)) next.add(pageId);
      return next;
    });
  }
  async function confirmOperation() {
    if (!operation || !port || operating) return;
    const { kind, pages: targets } = operation;
    setOperating(true);
    const batch = await runPageOperations(port, targets, kind);
    setOperating(false);
    setOperation(undefined);
    setStatus(operationStatus(batch, kind, targets.length));
    const finished = new Set(batch.done.map((page) => page.id));
    if (finished.size)
      setSelectedIds((previous) => new Set([...previous].filter((id) => !finished.has(id))));
    // A lost acknowledgment may have changed the page: show what the owner has.
    if (finished.size || batch.failed?.failure.code === 'UNKNOWN') await router.invalidate();
  }
  /** Add or remove one project on every selected page that does not have it yet (or still has it). */
  async function changeProject(name: string, present: boolean) {
    const changes: ProjectChange[] = [];
    let full = 0;
    for (const page of chosen) {
      if (Boolean(page.projects?.includes(name)) === present) continue;
      const change = withProject(page, name, present);
      if (change === 'capacity') full += 1;
      else changes.push(change);
    }
    if (!changes.length) {
      setStatus({
        text: full ? t.skippedFull(full) : t.nothingToChange,
        failed: full > 0,
      });
      return;
    }
    const outcome = await writing.save(changes);
    if (outcome.failure !== undefined) {
      setStatus({ text: writing.failureText(outcome.failure), failed: true });
      return;
    }
    const done = present
      ? t.addedToProject(name, changes.length)
      : t.removedFromProject(name, changes.length);
    setStatus({ text: full ? `${done} ${t.skippedFull(full)}` : done, failed: false });
  }
  const [projectsRequest, setProjectsRequest] = useState<{ pageId: string; count: number }>();
  const section = useRef<HTMLElement>(null);
  usePageListKeys({
    section,
    managed: !!port,
    selecting: chosen.length > 0,
    select: toggleSelected,
    clearSelection: () => setSelectedIds(new Set()),
    projects: (pageId) =>
      setProjectsRequest((previous) => ({ pageId, count: (previous?.count ?? 0) + 1 })),
    archive: (pageId) => {
      if (archived) return;
      const targets = pageId === undefined ? chosen : pages.filter((page) => page.id === pageId);
      if (targets.length) setOperation({ kind: 'page.archive', pages: targets });
    },
  });
  const writing: ProjectWriting = {
    async save(changes) {
      const outcome = await applyProjectChanges(port!, changes);
      if (outcome.updated) await router.invalidate();
      return outcome;
    },
    async manage(change) {
      try {
        await port!.manageProject(change);
      } catch (failure) {
        return { failure };
      }
      await router.invalidate();
      return {};
    },
    failureText: projectFailureText,
  };
  async function toggleOnPage(page: PageSummary, name: string) {
    const change = withProject(page, name, !page.projects?.includes(name));
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
  const filtered =
    filter.projects.length > 0 ||
    filter.authors.length > 0 ||
    filter.text !== '' ||
    filter.needsYou;
  const content = (
    <>
      {(counts.length > 0 || showNeeds) && (
        <ul className="project-filters" aria-label={t.filters}>
          {showNeeds && (
            <li>
              <button
                type="button"
                className="project-filter-button needs-filter-button"
                data-needs-filter
                aria-pressed={filter.needsYou}
                aria-label={t.needsYouFilter(waiting)}
                onClick={(event) => {
                  if (event.isTrusted) setSearch({ needs: filter.needsYou ? undefined : true });
                }}
              >
                <span>
                  <span className="needs-filter-mark" aria-hidden="true">
                    ◆
                  </span>{' '}
                  {t.needsYou}
                </span>
                <span className="project-count" aria-hidden="true">
                  {waiting}
                </span>
              </button>
            </li>
          )}
          {counts.map(({ name, count }) => (
            <li key={name}>
              <button
                type="button"
                className="project-filter-button"
                aria-pressed={filter.projects.includes(name)}
                aria-label={t.projectFilter(name, count)}
                onClick={(event) => {
                  if (event.isTrusted) replaceDraft(toggleProject(edit, name));
                }}
              >
                <span>{name}</span>
                <span className="project-count" aria-hidden="true">
                  {count}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <p
        className="page-list-status"
        role={status?.failed ? 'alert' : 'status'}
        data-tone={status?.failed ? 'error' : undefined}
      >
        {status?.text}
      </p>
      {pages.length ? (
        <BrowserList label={text.pages} className="pages">
          {pages.map((p) => {
            const updated = pageUpdate(p.lastUpdateAtMs, now);
            return (
              <BrowserListRow
                key={p.id}
                data-page-id={p.id}
                data-selected={selectedIds.has(p.id) ? '' : undefined}
                leading={
                  port ? (
                    <input
                      type="checkbox"
                      className="page-select"
                      aria-label={t.selectPage(pageTitle(p))}
                      checked={selectedIds.has(p.id)}
                      onChange={() => toggleSelected(p.id)}
                    />
                  ) : undefined
                }
                title={
                  <>
                    <HomeTitleLink page={p} hints={titleHints} scope={scope} />
                    {p.summary?.waiting ? (
                      <span className="page-waiting" data-waiting>
                        <span className="page-waiting-mark" aria-hidden="true">
                          ◆
                        </span>{' '}
                        {t.waitingForYou}
                        {p.summary.needsYou > 0 ? ` · ${p.summary.needsYou}` : ''}
                      </span>
                    ) : null}
                    {p.projects?.length ? (
                      <ul className="page-projects" aria-label={t.projects}>
                        {p.projects.map((name) => (
                          <li key={name}>
                            <button
                              type="button"
                              className="page-project"
                              aria-pressed={filter.projects.includes(name)}
                              onClick={(event) => {
                                if (event.isTrusted) replaceDraft(toggleProject(edit, name));
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
                  <PageMetadata
                    author={p.summary?.author}
                    comments={p.summary?.comments}
                    updated={updated}
                  />
                }
                state={
                  p.archived ? text.archived : p.sharing === 'private' ? text.private : text.shared
                }
                actions={
                  port ? (
                    <>
                      <OpenInNewTabAction page={p} />
                      <PageProjectsMenu
                        page={p}
                        openRequest={
                          projectsRequest?.pageId === p.id ? projectsRequest.count : undefined
                        }
                        names={projectNames}
                        toggle={(page, name) => void toggleOnPage(page, name)}
                        create={(page) => setDialog({ kind: 'new', initial: [page.id] })}
                      />
                      {!p.archived && (
                        <ArchiveAction
                          page={p}
                          operate={() => setOperation({ kind: 'page.archive', pages: [p] })}
                        />
                      )}
                      <PageListActions
                        page={p}
                        operate={(kind, page) => setOperation({ kind, pages: [page] })}
                      />
                    </>
                  ) : undefined
                }
              />
            );
          })}
        </BrowserList>
      ) : (
        <div className="page-list-empty">
          {scope !== undefined && scoped.length === 0 ? (
            <p>{archived ? t.noArchivedInProject(scope) : t.noPagesInProject(scope)}</p>
          ) : filtered && scoped.length ? (
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
              <code className="tmt-ui-code">
                tmt colab page create --title "Notes" --layout column
              </code>
              .
            </p>
          )}
        </div>
      )}
    </>
  );
  return (
    <section className="home" ref={section}>
      <p className="eyebrow">{text.pages}</p>
      <h1>{space.title}</h1>
      <p className="intro">{text.intro}</p>
      {chosen.length > 0 ? (
        <BulkBar
          count={chosen.length}
          projects={projectNames}
          carried={projectNames.filter((name) =>
            chosen.some((page) => page.projects?.includes(name)),
          )}
          archiving={!archived}
          onAdd={(name) => void changeProject(name, true)}
          onRemove={(name) => void changeProject(name, false)}
          onNewProject={() => setDialog({ kind: 'new', initial: chosen.map((page) => page.id) })}
          onArchive={() => setOperation({ kind: 'page.archive', pages: chosen })}
          onDelete={() => setOperation({ kind: 'page.delete', pages: chosen })}
          onClear={() => setSelectedIds(new Set())}
        />
      ) : (
        <div className="page-list-controls">
          <div className="page-list-search">
            <PageListSearch
              edit={edit}
              onChange={setEdit}
              resetKey={resetKey}
              projects={counts}
              authors={authors}
              onApply={apply}
              onClear={() => {
                // Escape on an empty search leaves it, so the list's keys work from the page again.
                if (!filtered && edit.value === '' && edit.tokens.length === 0) {
                  (document.activeElement as HTMLElement | null)?.blur();
                  return;
                }
                replaceDraft(editFromSearch([], ''));
                if (filter.needsYou) setSearch({ needs: undefined });
              }}
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
        </div>
      )}
      {port ? (
        <BrowserTabs
          label={t.views}
          variant="flush"
          tabs={viewTabs}
          selected={archived ? 'archived' : 'all'}
          onSelect={(id) => setSearch({ archived: id === 'archived' ? true : undefined })}
        >
          {(tab) => (tab.id === (archived ? 'archived' : 'all') ? content : null)}
        </BrowserTabs>
      ) : (
        content
      )}
      {port && (
        <>
          <PageOperationDialog
            operation={operation?.kind}
            titles={operation ? operation.pages.map(pageTitle) : []}
            busy={operating}
            onConfirm={() => void confirmOperation()}
            onCancel={() => setOperation(undefined)}
          />
          <NewProjectDialog
            open={dialog?.kind === 'new'}
            pages={space.pages}
            existing={projectNames}
            initial={dialog?.kind === 'new' ? dialog.initial : []}
            writing={writing}
            onClose={() => setDialog(undefined)}
          />
          <ManageProjectsDialog
            open={dialog?.kind === 'manage'}
            pages={space.pages}
            projects={projectCounts(space.pages, space.projects)}
            writing={writing}
            onClose={() => setDialog(undefined)}
          />
        </>
      )}
      {slot &&
        (port || projectNames.length > 0) &&
        createPortal(
          <>
            <span className="colab-title-divider" aria-hidden="true">
              /
            </span>
            <ProjectScopeMenu
              rows={scopeRows(shown, projectNames)}
              scope={scope}
              onScope={chooseScope}
              {...(port
                ? {
                    onNew: () => setDialog({ kind: 'new', initial: [] }),
                    onManage: () => setDialog({ kind: 'manage' }),
                  }
                : {})}
            />
          </>,
          slot,
        )}
    </section>
  );
}
