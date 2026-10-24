import { getRouteApi, useNavigate } from '@tanstack/react-router';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useHomeSpace } from './home-space.js';
import { ReadingDetailsQueue, type ReadingDetails } from './reading-details.js';
import type { Shell } from './shell.js';
import { shellCells } from './shell-grid.js';
import { ChapterPanel, type EmbedPage } from './shell-panel.js';
import { detailPageIds, shellModel, startOfDay } from './shell-model.js';
import {
  ShellAside,
  ShellCounts,
  ShellHead,
  ShellPages,
  ShellSummary,
  ShellUnavailable,
  shellItems,
  type Jump,
} from './shell-sections.js';
import { text } from './strings.js';
import type { SpaceHome } from './transport.js';
import { withViewTransition } from './view-transition.js';
import './shell-page.css';

const rootRoute = getRouteApi('__root__');
const pageRoute = getRouteApi('/pages/$pageId');
/** From this width a chapter opens beside the shell; narrower, it goes straight to its page. */
const WIDE = '(min-width: 900px)';
const t = text.shellPage;

/** The dashboard: the app owns the whole look, the document only supplies the data. Every number,
 * the order and each chapter's passage are computed here from the space list and the owner chapter
 * details (`shell-model.ts`); items are links to their source pages and carry no content of their
 * own. A page with no admitted value says so instead of drawing nothing. */
export function ShellPage({
  shell,
  title,
  embed,
}: {
  shell: Shell | undefined;
  /** The page's own title: the headline of the dashboard. */
  title: string;
  /** Hosts the chapter's page itself in its panel; absent where the app cannot (no panel). */
  embed?: EmbedPage;
}) {
  return shell ? (
    <ShellContent shell={shell} title={title} embed={embed} />
  ) : (
    <div className="shell-page tmt-ui-page">
      <p className="shell-state" role="status">
        {t.notShown}
      </p>
    </div>
  );
}

function ShellContent({
  shell,
  title,
  embed,
}: {
  shell: Shell;
  title: string;
  embed: EmbedPage | undefined;
}) {
  const { transport } = rootRoute.useRouteContext();
  const [home, setHome] = useState<SpaceHome | 'failed' | undefined>();
  useEffect(() => {
    let live = true;
    transport.spaceHome().then(
      (value) => live && setHome(value),
      () => live && setHome('failed'),
    );
    return () => {
      live = false;
    };
  }, [transport]);
  if (home === undefined)
    return (
      <div className="shell-page tmt-ui-page">
        <p className="shell-state" role="status">
          {t.loading}
        </p>
      </div>
    );
  return (
    <ShellBody
      shell={shell}
      title={title}
      home={home === 'failed' ? { title: '', pages: [] } : home}
      failed={home === 'failed'}
      embed={embed}
    />
  );
}

/** The owner's chapter details of the first chapters, read once per opening (a return to the shell
 * reads again, so a `Mark read` on the page shows). Optional and owner-only: a transport without
 * the port, a failed read or a pending page leaves a chapter without a passage, never a false one. */
function useChapterDetails(pageIds: readonly string[]): ReadonlyMap<string, ReadingDetails> {
  const { transport } = rootRoute.useRouteContext();
  const [details, setDetails] = useState<ReadonlyMap<string, ReadingDetails>>(new Map());
  const key = pageIds.join(' ');
  useEffect(() => {
    const read = transport.readingDetails?.bind(transport);
    if (!read || !key) return;
    const queue = new ReadingDetailsQueue(read, (id, value) =>
      setDetails((previous) => new Map(previous).set(id, value)),
    );
    queue.enqueue(key.split(' '));
    return () => {
      void queue.close();
    };
  }, [transport, key]);
  return details;
}

function useWide(): boolean {
  const [wide, setWide] = useState(() => window.matchMedia(WIDE).matches);
  useEffect(() => {
    const query = window.matchMedia(WIDE);
    const change = () => setWide(query.matches);
    query.addEventListener('change', change);
    return () => query.removeEventListener('change', change);
  }, []);
  return wide;
}

