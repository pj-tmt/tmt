import { describe, expect, it } from 'vitest';
import { officeDeployment, officeFirebaseConfig } from './firebase-config.js';
import deployments from '../../../../../../../contracts/office/deployment-examples.json';

const settings = {
  VITE_FIREBASE_API_KEY: 'public-key',
  VITE_FIREBASE_PROJECT_ID: 'example-office',
  VITE_FIREBASE_AUTH_DOMAIN: 'example-office.firebaseapp.com',
  VITE_FIREBASE_APP_ID: 'public-app-id',
};
describe('public deployment projection', () => {
  it('matches independent native fixtures without publishing private settings', () => {
    expect(deployments.valid).toHaveLength(3);
    for (const { descriptor, pairingSetting } of deployments.valid) {
      expect(
        officeDeployment(descriptor.mode, {
          ...settings,
          VITE_FIREBASE_API_KEY: descriptor.apiKey,
          VITE_OFFICE_PAIRING_URL: pairingSetting ?? descriptor.pairingUrl,
          PRIVATE_SERVICE_ACCOUNT: 'never-publish',
          VITE_UNRELATED_SECRET: 'never-publish-either',
        })
      ).toEqual(descriptor);
    }
  });
  it('keeps preview and unconfigured cloud unavailable', () => {
    expect(officeDeployment('production', settings)).toBeUndefined();
    expect(officeDeployment('cloud')).toBeUndefined();
    expect(officeDeployment('cloud', settings)).toBeUndefined();
    expect(officeDeployment('emulator')?.projectId).toBe('demo-tmt-office');
  });
  it('rejects configured discovery instead of advertising a mismatched runtime', () => {
    const configured = {
      ...settings,
      VITE_OFFICE_PAIRING_URL: 'https://issuer.example/officePairing',
    };
    for (const changes of [
      { VITE_FIREBASE_PROJECT_ID: 'demo-tmt-office' },
      { VITE_FIREBASE_AUTH_DOMAIN: 'other.example' },
      { VITE_FIREBASE_API_KEY: 'key?query' },
      { VITE_FIREBASE_API_KEY: 'x'.repeat(257) },
      { VITE_OFFICE_PAIRING_URL: 'https://issuer.example/?token=secret' },
      { VITE_OFFICE_PAIRING_URL: 'https://issuer.example/officePairing//' },
      { VITE_OFFICE_PAIRING_URL: `https://issuer.example/${'x'.repeat(2048)}` },
    ])
      expect(() => officeDeployment('cloud', { ...configured, ...changes })).toThrow();
  });
});
describe('explicit cloud configuration', () => {
  it('uses only the supplied public configuration in cloud mode', () => {
    expect(officeFirebaseConfig('cloud', 'localhost', settings)).toEqual({
      apiKey: 'public-key',
      projectId: 'example-office',
      authDomain: 'example-office.firebaseapp.com',
      appId: 'public-app-id',
    });
  });
  it('never leaks cloud configuration into preview or emulator selection', () => {
    expect(officeFirebaseConfig('production', 'localhost', settings)).toBeUndefined();
    expect(officeFirebaseConfig('emulator', 'localhost', settings)?.projectId).toBe(
      'demo-tmt-office'
    );
  });
  it.each(Object.keys(settings))('rejects missing %s without fallback', (key) => {
    expect(() => officeFirebaseConfig('cloud', 'localhost', { ...settings, [key]: '' })).toThrow(
      'complete Firebase'
    );
  });
  it('rejects a mismatched auth domain', () => {
    expect(() =>
      officeFirebaseConfig('cloud', 'localhost', {
        ...settings,
        VITE_FIREBASE_AUTH_DOMAIN: 'other.example',
      })
    ).toThrow('complete Firebase');
  });
});
