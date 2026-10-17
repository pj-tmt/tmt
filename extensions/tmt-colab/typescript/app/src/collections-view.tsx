import { BrowserAction, BrowserMenu } from '@tmt/browser-ui/react';
import { getRouteApi, Link, useRouter } from '@tanstack/react-router';
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
} from 'react';
import {
  CollectionError,
  type CollectionChange,
  type CollectionPort,
} from './collection-client.js';
import {
  manualPages,
  smartPages,
  type Collection,
  type CollectionDetail,
  type CollectionItem,
  type CollectionSort,
} from './collections.js';
import { ChapterRow } from './chapter-row.js';
import { ChapterSurfaceSlot, useChapterSurfaces } from './chapter-surfaces.js';
import { HomeNav } from './today.js';
import { HomeTitleLink, type HomeTitleHints } from './home-titles.js';
import { useHomeSpace, useSpaceTitle } from './home-space.js';
import { pageUpdate } from './page-index.js';
import { openChapter, prepareCollection } from './chapter.js';
import { text } from './strings.js';
import type { PageSummary, SpaceHome } from './transport.js';
import './collections.css';

const t = text.collections;
const rootRoute = getRouteApi('__root__');
const directoryRoute = getRouteApi('/collections');
const detailRoute = getRouteApi('/collections/$collectionId');
export function Collections() {
  return <CollectionsView space={directoryRoute.useLoaderData()} />;
}
export function CollectionPage() {
  const { collectionId } = detailRoute.useParams();
  return <CollectionsView space={detailRoute.useLoaderData()} id={collectionId} />;
}
function sortOf(c: Collection): CollectionSort {
  return c.definition.kind === 'manual'
    ? c.definition.sort
    : (c.definition.search.sort ?? 'updated');
}
function meta(c: Collection): string {
  return [
    c.definition.kind === 'manual' ? t.manual : t.smart,
    c.maintainer ? t.by(c.maintainer) : '',
    t.sorts[sortOf(c)],
  ]
    .filter(Boolean)
    .join(' · ');
}
function unseen(detail: CollectionDetail, pages: readonly PageSummary[]): number | undefined {
  const marks = new Set(detail.seen.map((s) => s.id));
  if (detail.collection.definition.kind === 'manual')
    return detail.items.filter((item) => !marks.has(item.id)).length;
  const { search, needsYou } = detail.collection.definition;
  const result = smartPages(pages, search, needsYou);
  return result.pending ? undefined : result.pages.filter((page) => !marks.has(page.id)).length;
}
function queryLabel(c: Collection): string {
  if (c.definition.kind !== 'smart') return '';
  const { search, needsYou } = c.definition;
  return (
    [
      search.q,
      ...(search.label ?? []).map((label) => `label:${label}`),
      ...(search.by ?? []).map((by) => `by:${by}`),
      search.archived ? 'archived' : undefined,
      needsYou ? 'needs-you' : undefined,
    ]
      .filter(Boolean)
      .join(' ') || text.pages
  );
}
interface Loaded {
  id?: string;
  directory: Collection[];
  details: Map<string, CollectionDetail>;
}
/** A bounded metadata generation. No polling, page subscription or copied page sources. */
function useCollections(
  port: CollectionPort | undefined,
  id: string | undefined,
  space: SpaceHome,
) {
  const [loaded, setLoaded] = useState<Loaded>();
  const [failed, setFailed] = useState(false);
  const generation = useRef<object | undefined>(undefined);
  useEffect(() => {
    const current = {};
    generation.current = current;
    const controller = new AbortController();
    setLoaded(undefined);
    setFailed(false);
    if (!port) {
      setFailed(true);
      return () => controller.abort();
    }
    void (async () => {
      if (id) {
        const detail = await port.read(id, controller.signal);
        if (generation.current === current && !controller.signal.aborted)
          setLoaded({ id, directory: [detail.collection], details: new Map([[id, detail]]) });
        return;
      }
      const directory = await port.directory(controller.signal);
      if (generation.current !== current || controller.signal.aborted) return;
      const details = new Map<string, CollectionDetail>();
      setLoaded({ directory, details: new Map(details) });
      // At most 64 definitions. One bounded detail read at a time; no source fold is acquired.
      for (const collection of directory) {
        if (controller.signal.aborted) return;
        try {
          details.set(collection.id, await port.read(collection.id, controller.signal));
        } catch {
          if (controller.signal.aborted) return;
        }
        if (generation.current === current && !controller.signal.aborted)
          setLoaded({ directory, details: new Map(details) });
      }
    })().catch(() => {
      if (generation.current === current && !controller.signal.aborted) setFailed(true);
    });
    return () => {
      generation.current = undefined;
      controller.abort();
    };
  }, [port, id, space]);
  const replace = useCallback(
    (detail: CollectionDetail) =>
      setLoaded((previous) =>
        previous && previous.id === id
          ? {
              ...previous,
              directory: previous.directory.map((c) =>
                c.id === detail.collection.id ? detail.collection : c,
              ),
              details: new Map(previous.details).set(detail.collection.id, detail),
            }
          : previous,
      ),
    [id],
  );
  return { loaded, failed, replace, generation };
}
export function CollectionsView({ space, id }: { space: SpaceHome; id?: string }) {
  const { transport } = rootRoute.useRouteContext();
  const { pages, hints } = useHomeSpace(space);
  const { loaded, failed, replace, generation } = useCollections(transport.collections, id, space);
  const detail = id && loaded?.id === id ? loaded.details.get(id) : undefined;
  const directory =
    !id && loaded?.id === undefined ? loaded?.directory.filter((c) => !c.archived) : undefined;
  const [busy, setBusy] = useState(false),
    [failure, setFailure] = useState<string>(),
    [notice, setNotice] = useState<string>();
  const pendingWrite = useRef(false);
  const router = useRouter();
  const section = useRef<HTMLElement>(null);
  const current = useRef({ id, port: transport.collections });
  current.current = { id, port: transport.collections };
  useSpaceTitle(detail?.collection.name ?? t.title, text.product);
  useEffect(() => {
    pendingWrite.current = false;
    setBusy(false);
    setFailure(undefined);
    setNotice(undefined);
  }, [id]);
  useLayoutEffect(() => {
    if (detail && section.current) prepareCollection(router, section.current, detail.collection.id);
  }, [detail, router]);
  async function change(command: CollectionChange): Promise<boolean> {
    const port = transport.collections,
      bound = generation.current;
    if (!port || pendingWrite.current || !bound) return false;
    pendingWrite.current = true;
    setBusy(true);
    setFailure(undefined);
    setNotice(undefined);
    try {
      const result = await port.change(command);
      if (
        generation.current !== bound ||
        current.current.id !== id ||
        current.current.port !== port
      )
        return false;
      replace(result.detail);
      if (command.op === 'clear-seen') setNotice(t.cleared(result.detail.collection.name));
      return true;
    } catch (error) {
      if (generation.current !== bound) return false;
      setFailure(
        error instanceof CollectionError && error.code === 'COLAB_OUTCOME_UNKNOWN'
          ? t.unknown
          : t.failed,
      );
      return false;
    } finally {
      if (generation.current === bound) {
        pendingWrite.current = false;
        setBusy(false);
      }
    }
  }
  const manual = detail?.collection.definition.kind === 'manual';
  const rows = useMemo(() => {
    if (!detail)
      return {
        items: [] as { key: string; target: string; page?: PageSummary; item?: CollectionItem }[],
        pending: false,
      };
    const prefix = detail.collection.id;
    if (detail.collection.definition.kind === 'manual')
      return {
        items: manualPages(detail.items, pages, detail.collection.definition.sort).map(
          ({ item, page }) => ({
            key: `${prefix}|${item.id}|${item.pageId}|${item.anchor ?? ''}`,
            target: item.id,
            item,
            page,
          }),
        ),
        pending: false,
      };
    const result = smartPages(
      pages,
      detail.collection.definition.search,
      detail.collection.definition.needsYou,
    );
    return {
      items: result.pages.map((page) => ({
        key: `${prefix}|${page.id}`,
        target: page.id,
        page,
        item: undefined,
      })),
      pending: result.pending,
    };
  }, [detail, pages]);
  const order = useMemo(() => rows.items.filter((row) => row.page).map((row) => row.key), [rows]);
  useChapterSurfaces(order, generation.current ?? space, !!detail);
  const seen = new Set(detail?.seen.map((mark) => mark.id));
  return (
    <section className="today collections" ref={section}>
      <HomeNav current="collections" />
      <header className="collection-heading">
        <div>
          <h1>{detail?.collection.name ?? t.title}</h1>
          {detail && <p className="today-meta">{meta(detail.collection)}</p>}
        </div>
        {detail && transport.collections && (
          <CollectionMenu detail={detail} busy={busy} change={change} />
        )}
      </header>
      {notice && (
        <p role="status" className="collection-notice">
          {notice}
        </p>
      )}
      {failure && <p role="alert">{failure}</p>}
      {failed ? (
        <p role="status">{t.unavailable}</p>
      ) : !loaded ? (
        <p className="today-meta" role="status">
          {t.pending}
        </p>
      ) : !id ? (
        <>
          {directory?.length === 0 ? (
            <>
              <p>{t.empty}</p>
              <p className="today-meta">{t.emptyHint}</p>
            </>
          ) : (
            <ul className="collection-directory">
              {directory?.map((c) => {
                const d = loaded.details.get(c.id),
                  waiting = d ? unseen(d, pages) : undefined;
                return (
                  <li key={c.id}>
                    <div>
                      <h2>
                        <Link to="/collections/$collectionId" params={{ collectionId: c.id }}>
                          {c.name}
                        </Link>
                      </h2>
                      <p className="today-meta">
                        {[
                          c.definition.kind.toUpperCase(),
                          c.definition.kind === 'manual'
                            ? d
                              ? t.itemCount(d.items.length)
                              : '—'
                            : `“${queryLabel(c)}”`,
                          c.maintainer ? t.by(c.maintainer) : '',
                          c.archived ? text.archived : undefined,
                        ]
                          .filter(Boolean)
                          .join(' · ')}
                      </p>
                    </div>
                    {waiting !== undefined && waiting > 0 && (
                      <span className="collection-waiting">
                        <span aria-hidden="true">◆</span> {t.waitingCount(waiting)}
                      </span>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </>
      ) : (
        detail && (
          <>
            {rows.items.length === 0 && (
              <p className={rows.pending ? 'today-meta' : undefined}>
                {rows.pending ? t.pending : manual ? t.emptyManual : t.emptySmart}
              </p>
            )}
            <ol className="today-chapters collection-chapters">
              {rows.items.map((row) => (
                <CollectionChapter
                  key={row.key}
                  collectionId={id}
                  surfaceId={row.key}
                  target={row.target}
                  page={row.page}
                  item={row.item}
                  hints={hints}
                  seen={seen.has(row.target)}
                  busy={busy}
                  markSeen={() => change({ op: 'seen', id, target: row.target })}
                />
              ))}
            </ol>
            {rows.pending && rows.items.length > 0 && (
              <p className="today-meta" role="status">
                {t.partial}
              </p>
            )}
          </>
        )
      )}
    </section>
  );
}
function CollectionChapter({
  collectionId,
  surfaceId,
  target,
  page,
  item,
  hints,
  seen,
  busy,
  markSeen,
}: {
  collectionId: string;
  surfaceId: string;
  target: string;
  page?: PageSummary;
  item?: CollectionItem;
  hints?: HomeTitleHints;
  seen: boolean;
  busy: boolean;
  markSeen(): void;
}) {
  const router = useRouter();
  const title = useRef<HTMLHeadingElement>(null);
  const pageId = page?.id;
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => setNow(Date.now()), [page?.lastUpdateAtMs]);
  const open = useCallback(() => {
    if (pageId)
      openChapter(router, pageId, {
        scroll: window.scrollY,
        link: title.current?.querySelector('a') ?? null,
        replace: false,
        full: true,
        collection: { id: collectionId, item: target },
        section: item?.anchor,
      });
  }, [router, pageId, collectionId, target, item?.anchor]);
  function follow(event: MouseEvent<HTMLAnchorElement>) {
    if (
      !event.isTrusted ||
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.altKey ||
      event.shiftKey
    )
      return;
    event.preventDefault();
    open();
  }
  const url = page
    ? new URL(
        router.history.createHref(
          `/pages/${page.id}${item?.anchor ? `?section=${encodeURIComponent(item.anchor)}` : ''}`,
        ),
        location.href,
      ).href
    : '';
  const when = page ? pageUpdate(page.lastUpdateAtMs, now).label : undefined;
  return (
    <ChapterRow
      data-collection-item={target}
      data-seen={seen || undefined}
      before={
        <div className="collection-progress">
          <p className="today-kicker">
            {seen ? (
              <>
                <span aria-hidden="true">✓</span> {t.seen}
              </>
            ) : (
              <>
                <span className="today-mark" aria-hidden="true">
                  ◆
                </span>{' '}
                {t.waiting}
              </>
            )}
          </p>
          {!seen && (
            <BrowserAction
              type="button"
              variant="text"
              label={t.markSeen}
              disabled={busy}
              onActivate={(event) => {
                if (event.isTrusted) markSeen();
              }}
            />
          )}
        </div>
      }
      main={
        <>
          <h2 className="today-title" ref={title}>
            {page ? (
              <HomeTitleLink page={page} hints={hints} onClick={follow} section={item?.anchor} />
            ) : (
              t.pageUnavailable
            )}
          </h2>
          {page && (
            <p className="today-meta">
              {[page.summary?.author ? text.today.byAuthor(page.summary.author) : '', when]
                .filter(Boolean)
                .join(' · ')}
            </p>
          )}
          {item && (item.audience || item.note) && (
            <p className="today-meta collection-note">
              {[item.audience ? t.for(item.audience) : '', item.note].filter(Boolean).join(' · ')}
            </p>
          )}
        </>
      }
      state={
        page ? (
          <ChapterSurfaceSlot
            id={surfaceId}
            pageId={page.id}
            anchor={item?.anchor}
            unavailableText={t.itemUnavailable}
            open={open}
            url={url}
          />
        ) : null
      }
    />
  );
}
function CollectionMenu({
  detail,
  busy,
  change,
}: {
  detail: CollectionDetail;
  busy: boolean;
  change(command: CollectionChange): Promise<boolean>;
}) {
  const [edit, setEdit] = useState<'rename' | 'maintainer' | 'archive'>(),
    [value, setValue] = useState('');
  const c = detail.collection;
  const sorts: CollectionSort[] =
    c.definition.kind === 'manual' ? ['manual', 'updated', 'title'] : ['updated', 'title'];
  useEffect(() => {
    setEdit(undefined);
  }, [c.id]);
  return (
    <div className="collection-menu">
      <BrowserMenu
        label={t.menu}
        disabled={busy}
        items={[
          { key: 'rename', label: t.rename },
          { key: 'maintainer', label: t.maintainer },
          ...sorts.map((sort) => ({
            key: `sort:${sort}`,
            label: `${t.sort}: ${t.sorts[sort]}`,
            role: 'menuitemradio' as const,
            checked: sortOf(c) === sort,
          })),
          ...(detail.seen.length ? [{ key: 'clear-seen', label: t.clearSeen }] : []),
          ...(!c.archived ? [{ key: 'archive', label: t.archive }] : []),
        ]}
        onSelect={(key, event) => {
          if (!event.isTrusted) return;
          if (key.startsWith('sort:'))
            void change({ op: 'sort', id: c.id, sort: key.slice(5) as CollectionSort });
          else if (key === 'clear-seen') void change({ op: 'clear-seen', id: c.id });
          else {
            setEdit(key as 'rename' | 'maintainer' | 'archive');
            setValue(key === 'rename' ? c.name : c.maintainer);
          }
        }}
      />
      {edit && (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (!event.isTrusted || busy) return;
            void change(
              edit === 'archive'
                ? { op: 'archive', id: c.id, archived: true }
                : edit === 'rename'
                  ? { op: 'rename', id: c.id, name: value }
                  : { op: 'maintainer', id: c.id, maintainer: value },
            ).then((confirmed) => {
              if (confirmed) setEdit(undefined);
            });
          }}
        >
          {edit === 'archive' ? (
            <p>{t.archiveQuestion}</p>
          ) : (
            <label>
              {edit === 'rename' ? t.name : t.maintainerLabel}
              <input value={value} onChange={(e) => setValue(e.target.value)} autoFocus />
            </label>
          )}
          <button type="submit" disabled={busy}>
            {edit === 'archive' ? t.archiveConfirm : t.save}
          </button>
          <button
            type="button"
            onClick={(event) => {
              if (event.isTrusted) setEdit(undefined);
            }}
          >
            {t.cancel}
          </button>
        </form>
      )}
    </div>
  );
}
