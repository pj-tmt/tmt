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

/** Browser-local encryption keys; never certified, exported or shared across purposes. */
export type LocalKeyPurpose = 'title' | 'draft';

/** `title` keeps its original record and lock names so existing hints keep decrypting. */
async function localKey(purpose: LocalKeyPurpose, deviceId: string): Promise<CryptoKey> {
  return navigator.locks.request(`colab-${purpose}-key:${deviceId}`, async () => {
    let key = await record<CryptoKey>(`${purpose}-key:${deviceId}`);
    if (key === undefined) {
      key = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, [
        'encrypt',
        'decrypt',
      ]);
      await record(`${purpose}-key:${deviceId}`, key);
    }
    if (
      !(key instanceof CryptoKey) ||
      key.type !== 'secret' ||
      key.extractable ||
      key.algorithm.name !== 'AES-GCM' ||
      (key.algorithm as AesKeyAlgorithm).length !== 256 ||
      !key.usages.includes('encrypt') ||
      !key.usages.includes('decrypt')
    )
      throw new Error(`Local ${purpose} key unavailable`);
    return key;
  });
}
/** Browser-local title encryption only. */
export const titleKey = (deviceId: string) => localKey('title', deviceId);
/** Browser-local unsent-draft encryption only. */
export const draftKey = (deviceId: string) => localKey('draft', deviceId);
