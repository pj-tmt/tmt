import { initializeApp } from 'firebase-admin/app';
import { getAuth } from 'firebase-admin/auth';
import { getFirestore } from 'firebase-admin/firestore';
import { onRequest } from 'firebase-functions/v2/https';
import { serviceEnvironment } from './firebase-environment.js';
import { createPairingHandler } from './pairing-http.js';
import { createPairingService } from './pairing-service.js';
import { createPairingStore } from './pairing-store.js';

const environment = serviceEnvironment(process.env);
const app = initializeApp();
const handle = createPairingHandler(
  createPairingService(createPairingStore(getFirestore(app)), getAuth(app)),
  environment.origin
);

export const officePairing = onRequest(
  {
    cors: environment.origin ? [environment.origin] : false,
    maxInstances: 1,
    concurrency: 8,
    timeoutSeconds: 30,
    memory: '256MiB',
  },
  async (request, response) => {
    response.set('Cache-Control', 'no-store');
    if (!environment.enabled) {
      response.status(503).json({ error: { code: 'UNAVAILABLE' } });
      return;
    }
    const result = await handle({
      method: request.method,
      path: request.path,
      authorization: request.get('authorization'),
      origin: request.get('origin'),
      contentType: request.get('content-type'),
      bodyBytes: request.rawBody?.length ?? 0,
      body: request.body,
    });
    if (result.status === 429) response.set('Retry-After', '5');
    response.status(result.status).json(result.body);
  }
);
