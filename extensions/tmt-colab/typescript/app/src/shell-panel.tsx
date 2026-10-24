import { Link } from '@tanstack/react-router';
import { useState, type ReactNode } from 'react';
import { BrowserDrawer } from '@tmt/browser-ui/react';
import { AgentChip } from './components/agent-chip.js';
import { compactRelativeTime } from './display-time.js';
import { pageLinkKey } from './page-link.js';
import type { LatestComment, ReadingDetails } from './reading-details.js';
import type { Chapter } from './shell-model.js';
import { text } from './strings.js';

const t = text.shellPage;

/** Hosts a chapter's page itself in the panel. The page owns a live connection that closes when
 * this leaves the panel, so the panel drops it before the reader goes to the full page. */
export type EmbedPage = (pageId: string) => ReactNode;

function LatestBlock({ latest, now }: { latest: LatestComment; now: number }) {
  return (
    <div className="shell-panel-latest">
      <p className="shell-passage-lead">{t.latest}</p>
      <p className="shell-passage-text">{latest.text}</p>
      <p className="shell-passage-meta">
        {latest.who && <>{latest.who} · </>}
        {compactRelativeTime(latest.atMs, now)}
      </p>
    </div>
  );
}

/** A chapter opened beside the shell: everything its page lists that needs the owner, in the order
 * the page ranks it, then its latest comment, as text. Nothing here changes anything; each line
 * opens the page where it can be decided. A chapter whose details are not here shows its note. */
export function ChapterPanel({
  chapter,
  details,
  now,
  onDismiss,
  onOpen,
  embed,
}: {
  chapter: Chapter;
  details: ReadingDetails | undefined;
  now: number;
  onDismiss(): void;
  onOpen(): void;
  embed: EmbedPage | undefined;
}) {
  /** The page itself in place of the passage list: "Decide here". Panel state, not the address. */
  const [here, setHere] = useState(false);
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
        document.querySelector<HTMLElement>(`[data-page-id="${chapter.pageId}"] button.shell-row`)
      }
      onDismiss={onDismiss}
    >
      {here && embed ? (
        <div className="shell-panel shell-panel-page">
          <div className="shell-panel-bar">
            <button
              type="button"
              className="tmt-ui-action shell-open"
              data-variant="text"
              onClick={() => setHere(false)}
            >
              {t.backToSummary}
            </button>
            <Link
              className="tmt-ui-action shell-open"
              data-variant="text"
              to="/pages/$pageId"
              params={{ pageId: chapter.pageId }}
              onClick={() => {
                setHere(false);
                onOpen();
              }}
              onKeyDown={pageLinkKey}
            >
              {t.openFullPage}
            </Link>
          </div>
          {embed(chapter.pageId)}
        </div>
      ) : (
        <div className="shell-panel">
          {embed && (
            <button
              type="button"
              className="tmt-ui-action shell-open"
              data-variant="text"
              onClick={() => setHere(true)}
            >
              {t.decideHere}
            </button>
          )}
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
                  {need.kind === 'reading' ? (
                    <Link
                      className="tmt-ui-action shell-open"
                      data-variant="text"
                      to="/pages/$pageId"
                      params={{ pageId: chapter.pageId }}
                      search={{ read: need.id }}
                      onClick={onOpen}
                      onKeyDown={pageLinkKey}
                    >
                      {t.openPassage}
                    </Link>
                  ) : (
                    <Link
                      className="tmt-ui-action shell-open"
                      data-variant="text"
                      to="/pages/$pageId"
                      params={{ pageId: chapter.pageId }}
                      onClick={onOpen}
                      onKeyDown={pageLinkKey}
                    >
                      {t.openOnPage}
                    </Link>
                  )}
                </li>
              ))}
            </ol>
          ) : latest ? (
            <LatestBlock latest={latest} now={now} />
          ) : (
            <p className="shell-passage-note">
              {chapter.passage.kind === 'note' ? chapter.passage.note : t.panelNothing}
            </p>
          )}
          {needs.length > 0 && latest && <LatestBlock latest={latest} now={now} />}
          {needs.length === 0 && chapter.anchor !== undefined && (
            <Link
              className="tmt-ui-action shell-open"
              data-variant="text"
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
            className="tmt-ui-action shell-open"
            data-variant="text"
            to="/pages/$pageId"
            params={{ pageId: chapter.pageId }}
            onClick={onOpen}
            onKeyDown={pageLinkKey}
          >
            {t.openFullPage}
          </Link>
        </div>
      )}
    </BrowserDrawer>
  );
}
