import { describe, expect, it } from 'vite-plus/test';
import vectors from '../../../../../contracts/avatar-pack-vectors.json';
import { decodeAvatarPack } from './avatar-contract.js';
import detailed from '../../../../../contracts/avatar-pack-v2-sample.tmtavatar.json';
import detailVectors from '../../../../../contracts/avatar-pack-v2-vectors.json';

describe('avatar-pack projection', () => {
  it('matches v2 literal vectors with full-byte indices and exact dimensions', () => {
    expect(decodeAvatarPack(detailed).formatVersion).toBe(2);
    for (const testCase of detailVectors.cases) {
      const source = structuredClone(detailed);
      const parts = testCase.path.slice(1).split('/');
      const leaf = parts.pop()!;
      const parent = parts.reduce<Record<string, unknown>>(
        (value, key) => value[key] as Record<string, unknown>,
        source as unknown as Record<string, unknown>
      );
      parent[leaf] = testCase.value;
      if (testCase.valid) expect(() => decodeAvatarPack(source), testCase.name).not.toThrow();
      else expect(() => decodeAvatarPack(source), testCase.name).toThrow();
    }
  });

  it('retains the 6,144-cell budget and admits palette index ff only in a complete palette', () => {
    const source = structuredClone(detailed);
    source.avatars = Array.from({ length: 4 }, (_, index) => ({
      ...structuredClone(detailed.avatars[0]!),
      key: `bot-${index}`,
    }));
    expect(decodeAvatarPack(source).avatars).toHaveLength(4);
    source.avatars.push({ ...structuredClone(detailed.avatars[0]!), key: 'fifth' });
    expect(() => decodeAvatarPack(source)).toThrow();
    source.avatars = structuredClone(detailed.avatars);
    source.palette = ['#00000000', ...Array.from({ length: 255 }, () => '#ffffffff')];
    source.avatars[0]!.pixels[0] = `ff${'00'.repeat(31)}`;
    expect(() => decodeAvatarPack(source)).not.toThrow();
    source.palette.push('#ffffffff');
    expect(() => decodeAvatarPack(source)).toThrow();
    source.palette.pop();
    source.avatars[0]!.pixels = Array.from({ length: 48 }, () => '00'.repeat(32));
    expect(() => decodeAvatarPack(source)).toThrow();
    source.avatars[0]!.pixels = Array.from({ length: 47 }, () => '01'.repeat(32));
    expect(() => decodeAvatarPack(source)).toThrow();
  });
  it('matches the shared literal vectors', () => {
    for (const testCase of vectors.packCases) {
      if (testCase.valid)
        expect(() => decodeAvatarPack(testCase.value), testCase.name).not.toThrow();
      else expect(() => decodeAvatarPack(testCase.value), testCase.name).toThrow();
    }
    for (const testCase of vectors.boundaryCases) {
      const source = structuredClone(vectors.packCases[0]!.value);
      if (testCase.field === 'avatarKey') source.avatars[0]!.key = testCase.value;
      else if (testCase.field === 'packLabel') source.label = testCase.value;
      else source.palette[1] = testCase.value;
      if (testCase.valid) expect(() => decodeAvatarPack(source), testCase.name).not.toThrow();
      else expect(() => decodeAvatarPack(source), testCase.name).toThrow();
    }
  });

  it('rejects unknown fields and unresolved indices', () => {
    const source = structuredClone(vectors.packCases[0]!.value) as Record<string, unknown>;
    expect(() => decodeAvatarPack({ ...source, extra: true })).toThrow();
    const avatars = structuredClone(source.avatars) as Array<Record<string, unknown>>;
    avatars[0]!.pixels = Array.from({ length: 24 }, () => '2222222222222222');
    expect(() => decodeAvatarPack({ ...source, avatars })).toThrow();
    expect(() => decodeAvatarPack({ ...source, label: '\ud800' })).toThrow();
    expect(() => decodeAvatarPack({ ...source, label: 'friendly 🤖' })).not.toThrow();
  });

  it('admits the exact 16-avatar and 6,144-cell capacity', () => {
    const source = structuredClone(vectors.packCases[0]!.value);
    source.avatars = Array.from({ length: 16 }, (_, index) => ({
      ...structuredClone(source.avatars[0]!),
      key: `bot-${index}`,
    }));
    expect(decodeAvatarPack(source).avatars).toHaveLength(16);
    source.avatars.push({ ...structuredClone(source.avatars[0]!), key: 'one-more' });
    expect(() => decodeAvatarPack(source)).toThrow();
  });
});
