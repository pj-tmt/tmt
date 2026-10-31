/** Per-tab unsent person names; a Remote never receives another Remote's drafts. */
export type PersonDraft = { baseline: string; value: string };
type DraftStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
const key = (remote: string) => `tmt-remote-person-drafts-v1:${remote}`;

export function readPersonDrafts(storage: DraftStorage, remote: string): Map<string, PersonDraft> {
  const raw = storage.getItem(key(remote));
  if (!raw) return new Map();
  try {
    const data: unknown = JSON.parse(raw);
    if (!Array.isArray(data)) return new Map();
    return new Map(
      data.filter(
        (item): item is [string, PersonDraft] =>
          Array.isArray(item) &&
          item.length === 2 &&
          typeof item[0] === 'string' &&
          item[1] !== null &&
          typeof item[1] === 'object' &&
          typeof item[1].baseline === 'string' &&
          typeof item[1].value === 'string',
      ),
    );
  } catch {
    return new Map();
  }
}
export function writePersonDraft(
  storage: DraftStorage,
  remote: string,
  member: string,
  draft: PersonDraft | undefined,
): void {
  const drafts = readPersonDrafts(storage, remote);
  if (draft) drafts.set(member, draft);
  else drafts.delete(member);
  if (drafts.size) storage.setItem(key(remote), JSON.stringify([...drafts]));
  else storage.removeItem(key(remote));
}
