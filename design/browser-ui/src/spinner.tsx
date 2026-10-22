import { busyDotRadius, busyDots, busyFrames, browserUiClasses as c } from './static';

export { busyFrames };

/**
 * The one leaf busy mark (Ben's pick a2): six dots of a 2x3 cell in the current colour, three lit
 * at a time, going round the ring with eased fades once per --tmt-ui-spin-duration. It is
 * decorative; the action or row that shows it names the state. Under reduced motion it holds one
 * still frame. The dots carry no inline style, so the mark works under a strict style policy.
 */
export function BusyMark({ className }: { className?: string }) {
  return (
    <span className={className ? `${c.busyMark} ${className}` : c.busyMark} aria-hidden="true">
      <svg viewBox="0 0 16 16">
        {busyDots.map(([cx, cy]) => (
          <circle key={`${cx},${cy}`} className={c.busyDot} cx={cx} cy={cy} r={busyDotRadius} />
        ))}
      </svg>
    </span>
  );
}
