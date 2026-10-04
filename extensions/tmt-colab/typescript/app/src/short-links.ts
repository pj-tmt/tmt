import { generatedId, requireValue } from '@tmt/colab-client';
/** Page prefixes are display-only aliases over the complete admitted catalog. */
export function validPagePrefix(prefix: string): boolean {
  return (
    prefix.length >= 8 &&
    prefix.length <= 36 &&
    [...prefix].every((c, i) => ([8, 13, 18, 23].includes(i) ? c === '-' : /^[0-9a-f]$/.test(c)))
  );
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
