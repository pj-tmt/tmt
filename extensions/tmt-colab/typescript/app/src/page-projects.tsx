import { projectTone } from './projects.js';
import { BrowserIconAction, BrowserMenu, useShellBar } from '@tmt/browser-ui/react';
import { Alert, AlertDescription, UiTheme } from '@tmt/browser-ui/ui';
import '@tmt/browser-ui/ui.css';
import { Pin, Plus, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import { rememberPages, rememberedPages } from './quick-switcher-source.js';
import { NewProjectDialog, projectFailureText, type ProjectWriting } from './project-dialogs.js';
import { pageTitle } from './page-index.js';
import { pinFailureText, type PagePinRequest, type PagePinState } from './pins.js';
import { applyProjectChanges, projectCounts, withProject } from './projects.js';
import { text } from './strings.js';
import { getTheme, subscribeTheme } from './theme.js';
import { useMedia } from './use-media.js';
import type { PageSummary, PageTransport, SpaceHome } from './transport.js';
import './page-projects.css';

const t = text.pageList;
/** A page is in at most this many projects (the list row and the page header share the picker). */
const PAGE_LIMIT = 8;
/** The chips beside the title; the rest are a count, the picker names them all. */
const CHIPS = 3;

/** The phone keeps the header mark and also tells the refused action in readable words. */
export function PagePinNotice({ failure, close }: { failure?: string; close(): void }) {
  const phone = useMedia('(max-width: 479px)');
  const bar = useShellBar();
  const theme = useSyncExternalStore(subscribeTheme, getTheme);
  if (!phone || failure === undefined) return null;
  const notice = (
    <div className="page-pin-notice">
      <UiTheme theme={theme}>
        <Alert>
          <AlertDescription>
            <div className="page-pin-notice-content">
              <span>{failure}</span>
              <BrowserIconAction
                type="button"
                variant="text"
                label={t.pinNoticeClose}
                icon={<X />}
                onActivate={close}
              />
            </div>
          </AlertDescription>
        </Alert>
      </UiTheme>
    </div>
  );
  // The shell bar owns the row above its phone sub-bar, so its growth moves both navigation and page.
  return bar ? createPortal(notice, bar) : notice;
}

/** One page's project picker: the list row's and the page header's are this one menu. */
export function PageProjectsMenu({
  page,
  names,
  toggle,
  create,
  openRequest,
  onOpenChange,
}: {
  page: PageSummary;
  names: readonly string[];
  toggle(page: PageSummary, name: string): void;
  create(page: PageSummary): void;
  /** A new value opens the menu: `l`, row Projects… or the page header's admitted request. */
  openRequest?: number;
  onOpenChange?(open: boolean): void;
}) {
  const carried = new Set(page.projects ?? []);
  const full = carried.size >= PAGE_LIMIT;
  return (
    <BrowserMenu
      label={t.projectsFor(pageTitle(page))}
      icon={<Plus />}
      openRequest={openRequest}
      onOpenChange={onOpenChange}
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
 * the list row (#4116). The chips come from the list the space home last loaded, so arriving from the
 * list reads nothing extra. A page opened by a link has no list yet: it is read once on open, and
 * leaving the page cancels that read (#4130). The owner list is also read when the picker opens
 * (after showing what is known) and again after each write. A viewer who cannot write projects shows
 * nothing.
 */
export function PageProjects({
  transport,
  space,
  pageId,
  title,
  request,
  from,
  open,
  pinRequest,
  onPinState,
}: {
  transport: PageTransport;
  space: string | undefined;
  pageId: string;
  title: string;
  /** Changes to open the picker, from the chips and from Projects… in the More menu. */
  request: number;
  /** Where the request came from when it is not the chips: the More trigger's box, which the picker
   * opens under (right-aligned like the More menu itself) instead of under the chips. */
  from?: { top: number; left: number; width: number; height: number };
  open(): void;
  /** A new request from the phone's More menu uses the header pin's existing write. */
  pinRequest?: PagePinRequest;
  onPinState?(state: PagePinState | undefined): void;
}) {
  const port = transport.management;
  const [home, setHome] = useState<SpaceHome>();
  const [failure, setFailure] = useState<{ kind: 'project' | 'pin'; message: string }>();
  const [saved, setSaved] = useState(false);
  const [creating, setCreating] = useState(false);
  const [pinning, setPinning] = useState(false);
  const phone = useMedia('(max-width: 479px)');
  useEffect(() => setFailure(undefined), [pageId]);
  // The picker opens once its data is here: this counts those openings, not the requests.
  const [opened, setOpened] = useState(0);
  const [pickerOpen, setPickerOpen] = useState(false);
  const origin = useRef<HTMLElement | null>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  function openFromControl(event: React.MouseEvent<HTMLButtonElement>) {
    origin.current = event.currentTarget;
    open();
  }
  // A new transport (a refreshed session) is read through, never a reason to read the list again.
  const latest = useRef(transport);
  latest.current = transport;
  // Leaving the page cancels whatever list read is still running.
  const leaving = useRef<AbortController | undefined>(undefined);
  const reading = useRef<Promise<void> | undefined>(undefined);
  useEffect(() => {
    const controller = (leaving.current = new AbortController());
    return () => {
      controller.abort();
      reading.current = undefined;
    };
  }, []);
  const known = home?.pages ?? rememberedPages(transport, space);
  // One read at a time: a request that finds one running joins it.
  const load = useCallback(() => {
    const read = async () => {
      const signal = leaving.current?.signal;
      try {
        const next = await latest.current.spaceHome(signal);
        if (signal?.aborted) return;
        rememberPages(latest.current, spaceRef.current, next.pages, next.projects);
        setHome(next);
      } catch {
        // The picker keeps what it showed: the page itself is unaffected.
      }
    };
    if (!reading.current) {
      const run: Promise<void> = read().finally(() => {
        if (reading.current === run) reading.current = undefined;
      });
      reading.current = run;
    }
    return reading.current;
  }, []);
  const spaceRef = useRef(space);
  spaceRef.current = space;
  // A page opened by a link has no list to show chips from: read it once.
  useEffect(() => {
    if (port && !rememberedPages(transport, space)) void load();
    // Only the page opening reads; later reads are the picker's and the writes'.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageId, space, !!port]);
  const first = useRef(request);
  useEffect(() => {
    if (request === first.current || !port) return;
    first.current = request;
    if (from) origin.current = document.activeElement as HTMLElement | null;
    // Show what is known at once and refresh behind it; with nothing known, open when the read lands.
    if (known) {
      setOpened((count) => count + 1);
      void load();
    } else
      void load().then(() => !leaving.current?.signal.aborted && setOpened((count) => count + 1));
    // Only a new request opens the picker: the data it finds is read as it stands.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [request]);
  const listedPage = known?.find((candidate) => candidate.id === pageId);
  const pinned =
    port && listedPage && !listedPage.archived ? listedPage.pinned === true : undefined;
  useEffect(() => {
    onPinState?.(
      pinned === undefined
        ? undefined
        : { pinned, busy: pinning, failure: failure?.kind === 'pin' ? failure.message : undefined },
    );
  }, [pinned, pinning, failure, onPinState]);
  const firstPin = useRef(pinRequest);
  useEffect(() => {
    if (pinRequest === firstPin.current) return;
    firstPin.current = pinRequest;
    if (pinRequest?.action === 'dismiss')
      setFailure((current) => (current?.kind === 'pin' ? undefined : current));
    else if (pinRequest?.action === 'toggle' && pinned !== undefined) void togglePin();
    // Only a new action asks for a write, never a state projection or owner-list refresh.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pinRequest]);
  if (!port) return null;
  const page: PageSummary = listedPage ?? {
    id: pageId,
    title,
    sharing: 'private',
  };
  const names = projectCounts(known ?? [], home?.projects).map((project) => project.name);
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
    if (change === 'capacity') return setFailure({ kind: 'project', message: t.pageFull });
    const outcome = await writing.save([change]);
    setFailure(
      outcome.failure === undefined
        ? undefined
        : { kind: 'project', message: projectFailureText(outcome.failure) },
    );
    setSaved(outcome.failure === undefined);
  }
  /** Pins the page to the top of Home, or takes the pin away; the owner list is read again after. */
  async function togglePin() {
    if (pinning || pinned === undefined) return;
    setPinning(true);
    try {
      await port!.setPinned(page.id, !page.pinned);
      setFailure(undefined);
      setSaved(false);
      await load();
    } catch (error) {
      setFailure({ kind: 'pin', message: pinFailureText(error) });
    }
    setPinning(false);
  }
  return (
    <>
      <span className="page-header-projects">
        {projects.length > 0 && (
          <button
            type="button"
            className="page-header-chips"
            aria-haspopup="menu"
            aria-label={`${t.projectsFor(pageTitle(page))}: ${projects.join(', ')}`}
            aria-expanded={pickerOpen}
            onClick={openFromControl}
          >
            {projects.slice(0, CHIPS).map((name) => (
              <span
                key={name}
                className="page-header-project-chip"
                data-project-tone={projectTone(name)}
              >
                {name}
              </span>
            ))}
            {projects.length > CHIPS && (
              <span className="page-header-project-chip">+{projects.length - CHIPS}</span>
            )}
          </button>
        )}
        <span className="page-header-project-add">
          <button
            ref={addButton}
            type="button"
            className="page-header-add-project"
            aria-label={t.addToProject}
            aria-haspopup="menu"
            aria-expanded={pickerOpen}
            onClick={openFromControl}
          >
            <Plus size={14} aria-hidden="true" />
          </button>
          {/* The existing request/read pipeline admits opening; this hidden trigger anchors its menu. */}
          <span
            className="page-header-projects-anchor"
            data-from={from ? 'more' : 'chips'}
            style={
              from && { top: from.top, left: from.left, width: from.width, height: from.height }
            }
          >
            <PageProjectsMenu
              page={page}
              names={names}
              openRequest={opened}
              onOpenChange={(value) => {
                setPickerOpen(value);
                if (!value)
                  queueMicrotask(() => {
                    // The leaf restores its hidden anchor first. Do not steal focus from a new dialog.
                    const active = document.activeElement;
                    if (
                      active !== document.body &&
                      !active?.closest('.page-header-projects-anchor')
                    )
                      return;
                    const target = origin.current?.isConnected ? origin.current : addButton.current;
                    target?.focus({ preventScroll: true });
                  });
              }}
              toggle={(target, name) => void toggle(target, name)}
              create={() => setCreating(true)}
            />
          </span>
        </span>
        {/* A page the owner list does not name, or an archived one, has nothing to pin. */}
        {pinned !== undefined && (
          <span className="page-header-pin">
            <BrowserIconAction
              type="button"
              variant="text"
              label={page.pinned ? t.unpinPage(pageTitle(page)) : t.pinPage(pageTitle(page))}
              icon={<Pin />}
              pressed={page.pinned === true}
              busy={pinning}
              onActivate={() => void togglePin()}
            />
          </span>
        )}
        {failure !== undefined && (
          <span
            className="page-attention"
            data-attention="projects"
            role={phone && failure.kind === 'pin' ? undefined : 'alert'}
          >
            <span aria-hidden="true">△</span>
            <span className="page-attention-word">{failure.message}</span>
          </span>
        )}
        <span className="page-header-projects-status tmt-ui-visually-hidden" role="status">
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
