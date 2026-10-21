/**
 * The honest empty state of a page whose read does not exist yet: it names what the page needs and
 * links the request for that read. Product builds show this, never sample values; mocks stay in
 * tests, the gallery and the dev preview.
 */
export const READ_API_REQUEST = 'https://github.com/pj-tmt/tmt/issues/3937#issuecomment-6321040144';

/** The words a source gives for a read it does not have. */
export function needs(read: string): string {
  return `Not available yet: needs ${read}.`;
}

/** Show `text` in `note` with a link to the read request. */
export function notAvailable(note: HTMLElement, text: string): void {
  const link = document.createElement('a');
  link.href = READ_API_REQUEST;
  link.textContent = 'Read request';
  note.replaceChildren(`${text} `, link);
  note.hidden = false;
}
