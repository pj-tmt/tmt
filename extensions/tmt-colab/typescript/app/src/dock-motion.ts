import { useLayoutEffect, useRef, type RefObject } from 'react';

/** One retained host for both conversations. */
const PANEL = '.page-conversation-window';
export type DockPanel = 'chat' | 'comments';
/** The animation's name, for the tests and for cancelling a closing motion that is still held. */
export const MOTION = 'dock-motion';
/** The capsule that holds the page's floating actions (#4919). */
export const DOCK = '.page-floating-actions';

/** A CSS time such as `180ms` or `0.2s`, in milliseconds. */
function milliseconds(value: string) {
  const time = parseFloat(value);
  return Number.isFinite(time) ? (value.trim().endsWith('ms') ? time : time * 1000) : 0;
}

/** Which dock panel is open, if any: the one the motion follows. */
export const dockPanel = (panel: string | null): DockPanel | null =>
  panel === 'chat' || panel === 'comments' ? panel : null;

function clip(inset: [number, number, number, number], radius: number) {
  return `inset(${inset.map((side) => `${side}px`).join(' ')} round ${radius}px)`;
}

/**
 * The one conversation host opens out of the dock and closes back into it. Switching
 * Chat/Comments keeps the same open host and geometry, so it starts no motion. The leaf
 * owns focus; this hook paints only. Reduced motion makes the token duration zero.
 */
export function useDockMotion(open: DockPanel | null, scope: RefObject<HTMLElement | null>) {
  const previous = useRef<DockPanel | null>(open);
  useLayoutEffect(() => {
    const was = previous.current;
    previous.current = open;
    if (was === open || (was && open)) return;
    run(open !== null, scope.current);
  }, [open, scope]);
}

function run(open: boolean, scope: HTMLElement | null) {
  const element = scope?.querySelector<HTMLElement>(PANEL);
  const dock = scope?.querySelector<HTMLElement>(DOCK);
  if (!element || !dock || typeof element.animate !== 'function') return;
  const style = getComputedStyle(element);
  const duration = milliseconds(style.getPropertyValue('--tmt-ui-motion-base'));
  if (duration <= 0) return;
  const box = element.getBoundingClientRect();
  const origin = dock.getBoundingClientRect();
  if (box.width === 0 || box.height === 0 || origin.width === 0 || origin.height === 0) return;
  const docked = clip(
    [
      origin.top - box.top,
      box.right - origin.right,
      box.bottom - origin.bottom,
      origin.left - box.left,
    ],
    origin.height / 2,
  );
  const placed = clip([0, 0, 0, 0], parseFloat(style.borderTopLeftRadius) || 0);
  // One motion at a time: opening cancels a closing motion that is still held at the dock's clip.
  for (const old of element.getAnimations()) if (old.id === MOTION) old.cancel();
  element.animate(
    { clipPath: open ? [docked, placed] : [placed, docked] },
    {
      id: MOTION,
      duration,
      easing: style.getPropertyValue(open ? '--tmt-ui-motion-ease-out' : '--tmt-ui-motion-ease-in'),
      // Opening ends where the element already is; closing holds the dock's clip until the leaving
      // element stops rendering, so its last frame is not the whole panel.
      fill: open ? 'backwards' : 'forwards',
    },
  );
}
