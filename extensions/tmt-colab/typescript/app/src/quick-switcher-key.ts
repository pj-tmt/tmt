export function usesCommandKey(platform = globalThis.navigator?.platform ?? '') {
  return /Mac|iPhone|iPad|iPod/.test(platform);
}

/** Untrusted cosmetic request: it can open search, never choose its destination. */
export interface SwitcherKeyRequest {
  key: 'K';
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: false;
  shiftKey: false;
}

/** The platform modifier is shared by chrome and the closed renderer-key validator. */
export function switcherModifiers(
  value: Pick<KeyboardEvent, 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>,
  command = usesCommandKey(),
) {
  return (
    !value.altKey &&
    !value.shiftKey &&
    (command ? value.metaKey && !value.ctrlKey : value.ctrlKey && !value.metaKey)
  );
}

/** Editors keep their own link/search shortcut; no synthetic event opens chrome. */
export function admitsSwitcherShortcut(event: KeyboardEvent) {
  const editing = event
    .composedPath()
    .some(
      (node) =>
        node instanceof HTMLElement &&
        (node.isContentEditable ||
          !!node.closest('input,textarea,select,[role="textbox"],[role="combobox"]')),
    );
  return (
    event.isTrusted &&
    !event.defaultPrevented &&
    !event.repeat &&
    !event.isComposing &&
    event.keyCode !== 229 &&
    event.key.toLowerCase() === 'k' &&
    !editing &&
    switcherModifiers(event)
  );
}
