import { describe, expect, it } from 'vite-plus/test';
import { compareVersions, isAlphaVersion } from '../../scripts/release-versions.mjs';

describe('development version precedence', () => {
  it.each(['0.1.0', '5.0.0'])(
    'orders exact %s-dev below every release with the same core',
    (core) => {
      for (const release of [
        `${core}-0`,
        `${core}-alpha.1`,
        `${core}-alpha.999999`,
        `${core}-beta.1`,
        `${core}-rc.1`,
        core,
      ]) {
        expect(compareVersions(`${core}-dev`, release), release).toBe(-1);
        expect(compareVersions(release, `${core}-dev`), release).toBe(1);
      }
      expect(compareVersions(`${core}-dev`, `${core}-dev`)).toBe(0);
    }
  );

  it.each([
    ['0.1.0-dev', '0.1.1-alpha.1'],
    ['0.1.0', '0.1.1-dev'],
    ['0.1.9', '0.2.0-dev'],
    ['4.99.99', '5.0.0-dev'],
    ['5.0.0-dev', '5.0.1-dev'],
  ])('preserves core-version ordering between %s and %s', (older, newer) => {
    expect(compareVersions(older, newer)).toBe(-1);
    expect(compareVersions(newer, older)).toBe(1);
  });

  it.each(['5.0.0-dev.1', '5.0.0-dev-local', '5.0.0-development', '5.0.0-DEV'])(
    'keeps ordinary semver precedence for %s',
    (version) => {
      const expected = version.endsWith('-DEV') ? -1 : 1;
      expect(compareVersions(version, '5.0.0-alpha.1')).toBe(expected);
      expect(compareVersions('5.0.0-alpha.1', version)).toBe(-expected);
    }
  );
});

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
    '5.0.0-dev',
    '0.1.0-dev',
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
