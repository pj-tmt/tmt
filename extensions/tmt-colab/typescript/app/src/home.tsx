import { useShortcuts } from './use-shortcuts.js';
import { ShortcutDialog } from './shortcut-dialogs.js';
import type { Shortcut } from './shortcuts.js';
import { rereadInPlace, useSummaryFill } from './home-fill.js';
import { useHomeSpace, useSpaceTitle } from './home-space.js';
import {
  BrowserAction,
  BrowserIconAction,
  BrowserList,
  BrowserMenu,
  BrowserTabs,
  BrowserCheckbox,
  type BrowserTab,
} from '@tmt/browser-ui/react';
import { getRouteApi, useRouter } from '@tanstack/react-router';
import { Archive, ArchiveRestore, ExternalLink, Pin, Plus, Search, Settings } from 'lucide-react';
import { useContext, useEffect, useLayoutEffect, useRef, useState } from 'react';
import {
  ProjectWriting,
  ManageProjectsDialog,
  NewProjectDialog,
  projectFailureText,
} from './project-dialogs.js';
import { PageRowProjects } from './page-projects.js';
import { PROJECT_PINS } from './bootstrap.js';
import { menuProjects, withPin } from './project-pins.js';
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
import { ProjectMenuActionsSlot } from './project-menu-actions.js';
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
import { PageListItem, PinnedPages } from './page-list-items.js';
import { RetentionHint } from './retention-hint.js';
import { text } from './strings.js';
import type { PageSummary } from './transport.js';
import './page-list.css';

