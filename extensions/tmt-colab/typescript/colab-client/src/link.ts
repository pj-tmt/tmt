/** Purpose-separated link keys derived from a bearer seed, and the link-device chain a reader presents. */
import {
  binary,
  concat,
  copy,
  decimal,
  encodeBinary,
  frame,
  generatedId,
  requireValue,
  spaceId,
  text,
  time,
  type Bytes,
} from './bytes.js';
import * as certificate from './certificate.js';
import { hkdf, sign } from './crypto.js';
import { RecipientKey } from './keys.js';

const ED25519_PKCS8 = Uint8Array.of(
  0x30,
  0x2e,
  0x02,
  0x01,
  0x00,
  0x30,
  0x05,
  0x06,
  0x03,
  0x2b,
  0x65,
  0x70,
  0x04,
  0x22,
  0x04,
  0x20,
);
const X25519_PKCS8 = Uint8Array.of(
  0x30,
  0x2e,
  0x02,
  0x01,
  0x00,
  0x30,
  0x05,
  0x06,
  0x03,
  0x2b,
  0x65,
  0x6e,
  0x04,
  0x22,
  0x04,
  0x20,
);
export interface LinkKeys {
  space: string;
  id: string;
  /** Non-extractable signing handle; certifies reader devices. */
  sign: CryptoKey;
  signingPublic: Bytes;
  /** Opens the link-addressed epoch wraps. */
  encryption: RecipientKey;
}
async function derive(seed: Uint8Array, label: string, space: string, id: string) {
  return hkdf(copy(seed, 32), frame(text(label), text(space), text(id)));
}
/** Same derivation as the Rust model's `link::Keys`; the seed is only borrowed. */
export async function deriveLink(seed: Uint8Array, space: string, id: string): Promise<LinkKeys> {
  spaceId(space);
  generatedId(id);
  const signingSeed = await derive(seed, 'tmt-colab-link-signing-seed-v1', space, id),
    encryptionSeed = await derive(seed, 'tmt-colab-link-encryption-seed-v1', space, id);
  try {
    const signingBytes = concat(ED25519_PKCS8, signingSeed);
    // WebCrypto derives no public key from a private one; read it from a throwaway extractable copy.
    const probe = await crypto.subtle.importKey('pkcs8', signingBytes, 'Ed25519', true, ['sign']);
    const jwk = await crypto.subtle.exportKey('jwk', probe);
    requireValue(typeof jwk.x === 'string');
    const signingPublic = binary(jwk.x, 32, 32),
      handle = await crypto.subtle.importKey('pkcs8', signingBytes, 'Ed25519', false, ['sign']);
    signingBytes.fill(0);
    const encryptionHandle = await crypto.subtle.importKey(
        'pkcs8',
        concat(X25519_PKCS8, encryptionSeed),
        'X25519',
        false,
        ['deriveBits'],
      ),
      base = new Uint8Array(32);
    base[0] = 9;
    const encryptionPublic = new Uint8Array(
      await crypto.subtle.deriveBits(
        { name: 'X25519', public: await crypto.subtle.importKey('raw', base, 'X25519', false, []) },
        encryptionHandle,
        256,
      ),
    );
    return {
      space,
      id,
      sign: handle,
      signingPublic,
      encryption: await RecipientKey.fromHandle(encryptionHandle, encryptionPublic),
    };
  } finally {
    signingSeed.fill(0);
    encryptionSeed.fill(0);
  }
}
export interface LinkDevice {
  deviceId: string;
  signingPublic: Uint8Array;
  encryptionPublic: Uint8Array;
  /** The link.add statement that introduced the link: its revision and 32-byte hash. */
  membershipRevision: string;
  issuerStatement: Uint8Array;
  issuedAt: number;
  expiresAt: number;
}
/** The exact chain JSON the reader challenge carries (`chain` is its base64url). */
export async function certifyDevice(link: LinkKeys, device: LinkDevice): Promise<Bytes> {
  generatedId(device.deviceId);
  decimal(device.membershipRevision);
  time(device.issuedAt);
  time(device.expiresAt);
  const input = certificate.input({
    space: link.space,
    issuerKind: 'link',
    issuerId: link.id,
    deviceId: device.deviceId,
    signingKey: device.signingPublic,
    encryptionKey: copy(device.encryptionPublic, 32),
    membershipRevision: device.membershipRevision,
    issuedAt: device.issuedAt,
    expiresAt: device.expiresAt,
  });
  return text(
    JSON.stringify({
      version: 1,
      issuerStatement: encodeBinary(copy(device.issuerStatement, 32)),
      deviceCertificate: encodeBinary(input),
      issuerSignature: encodeBinary(await sign(link.sign, input)),
    }),
  );
}
export interface ReaderChallenge {
  challengeId: string;
  nonce: Uint8Array;
  space: string;
  page: string;
  epoch: string;
  chainDigest: Uint8Array;
  expiresAt: number;
}
/** The bytes the link device signs to turn a challenge into a session. */
export function readerSessionInput(c: ReaderChallenge): Bytes {
  generatedId(c.challengeId);
  spaceId(c.space);
  generatedId(c.page);
  decimal(c.epoch);
  time(c.expiresAt);
  return frame(
    text('tmt-colab-reader-session-v1'),
    text('1'),
    text(c.challengeId),
    copy(c.nonce, 32),
    text(c.space),
    text(c.page),
    text(c.epoch),
    copy(c.chainDigest, 32),
    text(String(c.expiresAt)),
  );
}
