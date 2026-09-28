export function serviceEnvironment(env: NodeJS.ProcessEnv): { enabled: boolean; origin?: string } {
  const auth = env.FIREBASE_AUTH_EMULATOR_HOST;
  const firestore = env.FIRESTORE_EMULATOR_HOST;
  const emulator = env.FUNCTIONS_EMULATOR === 'true';
  if (emulator || auth || firestore) {
    if (
      !emulator ||
      env.GCLOUD_PROJECT !== 'demo-tmt-office' ||
      auth !== '127.0.0.1:9099' ||
      firestore !== '127.0.0.1:8080'
    )
      throw new Error('Office pairing requires the isolated demo emulator configuration.');
    return { enabled: true, origin: 'http://127.0.0.1:4173' };
  }
  const origin = env.TMT_OFFICE_ORIGIN;
  if (env.TMT_OFFICE_PAIRING_ENABLED !== 'true') return { enabled: false };
  if (!origin) throw new Error('Office pairing requires an explicit HTTPS browser origin.');
  const url = new URL(origin);
  if (url.protocol !== 'https:' || url.origin !== origin || url.username || url.password)
    throw new Error('Office pairing requires an exact HTTPS browser origin.');
  return { enabled: true, origin };
}
