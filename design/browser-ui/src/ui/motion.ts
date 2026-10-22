/** Enter and exit classes for the floating layers, as literal names the UI generator reads. Radix
 * keeps a layer mounted while its exit animation runs (data-state=closed), and the animation
 * durations are 0ms under prefers-reduced-motion. A menu, popover or tooltip pops from its anchor
 * in the fast duration; a dialog pops and a sheet slides in the base duration. */
export const popMotion =
  'tmt:origin-(--radix-popper-transform-origin) tmt:data-[state=open]:animate-pop-fast-in tmt:data-[state=closed]:animate-pop-fast-out';
export const dialogMotion =
  'tmt:data-[state=open]:animate-pop-in tmt:data-[state=closed]:animate-pop-out';
export const scrimMotion =
  'tmt:data-[state=open]:animate-fade-in tmt:data-[state=closed]:animate-fade-out';
export const slideMotion = {
  right:
    'tmt:data-[state=open]:animate-slide-in-right tmt:data-[state=closed]:animate-slide-out-right',
  left: 'tmt:data-[state=open]:animate-slide-in-left tmt:data-[state=closed]:animate-slide-out-left',
  top: 'tmt:data-[state=open]:animate-slide-in-top tmt:data-[state=closed]:animate-slide-out-top',
  bottom:
    'tmt:data-[state=open]:animate-slide-in-bottom tmt:data-[state=closed]:animate-slide-out-bottom',
} as const;
