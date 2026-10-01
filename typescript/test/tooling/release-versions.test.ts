import { describe, expect, it } from 'vitest';
import { isAlphaVersion } from '../../scripts/release-versions.mjs';

describe('isAlphaVersion', () => {
  it.each(['5.0.0-alpha.9', '5.0.0-alpha.10', '0.1.0-alpha.1', '12.34.56-alpha.789'])(
    'accepts the alpha version %s',
    (version) => {
      expect(isAlphaVersion(version)).toBe(true);
    }
  );

  it.each([
    '5.0.0',
    '0.1.0',
    '5.0.0-beta.1',
    '5.0.0-rc.1',
    '5.0.0-alpha',
    '5.0.0-alpha.',
    '5.0.0-alpha.x',
    '5.0.0-alpha.1.2',
    '5.0.0-alpha.9-rc.1',
    '5.0.0-alpha.9+build.1',
    '5.0-alpha.9',
    'v5.0.0-alpha.9',
    '5.0.0-ALPHA.9',
    ' 5.0.0-alpha.9',
    '5.0.0-alpha.9\n',
  ])('refuses %j', (version) => {
    expect(isAlphaVersion(version)).toBe(false);
  });
});
