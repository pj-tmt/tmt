export const WORLD_ID_PATTERN = '[a-zA-Z0-9]{20}';
const worldIdPattern = new RegExp(`^${WORLD_ID_PATTERN}$`);

export function validWorldId(id: string): boolean {
  return worldIdPattern.test(id);
}

export interface World {
  id: string;
  name: string;
  ownerUid: string;
  createdAtMs: number;
}

export interface WorldDraft {
  id: string;
  name: string;
}

/** Mirrors the feedback subset of contracts/office/private-world.md. Rules enforce it. */
export function validWorldName(name: string): boolean {
  const characters = [...name];
  return (
    characters.length > 0 &&
    characters.length <= 80 &&
    name.trim().length > 0 &&
    characters.every((character) => {
      const point = character.codePointAt(0)!;
      return point >= 32 && point !== 127 && (point < 0xd800 || point > 0xdfff);
    })
  );
}

export interface WorldPort {
  draft(name: string): WorldDraft;
  create(draft: WorldDraft, uid: string): Promise<string>;
  watch(id: string, changed: (world: World | null) => void, failed: () => void): () => void;
  watchAdmission(uid: string, changed: (enabled: boolean) => void, failed: () => void): () => void;
}
