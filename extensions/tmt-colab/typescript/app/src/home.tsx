import { HomeTitleLink } from './home-titles.js';
import { useHomeSpace, useSpaceTitle } from './home-space.js';
import { BrowserAction, BrowserList, BrowserListRow, BrowserMenu } from '@tmt/browser-ui/react';
import { getRouteApi, useRouter } from '@tanstack/react-router';
import { Settings, Tag } from 'lucide-react';
import { useContext, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { ProjectWriting, ManageProjectsDialog, NewProjectDialog } from './project-dialogs.js';
import {
  applyProjectChanges,
  authorCounts,
  waitingCount,
  filterPages,
  projectCounts,
  projectNameError,
  withProject,
  type PageSort,
} from './projects.js';
import { AUTHOR_BYTES } from './bootstrap.js';
import { HeaderSlot } from './header-slot.js';
import { ProjectScopeMenu } from './project-scope-menu.js';
import { inScope, knownScope, scopeRows, writeScope } from './project-scope.js';
import { ManagementError } from './management.js';
import { ManagePage } from './manage-page.js';
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

function PageProjectsMenu({
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
  const carried = new Set(page.projects ?? []);
  const full = carried.size >= 8;
  return (
    <BrowserMenu
      label={t.projectsFor(pageTitle(page))}
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
          label: t.newProjectItem,
          ...(full ? { disabled: true, disabledReason: t.pageFull } : {}),
        },
      ]}
      onSelect={(key) => (key === 'new' ? create(page) : toggle(page, key.slice('label:'.length)))}
    />
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
    failureText(failure) {
      const code = failure instanceof ManagementError ? failure.code : 'UNKNOWN';
      return t.failures[code as keyof typeof t.failures] ?? t.failures.UNKNOWN;
    },
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
            projects={counts}
            authors={authors}
            onApply={apply}
            onClear={() => {
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
            ]}
            onSelect={() => setSearch({ archived: archived ? undefined : true })}
          />
        )}
      </div>
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
        {[archived ? t.archivedView : '', status?.text].filter(Boolean).join(' · ')}
      </p>
      {pages.length ? (
        <BrowserList label={text.pages} className="pages">
          {pages.map((p) => {
            const updated = pageUpdate(p.lastUpdateAtMs, now);
            return (
              <BrowserListRow
                key={p.id}
                data-page-id={p.id}
                title={
                  <>
                    <HomeTitleLink page={p} hints={titleHints} scope={scope} />
                    {p.summary?.waiting ? (
                      <span className="page-waiting" data-waiting>
                        <span aria-hidden="true">◆</span> {t.waitingForYou}
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
                      <PageProjectsMenu
                        page={p}
                        names={projectNames}
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
          {scope !== undefined && scoped.length === 0 ? (
            <p>{t.noPagesInProject(scope)}</p>
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
              <code className="tmt-ui-code">tmt colab page create --title "Notes"</code>.
            </p>
          )}
        </div>
      )}
      {port && (
        <>
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
