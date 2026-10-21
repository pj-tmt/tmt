import { getRouteApi } from '@tanstack/react-router';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useHomeSpace } from './home-space.js';
import { ReadingDetailsQueue, type ReadingDetails } from './reading-details.js';
import type { Shell } from './shell.js';
import { shellCells } from './shell-grid.js';
import { detailPageIds, shellModel, startOfDay } from './shell-model.js';
import {
  ShellChapter,
  ShellCounts,
  ShellEarlier,
  ShellGreeting,
  ShellUnavailable,
  type Jump,
} from './shell-sections.js';
import { text } from './strings.js';
import type { SpaceHome } from './transport.js';
import './shell-page.css';

const rootRoute = getRouteApi('__root__');
const t = text.shellPage;

/** The shell page: the app owns the whole look, the document only supplies the data. Every number,
 * the order and each chapter's passage are computed here from the space list and the owner chapter
 * details (`shell-model.ts`); items are links to their source pages and carry no content of their
 * own. A page with no admitted value says so instead of drawing nothing. */
export function ShellPage({ shell }: { shell: Shell | undefined }) {
  return shell ? (
    <ShellContent shell={shell} />
  ) : (
    <div className="shell-page">
      <p className="shell-state" role="status">
        {t.notShown}
      </p>
    </div>
  );
}

function ShellContent({ shell }: { shell: Shell }) {
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
      <div className="shell-page">
        <p className="shell-state" role="status">
          {t.loading}
        </p>
      </div>
    );
  return (
    <ShellBody
      shell={shell}
      home={home === 'failed' ? { title: '', pages: [] } : home}
      failed={home === 'failed'}
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

function ShellBody({ shell, home, failed }: { shell: Shell; home: SpaceHome; failed: boolean }) {
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

  // Coming back from a page restores focus to the chapter's title; the browser restores the scroll.
  useEffect(() => {
    const id = (window.history.state as { shellFocus?: string } | null)?.shellFocus;
    if (!id) return;
    const link = root.current?.querySelector<HTMLElement>(`[data-page-id="${id}"] a`);
    link?.focus({ preventScroll: true });
    link?.scrollIntoView({ block: 'nearest' });
  }, []);
  const remember = (pageId: string) => () => {
    window.history.replaceState({ ...window.history.state, shellFocus: pageId }, '');
  };
  const jump = (to: Jump) => {
    const row = root.current?.querySelector<HTMLElement>(
      to === 'today' ? '[data-today]' : `.shell-chapter[data-stage="${to}"]`,
    );
    if (!row) return;
    const calm = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    row.scrollIntoView({ block: 'start', behavior: calm ? 'auto' : 'smooth' });
    row.querySelector<HTMLElement>('a')?.focus({ preventScroll: true });
  };

  const empty = cells.length === 0;
  return (
    <div className="shell-page" ref={root}>
      <ShellGreeting model={model} shell={shell} cells={cells} now={now} />
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
          <ol className="shell-chapters" aria-label={t.chapters}>
            {model.chapters.map((chapter) => (
              <ShellChapter
                key={chapter.pageId}
                chapter={chapter}
                page={byId.get(chapter.pageId)}
                hints={hints}
                now={now}
                today={today}
                onOpen={remember(chapter.pageId)}
              />
            ))}
          </ol>
          <ShellEarlier
            rows={model.earlier}
            start={model.chapters.length + 1}
            pages={byId}
            hints={hints}
            now={now}
            today={today}
            onOpen={() => undefined}
          />
          <ShellUnavailable count={model.unavailable.length} />
        </>
      )}
    </div>
  );
}
