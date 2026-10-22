import type { MouseEvent } from 'react';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { BrowserIconAction } from './icon-action.js';
import { browserUiClasses as c } from './static.js';

export interface BrowserPagerProps {
  /** What is paged, such as `Thread`; read as "Thread 1 of 3". */
  label: string;
  /** The 1-based position. */
  position: number;
  count: number;
  previousLabel: string;
  nextLabel: string;
  onPrevious(event: MouseEvent<HTMLButtonElement>): void;
  onNext(event: MouseEvent<HTMLButtonElement>): void;
}

/** `‹ 1/3 ›`: the host owns the position; the arrows stop at the ends. */
export function BrowserPager({
  label,
  position,
  count,
  previousLabel,
  nextLabel,
  onPrevious,
  onNext,
}: BrowserPagerProps) {
  return (
    <div className={c.pager} role="group" aria-label={label}>
      <BrowserIconAction
        type="button"
        variant="text"
        label={previousLabel}
        icon={<ChevronLeft />}
        disabled={position <= 1}
        onActivate={onPrevious}
      />
      <span
        className={c.pagerPosition}
        role="status"
        aria-label={`${label} ${position} of ${count}`}
      >
        <span aria-hidden="true">
          {position}/{count}
        </span>
      </span>
      <BrowserIconAction
        type="button"
        variant="text"
        label={nextLabel}
        icon={<ChevronRight />}
        disabled={position >= count}
        onActivate={onNext}
      />
    </div>
  );
}
