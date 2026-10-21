/**
 * The empty state of a page whose browser read does not exist yet. It tells the person what they
 * can do instead: the terminal command that shows the same facts. Product builds show this, never
 * sample values; mocks stay in tests, the gallery and the dev preview.
 */
export function notInBrowser(page: string, command: string): string {
  return `${page} is not in the browser yet. Run \`${command}\` in a terminal.`;
}

/** Show `text` in `note`, each `command` in backticks as code. */
export function notAvailable(note: HTMLElement, text: string): void {
  note.hidden = false;
  if (!text.includes('`')) {
    note.textContent = text;
    return;
  }
  note.replaceChildren(
    ...text.split('`').map((part, index) => {
      if (index % 2 === 0) return part;
      const code = document.createElement('code');
      code.textContent = part;
      return code;
    }),
  );
}
