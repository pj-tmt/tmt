import { useLayoutEffect, useRef } from 'react';

const CHAT = '.tmt-ui-drawer[data-panel="chat"]';
/** The animation's name, for the tests and for cancelling a closing motion that is still held. */
export const MOTION = 'chat-dock';
/** The capsule that holds the page's two floating actions (#4919). */
export const DOCK = '.page-floating-actions';

/** A CSS time such as `180ms` or `0.2s`, in milliseconds. */
function milliseconds(value: string) {
  const time = parseFloat(value);
  return Number.isFinite(time) ? (value.trim().endsWith('ms') ? time : time * 1000) : 0;
}

/**
 * Chat opens out of the dock and closes back into it (#4919): the drawer is the same dialog, clipped to
 * the dock's rectangle at one end of the motion and to its own at the other. The drawer's own slide is
 * switched off for Chat in style.css; its leaving transition keeps the closed dialog rendered for the
 * motion's length, so the same clip runs in reverse on it. The length and easing are the leaf's motion
 * tokens, which are 0ms under reduced motion: then there is no animation at all.
 *
 * The leaf owns the dialog and its focus; this only paints, and a page with no dock or no animation
 * support opens and closes as before.
 */
export function useChatDockMotion(open: boolean) {
  const previous = useRef(open);
  useLayoutEffect(() => {
    if (previous.current === open) return;
    previous.current = open;
    const dialog = document.querySelector<HTMLElement>(CHAT);
    const dock = document.querySelector<HTMLElement>(DOCK);
    if (!dialog || !dock || typeof dialog.animate !== 'function') return;
    const style = getComputedStyle(dialog);
    const duration = milliseconds(style.getPropertyValue('--tmt-ui-motion-base'));
    if (duration <= 0) return;
    const box = dialog.getBoundingClientRect();
    const origin = dock.getBoundingClientRect();
    if (box.width === 0 || box.height === 0 || origin.width === 0 || origin.height === 0) return;
    const clip = (inset: [number, number, number, number], radius: number) =>
      `inset(${inset.map((side) => `${side}px`).join(' ')} round ${radius}px)`;
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
    for (const old of dialog.getAnimations()) if (old.id === MOTION) old.cancel();
    dialog.animate(
      { clipPath: open ? [docked, placed] : [placed, docked] },
      {
        id: MOTION,
        duration,
        easing: style.getPropertyValue(
          open ? '--tmt-ui-motion-ease-out' : '--tmt-ui-motion-ease-in',
        ),
        // Opening ends where the dialog already is; closing holds the dock's clip until the leaving
        // dialog stops rendering, so its last frame is not the whole drawer.
        fill: open ? 'backwards' : 'forwards',
      },
    );
  }, [open]);
}
