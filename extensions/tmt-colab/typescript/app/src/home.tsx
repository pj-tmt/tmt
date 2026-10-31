import { useShortcuts } from './use-shortcuts.js';
import { useMedia } from './use-media.js';
import { ShortcutDialog } from './shortcut-dialogs.js';
import type { Shortcut } from './shortcuts.js';
import { rereadInPlace, useSummaryFill } from './home-fill.js';
import { useHomeSpace, useSpaceTitle } from './home-space.js';
import {
  BrowserAction,
  BrowserIconAction,
  BrowserListTable,
  BrowserMenu,
  BrowserTabs,
  BrowserCheckbox,
  BrowserChip,
  type BrowserTab,
} from '@tmt/browser-ui/react';
import { getRouteApi, useRouter } from '@tanstack/react-router';
import {
  Archive,
  ArchiveRestore,
  ArrowDownUp,
  Bookmark,
  ExternalLink,
  Funnel,
  Pencil,
  Pin,
  Plus,
} from 'lucide-react';
import { useContext, useEffect, useLayoutEffect, useRef, useState } from 'react';
import {
  ProjectWriting,
  ManageProjectsDialog,
  NewProjectDialog,
  projectFailureText,
} from './project-dialogs.js';
import { PageRowProjects } from './page-projects.js';
import { pinFailureText } from './pins.js';
import {
  applyProjectChanges,
  authorCounts,
  waitingCount,
  filterPages,
  projectCounts,
  projectTone,
  withProject,
  type PageSort,
  type ProjectChange,
} from './projects.js';
import {
  DEFAULT_VIEW,
  HOME_STAGES,
  type HomeView,
  pageStage,
  stageCounts,
  takeStage,
  type PageStage,
  type HomeStage,
} from './list-stage.js';
import { ShortcutMenuRequestSlot } from './shortcut-menu-request.js';
import { inScope, knownScope, writeScope } from './project-scope.js';
import { ManagePage } from './manage-page.js';
import { PageOperationDialog } from './page-operation-dialog.js';
import { runPageOperations, type PageOperation } from './page-operations.js';
import { operationStatus } from './page-operation-status.js';
import { usePageListKeys } from './page-list-keys.js';
import { useShownRows } from './page-list-rows.js';
import { BulkBar } from './bulk-bar.js';
import { NewPageDialog } from './new-page-dialog.js';
import { pageTitle } from './page-index.js';
import {
  editFromSearch,
  PageListSearch,
  searchFromEdit,
  toggleProject,
  type SearchEdit,
} from './page-list-search.js';
import { pageListColumns, pageListRow, PinnedPages } from './page-list-items.js';
import { RetentionHint } from './retention-hint.js';
import { text } from './strings.js';
import type { PageSummary } from './transport.js';
import './page-list.css';
import { AppHeader } from './app-header.js';

const t = text.pageList;
const columnLabels = {
  select: '',
  title: t.titleColumn,
  projects: t.projectColumn,
  stage: t.stageColumn,
  updated: t.updatedColumn,
  author: t.authorColumn,
  access: t.accessColumn,
  actions: '',
} as const;
const rootRoute = getRouteApi('__root__');
const homeRoute = getRouteApi('/');
export { validateHomeSearch, type HomeSearch } from './page-list-filter.js';
import { pageListFilter, type HomeSearch } from './page-list-filter.js';

