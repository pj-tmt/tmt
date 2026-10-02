export interface PageSummary {
  readonly id: string;
  readonly title: string;
  readonly sharing: 'private' | 'link' | 'public';
}
export interface PageSnapshot extends PageSummary {
  readonly source: string;
}
export interface SpaceHome {
  readonly title: string;
  readonly pages: readonly PageSummary[];
}

/** App data port. Future Remote adapters own authentication and admission;
 * neither HTML nor the renderer receives that adapter or its capabilities. */
export interface PageTransport {
  spaceHome(): Promise<SpaceHome>;
  page(id: string): Promise<PageSnapshot>;
}

/** Detached, read-only local fixture adapter; not a persisted or authenticated backend. */
export function localTransport(title: string, input: readonly PageSnapshot[]): PageTransport {
  const pages = new Map(input.map((page) => [page.id, Object.freeze({ ...page })]));
  if (pages.size !== input.length) throw new Error('Duplicate local page ID');
  return {
    async spaceHome() {
      return {
        title,
        pages: [...pages.values()].map(({ id, title, sharing }) => ({ id, title, sharing })),
      };
    },
    async page(id) {
      const page = pages.get(id);
      if (!page) throw new Error('Unknown page');
      return { ...page };
    },
  };
}
