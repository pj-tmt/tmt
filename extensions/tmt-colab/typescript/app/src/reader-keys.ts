import { copy, deriveSpaceId, link as linkKeys, requireValue } from '@tmt/colab-client';
import type { ReaderLink } from './reader-link.js';

/** One key/proof owner for native and hosted readers. Consume the seed before any request. */
export async function deriveReaderKeys(target: ReaderLink): Promise<{
  link: linkKeys.LinkKeys;
  device: linkKeys.DerivedDevice;
  chain: Uint8Array;
}> {
  const seed = new Uint8Array(target.seed);
  target.seed.fill(0);
  const selected = {
    ...target,
    root: new Uint8Array(target.root),
    statement: new Uint8Array(target.statement),
  };
  try {
    requireValue((await deriveSpaceId(copy(selected.root, 32))) === selected.space);
    const link = await linkKeys.deriveLink(seed, selected.space, selected.link),
      device = await linkKeys.deriveDevice(seed, selected.space, selected.link, 'door');
    // Every open presents byte-identical proof; the bearer token comes from the same model.
    const chain = await linkKeys.certifyDevice(link, {
      deviceId: device.id,
      signingPublic: device.signingPublic,
      encryptionPublic: link.encryption.publicKey(),
      membershipRevision: selected.revision,
      issuerStatement: selected.statement,
      issuedAt: linkKeys.DEVICE_ISSUED_AT,
      expiresAt: linkKeys.DEVICE_EXPIRES_AT,
    });
    return { link, device, chain };
  } finally {
    seed.fill(0);
  }
}
