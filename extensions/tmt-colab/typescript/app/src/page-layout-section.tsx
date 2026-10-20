import { useLayoutEffect, useRef, useState } from 'react';
import { BrowserAction } from '@tmt/browser-ui/react';
import type { Projection } from './fold-protocol.js';
import { text } from './strings.js';

const names = {
  column: text.pageLayout.column,
  full: text.pageLayout.full,
  canvas: text.pageLayout.canvas,
};

/** The "Layout" row of the About drawer: how Colab frames this page. A canvas page names why it
 * draws its own frame; a long reason is clamped to three lines and expands in place. */
export function PageLayoutSection({
  layout,
  reason,
}: Pick<Projection, 'layout' | 'layoutReason'> & { reason?: string }) {
  const [expanded, setExpanded] = useState(false);
  const [clamped, setClamped] = useState(false);
  const line = useRef<HTMLParagraphElement>(null);
  useLayoutEffect(() => {
    const element = line.current;
    if (!element || expanded) return;
    const measure = () => setClamped(element.scrollHeight > element.clientHeight + 1);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [reason, expanded]);
  return (
    <section className="page-layout-info" aria-labelledby="page-layout-heading">
      <dl>
        <dt id="page-layout-heading">{text.pageLayout.heading}</dt>
        <dd>
          {layout === undefined ? text.pageLayout.freeform : names[layout]}
          {layout === 'canvas' && reason !== undefined && (
            <>
              <p
                ref={line}
                className="page-layout-reason"
                data-expanded={expanded ? '' : undefined}
                title={reason}
              >
                {reason}
              </p>
              {(clamped || expanded) && (
                <BrowserAction
                  type="button"
                  label={expanded ? text.pageLayout.showLess : text.pageLayout.showAll}
                  variant="text"
                  onActivate={() => setExpanded(!expanded)}
                />
              )}
            </>
          )}
        </dd>
      </dl>
    </section>
  );
}
