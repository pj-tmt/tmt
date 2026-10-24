import { pageLinkKey } from './page-link.js';
import { Link } from '@tanstack/react-router';
import { useEffect, useRef, type MouseEvent, type RefObject } from 'react';
import { pageTitle } from './page-index.js';
import { visibleElement } from './visible-element.js';
import type { PageSummary, PageTransport, TitleReadBudget } from './transport.js';

export const HOME_TITLE_PAGES = 24;
export const HOME_TITLE_BYTES = 16 * 1024 * 1024;

/** One Home generation's missing-title reads. Cached rows never enter this queue. */
export class HomeTitleHints {
  #lifetime = new AbortController();
  #budget: TitleReadBudget = { remainingBytes: HOME_TITLE_BYTES };
  #attempted = new Set<string>();
  #pending = new Map<string, () => boolean>();
  #running: Promise<void> | undefined;
  constructor(
    private readonly read: NonNullable<PageTransport['readTitle']>,
    private readonly publish: (id: string, title: string) => void,
  ) {}
  enqueue(id: string, visible: () => boolean) {
    if (
      this.#lifetime.signal.aborted ||
      this.#attempted.has(id) ||
      this.#attempted.size >= HOME_TITLE_PAGES ||
      this.#pending.size >= HOME_TITLE_PAGES
    )
      return;
    this.#pending.set(id, visible);
    this.#start();
  }
  #start() {
    if (this.#running) return;
    this.#running = this.#drain().finally(() => {
      this.#running = undefined;
      if (this.#pending.size) this.#start();
    });
  }
  async #drain() {
    while (
      this.#pending.size &&
      !this.#lifetime.signal.aborted &&
      this.#attempted.size < HOME_TITLE_PAGES &&
      this.#budget.remainingBytes > 0
    ) {
      const [id, visible] = this.#pending.entries().next().value!;
      this.#pending.delete(id);
      // Observer delivery can trail scrolling: admit the actual row, not its old callback.
      if (!visible()) continue;
      this.#attempted.add(id);
      try {
        const title = await this.read(id, this.#budget, this.#lifetime.signal);
        if (!this.#lifetime.signal.aborted && title !== undefined) this.publish(id, title);
      } catch {
        // A title is optional. Failed reads consume their page/byte budget and keep the label.
      }
    }
    this.#pending.clear();
  }
  close(): Promise<void> {
    this.#lifetime.abort();
    this.#pending.clear();
    return this.#running ?? Promise.resolve();
  }
}

/** Asks `hints` for the page's title once its element is on screen, when the list has none yet. */
export function useTitleHint(
  page: Pick<PageSummary, 'id' | 'titleMissing'>,
  hints: HomeTitleHints | undefined,
  element: RefObject<HTMLElement | null>,
) {
  useEffect(() => {
    if (!page.titleMissing || !hints) return;
    let intersecting = false;
    const observer = new IntersectionObserver((entries) => {
      intersecting = entries.some((entry) => entry.isIntersecting);
      if (intersecting)
        hints.enqueue(page.id, () => intersecting && visibleElement(element.current));
    });
    observer.observe(element.current!);
    return () => {
      intersecting = false;
      observer.disconnect();
    };
  }, [page.id, page.titleMissing, hints, element]);
}

/** The existing page link also supplies the visibility fence for optional title reads. */
export function HomeTitleLink({
  page,
  hints,
  scope,
  onClick,
}: {
  page: PageSummary;
  hints?: HomeTitleHints;
  /** The project the list is scoped to: the page's header then offers the way back to it. */
  scope?: string;
  onClick?: (event: MouseEvent<HTMLAnchorElement>) => void;
}) {
  const link = useRef<HTMLAnchorElement>(null);
  useTitleHint(page, hints, link);
  const title = pageTitle(page);
  return (
    <Link
      ref={link}
      to="/pages/$pageId"
      params={{ pageId: page.id }}
      {...(scope === undefined ? {} : { state: { scope } as never })}
      title={title}
      onClick={onClick}
      onKeyDown={pageLinkKey}
    >
      {title}
    </Link>
  );
}
