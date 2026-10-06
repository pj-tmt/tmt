/** Safari can end composition before its confirming Enter keydown. */
export function isImeConfirmation(
  event: { isComposing?: boolean; keyCode?: number },
  composing: boolean,
): boolean {
  return composing || event.isComposing === true || event.keyCode === 229;
}

/** Keep the single-key shortcut out of text entry and modified key combinations. */
export function isColabShortcut(
  event: {
    code: string;
    altKey: boolean;
    shiftKey: boolean;
    ctrlKey: boolean;
    metaKey: boolean;
    repeat: boolean;
    isComposing?: boolean;
    keyCode?: number;
  },
  editing: boolean,
): boolean {
  return (
    !editing &&
    !event.repeat &&
    !isImeConfirmation(event, false) &&
    event.code === "KeyC" &&
    !event.altKey &&
    !event.shiftKey &&
    !event.ctrlKey &&
    !event.metaKey
  );
}
