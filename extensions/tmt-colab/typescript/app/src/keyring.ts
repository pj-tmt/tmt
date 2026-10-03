import { RecipientKey, sign, signingKey, strictVerify, text } from '@tmt/colab-client';

import { record } from './storage.js';

export interface DeviceKeys {
  sign: CryptoKey;
  signPublic: Uint8Array<ArrayBuffer>;
  enc: RecipientKey;
}
interface Stored {
  sign: CryptoKey;
  signPublic: Uint8Array<ArrayBuffer>;
  enc: CryptoKey;
  encPublic: Uint8Array<ArrayBuffer>;
}

/** Only Colab-owned records; Remote's keyring and private bytes never enter here. */
export async function deviceKeys(deviceId: string): Promise<DeviceKeys> {
  return navigator.locks.request(`colab-keys:${deviceId}`, async () => {
    let stored = await record<Stored>(`keys:${deviceId}`);
    if (!stored) {
      const pair = (await crypto.subtle.generateKey('Ed25519', false, [
        'sign',
        'verify',
      ])) as CryptoKeyPair;
      const enc = await RecipientKey.generate();
      stored = {
        sign: pair.privateKey,
        signPublic: new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey)),
        enc: enc.handle(),
        encPublic: enc.publicKey(),
      };
      await record(`keys:${deviceId}`, stored);
    }
    signingKey(stored.sign);
    const probe = text('tmt-colab-keyring-check-v1');
    if (!(await strictVerify(stored.signPublic, await sign(stored.sign, probe), probe)))
      throw new Error('Device signing key mismatch');
    return {
      sign: stored.sign,
      signPublic: new Uint8Array(stored.signPublic),
      enc: await RecipientKey.fromHandle(stored.enc, stored.encPublic),
    };
  });
}
