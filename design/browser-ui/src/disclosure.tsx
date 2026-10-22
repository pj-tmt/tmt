import type { DetailsHTMLAttributes, ReactNode } from 'react';
import { browserUiClasses as c } from './static.js';

export interface BrowserDisclosureProps extends Omit<
  DetailsHTMLAttributes<HTMLDetailsElement>,
  'className' | 'style' | 'children'
> {
  /** The always-visible line that opens and closes the rest. */
  summary: ReactNode;
  /** Set the disclosure off from the content above with a rule. */
  divided?: boolean;
  children: ReactNode;
}
/** A native `<details>`: keyboard, find-in-page and no-script behaviour stay the browser's. */
export function BrowserDisclosure({
  summary,
  divided,
  children,
  ...attributes
}: BrowserDisclosureProps) {
  return (
    <details {...attributes} className={c.disclosure} data-divided={divided || undefined}>
      <summary>{summary}</summary>
      {children}
    </details>
  );
}
