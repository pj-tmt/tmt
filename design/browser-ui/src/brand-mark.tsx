import { useId } from 'react';
import { brandMarkInner, brandMarkSmall } from './brand-mark-svg.js';

/** The 04D-1 mark (`brand-mark-svg.ts` holds its one definition). Every surface shows this one
 * mark, tuned per theme by the brand roles; print shows the mono beam. */
export interface BrandMarkProps {
  /** Rendered size in px; selects the simplified form below 28. */
  size?: number;
  className?: string;
}

export function BrandMark({ size, className }: BrandMarkProps) {
  const id = useId().replaceAll(':', '');
  const small = brandMarkSmall(size);
  return (
    <svg
      className={className ? `tmt-brand-mark ${className}` : 'tmt-brand-mark'}
      data-form={small ? 'small' : 'full'}
      viewBox="0 0 96 96"
      aria-hidden="true"
      width={size}
      height={size}
      // Trusted constant markup from the mark's single definition, never user input.
      dangerouslySetInnerHTML={{ __html: brandMarkInner(small, id) }}
    />
  );
}

/** Bold lowercase `tmt` with `terminal team` beneath, beside the mark. */
export function BrandLockup({ size = 48 }: { size?: number }) {
  return (
    <span className="tmt-brand-lockup" style={{ ['--tmt-brand-size' as string]: `${size}px` }}>
      <BrandMark size={size} />
      <span className="tmt-brand-lockup-words">
        <span className="tmt-brand-lockup-name">tmt</span>
        <span className="tmt-brand-lockup-tag">terminal team</span>
      </span>
    </span>
  );
}
