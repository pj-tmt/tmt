export interface PageUi {
  readonly staticCss: string;
  readonly pageUiStyle: string;
  readonly iconNodes: Record<string, string>;
  readonly pageIcons: string;
  readonly pageUiScript: string;
}
export function buildPageUi(): Promise<PageUi>;
