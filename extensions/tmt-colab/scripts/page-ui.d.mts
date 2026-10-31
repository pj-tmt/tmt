export interface PageUi {
  readonly staticCss: string;
  readonly editorialCss: string;
  readonly pageUiStyle: string;
  readonly iconNodes: Record<string, string>;
  readonly pageIcons: string;
  readonly pageUiScript: string;
}
export const PAGE_UI_LIMIT: number;
export function buildPageUi(): Promise<PageUi>;
