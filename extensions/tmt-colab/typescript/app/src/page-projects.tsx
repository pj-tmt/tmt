import { BrowserMenu } from '@tmt/browser-ui/react';
import { Tag } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { NewProjectDialog, projectFailureText, type ProjectWriting } from './project-dialogs.js';
import { pageTitle } from './page-index.js';
import { applyProjectChanges, projectCounts, withProject } from './projects.js';
import { rememberPages } from './quick-switcher-source.js';
import { text } from './strings.js';
import type { PageSummary, PageTransport, SpaceHome } from './transport.js';
import './page-projects.css';

const t = text.pageList;
/** A page is in at most this many projects (the list row and the page header share the picker). */
const PAGE_LIMIT = 8;
/** The chips beside the title; the rest are a count, the picker names them all. */
const CHIPS = 3;

/** One page's project picker: the list row's and the page header's are this one menu. */
export function PageProjectsMenu({
  page,
  names,
  toggle,
  create,
  openRequest,
}: {
  page: PageSummary;
  names: readonly string[];
  toggle(page: PageSummary, name: string): void;
  create(page: PageSummary): void;
  /** A new value opens the menu: the `l` shortcut, or Projects… in the page header's More menu. */
  openRequest?: number;
}) {
  const carried = new Set(page.projects ?? []);
  const full = carried.size >= PAGE_LIMIT;
  return (
    <BrowserMenu
      label={t.projectsFor(pageTitle(page))}
      icon={<Tag />}
      openRequest={openRequest}
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

/**
 * The page's own projects, after its title: the current ones as small chips and the same picker as
 * the list row (#4116). A page the owner list does not hold, or a viewer who cannot write projects,
 * shows nothing. The owner list is read once when the page opens and again after each write.
 */
export function PageProjects({
  transport,
  space,
  pageId,
  request,
  open,
  onLoaded,
}: {
  transport: PageTransport;
  space: string | undefined;
  pageId: string;
  /** Changes to open the picker, from the chips and from Projects… in the More menu. */
  request: number;
  open(): void;
  /** Whether the picker can open: the viewer writes projects and the page is in the owner list. */
  onLoaded(available: boolean): void;
}) {
  const port = transport.management;
  const [home, setHome] = useState<SpaceHome>();
  const [failure, setFailure] = useState<string>();
  const [saved, setSaved] = useState(false);
  const [creating, setCreating] = useState(false);
  const current = useRef(true);
  const load = useCallback(async () => {
    try {
      const next = await transport.spaceHome();
      if (!current.current) return;
      rememberPages(transport, space, next.pages);
      setHome(next);
    } catch {
      // The picker stays unavailable (or keeps what it showed): the page itself is unaffected.
    }
  }, [transport, space]);
  useEffect(() => {
    current.current = true;
    if (port) void load();
    return () => {
      current.current = false;
    };
  }, [port, load, pageId]);
  const page = home?.pages.find((candidate) => candidate.id === pageId);
  useEffect(() => onLoaded(!!port && !!page), [onLoaded, port, page]);
  if (!port || !home || !page) return null;
  const names = projectCounts(home.pages, home.projects).map((project) => project.name);
  const projects = page.projects ?? [];
  const writing: ProjectWriting = {
    async save(changes) {
      const outcome = await applyProjectChanges(port, changes);
      if (outcome.updated) await load();
      return outcome;
    },
    async manage(change) {
      try {
        await port.manageProject(change);
      } catch (error) {
        return { failure: error };
      }
      await load();
      return {};
    },
    failureText: projectFailureText,
  };
  async function toggle(target: PageSummary, name: string) {
    const change = withProject(target, name, !target.projects?.includes(name));
    if (change === 'capacity') return setFailure(t.pageFull);
    const outcome = await writing.save([change]);
    setFailure(outcome.failure === undefined ? undefined : projectFailureText(outcome.failure));
    setSaved(outcome.failure === undefined);
  }
  return (
    <>
      <span className="page-header-projects">
        {projects.slice(0, CHIPS).map((name) => (
          // The picker's own trigger is the accessible control; a chip is the same press for the mouse.
          <button
            key={name}
            type="button"
            className="page-header-project-chip"
            tabIndex={-1}
            aria-hidden="true"
            onClick={open}
          >
            {name}
          </button>
        ))}
        {projects.length > CHIPS && (
          <button
            type="button"
            className="page-header-project-chip"
            tabIndex={-1}
            aria-hidden="true"
            onClick={open}
          >
            +{projects.length - CHIPS}
          </button>
        )}
        <PageProjectsMenu
          page={page}
          names={names}
          openRequest={request}
          toggle={(target, name) => void toggle(target, name)}
          create={() => setCreating(true)}
        />
        {failure !== undefined && (
          <span className="page-attention" data-attention="projects" role="alert">
            <span aria-hidden="true">△</span>
            {failure}
          </span>
        )}
        <span className="page-header-projects-status" role="status">
          {saved ? t.saved : ''}
        </span>
      </span>
      <NewProjectDialog
        open={creating}
        pages={[page]}
        existing={names}
        initial={[page.id]}
        writing={writing}
        onClose={() => setCreating(false)}
      />
    </>
  );
}
