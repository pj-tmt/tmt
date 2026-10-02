import { describe, expect, it } from 'vite-plus/test';
import vectors from '../../../../../contracts/avatar-pack-vectors.json';
import detailed from '../../../../../contracts/avatar-pack-v2-sample.tmtavatar.json';
import robots from '../../../../../contracts/modular-robots-v2.tmtavatar.json';
import {
  avatarOptions,
  decodeAvatarCatalog,
  resolveAvatar,
  validAvatarReference,
} from './avatar-catalog.js';

const digest = `sha256:${'1'.repeat(64)}`;
const source = {
  catalogRevision: 1,
  packs: [{ digest, pack: vectors.packCases[0]!.value }],
};

describe('avatar catalog', () => {
  it('admits the retained quota plus one bundled robot pack without accepting extra capacity', () => {
    const pack = {
      ...detailed,
      avatars: Array.from({ length: 4 }, (_, index) => ({
        ...detailed.avatars[0]!,
        key: `bot-${index}`,
      })),
    };
    const packs = Array.from({ length: 64 }, (_, index) => ({
      digest: `sha256:${index.toString(16).padStart(64, '0')}`,
      pack,
    }));
    const combined = { catalogRevision: 64, packs: [...packs, { digest, pack: robots }] };
    expect(avatarOptions(decodeAvatarCatalog(combined))).toHaveLength(260);
    expect(() =>
      decodeAvatarCatalog({
        ...combined,
        packs: [...combined.packs, { digest: `sha256:${'2'.repeat(64)}`, pack: detailed }],
      })
    ).toThrow();
  });
  it('preserves v2 index width through immutable selection without changing fallback semantics', () => {
    const catalog = decodeAvatarCatalog({
      catalogRevision: 1,
      packs: [{ digest, pack: detailed }],
    });
    const ref = `${digest}/signal-bot`;
    expect(resolveAvatar(ref, catalog)).toMatchObject({
      status: 'available',
      art: { indexWidth: 2, pixels: detailed.avatars[0]!.pixels },
    });
    expect(resolveAvatar(ref, { catalogRevision: 2, packs: [] })).toEqual({
      status: 'unavailable',
      ref,
    });
  });
  it('decodes bounded packs and resolves one immutable selection', () => {
    const catalog = decodeAvatarCatalog(source);
    const ref = `${digest}/signal-bot`;
    expect(avatarOptions(catalog)).toEqual([{ ref, label: 'Signal bots · Signal bot' }]);
    expect(resolveAvatar(ref, catalog)).toMatchObject({
      status: 'available',
      ref,
      art: { palette: ['#00000000', '#ffffffff'] },
    });
    expect(resolveAvatar(undefined, catalog)).toEqual({ status: 'default' });
    expect(resolveAvatar(`${digest}/missing`, catalog)).toEqual({
      status: 'unavailable',
      ref: `${digest}/missing`,
    });
  });

  it('rejects malformed references, duplicate packs and catalog overflow', () => {
    expect(validAvatarReference(`${digest}/signal-bot`)).toBe(true);
    expect(validAvatarReference(`${digest}/Signal`)).toBe(false);
    expect(() =>
      decodeAvatarCatalog({ ...source, packs: [...source.packs, ...source.packs] })
    ).toThrow();
    expect(() =>
      decodeAvatarCatalog({
        catalogRevision: 1,
        packs: Array.from({ length: 66 }, (_, index) => ({
          digest: `sha256:${index.toString(16).padStart(64, '0')}`,
          pack: vectors.packCases[0]!.value,
        })),
      })
    ).toThrow();
  });
});
