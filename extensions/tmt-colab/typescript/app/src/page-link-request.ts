import { validPagePrefix } from './short-links.js';
import { validShellAnchor } from './shell.js';

export const PAGE_LINK_BYTES = 8 * 1024;
export interface PageLinkRequest {
  type: 'colab.render.link';
  renderId: string;
  href: string;
  newTab: boolean;
}

/** Frame identity fences a lifetime, never proof of a genuine link activation. */
export function pageLinkRequest(value: unknown, renderId: string): value is PageLinkRequest {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  return (
    Object.keys(v).length === 4 &&
    v.type === 'colab.render.link' &&
    v.renderId === renderId &&
    typeof v.href === 'string' &&
    v.href.length <= PAGE_LINK_BYTES &&
    new TextEncoder().encode(v.href).length <= PAGE_LINK_BYTES &&
    typeof v.newTab === 'boolean'
  );
}

/** The app route of a `/p/<prefix>[#anchor]` link: undefined for any other path, `refused` for a
 * link of that form whose anchor cannot be read. */
export function shortLinkRoute(
  pathname: string,
  hash: string,
): { prefix: string; route: string; anchor?: string } | 'refused' | undefined {
  const short = pathname.match(/^\/p\/([0-9a-f-]+)$/);
  if (!short || !validPagePrefix(short[1])) return undefined;
  let anchor: string | undefined;
  try {
    anchor = hash ? decodeURIComponent(hash.slice(1)) : undefined;
  } catch {
    return 'refused';
  }
  if (anchor?.startsWith('anchor=')) anchor = anchor.slice(7);
  if (anchor !== undefined && !validShellAnchor(anchor)) return 'refused';
  return { prefix: short[1], route: `/short/${short[1]}`, ...(anchor ? { anchor } : {}) };
}

export type PageLinkTarget =
  | { kind: 'refused' }
  | { kind: 'external'; href: string }
  | { kind: 'internal'; href: string; route: string; anchor?: string; label: string };

/** Only this trusted base and space may select app navigation; author bases have no role. */
export function classifyPageLink(href: string, base: string, space?: string): PageLinkTarget {
  if (
    !href ||
    href.length > PAGE_LINK_BYTES ||
    new TextEncoder().encode(href).length > PAGE_LINK_BYTES ||
    [...href].some((c) => c.charCodeAt(0) <= 32 || c.charCodeAt(0) === 127 || c === '\\')
  )
    return { kind: 'refused' };
  let url: URL;
  const owner = new URL(base);
  try {
    url = new URL(href, owner);
  } catch {
    return { kind: 'refused' };
  }
  if (url.username || url.password) return { kind: 'refused' };
  if (space !== undefined && url.origin === owner.origin) {
    if (url.search) return { kind: 'refused' };
    const short = shortLinkRoute(url.pathname, url.hash);
    let prefix: string | undefined;
    let route: string | undefined;
    let anchor: string | undefined;
    if (short === 'refused') return { kind: 'refused' };
    if (short) {
      ({ prefix, route, anchor } = short);
    } else if (url.pathname === owner.pathname) {
      const fields = new URLSearchParams(url.hash.slice(1));
      const keys = [...fields.keys()].sort().join(',');
      const path = fields.get('path') ?? '';
      const page = path.match(/^\/(pages|short)\/([0-9a-f-]+)$/);
      if (
        (keys !== 'path,space' && keys !== 'anchor,path,space') ||
        fields.get('space') !== space ||
        !page ||
        !validPagePrefix(page[2]) ||
        (page[1] === 'pages' &&
          !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(page[2]))
      )
        return { kind: 'refused' };
      prefix = page[2];
      route = path;
      anchor = fields.get('anchor') ?? undefined;
    } else return { kind: 'refused' };
    if (anchor !== undefined && !validShellAnchor(anchor)) return { kind: 'refused' };
    return {
      kind: 'internal',
      href: url.href,
      route,
      ...(anchor ? { anchor } : {}),
      label: `/p/${prefix}${anchor ? `#${anchor}` : ''}`,
    };
  }
  // URL canonicalization accepts legacy numeric IPv4 and expanded IPv6 spellings. The HTTP
  // exception is literal loopback, so inspect an authored authority before normalization too.
  const authority = href.match(/^(?:http:)?\/\/(\[[^\]]+\]|[^:/?#]+)(?::[0-9]+)?(?:[/?#]|$)/i);
  const host = authority ? authority[1].toLowerCase() : owner.hostname;
  const loopback = host === url.hostname && ['localhost', '127.0.0.1', '[::1]'].includes(host);
  return url.protocol === 'https:' || (url.protocol === 'http:' && loopback)
    ? { kind: 'external', href: url.href }
    : { kind: 'refused' };
}