const t = text.pageList;
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
  const space = homeRoute.useLoaderData();
  const { transport, space: spaceId } = rootRoute.useRouteContext();
  const offerProjectActions = useContext(ProjectMenuActionsSlot);
  const search = homeRoute.useSearch();
  const navigate = homeRoute.useNavigate();
  const router = useRouter();
  const { pages: homePages, hints: titleHints } = useHomeSpace(space);
  const port = transport.management;
  const shortcuts = useShortcuts(spaceId, space.projectPins);
  const [shortcutDialog, setShortcutDialog] = useState<'save' | 'edit' | 'manage'>();
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
    const phone = matchMedia('(max-width: 650px)');
    const scopeControl = (target: EventTarget | null) =>
      target instanceof HTMLElement &&
      root.contains(target) &&
      target.closest('.page-scope-filters')
        ? target
        : undefined;
    // A scope change restarts this observer while its same button can retain focus.
    let focusedScope = scopeControl(document.activeElement);
    // Observe normal flow, never the sticky row: its changing height must not move the trigger.
    const measure = () => {
      const next = boundary.getBoundingClientRect().bottom <= root.getBoundingClientRect().top;
      const active = document.activeElement;
      if (active instanceof HTMLElement && root.contains(active)) {
        if (next && active.closest('.page-list-hero'))
          compactFocus.current = '.page-list-compact-create button';
        else if (!next && active.closest('.page-list-compact-create'))
          compactFocus.current = '.page-list-hero .tmt-ui-action';
        else if (
          next &&
          (active.closest('.page-list-sort') ||
            (phone.matches && active.closest('.page-scope-filters')))
        )
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
    // Some browsers clear focus as the media rule hides a project button, before the
    // change event. Keep that handoff through layout reflow; focus entering another control clears it.
    const remember = (event: FocusEvent) => {
      focusedScope = scopeControl(event.target);
    };
    const leave = (event: FocusEvent) => {
      if (!phone.matches || event.relatedTarget !== null) focusedScope = undefined;
    };
    const fit = () => {
      const active = document.activeElement;
      const scope =
        active instanceof HTMLElement && active.closest('.page-scope-filters')
          ? active
          : active === document.body
            ? focusedScope
            : undefined;
      // CSS can hide the focused scope before the media-query change runs.
      if (phone.matches && scope?.isConnected && !scope.getClientRects().length) {
        root
          .querySelector<HTMLElement>('.page-list-search [contenteditable]')
          ?.focus({ preventScroll: true });
        focusedScope = undefined;
      } else if (!phone.matches) focusedScope = undefined;
    };
    root.addEventListener('focusin', remember);
    root.addEventListener('focusout', leave);
    phone.addEventListener('change', fit);
    return () => {
      observer.disconnect();
      resize.disconnect();
      root.removeEventListener('scroll', measure);
      root.removeEventListener('focusin', remember);
      root.removeEventListener('focusout', leave);
      phone.removeEventListener('change', fit);
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
  // The shell's module menu offers the project dialogs while this list can manage them.
  useEffect(() => {
    if (!offerProjectActions || !port) return;
    offerProjectActions({
      newProject: () => setDialog({ kind: 'new', initial: [] }),
      manage: () => setDialog({ kind: 'manage' }),
    });
    return () => offerProjectActions(undefined);
  }, [offerProjectActions, port]);
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
  // The side menu's pin for the project the list is in (owner-written, one request for the list).
  const [pinningProject, setPinningProject] = useState(false);
  async function togglePinnedProject(name: string, pin: boolean) {
    if (pinningProject) return;
    const next = withPin(menuProjects(projectNames, space.projectPins).names, name, pin);
    if (next.length > PROJECT_PINS)
      return setStatus({ text: t.projectPinsFull(PROJECT_PINS), failed: true });
    setPinningProject(true);
    const outcome = await writing.manage({ action: 'pins', names: next });
    setPinningProject(false);
    setStatus(
      outcome.failure === undefined
        ? { text: pin ? t.projectPinned(name) : t.projectUnpinned(name), failed: false }
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
        <div className="page-scope-filters" aria-label={t.projectScopes}>
          {[undefined, ...projectNames].map((name) => (
            <button
              key={name === undefined ? 'all-projects' : `project:${name}`}
              type="button"
              className="project-filter-button"
              aria-label={name === undefined ? t.allProjects : t.scopeTo(name)}
              aria-pressed={scope === name}
              onClick={(event) => {
                if (!event.isTrusted) return;
                writeScope(spaceId, name);
                setSearch({ scope: name });
              }}
            >
              <span className="page-scope-name">{name ?? t.allProjects}</span>
              <span className="project-count">
                {
                  inScope(
                    homePages.filter((page) => !page.archived),
                    name,
                  ).length
                }
              </span>
            </button>
          ))}
        </div>
        <div className="page-list-search">
          <Search className="page-list-search-mark" size={14} aria-hidden="true" />
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
        <span className="page-list-sort">
          <BrowserMenu
            label={`${t.sort}: ${sortLabel}`}
            triggerText={sortLabel}
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
          <BrowserAction
            type="button"
            variant="primary"
            presentation="compact"
            label={t.newPage}
            leadingIcon={<Plus size={14} />}
            onActivate={() => setNewPage(true)}
          />
        </span>
      </div>
    </>
  );
  const newPageAction = (
    <BrowserAction
      type="button"
      variant="primary"
      label={t.newPage}
      leadingIcon={<Plus size={14} />}
      onActivate={() => setNewPage(true)}
    />
  );
  const content = (
    <>
      {port && (
        <div className="page-list-state-filters">
          <BrowserMenu
            label={`Page state: ${search.state ?? 'Any'}`}
            triggerText={`State: ${search.state === 'in-progress' ? 'In progress' : search.state === 'todo' ? 'To do' : search.state === 'done' ? 'Done' : 'Any'}`}
            items={(['', 'todo', 'in-progress', 'done'] as const).map((state) => ({
              key: state || 'any',
              label:
                state === 'todo'
                  ? 'To do'
                  : state === 'in-progress'
                    ? 'In progress'
                    : state === 'done'
                      ? 'Done'
                      : 'Any state',
              role: 'menuitemradio' as const,
              checked: (search.state ?? '') === state,
            }))}
            onSelect={(key) =>
              setSearch({ state: key === 'any' ? undefined : (key as HomeSearch['state']) })
            }
          />
          <label>
            <BrowserCheckbox
              checked={!!search.includeArchived}
              onChange={(event) =>
                setSearch({ includeArchived: event.target.checked ? true : undefined })
              }
            />
            Include archived
          </label>
        </div>
      )}
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
          <div className="page-list-table">
            <div className="page-list-columns">
              <span>
                {port ? (
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
                ) : null}
              </span>
              <span>{t.titleColumn}</span>
              <span>{t.projectColumn}</span>
              <span>{t.stageColumn}</span>
              <span>{t.updatedColumn}</span>
              <span>{t.authorColumn}</span>
              <span>{t.accessColumn}</span>
              <span />
            </div>
            <BrowserList label={text.pages} className="pages">
              {pages.slice(0, drawn).map((p) => {
                return (
                  <PageListItem
                    key={p.id}
                    page={p}
                    hints={titleHints}
                    scope={scope}
                    now={now}
                    selected={selectedIds.has(p.id)}
                    leading={
                      port ? (
                        <BrowserCheckbox
                          aria-label={t.selectPage(pageTitle(p))}
                          checked={selectedIds.has(p.id)}
                          onChange={() => toggleSelected(p.id)}
                        />
                      ) : undefined
                    }
                    projects={
                      port ? (
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
                      ) : null
                    }
                    actions={
                      port ? (
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
                      ) : undefined
                    }
                  />
                );
              })}
            </BrowserList>
          </div>
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
      <header className="page-list-hero">
        <div className="page-list-hero-text">
          <h1 title={scope ?? undefined}>{scope ?? t.heroTitle}</h1>
          <p>{t.heroBody}</p>
          {port && (
            <div className="page-list-shortcuts">
              <BrowserAction
                type="button"
                variant="ghost"
                presentation="compact"
                label={currentShortcut ? 'Edit shortcut' : 'Save as shortcut'}
                onActivate={() => setShortcutDialog(currentShortcut ? 'edit' : 'save')}
              />
              <BrowserIconAction
                type="button"
                variant="text"
                icon={<Settings />}
                label="Manage shortcuts"
                onActivate={() => setShortcutDialog('manage')}
              />
            </div>
          )}
          {shortcuts.error && <p role="alert">{shortcuts.error}</p>}
          {port && scope !== undefined && space.projectPins && (
            <span className="page-list-hero-pin">
              <BrowserAction
                type="button"
                variant="ghost"
                presentation="compact"
                label={
                  menuProjects(projectNames, space.projectPins).names.includes(scope)
                    ? t.unpinFromSidebar
                    : t.pinToSidebar
                }
                leadingIcon={<Pin size={14} />}
                disabled={pinningProject}
                onActivate={() =>
                  void togglePinnedProject(
                    scope,
                    !menuProjects(projectNames, space.projectPins).names.includes(scope),
                  )
                }
              />
            </span>
          )}
        </div>
        <span className="page-list-hero-create" inert={compact}>
          {newPageAction}
        </span>
      </header>
      {shortcutDialog && (
        <ShortcutDialog
          key={`${shortcutDialog}:${currentShortcut?.id ?? ''}`}
          mode={shortcutDialog}
          initial={
            shortcutDialog === 'edit'
              ? currentShortcut
              : shortcutDialog === 'manage'
                ? shortcuts.items[0]
                : undefined
          }
          search={shortcutDialog === 'manage' ? {} : search}
          store={shortcuts.store}
          items={shortcuts.items}
          pages={homePages}
          projects={projectNames}
          error={shortcuts.error}
          onClose={() => setShortcutDialog(undefined)}
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
            compact ? '.page-list-compact-create button' : '.page-list-hero .tmt-ui-action',
          ) ?? null
        }
      />
      {port ? (
        <div className="page-stage-tabs">
          <BrowserTabs
            label={t.views}
            variant="flat"
            wrap
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
            pins={space.projectPins}
            onNewProject={() => setDialog({ kind: 'new', initial: [] })}
            writing={writing}
            onClose={() => setDialog(undefined)}
          />
        </>
      )}
    </section>
  );
}
