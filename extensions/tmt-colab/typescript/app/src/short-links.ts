import { generatedId, requireValue } from '@tmt/colab-client';
/** Page prefixes are display-only aliases over the complete admitted catalog. */
export function validPagePrefix(prefix: string): boolean {
  return (
    prefix.length >= 8 &&
    prefix.length <= 36 &&
    [...prefix].every((c, i) => ([8, 13, 18, 23].includes(i) ? c === '-' : /^[0-9a-f]$/.test(c)))
  );
}
/** The short location of a page: `/p/<prefix>` on its own origin, `/colab/p/<prefix>` as a route of
 * the one TMT web app, where a click stays in the open document. */
export function shortLocation(prefix: string, shell: boolean): string {
  requireValue(validPagePrefix(prefix));
  return `${shell ? '/colab' : ''}/p/${prefix}`;
}
/** The prefix a short location names: `/p/<prefix>` anywhere, `/colab/p/<prefix>` only in the app. */
export function shortLocationPrefix(pathname: string, shell: boolean): string | undefined {
  const match = (shell ? /^(?:\/colab)?\/p\/([0-9a-fA-F-]+)$/ : /^\/p\/([0-9a-fA-F-]+)$/).exec(
    pathname,
  );
  return match?.[1];
}
export function shortPageId(page: string, pages: readonly string[]): string {
  generatedId(page);
  if (!pages.includes(page)) return page;
  for (let length = 8; length < page.length; length++) {
    const prefix = page.slice(0, length);
    if (pages.filter((id) => id.startsWith(prefix)).length === 1) return prefix;
  }
  return page;
}
export function shortPageUrl(origin: string, page: string, prefix: string): string {
  generatedId(page);
  requireValue(validPagePrefix(prefix) && page.startsWith(prefix));
  const url = new URL(origin);
  requireValue(['http:', 'https:'].includes(url.protocol) && !url.username && !url.password);
  return new URL(`/p/${prefix}`, url.origin).href;
}
