import type { ReactNode } from 'react';
import type { PageHint } from './reading-link.js';

/**
 * Where an open page sends the reader: a link inside the document, another page, home and the
 * project it was opened from. The route's implementation lives with the router; a page shown
 * somewhere else supplies its own, so the page itself never reaches for the router (#4192).
 */
export interface PageRouting {
  /** A link inside the document to a page route, optionally at one element of it. */
  open(route: string, anchor?: string): void;
  /** The address of a page, optionally at one reading, element or task (the work band and Needs you
   * link to these). */
  pageHref(pageId: string, hint?: PageHint): string;
  /** The header brand's link to the space home; `name` is its accessible name and tooltip. */
  home(brand: ReactNode, name: string): ReactNode;
  /** The project the reader came from, as a link back to its list. */
  scopeCrumb(scope: string): ReactNode;
}
