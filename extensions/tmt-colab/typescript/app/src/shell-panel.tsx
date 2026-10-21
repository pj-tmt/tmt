import { Link } from '@tanstack/react-router';
import { BrowserDrawer } from '@tmt/browser-ui/react';
import { AgentChip } from './components/agent-chip.js';
import { compactRelativeTime } from './display-time.js';
import { pageLinkKey } from './page-link.js';
import type { ReadingDetails } from './reading-details.js';
import type { Chapter } from './shell-model.js';
import { text } from './strings.js';

const t = text.shellPage;

/** A chapter opened beside the shell: everything its page lists that needs the owner, in the order
 * the page ranks it, then its latest comment, as text. Nothing here changes anything; each line
 * opens the page where it can be decided. A chapter whose details are not here shows its note. */
export function ChapterPanel({
  chapter,
  details,
  now,
  onDismiss,
  onOpen,
}: {
  chapter: Chapter;
  details: ReadingDetails | undefined;
  now: number;
  onDismiss(): void;
  onOpen(): void;
}) {
  const ready = details?.state === 'ready' ? details : undefined;
  const needs = ready?.needs ?? [];
  const latest = ready?.latest;
  return (
    <BrowserDrawer
      open
      title={chapter.title || text.unknownPageTitle}
      caption={
        <>
          {chapter.author && <AgentChip name={chapter.author} />}
          {chapter.lastUpdateAtMs !== null && (
            <>
              {chapter.author ? ' · ' : ''}
              {compactRelativeTime(chapter.lastUpdateAtMs, now)}
            </>
          )}
        </>
      }
      panel="shell-chapter"
      focusFallback={() =>
        document.querySelector<HTMLElement>(
          `[data-page-id="${chapter.pageId}"] .shell-chapter-title a`,
        )
      }
      onDismiss={onDismiss}
    >
      <div className="shell-panel">
        {needs.length > 0 ? (
          <ol className="shell-panel-needs" aria-label={t.panelNeeds}>
            {needs.map((need) => (
              <li key={`${need.kind}:${need.id}`} data-need={need.kind}>
                <p className="shell-passage-lead">
                  <span className="shell-mark" aria-hidden="true">
                    ◆
                  </span>{' '}
                  {t.needKind[need.kind]}
                  {need.agent && (
                    <>
                      {' · '}
                      <AgentChip name={need.agent} />
                    </>
                  )}
                </p>
                <p className="shell-passage-title">{need.title}</p>
                {need.why && <p className="shell-passage-why">{need.why}</p>}
                {need.kind === 'reading' && (
                  <Link
                    className="shell-open"
                    to="/pages/$pageId"
                    params={{ pageId: chapter.pageId }}
                    search={{ read: need.id }}
                    onClick={onOpen}
                    onKeyDown={pageLinkKey}
                  >
                    {t.openPassage}
                  </Link>
                )}
              </li>
            ))}
          </ol>
        ) : latest ? (
          <div className="shell-panel-latest">
            <p className="shell-passage-lead">{t.latest}</p>
            <p className="shell-passage-text">{latest.text}</p>
            <p className="shell-passage-meta">
              {latest.who && <>{latest.who} · </>}
              {compactRelativeTime(latest.atMs, now)}
            </p>
          </div>
        ) : (
          <p className="shell-passage-note">
            {chapter.passage.kind === 'note' ? chapter.passage.note : t.panelNothing}
          </p>
        )}
        {needs.length > 0 && latest && (
          <div className="shell-panel-latest">
            <p className="shell-passage-lead">{t.latest}</p>
            <p className="shell-passage-text">{latest.text}</p>
          </div>
        )}
        {chapter.anchor !== undefined && (
          <Link
            className="shell-open"
            to="/pages/$pageId"
            params={{ pageId: chapter.pageId }}
            search={{ anchor: chapter.anchor }}
            onClick={onOpen}
            onKeyDown={pageLinkKey}
          >
            {t.openPassage}
          </Link>
        )}
        <Link
          className="shell-open"
          to="/pages/$pageId"
          params={{ pageId: chapter.pageId }}
          onClick={onOpen}
          onKeyDown={pageLinkKey}
        >
          {t.openFullPage}
        </Link>
      </div>
    </BrowserDrawer>
  );
}
