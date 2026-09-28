import { avatarRaster, decodeAvatarPack } from './avatar-contract.js';
import type { AvatarArt } from '../profiles/avatar-art.js';
import {
  validImmutableArtDigest,
  validImmutableArtReference,
} from '../rendering/immutable-art-reference.js';

export interface AvatarCatalogPack {
  digest: string;
  pack: ReturnType<typeof decodeAvatarPack>;
}

export interface AvatarCatalog {
  catalogRevision: number;
  packs: AvatarCatalogPack[];
}

export interface AvatarOption {
  ref: string;
  label: string;
}

export type AvatarResolution =
  | { status: 'default' }
  | { status: 'available'; ref: string; label: string; art: AvatarArt }
  | { status: 'unavailable'; ref: string };

export function validAvatarReference(value: unknown): value is string {
  return validImmutableArtReference(value);
}

export function decodeAvatarCatalog(value: unknown): AvatarCatalog {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Invalid avatar catalog.');
  const record = value as Record<string, unknown>;
  if (
    Object.keys(record).sort().join(',') !== 'catalogRevision,packs' ||
    !Number.isSafeInteger(record.catalogRevision) ||
    (record.catalogRevision as number) < 0 ||
    !Array.isArray(record.packs) ||
    record.packs.length > 65
  )
    throw new Error('Invalid avatar catalog.');
  const seen = new Set<string>();
  let avatarCount = 0;
  const packs = record.packs.map((value) => {
    if (!value || typeof value !== 'object' || Array.isArray(value))
      throw new Error('Invalid avatar catalog pack.');
    const packRecord = value as Record<string, unknown>;
    if (
      Object.keys(packRecord).sort().join(',') !== 'digest,pack' ||
      typeof packRecord.digest !== 'string' ||
      !validImmutableArtDigest(packRecord.digest) ||
      seen.has(packRecord.digest)
    )
      throw new Error('Invalid avatar catalog pack.');
    seen.add(packRecord.digest);
    const pack = decodeAvatarPack(packRecord.pack);
    avatarCount += pack.avatars.length;
    if (avatarCount > 260) throw new Error('Avatar catalog exceeds its bound.');
    return { digest: packRecord.digest, pack };
  });
  return { catalogRevision: record.catalogRevision as number, packs };
}

export function avatarOptions(catalog: AvatarCatalog): AvatarOption[] {
  return catalog.packs.flatMap(({ digest, pack }) =>
    pack.avatars.map((avatar) => ({
      ref: `${digest}/${avatar.key}`,
      label: `${pack.label} · ${avatar.label}`,
    }))
  );
}

export function resolveAvatar(
  avatarRef: string | undefined,
  catalog: AvatarCatalog
): AvatarResolution {
  if (!avatarRef) return { status: 'default' };
  for (const { digest, pack } of catalog.packs) {
    const prefix = `${digest}/`;
    if (!avatarRef.startsWith(prefix)) continue;
    const key = avatarRef.slice(prefix.length);
    const avatar = pack.avatars.find((candidate) => candidate.key === key);
    if (avatar)
      return {
        status: 'available',
        ref: avatarRef,
        label: `${pack.label} · ${avatar.label}`,
        art: avatarRaster(pack, avatar),
      };
  }
  return { status: 'unavailable', ref: avatarRef };
}
