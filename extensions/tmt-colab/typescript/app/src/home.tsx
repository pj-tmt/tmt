import { BrowserList, BrowserListRow, BrowserMenu, BrowserToggle } from '@tmt/browser-ui/react';
import { getRouteApi, Link } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { ManagePage } from './manage-page.js';
import { orderPages, pageTitle, pageUpdate } from './page-index.js';
import { RetentionHint } from './retention-hint.js';
import { text } from './strings.js';
import type { PageSummary } from './transport.js';

const rootRoute = getRouteApi('__root__');
const homeRoute = getRouteApi('/');

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

export function Home() {
  const space = homeRoute.useLoaderData();
  const { transport } = rootRoute.useRouteContext();
  const [archived, setArchived] = useState(false);
  const pages = orderPages(space.pages.filter((p) => Boolean(p.archived) === archived));
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => setNow(Date.now()), [space.pages, archived]);
  useEffect(() => {
    document.title =
      space.title === text.product ? text.product : `${space.title} · ${text.product}`;
  }, [space.title]);
  return (
    <section className="home">
      <p className="eyebrow">{text.pages}</p>
      <h1>{space.title}</h1>
      <p className="intro">{text.intro}</p>
      {transport.management && (
        <BrowserToggle
          pressed={archived}
          label={text.showArchived}
          onActivate={(event) => {
            if (event.isTrusted) setArchived(!archived);
          }}
        />
      )}
      {pages.length ? (
        <BrowserList label={text.pages} className="pages">
          {pages.map((p) => {
            const title = pageTitle(p);
            const updated = pageUpdate(p.lastUpdateAtMs, now);
            return (
              <BrowserListRow
                key={p.id}
                data-page-id={p.id}
                title={
                  <Link to="/pages/$pageId" params={{ pageId: p.id }} title={title}>
                    {title}
                  </Link>
                }
                metadata={
                  updated.dateTime ? (
                    <time
                      dateTime={updated.dateTime}
                      title={updated.absolute}
                      aria-label={`${text.lastUpdated}: ${updated.label}`}
                    >
                      {updated.label}
                    </time>
                  ) : (
                    updated.label
                  )
                }
                state={
                  p.archived ? text.archived : p.sharing === 'private' ? text.private : text.shared
                }
                actions={transport.management ? <PageListActions page={p} /> : undefined}
              />
            );
          })}
        </BrowserList>
      ) : (
        <p>
          {archived ? (
            'No archived pages.'
          ) : space.pages.length ? (
            'No active pages.'
          ) : (
            <>
              {text.empty}
              <code className="tmt-ui-code">tmt colab page create --title "Notes"</code>.
            </>
          )}
        </p>
      )}
    </section>
  );
}