/** The ⋯ menu: management and the actions a phone has no room for beside the row. */
function PageListActions({
  page,
  operate,
  manage,
  projects,
}: {
  page: PageSummary;
  manage(): void;
  projects(): void;
  operate(operation: PageOperation, page: PageSummary): void;
}) {
  const port = rootRoute.useRouteContext().transport.management;
  return (
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
              { key: 'projects', label: text.projectsItem },
              page.archived
                ? { key: 'restore', label: t.restoreItem }
                : { key: 'archive', label: t.archiveItem },
              { key: 'delete', label: t.deleteItem, separated: true },
            ]
          : []
      }
      onSelect={(key) =>
        key === 'manage'
          ? manage()
          : key === 'projects'
            ? projects()
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
  const phone = useMedia('(max-width: 760px)');
  const space = homeRoute.useLoaderData();
  const { transport, space: spaceId } = rootRoute.useRouteContext();
  const shortcutRequest = useContext(ShortcutMenuRequestSlot);
  const search = homeRoute.useSearch();
  const navigate = homeRoute.useNavigate();
  const router = useRouter();
  const { pages: homePages, hints: titleHints } = useHomeSpace(space);
  const port = transport.management;
  const shortcuts = useShortcuts(spaceId);
  const [shortcutDialog, setShortcutDialog] = useState<'save' | 'edit' | 'manage'>();
  const [shortcutTarget, setShortcutTarget] = useState<string>();
  const shortcutFromMenu = useRef(false);
  const currentShortcut = shortcuts.items.find((item) => item.id === search.shortcut);
  function savedShortcut(item: Shortcut) {
    setShortcutDialog(undefined);
    void navigate({ search: { ...item.search, shortcut: item.id }, replace: true });
  }

  // The owner folds summaries in the background: while an active page has none the list reads
  // again, and an empty stage tab says it is reading until the pages are in or the tries are up.
  const reading = useSummaryFill(
    port ? homePages.filter((p) => !p.archived && !p.summary).length : 0,
    () => rereadInPlace(() => router.invalidate()),
  );
  const archived = !!port && search.archived === true;
  // Only the owner app has tabs; a list without them is every active page, as All is.
  const view: HomeView = archived
    ? 'archived'
    : port && search.needs === true
      ? 'needs'
      : port
        ? (search.stage ?? DEFAULT_VIEW)
        : 'all';
  const sort: PageSort = search.sort ?? 'updated';
  const filter = pageListFilter({
    ...search,
    scope: undefined,
    ...(!port ? { stage: undefined } : {}),
  });
  const shown = homePages.filter((p) =>
    archived ? Boolean(p.archived) : search.includeArchived || !p.archived,
  );
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
  const tab = (id: HomeView, label: string, count: number, state?: BrowserTab['state']) => ({
    id,
    tabId: `page-list-tab-${id}`,
    panelId: `page-list-panel-${id}`,
    label,
    count,
    ...(state ? { state } : {}),
  });
  const waitingInScope = waitingCount(activeInScope);
  // All first; Needs you is a view of every active page, marked while any waits.
  const viewTabs = [
    tab('all', t.tabAll, activeInScope.length),
    tab(
      'needs',
      t.tabNeedsYou,
      waitingInScope,
      waitingInScope ? { mark: '◆', tone: 'waiting', word: t.tabNeedsYouWaiting } : undefined,
    ),
    tab('discussion', t.tabDiscussion, stages.discussion),
    tab('in-progress', t.tabInProgress, stages['in-progress']),
    tab('done', t.tabDone, stages.done),
    tab('archived', t.tabArchived, archivedInScope),
  ];
  const pages = filterPages(scoped, filter, sort);
  const section = useRef<HTMLElement>(null);
  const listEnd = useRef<HTMLParagraphElement>(null);
  const controlsBoundary = useRef<HTMLDivElement>(null);
  const [compact, setCompact] = useState(false);
  const compactFocus = useRef<string | undefined>(undefined);
  useLayoutEffect(() => {
    if (!compactFocus.current) return;
    section.current
      ?.querySelector<HTMLElement>(compactFocus.current)
      ?.focus({ preventScroll: true });
    compactFocus.current = undefined;
  }, [compact]);
  useEffect(() => {
    const root = section.current;
    const boundary = controlsBoundary.current;
    if (!root || !boundary) return;
    // Observe normal flow, never the sticky row: its changing height must not move the trigger.
    const measure = () => {
      const next = boundary.getBoundingClientRect().bottom <= root.getBoundingClientRect().top;
      const active = document.activeElement;
      if (active instanceof HTMLElement && root.contains(active)) {
        if (next && active.closest('.page-list-hero-create'))
          compactFocus.current = '.page-list-compact-create button';
        else if (!next && active.closest('.page-list-compact-create'))
          compactFocus.current = '.page-list-hero-create button';
        else if (next && active.closest('.page-list-sort'))
          compactFocus.current = '.page-list-search [contenteditable]';
      }
      setCompact(next);
    };
    const observer = new IntersectionObserver(measure, { root });
    // An instant scroll can jump the marker from above to below a very short viewport
    // without changing its intersection ratio. The same geometry handles that jump.
    root.addEventListener('scroll', measure, { passive: true });
    const resize = new ResizeObserver(measure);
    resize.observe(root);
    for (const node of root.querySelectorAll('.page-list-hero, .page-pins, .tmt-ui-tabs-header'))
      resize.observe(node);
    observer.observe(boundary);
    return () => {
      observer.disconnect();
      resize.disconnect();
      root.removeEventListener('scroll', measure);
    };
  }, [view, scope, space.pages]);
  const drawn = useShownRows(
    section,
    listEnd,
    pages.length,
    JSON.stringify([view, filter, sort, scope]),
  );
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
  // The side menu lists shortcuts, not projects: the owner's explicit pinned projects become
  // project shortcuts once, in order. A failed write leaves the migration for the next load.
  useEffect(() => {
    if (!port || !shortcuts.store) return;
    try {
      shortcuts.store.migrate(space.projectPins);
    } catch {
      // Unreadable or unwritable storage: the shortcut hook reports a read failure.
    }
  }, [port, shortcuts.store, space.projectPins]);
  // The shell's Shortcuts commands open this list's own dialogs.
  useEffect(() => {
    const held = shortcutRequest?.request;
    if (!held || !port || !shortcuts.store) return;
    if (held.kind === 'edit' && !shortcuts.items.some((item) => item.id === held.shortcut)) {
      try {
        // The store may have loaded before this hook's first subscription refresh.
        if (shortcuts.store.read().items.some((item) => item.id === held.shortcut)) return;
      } catch {
        // The hook reports an unreadable record; do not open an unrelated editor.
      }
      shortcutRequest.done(held.id);
      return;
    }
    shortcutRequest.done(held.id);
    shortcutFromMenu.current = true;
    setShortcutTarget(held.shortcut);
    setShortcutDialog(held.kind);
  }, [shortcutRequest, port, shortcuts.store, shortcuts.items]);
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
  function stageSearch(stage: HomeView): Partial<HomeSearch> {
    return {
      stage:
        stage === 'archived' || stage === 'needs' || stage === DEFAULT_VIEW ? undefined : stage,
      archived: stage === 'archived' ? true : undefined,
      needs: stage === 'needs' ? true : undefined,
    };
  }
  /** A tab click rewrites the stage: a `stage:name` still typed in the draft goes, so there is one. */
  function chooseView(id: HomeView) {
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

  function clearFilters() {
    const next = editFromSearch([], '');
    lastApplied.current = JSON.stringify([[], [], '']);
    setEdit(next);
    setResetKey((key) => key + 1);
    setSearch({
      project: undefined,
      by: undefined,
      q: undefined,
      state: undefined,
      includeArchived: undefined,
      ...(!port ? { needs: undefined } : {}),
    });
  }

  const [status, setStatus] = useState<{ text: string; failed: boolean }>();
  const [dialog, setDialog] = useState<
    { kind: 'new'; initial: string[] } | { kind: 'manage' } | undefined
  >();
  const [operation, setOperation] = useState<{ kind: PageOperation; pages: PageSummary[] }>();
  const [operating, setOperating] = useState(false);
  const [newPage, setNewPage] = useState(false);
  // The list owns the dialog even when a refreshed list no longer contains its page.
  const [managedPage, setManagedPage] = useState<Pick<PageSummary, 'id' | 'title'>>();
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
  function openProjects(pageId: string) {
    setProjectsRequest((previous) => ({ pageId, count: (previous?.count ?? 0) + 1 }));
  }
  usePageListKeys({
    section,
    managed: !!port,
    selecting: chosen.length > 0,
    select: toggleSelected,
    clearSelection: () => setSelectedIds(new Set()),
    projects: openProjects,
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
    filter.needsYou ||
    !!filter.state ||
    !!search.includeArchived;
  const controls = (
    <>
      <div className="page-list-controls-boundary" ref={controlsBoundary} aria-hidden="true" />
      <div className="page-list-controls">
        <div className="page-list-search">
          <div className="page-list-search-box">
            <div className="page-list-condition-chips">
              {scope && (
                <BrowserChip
                  label={text.shortcuts.project(scope)}
                  removeLabel={text.shortcuts.removeCondition(text.shortcuts.project(scope))}
                  onRemove={() => {
                    writeScope(spaceId, undefined);
                    setSearch({ scope: undefined });
                  }}
                />
              )}
              {(search.state
                ? Array.isArray(search.state)
                  ? search.state
                  : [search.state]
                : []
              ).map((state) => (
                <BrowserChip
                  key={state}
                  label={text.shortcuts.filterState(text.workBand.taskState[state])}
                  removeLabel={text.shortcuts.removeCondition(text.workBand.taskState[state])}
                  onRemove={() => {
                    const remaining = (
                      Array.isArray(search.state) ? search.state : [search.state]
                    ).filter(
                      (item): item is NonNullable<typeof item> =>
                        item !== undefined && item !== state,
                    );
                    setSearch({ state: remaining.length ? remaining : undefined });
                  }}
                />
              ))}
              {search.needs && (
                <BrowserChip
                  label={text.shortcuts.needsMe}
                  removeLabel={text.shortcuts.removeCondition(text.shortcuts.needsMe)}
                  onRemove={() => setSearch({ needs: undefined })}
                />
              )}
              {search.includeArchived && (
                <BrowserChip
                  label={text.shortcuts.includeArchived}
                  removeLabel={text.shortcuts.removeCondition(text.shortcuts.includeArchived)}
                  onRemove={() => setSearch({ includeArchived: undefined })}
                />
              )}
              {search.archived && (
                <BrowserChip
                  label={text.shortcuts.archivedOnly}
                  removeLabel={text.shortcuts.removeCondition(text.archived)}
                  onRemove={() => chooseView('all')}
                />
              )}
              {search.stage && (
                <BrowserChip
                  label={text.shortcuts.stage(search.stage)}
                  removeLabel={text.shortcuts.removeCondition(search.stage)}
                  onRemove={() => setSearch({ stage: undefined })}
                />
              )}
            </div>
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
                if (!filtered && edit.value === '' && edit.tokens.length === 0) {
                  (document.activeElement as HTMLElement | null)?.blur();
                  return;
                }
                clearFilters();
              }}
            />
          </div>
          <span className="page-list-filter-menu">
            <Funnel aria-hidden="true" />
            <BrowserMenu
              label={text.shortcuts.filter}
              triggerText={phone ? undefined : text.shortcuts.filter}
              icon={<Funnel />}
              popupLayout="grouped"
              items={[
                ...(['todo', 'in-progress', 'done'] as const).map((state, index) => ({
                  key: `state:${state}`,
                  heading: index === 0 ? text.shortcuts.pageState : undefined,
                  label: text.workBand.taskState[state],
                  role: 'menuitemcheckbox' as const,
                  checked: (Array.isArray(search.state) ? search.state : [search.state]).includes(
                    state,
                  ),
                })),
                ...projectNames.map((project, index) => ({
                  key: `project:${project}`,
                  label: project,
                  heading: index === 0 ? text.shortcuts.projectGroup : undefined,
                  role: 'menuitemcheckbox' as const,
                  checked: scope === project,
                  separated: index === 0,
                })),
                ...(port
                  ? [
                      {
                        key: 'manage-projects',
                        label: `${t.manageProjects}…`,
                        heading: projectNames.length ? undefined : text.shortcuts.projectGroup,
                        separated: projectNames.length === 0,
                      },
                    ]
                  : []),
                {
                  key: 'needs',
                  label: text.shortcuts.needsMe,
                  heading: text.shortcuts.also,
                  role: 'menuitemcheckbox' as const,
                  checked: !!search.needs,
                  separated: true,
                },
                {
                  key: 'archived',
                  label: text.shortcuts.includeArchived,
                  role: 'menuitemcheckbox' as const,
                  checked: !!search.includeArchived,
                },
              ]}
              onSelect={(key) => {
                if (key === 'manage-projects') setDialog({ kind: 'manage' });
                else if (key === 'needs') setSearch({ needs: search.needs ? undefined : true });
                else if (key === 'archived')
                  setSearch({ includeArchived: search.includeArchived ? undefined : true });
                else if (key.startsWith('project:')) {
                  const project = key.slice(8);
                  const next = scope === project ? undefined : project;
                  writeScope(spaceId, next);
                  setSearch({ scope: next });
                } else if (key.startsWith('state:')) {
                  const value = key.slice(6) as 'todo' | 'in-progress' | 'done';
                  const states = search.state
                    ? Array.isArray(search.state)
                      ? search.state
                      : [search.state]
                    : [];
                  const next = states.includes(value)
                    ? states.filter((state) => state !== value)
                    : [...states, value];
                  setSearch({ state: next.length ? next : undefined });
                }
              }}
            />
          </span>
        </div>
        <span className="page-list-sort">
          <BrowserMenu
            label={`${t.sort}: ${sortLabel}`}
            triggerText={phone ? undefined : sortLabel}
            icon={<ArrowDownUp />}
            items={(['updated', 'title'] as const).map((key) => ({
              key,
              label: t.sortOptions[key],
              role: 'menuitemradio' as const,
              checked: sort === key,
            }))}
            onSelect={(key) => setSearch({ sort: key === 'title' ? 'title' : undefined })}
          />
        </span>
        <span className="page-list-compact-create" inert={!compact}>
          {phone ? (
            <BrowserIconAction
              type="button"
              variant="primary"
              label={t.newPage}
              icon={<Plus />}
              onActivate={() => setNewPage(true)}
            />
          ) : (
            <BrowserAction
              type="button"
              variant="primary"
              presentation="compact"
              label={t.newPage}
              leadingIcon={<Plus />}
              onActivate={() => setNewPage(true)}
            />
          )}
        </span>
      </div>
    </>
  );
  const newPageAction = (
    <BrowserAction
      type="button"
      variant="primary"
      label={t.newPage}
      visibleLabel={phone ? t.newShort : undefined}
      leadingIcon={<Plus size={14} />}
      onActivate={() => setNewPage(true)}
    />
  );
  const content = (
    <>
      {controls}
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
      ) : null}

      {showNeeds && !port ? (
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
          <span className="needs-filter-mark" aria-hidden="true">
            ◆
          </span>
          {t.needsYou} {waiting}
        </button>
      ) : null}
      <p
        className="page-list-status"
        role={status?.failed ? 'alert' : 'status'}
        data-tone={status?.failed ? 'error' : undefined}
      >
        {status?.text}
      </p>
      {pages.length ? (
        <div className="page-list-scroll">
          <BrowserListTable
            label={text.pages}
            className="page-list-table pages"
            columns={pageListColumns.map((id) => ({
              id,
              label: columnLabels[id],
              header:
                id === 'select' ? (
                  port ? (
                    <BrowserCheckbox
                      aria-label={t.selectAllPages}
                      checked={pages.length > 0 && chosen.length === pages.length}
                      indeterminate={chosen.length > 0 && chosen.length < pages.length}
                      onChange={(event) =>
                        setSelectedIds(
                          event.target.checked ? new Set(pages.map((page) => page.id)) : new Set(),
                        )
                      }
                    />
                  ) : (
                    ''
                  )
                ) : undefined,
            }))}
            rows={pages.slice(0, drawn).map((p) =>
              pageListRow({
                page: p,
                hints: titleHints,
                scope: scope,
                now: now,
                selected: selectedIds.has(p.id),
                leading: port ? (
                  <BrowserCheckbox
                    aria-label={t.selectPage(pageTitle(p))}
                    checked={selectedIds.has(p.id)}
                    onChange={() => toggleSelected(p.id)}
                  />
                ) : undefined,
                projects: port ? (
                  <PageRowProjects
                    page={p}
                    names={projectNames}
                    openRequest={
                      projectsRequest?.pageId === p.id ? projectsRequest.count : undefined
                    }
                    open={() => openProjects(p.id)}
                    toggle={(page, name) => void toggleOnPage(page, name)}
                    create={(page) => setDialog({ kind: 'new', initial: [page.id] })}
                  />
                ) : p.projects?.length ? (
                  <ul className="page-projects" aria-label={t.projects}>
                    {p.projects.map((name) => (
                      <li key={name}>
                        <button
                          type="button"
                          className="page-project"
                          title={name}
                          data-project-tone={projectTone(name)}
                          aria-pressed={filter.projects.includes(name)}
                          onClick={(event) => {
                            if (event.isTrusted) replaceDraft(toggleProject(edit, name));
                          }}
                        >
                          <span>{name}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                ) : null,
                actions: port ? (
                  <>
                    <span className="page-row-shortcuts">
                      <OpenInNewTabAction page={p} />
                      <PagePinAction
                        page={p}
                        busy={pinning === p.id}
                        toggle={() => void togglePin(p)}
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
                    </span>
                    <PageListActions
                      page={p}
                      manage={() => setManagedPage({ id: p.id, title: p.title })}
                      projects={() => openProjects(p.id)}
                      operate={(kind, page) => setOperation({ kind, pages: [page] })}
                    />
                  </>
                ) : undefined,
              }),
            )}
          />
          <p className="page-list-count" data-page-list-count ref={listEnd}>
            <span>{t.showing(Math.min(drawn, pages.length), pages.length)}</span>
            <span>{port ? t.ownerAccess : t.viewerAccess}</span>
          </p>
        </div>
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
                onActivate={clearFilters}
              />
            </>
          ) : archived ? (
            <p>{t.noArchived}</p>
          ) : space.pages.length ? (
            <p>{t.noActive}</p>
          ) : (
            <p>
              {text.intro} {text.empty}
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
    <section
      className="home tmt-ui-page"
      data-page-list
      data-compact={compact ? '' : undefined}
      ref={section}
    >
      <AppHeader
        title={currentShortcut?.name ?? scope ?? text.pageList.allPages}
        remote={{ deviceName: transport.backendName }}
      />
      <header className="page-list-hero">
        {port && (
          <>
            <span className="page-list-shortcut-wide">
              <BrowserAction
                type="button"
                variant="secondary"
                label={currentShortcut ? text.shortcuts.edit : text.shortcuts.saveAs}
                leadingIcon={currentShortcut ? <Pencil /> : <Bookmark />}
                disabled={!shortcuts.store}
                onActivate={() => {
                  shortcutFromMenu.current = false;
                  setShortcutTarget(undefined);
                  setShortcutDialog(currentShortcut ? 'edit' : 'save');
                }}
              />
            </span>
            <span className="page-list-shortcut-phone">
              <BrowserIconAction
                type="button"
                variant="secondary"
                label={currentShortcut ? text.shortcuts.edit : text.shortcuts.saveAs}
                icon={currentShortcut ? <Pencil /> : <Bookmark />}
                disabled={!shortcuts.store}
                onActivate={() => {
                  shortcutFromMenu.current = false;
                  setShortcutTarget(undefined);
                  setShortcutDialog(currentShortcut ? 'edit' : 'save');
                }}
              />
            </span>
          </>
        )}
        <span className="page-list-hero-create" inert={compact}>
          {newPageAction}
        </span>
      </header>
      {shortcuts.error && <p role="alert">{shortcuts.error}</p>}
      {shortcutDialog && shortcuts.store && (
        <ShortcutDialog
          key={`${spaceId}:${shortcutDialog}`}
          mode={shortcutDialog}
          initial={
            shortcutDialog === 'edit'
              ? shortcutTarget
                ? shortcuts.items.find((item) => item.id === shortcutTarget)
                : currentShortcut
              : shortcutDialog === 'manage'
                ? shortcuts.items[0]
                : undefined
          }
          search={
            shortcutDialog === 'manage'
              ? {}
              : shortcutDialog === 'edit' && shortcutTarget
                ? (shortcuts.items.find((item) => item.id === shortcutTarget)?.search ?? {})
                : { ...search, scope }
          }
          store={shortcuts.store}
          items={shortcuts.items}
          pages={homePages}
          projects={projectNames}
          error={shortcuts.error}
          onClose={() => {
            setShortcutDialog(undefined);
            if (shortcutFromMenu.current) {
              requestAnimationFrame(() => {
                const opener = [
                  ...document.querySelectorAll<HTMLElement>('.tmt-ui-shell-open'),
                ].find((node) => node.getClientRects().length > 0);
                opener?.focus({ preventScroll: true });
              });
            }
          }}
          focusFallback={() => document.querySelector<HTMLElement>('.tmt-ui-shell-open')}
          onSaved={savedShortcut}
        />
      )}
      <PinnedPages
        pages={
          port && !archived
            ? inScope(
                homePages.filter((page) => page.pinned && !page.archived),
                scope,
              )
            : []
        }
        hints={titleHints}
        scope={scope}
      />
      <NewPageDialog
        open={newPage}
        onClose={() => setNewPage(false)}
        focusFallback={() =>
          section.current?.querySelector<HTMLElement>(
            compact ? '.page-list-compact-create button' : '.page-list-hero-create button',
          ) ?? null
        }
      />
      {port ? (
        <div className="page-stage-tabs">
          <BrowserTabs
            label={t.views}
            variant="flat"
            tabs={viewTabs}
            selected={view}
            onSelect={(id) => chooseView(id as HomeView)}
          >
            {(current) => (current.id === view ? content : null)}
          </BrowserTabs>
        </div>
      ) : (
        content
      )}
      {port && (
        <>
          {managedPage && (
            <ManagePage
              pageId={managedPage.id}
              title={managedPage.title}
              open
              close={() => setManagedPage(undefined)}
            />
          )}
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
            onNewProject={() => setDialog({ kind: 'new', initial: [] })}
            writing={writing}
            onClose={() => setDialog(undefined)}
          />
        </>
      )}
    </section>
  );
}
