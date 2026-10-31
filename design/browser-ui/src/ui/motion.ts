/** Enter and exit classes for the floating layers, as literal names the UI generator reads. Radix
 * keeps a layer mounted while its exit animation runs (data-state=closed), and the animation
 * durations are 0ms under prefers-reduced-motion. A leaving layer takes no pointer input (its
 * Escape and outside presses are released in `admitLayerDismiss`). A menu, popover, Select or
 * tooltip fades in and rises from the side it opens toward (Radix `data-side`) in the fast duration
 * and fades out quicker (#5821); a dialog pops in the base duration and leaves in the fast one; a
 * sheet slides in on base and out on fast; the scrim fades the same way. */
export const popMotion =
  'tmt:data-[state=open]:data-[side=bottom]:animate-rise-in-bottom tmt:data-[state=open]:data-[side=top]:animate-rise-in-top tmt:data-[state=open]:data-[side=left]:animate-rise-in-left tmt:data-[state=open]:data-[side=right]:animate-rise-in-right tmt:data-[state=delayed-open]:data-[side=bottom]:animate-rise-in-bottom tmt:data-[state=delayed-open]:data-[side=top]:animate-rise-in-top tmt:data-[state=delayed-open]:data-[side=left]:animate-rise-in-left tmt:data-[state=delayed-open]:data-[side=right]:animate-rise-in-right tmt:data-[state=closed]:animate-fade-quick-out tmt:data-[state=closed]:pointer-events-none!';
export const dialogMotion =
  'tmt:data-[state=open]:animate-pop-in tmt:data-[state=closed]:animate-pop-fast-out tmt:data-[state=closed]:pointer-events-none!';
export const scrimMotion =
  'tmt:data-[state=open]:animate-fade-in tmt:data-[state=closed]:animate-fade-fast-out tmt:data-[state=closed]:pointer-events-none!';
export const slideMotion = {
  right:
    'tmt:data-[state=open]:animate-slide-in-right tmt:data-[state=closed]:animate-slide-out-right tmt:data-[state=closed]:pointer-events-none!',
  left: 'tmt:data-[state=open]:animate-slide-in-left tmt:data-[state=closed]:animate-slide-out-left tmt:data-[state=closed]:pointer-events-none!',
  top: 'tmt:data-[state=open]:animate-slide-in-top tmt:data-[state=closed]:animate-slide-out-top tmt:data-[state=closed]:pointer-events-none!',
  bottom:
    'tmt:data-[state=open]:animate-slide-in-bottom tmt:data-[state=closed]:animate-slide-out-bottom tmt:data-[state=closed]:pointer-events-none!',
} as const;
