import { useLayoutEffect, useRef } from 'react';
import type { PanelMode } from './panel-mode.js';

/** The panels that open out of the dock, and the element each one is. */
const PANELS = {
  chat: '.tmt-ui-panel[data-panel="chat"]',
  comments: '.tmt-ui-panel[data-panel="comments"]',
} as const;
export type DockPanel = keyof typeof PANELS;
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
 * A panel opens out of the dock and closes back into it (#4919, #5192): the panel is the same element,
 * clipped to the dock's rectangle at one end of the motion and to its own at the other. Chat's drawer
 * and the Comments panel switch their own slide off in style.css; each one's leaving transition keeps
 * the closed element rendered for the motion's length, so the same clip runs in reverse on it. The
 * length and easing are the leaf's motion tokens, which are 0ms under reduced motion: then there is no
 * animation at all.
 *
 * A Comments column is a track of the page grid and leaves it the moment it closes, so it only opens
 * with the motion. The leaf owns the panel and its focus; this only paints, and a page with no dock or
 * no animation support opens and closes as before.
 */
export function useDockMotion(open: DockPanel | null, mode: PanelMode) {
  const previous = useRef<DockPanel | null>(open);
  useLayoutEffect(() => {
    const was = previous.current;
    previous.current = open;
    if (was === open) return;
    if (was) run(was, false, mode);
    if (open) run(open, true, mode);
  }, [open, mode]);
}

function run(panel: DockPanel, open: boolean, mode: PanelMode) {
  if (!open && panel === 'comments' && mode === 'column') return;
  const element = document.querySelector<HTMLElement>(PANELS[panel]);
  const dock = document.querySelector<HTMLElement>(DOCK);
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