function ShellBody({
  shell,
  title,
  home,
  failed,
  embed,
}: {
  shell: Shell;
  title: string;
  home: SpaceHome;
  failed: boolean;
  embed: EmbedPage | undefined;
}) {
  const { pages, hints } = useHomeSpace(home);
  const [now] = useState(() => Date.now());
  const today = startOfDay(now);
  const cells = useMemo(
    () =>
      failed
        ? shellCells(shell, []).map((cell) => ({ ...cell, unavailable: false, titleMissing: true }))
        : shellCells(shell, pages),
    [shell, pages, failed],
  );
  // The order and the page IDs to ask about do not depend on the details; only passages do.
  const base = useMemo(() => shellModel(cells, new Map(), now), [cells, now]);
  const askAbout = useMemo(() => (failed ? [] : detailPageIds(base)), [failed, base]);
  const details = useChapterDetails(askAbout);
  const model = useMemo(() => shellModel(cells, details, now), [cells, details, now]);
  const byId = useMemo(() => new Map(pages.map((page) => [page.id, page])), [pages]);
  const root = useRef<HTMLDivElement>(null);
  const wide = useWide();
  const navigate = useNavigate();
  const { open } = pageRoute.useSearch();
  const opened = wide ? model.chapters.find((chapter) => chapter.pageId === open) : undefined;
  // The panel is a history entry of its own, so Back closes it and a reload or a pasted link opens
  // it again. Closing a panel this window opened steps back to the entry before it; one that arrived
  // by link only drops the parameter.
  const pushed = useRef(false);
  const openPanel = (pageId: string) => {
    pushed.current = true;
    void withViewTransition(() =>
      navigate({ to: '.', search: (previous) => ({ ...previous, open: pageId }) }),
    );
  };
  const closePanel = () =>
    void withViewTransition(() => {
      if (pushed.current) {
        pushed.current = false;
        window.history.back();
        return;
      }
      return navigate({
        to: '.',
        search: (previous) => ({ ...previous, open: undefined }),
        replace: true,
      });
    });

  // Coming back from a page selects that page again and puts focus on its row; the browser restores
  // the scroll.
  const items = useMemo(() => shellItems(model), [model]);
  const all = useMemo(() => [...items.chapters, ...items.earlier], [items]);
  const returned = useRef((window.history.state as { shellFocus?: string } | null)?.shellFocus);
  const [chosen, setChosen] = useState<string | undefined>(returned.current);
  const selected = all.find((item) => item.pageId === chosen) ?? all[0];
  useEffect(() => {
    const id = returned.current;
    if (!id) return;
    const row = root.current?.querySelector<HTMLElement>(`[data-page-id="${id}"] button.shell-row`);
    row?.focus({ preventScroll: true });
    row?.scrollIntoView({ block: 'nearest' });
  }, []);
  const remember = (pageId: string) => () => {
    window.history.replaceState({ ...window.history.state, shellFocus: pageId }, '');
  };
  const calm = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  // Where the list and the summary stack, a chosen row brings its summary into view.
  const select = (pageId: string) => {
    setChosen(pageId);
    const columns = root.current?.querySelector<HTMLElement>('.shell-columns');
    if (columns && getComputedStyle(columns).gridTemplateColumns.trim().split(/\s+/).length === 1)
      requestAnimationFrame(() =>
        root.current
          ?.querySelector('#shell-aside')
          ?.scrollIntoView({ block: 'nearest', behavior: calm() ? 'auto' : 'smooth' }),
      );
  };
  const jump = (to: Jump) => {
    const row = root.current?.querySelector<HTMLElement>(
      to === 'today' ? '.shell-row-item[data-today]' : `.shell-row-item[data-stage="${to}"]`,
    );
    if (!row) return;
    setChosen(row.dataset.pageId);
    row.scrollIntoView({ block: 'start', behavior: calm() ? 'auto' : 'smooth' });
    row.querySelector<HTMLElement>('button')?.focus({ preventScroll: true });
  };

  const empty = cells.length === 0;
  return (
    <div className="shell-page tmt-ui-page" ref={root}>
      <ShellHead title={title} model={model} />
      <section className="tmt-ui-card shell-glance" aria-labelledby="shell-glance-heading">
        <h2 id="shell-glance-heading" className="tmt-ui-card-title">
          {t.glance}
        </h2>
        <ShellSummary shell={shell} cells={cells} now={now} />
        {failed && (
          <p className="shell-state" role="status">
            {t.pagesUnavailable}
          </p>
        )}
        {empty ? (
          <p className="shell-state" role="status">
            {t.empty}
          </p>
        ) : (
          <>
            <ShellCounts model={model} onJump={jump} />
            <div className="shell-columns">
              <ShellPages
                items={items}
                pages={byId}
                hints={hints}
                now={now}
                today={today}
                selected={selected?.pageId}
                onSelect={select}
              />
              {selected && (
                <ShellAside
                  item={selected}
                  page={byId.get(selected.pageId)}
                  now={now}
                  onOpen={remember(selected.pageId)}
                  onPanel={wide ? openPanel : undefined}
                />
              )}
            </div>
            <ShellUnavailable count={model.unavailable.length} />
          </>
        )}
      </section>
      {opened && (
        <ChapterPanel
          chapter={opened}
          details={details.get(opened.pageId)}
          now={now}
          onDismiss={closePanel}
          onOpen={remember(opened.pageId)}
          embed={embed}
        />
      )}
    </div>
  );
}
