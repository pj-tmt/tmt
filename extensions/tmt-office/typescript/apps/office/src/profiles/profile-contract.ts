import { validImmutableArtReference } from '../rendering/immutable-art-reference.js';
import { decodeIdentityStatus } from '../identities/identity-status.js';
import type { IdentityStatusSnapshot } from '../identities/identity-status.js';
import { isPresence, type Presence } from '../identities/presence.js';

export const PROFILE_CATALOG = {
  hairStyles: ['short', 'bob', 'curls', 'tied', 'bald'],
  hairColors: ['ink', 'brown', 'gold', 'silver'],
  skinTones: ['light', 'warm', 'medium', 'deep'],
  shirtColors: ['blue', 'green', 'clay', 'plum', 'gold', 'ink'],
} as const;

export type HairStyle = (typeof PROFILE_CATALOG.hairStyles)[number];
export type HairColor = (typeof PROFILE_CATALOG.hairColors)[number];
export type SkinTone = (typeof PROFILE_CATALOG.skinTones)[number];
export type ShirtColor = (typeof PROFILE_CATALOG.shirtColors)[number];
export interface Appearance {
  hairStyle: HairStyle;
  hairColor: HairColor;
  skinTone: SkinTone;
  shirtColor: ShirtColor;
  shirtMark: string;
}
export interface Profile {
  displayLabel: string;
  description: string;
  appearance: Appearance;
  avatarRef?: string;
}
export interface ProfileSnapshot {
  identityId: string;
  identityName: string;
  exists: boolean;
  revision: number;
  profile: Profile;
  updatedAtMs: number | null;
  catalog: typeof PROFILE_CATALOG;
}
export interface ProfileProjection extends ProfileSnapshot {
  presence: Presence;
  lifetime: 'saved' | 'temporary';
  selfReportedStatus: IdentityStatusSnapshot | null;
}
export interface ProfileMutation extends ProfileSnapshot {
  changed: boolean;
}
export class ProfileConflict extends Error {
  constructor() {
    super('This profile changed. Your draft is preserved; review the latest saved profile.');
  }
}
export class ProfileAvatarUnavailable extends Error {
  constructor() {
    super('The selected avatar is no longer installed. Your draft is preserved.');
  }
}
export interface ProfilePort {
  list(signal?: AbortSignal): Promise<ProfileProjection[]>;
  show(identityId: string): Promise<ProfileSnapshot>;
  apply(identityId: string, expectedRevision: number, profile: Profile): Promise<ProfileMutation>;
}

const uuidV4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const bytes = (value: string) => new TextEncoder().encode(value).length;
const plain = (value: string, max: number, multiline: boolean) =>
  bytes(value) <= max &&
  !Array.from(value).some(
    (character) =>
      /\p{Cc}/u.test(character) && !(multiline && (character === '\n' || character === '\t'))
  );
export function validProfile(value: unknown): value is Profile {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const profile = value as Record<string, unknown>;
  const keys = Object.keys(profile).sort().join(',');
  if (
    keys !== 'appearance,description,displayLabel' &&
    keys !== 'appearance,avatarRef,description,displayLabel'
  )
    return false;
  if (
    !profile.appearance ||
    typeof profile.appearance !== 'object' ||
    Array.isArray(profile.appearance)
  )
    return false;
  const appearance = profile.appearance as Record<string, unknown>;
  if (
    Object.keys(appearance).sort().join(',') !== 'hairColor,hairStyle,shirtColor,shirtMark,skinTone'
  )
    return false;
  return (
    typeof profile.displayLabel === 'string' &&
    plain(profile.displayLabel, 80, false) &&
    typeof profile.description === 'string' &&
    plain(profile.description, 1024, true) &&
    typeof appearance.shirtMark === 'string' &&
    plain(appearance.shirtMark, 16, false) &&
    PROFILE_CATALOG.hairStyles.includes(appearance.hairStyle as HairStyle) &&
    PROFILE_CATALOG.hairColors.includes(appearance.hairColor as HairColor) &&
    PROFILE_CATALOG.skinTones.includes(appearance.skinTone as SkinTone) &&
    PROFILE_CATALOG.shirtColors.includes(appearance.shirtColor as ShirtColor) &&
    (profile.avatarRef === undefined || validImmutableArtReference(profile.avatarRef))
  );
}
export function decodeProfileSnapshot(value: unknown): ProfileSnapshot {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Invalid profile.');
  const record = value as Record<string, unknown>;
  if (
    Object.keys(record).sort().join(',') !==
    'catalog,exists,identityId,identityName,profile,revision,updatedAtMs'
  )
    throw new Error('Invalid profile.');
  if (
    !uuidV4.test(String(record.identityId)) ||
    typeof record.identityName !== 'string' ||
    !record.identityName ||
    typeof record.exists !== 'boolean' ||
    !Number.isSafeInteger(record.revision) ||
    !validProfile(record.profile) ||
    JSON.stringify(record.catalog) !== JSON.stringify(PROFILE_CATALOG) ||
    (record.exists
      ? (record.revision as number) <= 0 ||
        !Number.isSafeInteger(record.updatedAtMs) ||
        (record.updatedAtMs as number) <= 0
      : record.revision !== 0 || record.updatedAtMs !== null)
  ) {
    throw new Error('Invalid profile.');
  }
  return record as unknown as ProfileSnapshot;
}
export function decodeProfileProjection(value: unknown): ProfileProjection {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Invalid profile projection.');
  const { presence, lifetime, selfReportedStatus, ...snapshot } = value as Record<string, unknown>;
  if (!isPresence(presence) || (lifetime !== 'saved' && lifetime !== 'temporary'))
    throw new Error('Invalid profile projection.');
  return {
    ...decodeProfileSnapshot(snapshot),
    presence,
    lifetime,
    selfReportedStatus: decodeIdentityStatus(selfReportedStatus),
  };
}
export function decodeProfileMutation(value: unknown): ProfileMutation {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Invalid profile mutation.');
  const { changed, ...snapshot } = value as Record<string, unknown>;
  if (typeof changed !== 'boolean') throw new Error('Invalid profile mutation.');
  return { ...decodeProfileSnapshot(snapshot), changed };
}
