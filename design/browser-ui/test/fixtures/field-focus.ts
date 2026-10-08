/** Browser assertions shared by the static and React fixtures; no product runtime. */
export function checkFieldFocus(controls: readonly HTMLElement[]): number {
  let assertions = 0;
  const failures: string[] = [];
  const check = (value: boolean, message: string) => {
    if (!value) failures.push(message);
    assertions += 1;
  };
  const measure = (control: HTMLElement) => {
    const style = getComputedStyle(control);
    const border = parseFloat(style.borderTopWidth);
    const outline = style.outlineStyle === 'none' ? 0 : parseFloat(style.outlineWidth);
    const offset = parseFloat(style.outlineOffset);
    return {
      rect: control.getBoundingClientRect(),
      border,
      outline,
      offset,
      edge: Math.max(border, outline ? -offset : 0),
      borderColor: style.borderTopColor,
      outlineColor: style.outlineColor,
      shadow: style.boxShadow,
      radius: style.borderTopLeftRadius,
    };
  };
  for (const control of controls) {
    control.blur();
    const ready = measure(control);
    control.focus({ preventScroll: true });
    const focused = measure(control);
    const name = control.id;
    check(control.matches(':focus-visible'), `${name}: focus cue was not exercised`);
    for (const state of [ready, focused]) {
      check(state.outline + state.offset <= 0, `${name}: focus paints outside the border box`);
      check(state.shadow === 'none' && state.radius === '0px', `${name}: field is not flat/square`);
    }
    for (const dimension of ['x', 'y', 'width', 'height'] as const)
      check(
        Math.abs(ready.rect[dimension] - focused.rect[dimension]) <= 1,
        `${name}: focus changed ${dimension}`,
      );
    check(
      focused.edge > ready.edge || focused.borderColor !== ready.borderColor,
      `${name}: focus has no visible edge change`,
    );
    check(
      ready.edge === 1 && focused.edge === 2,
      `${name}: edge did not strengthen from 1px to 2px`,
    );
    check(
      focused.outline === 0 || focused.outline + focused.offset === -focused.border,
      `${name}: focus edge has a gap or overlaps itself`,
    );
    check(
      focused.outline === 0 || focused.outlineColor === focused.borderColor,
      `${name}: focus edge has two colors`,
    );
    const color = document.createElement('span');
    color.style.color = getComputedStyle(control).getPropertyValue('--tmt-ui-color-focus');
    check(focused.borderColor === color.style.color, `${name}: edge does not use the focus color`);
    control.blur();
    const restored = measure(control);
    check(
      restored.edge === ready.edge && restored.borderColor === ready.borderColor,
      `${name}: blur did not restore the ready edge`,
    );
  }
  if (failures.length) throw new Error(failures.join('; '));
  return assertions;
}
