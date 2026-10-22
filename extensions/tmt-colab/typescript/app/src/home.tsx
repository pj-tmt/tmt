import { HomeTitleLink } from './home-titles.js';
import { rereadInPlace, useSummaryFill } from './home-fill.js';
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
import { Archive, ArchiveRestore, ExternalLink, Pin } from 'lucide-react';
import { useContext, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import {
  ProjectWriting,
  ManageProjectsDialog,
  NewProjectDialog,
  projectFailureText,
} from './project-dialogs.js';
import { PageProjectsMenu } from './page-projects.js';
import { pinFailureText } from './pins.js';
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
import {
  DEFAULT_VIEW,
  HOME_STAGES,
  pageStage,
  stageCounts,
  takeStage,
  type HomeStage,
  type PageStage,
} from './list-stage.js';
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
import { PageWork } from './page-work.js';
import { RetentionHint } from './retention-hint.js';
import { useScrollEdges } from './scroll-edges.js';
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
  /** The tab: absent is `DEFAULT_VIEW`; `archived` takes precedence. */
  stage?: HomeStage;
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
    stage: undefined,
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
  // The default is the absence of the key, so only another stage is a value worth keeping.
  const stage = HOME_STAGES.find((name) => name === search.stage);
  if (stage && stage !== DEFAULT_VIEW) out.stage = stage;
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
                page.archived
                  ? { key: 'restore', label: t.restoreItem }
                  : { key: 'archive', label: t.archiveItem },
                { key: 'delete', label: t.deleteItem, separated: true },
              ]
            : []
        }
        onSelect={(key) =>
          key === 'manage'
            ? setManageOpen(true)
            : operate(
                key === 'archive'
                  ? 'page.archive'
                  : key === 'restore'
                    ? 'page.unarchive'
                    : 'page.delete',
                page,
              )
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

/** Pins the page to the top of the list, or takes the pin away; an archived page has none. */
function PagePinAction({
  page,
  busy,
  toggle,
}: {
  page: PageSummary;
  busy: boolean;
  toggle(): void;
}) {
  if (page.archived) return null;
  return (
    <span className="page-pin-action">
      <BrowserIconAction
        type="button"
        variant="text"
        label={page.pinned ? t.unpinPage(pageTitle(page)) : t.pinPage(pageTitle(page))}
        icon={<Pin />}
        pressed={page.pinned === true}
        busy={busy}
        onActivate={toggle}
      />
    </span>
  );
}

/** A phone has the same item in the ⋯ menu. */
function PageArchiveAction({ page, operate }: { page: PageSummary; operate(): void }) {
  return (
    <span className="page-archive-action">
      <BrowserIconAction
        type="button"
        variant="text"
        label={page.archived ? t.restorePage(pageTitle(page)) : t.archivePage(pageTitle(page))}
        icon={page.archived ? <ArchiveRestore /> : <Archive />}
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

/**
 * A stage tab with no page. While the list is still reading the pages' tasks it says so instead of
 * claiming the tab is empty; an empty In progress names where pages without tasks are and offers
 * the way out, and never opens another tab by itself.
 */
function StageEmpty({
  stage,
  scope,
  reading,
  choose,
}: {
  stage: PageStage;
  scope: string | undefined;
  reading: boolean;
  choose(view: HomeStage): void;
}) {
  if (reading) return <p>{t.readingDetails}</p>;
  if (scope !== undefined) return <p>{t.noStageInProject[stage](scope)}</p>;
  if (stage === 'discussion') return <p>{t.noDiscussion}</p>;
  if (stage === 'done') return <p>{t.noDone}</p>;
  const empty = t.noInProgress;
  const link = (label: string, view: HomeStage) => (
    <button
      type="button"
      className="page-empty-link"
      onClick={(event) => {
        if (event.isTrusted) choose(view);
      }}
    >
      {label}
    </button>
  );
  return (
    <p>
      {empty.before}
      {link(empty.discussion, 'discussion')}
      {empty.between}
      {link(empty.all, 'all')}
      {empty.after}
    </p>
  );
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
  // The owner folds summaries in the background: while an active page has none the list reads
  // again, and an empty stage tab says it is reading until the pages are in or the tries are up.
  const reading = useSummaryFill(
    port ? homePages.filter((p) => !p.archived && !p.summary).length : 0,
    () => rereadInPlace(() => router.invalidate()),
  );
  const archived = !!port && search.archived === true;
  // Only the owner app has tabs; a list without them is every active page, as All is.
  const view: HomeStage | 'archived' = archived
    ? 'archived'
    : port
      ? (search.stage ?? DEFAULT_VIEW)
      : 'all';
  const sort: PageSort = search.sort ?? 'updated';
  const filter = {
    projects: search.project ?? [],
    authors: search.by ?? [],
    text: search.q ?? '',
    needsYou: search.needs === true,
    ...(view === 'in-progress' || view === 'discussion' || view === 'done' ? { stage: view } : {}),
  };
  const shown = homePages.filter((p) => Boolean(p.archived) === archived);
  // The pages the tab holds: a stage tab keeps the pages at that stage, so one that has not
  // reported its tasks yet is in none of them.
  const staged = filter.stage ? shown.filter((p) => pageStage(p) === filter.stage) : shown;
  const projectNames = projectCounts(space.pages, space.projects).map((project) => project.name);
  // The scope is the base: filters, counts and the needs-you chip see only the pages in it.
  const scope = knownScope(search.scope, projectNames);
  const scoped = inScope(staged, scope);
  // The tabs count what the scope holds: an archived page counts only under Archived, and a page
  // that has not reported its tasks yet under no stage.
  const archivedInScope = inScope(
    homePages.filter((p) => p.archived),
    scope,
  ).length;
  const activeInScope = inScope(
    homePages.filter((p) => !p.archived),
    scope,
  );
  const stages = stageCounts(activeInScope);
  const unreported = activeInScope.filter((p) => pageStage(p) === undefined).length;
  const tab = (id: HomeStage | 'archived', label: string) => ({
    id,
    tabId: `page-list-tab-${id}`,
    panelId: `page-list-panel-${id}`,
    label,
  });
  const viewTabs = [
    tab('in-progress', t.tabInProgress(stages['in-progress'])),
    tab('discussion', t.tabDiscussion(stages.discussion)),
    tab('done', t.tabDone(stages.done)),
    tab('archived', t.tabArchived(archivedInScope)),
    tab('all', t.tabAll),
  ];
  const tabsHost = useRef<HTMLDivElement>(null);
  useScrollEdges(tabsHost, `${Boolean(port)}|${viewTabs.map((item) => item.label).join('|')}`);
  const pages = filterPages(scoped, filter, sort);
  const counts = projectCounts(scoped);
  const waiting = waitingCount(scoped);
  const showNeeds = waiting > 0 || filter.needsYou;
  const authors = authorCounts(staged);
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
    const typed = searchFromEdit(next);
    // A typed `stage:name` selects the tab and leaves the search, as a tab click does.
    const { q, stage } = takeStage(typed.q);
    const { projects, authors } = typed;
    lastApplied.current = JSON.stringify([projects, authors, q]);
    setSearch({
      project: projects.length ? projects : undefined,
      by: authors.length ? authors : undefined,
      q: q || undefined,
      ...(stage ? stageSearch(stage) : {}),
    });
    if (stage) {
      setEdit(editFromSearch(projects, q, authors));
      setResetKey((key) => key + 1);
    }
  }
  /** The route search of a tab: the default is the absence of both, and a stage leaves Archived. */
  function stageSearch(stage: HomeStage | 'archived'): Partial<HomeSearch> {
    return {
      stage: stage === 'archived' || stage === DEFAULT_VIEW ? undefined : stage,
      archived: stage === 'archived' ? true : undefined,
    };
  }
  /** A tab click rewrites the stage: a `stage:name` still typed in the draft goes, so there is one. */
  function chooseView(id: HomeStage | 'archived') {
    const typed = searchFromEdit(edit);
    const { q, stage } = takeStage(typed.q);
    if (stage) {
      lastApplied.current = JSON.stringify([typed.projects, typed.authors, q]);
      setEdit(editFromSearch(typed.projects, q, typed.authors));
      setResetKey((key) => key + 1);
    }
    setSearch({ ...stageSearch(id), ...(stage ? { q: q || undefined } : {}) });
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
  const [pinning, setPinning] = useState<string>();
  async function togglePin(page: PageSummary) {
    if (pinning) return;
    setPinning(page.id);
    try {
      await port!.setPinned(page.id, !page.pinned);
      setStatus({
        text: page.pinned ? t.unpinned(pageTitle(page)) : t.pinned(pageTitle(page)),
        failed: false,
      });
    } catch (failure) {
      setStatus({ text: pinFailureText(failure), failed: true });
    }
    setPinning(undefined);
    await router.invalidate();
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
                    {p.pinned && !p.archived ? (
                      // Display only: the pin button beside the row says it, with its state.
                      <span className="page-pinned-mark" aria-hidden="true">
                        <Pin size={14} />
                      </span>
                    ) : null}
                    <HomeTitleLink page={p} hints={titleHints} scope={scope} />
                    {p.summary?.work ? <PageWork work={p.summary.work} /> : null}
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
                      <PagePinAction
                        page={p}
                        busy={pinning === p.id}
                        toggle={() => void togglePin(p)}
                      />
                      <PageProjectsMenu
                        page={p}
                        openRequest={
                          projectsRequest?.pageId === p.id ? projectsRequest.count : undefined
                        }
                        names={projectNames}
                        toggle={(page, name) => void toggleOnPage(page, name)}
                        create={(page) => setDialog({ kind: 'new', initial: [page.id] })}
                      />
                      <PageArchiveAction
                        page={p}
                        operate={() =>
                          setOperation({
                            kind: p.archived ? 'page.unarchive' : 'page.archive',
                            pages: [p],
                          })
                        }
                      />
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
          {filter.stage && scoped.length === 0 && space.pages.length ? (
            <StageEmpty
              stage={filter.stage}
              scope={scope}
              reading={reading && unreported > 0}
              choose={chooseView}
            />
          ) : scope !== undefined && scoped.length === 0 ? (
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
              stages={HOME_STAGES.map((name) => ({
                name,
                count: name === 'all' ? activeInScope.length : stages[name],
              }))}
              onStage={(stage) => chooseView(stage as HomeStage)}
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
        <div className="page-stage-tabs" ref={tabsHost}>
          <BrowserTabs
            label={t.views}
            variant="flush"
            tabs={viewTabs}
            selected={view}
            onSelect={(id) => chooseView(id as HomeStage | 'archived')}
          >
            {(current) => (current.id === view ? content : null)}
          </BrowserTabs>
        </div>
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
