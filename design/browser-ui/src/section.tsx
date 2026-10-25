import { useId } from 'react';
import type { ElementType, HTMLAttributes, ReactNode } from 'react';
import { browserUiClasses as c } from './static.js';

export interface BrowserSectionProps extends Omit<
  HTMLAttributes<HTMLElement>,
  'title' | 'className' | 'style'
> {
  /** The section's heading text; it labels the section for assistive technology. */
  heading: ReactNode;
  /** `label` is the small uppercase group label; `title` is the larger heading of a settings group. */
  variant?: 'label' | 'title';
  /** The heading element (default `h2`); `none` renders a styled block with no heading role. */
  headingLevel?: 'h2' | 'h3' | 'h4' | 'none';
  /** Separate this section from the next with a rule. */
  divided?: boolean;
  children: ReactNode;
}
export function BrowserSection({
  heading,
  variant = 'label',
  headingLevel = 'h2',
  divided,
  children,
  ...attributes
}: BrowserSectionProps) {
  // `none`: a styled block, for a sample that must stay out of the page's heading outline.
  const Heading: ElementType = headingLevel === 'none' ? 'div' : headingLevel;
  const headingId = useId();
  return (
    <section
      {...attributes}
      className={c.section}
      data-divided={divided || undefined}
      aria-labelledby={headingId}
    >
      <Heading id={headingId} className={c.sectionHeading} data-variant={variant}>
        {heading}
      </Heading>
      {children}
    </section>
  );
}
