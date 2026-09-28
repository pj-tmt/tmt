import type { FirebaseOptions } from 'firebase/app';

export function officeFirebaseConfig(
  mode: string,
  hostname: string,
  settings: Record<string, unknown> = {}
): FirebaseOptions | undefined {
  if (mode === 'emulator') {
    if (!['127.0.0.1', 'localhost', '[::1]'].includes(hostname))
      throw new Error('Local sign-in is only available on loopback.');
    return {
      apiKey: 'demo-tmt-office',
      projectId: 'demo-tmt-office',
      authDomain: 'demo-tmt-office.firebaseapp.com',
    };
  }
  if (mode !== 'cloud') return undefined;
  const {
    VITE_FIREBASE_API_KEY: apiKey,
    VITE_FIREBASE_PROJECT_ID: projectId,
    VITE_FIREBASE_AUTH_DOMAIN: authDomain,
    VITE_FIREBASE_APP_ID: appId,
  } = settings;
  if (
    typeof apiKey !== 'string' ||
    !apiKey.trim() ||
    typeof projectId !== 'string' ||
    !/^[a-z][a-z0-9-]{4,28}[a-z0-9]$/.test(projectId) ||
    projectId.startsWith('demo-') ||
    typeof authDomain !== 'string' ||
    authDomain !== `${projectId}.firebaseapp.com` ||
    typeof appId !== 'string' ||
    !appId.trim()
  ) {
    throw new Error('Cloud mode requires complete Firebase web app configuration.');
  }
  return { apiKey, projectId, authDomain, appId };
}

/** Public deployment configuration, never selected by an approval fragment. */
export function pairingEndpoint(
  mode: string,
  settings: Record<string, unknown>
): string | undefined {
  if (mode === 'emulator') return 'http://127.0.0.1:5001/demo-tmt-office/us-central1/officePairing';
  if (
    mode !== 'cloud' ||
    settings.VITE_OFFICE_PAIRING_URL === undefined ||
    settings.VITE_OFFICE_PAIRING_URL === ''
  )
    return undefined;
  const value = settings.VITE_OFFICE_PAIRING_URL;
  if (typeof value !== 'string') throw new Error('Invalid Office pairing service configuration.');
  const url = new URL(value);
  if (
    value.length > 2048 ||
    url.protocol !== 'https:' ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    url.pathname.endsWith('//') ||
    url.href !== value
  )
    throw new Error('Invalid Office pairing service configuration.');
  return url.href.replace(/\/$/, '');
}

/** Only this allowlisted projection is published; settings themselves never are. */
export function officeDeployment(mode: string, settings: Record<string, unknown> = {}) {
  if (mode !== 'cloud' && mode !== 'emulator') return undefined;
  const pairingUrl = pairingEndpoint(mode, settings);
  if (!pairingUrl) return undefined;
  const config = officeFirebaseConfig(mode, '127.0.0.1', settings);
  if (!config) return undefined;
  if (typeof config.apiKey !== 'string' || !/^[A-Za-z0-9_-]{1,256}$/.test(config.apiKey))
    throw new Error('Invalid Office public API key.');
  return {
    version: 1 as const,
    mode,
    projectId: config.projectId,
    apiKey: config.apiKey,
    pairingUrl,
  };
}
