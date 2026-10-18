import {
  binary,
  copy,
  decimal,
  encodeBinary,
  equal,
  exactKeys,
  requireValue,
  statement,
  strictJson,
} from '@tmt/colab-client';
import { type Admission, StatementTransfer, STATEMENT_ENVELOPE_BYTES } from './admission.js';

export interface HostedMembershipPin {
  revision: string;
  statement: Uint8Array;
}
/** The committed baseline must be available and pass the existing signed verifiers. */
export class HostedReshareRequired extends Error {
  readonly code = 'COLAB_HOSTED_RESHARE_REQUIRED';
  constructor() {
    super('ask the owner to share it again');
  }
}
/** Discovery order and unsigned routing never become membership authority. */
export async function admitHostedMembership(
  admission: Admission,
  originals: readonly Uint8Array[],
  pin: HostedMembershipPin,
): Promise<void> {
  const selected = originals.map((bytes) => new Uint8Array(bytes));
  const pinnedRevision = decimal(pin.revision),
    pinnedHash = copy(pin.statement, 32);
  requireValue(admission.reader !== undefined && selected.length > 0 && selected.length <= 4096);
  requireValue(
    selected.reduce((total, bytes) => total + Math.ceil((bytes.length * 4) / 3), 0) <=
      4 * 1024 * 1024,
  );
  const records: { bytes: Uint8Array; revision: bigint; hash: Uint8Array }[] = [];
  for (const bytes of selected) {
    const value = strictJson(bytes, STATEMENT_ENVELOPE_BYTES);
    exactKeys(value, ['statement', 'payload', 'signature']);
    const header = statement.decode(binary(value.statement, 1024));
    requireValue(header.space === admission.space);
    const envelope = statement.Envelope.fromJson(bytes);
    records.push({ bytes, revision: decimal(header.revision), hash: await envelope.hash() });
  }
  // These are only ordering claims. Admission.membership below verifies every new signature,
  // payload, predecessor and owner binding before dependent data can be used.
  records.sort((a, b) => (a.revision < b.revision ? -1 : a.revision > b.revision ? 1 : 0));
  records.forEach((record, index) => requireValue(record.revision === BigInt(index + 1)));
  requireValue(
    pinnedRevision <= BigInt(records.length) &&
      equal(records[Number(pinnedRevision) - 1].hash, pinnedHash),
  );
  const prior = admission.statements();
  requireValue(
    prior.length <= records.length && (admission.head?.revision ?? 0n) === BigInt(prior.length),
  );
  prior.forEach((known, index) => requireValue(equal(known.head.hash, records[index].hash)));
  const tail = records.slice(prior.length),
    target = records[records.length - 1];
  for (let index = 0; index < Math.max(tail.length, 1); index++) {
    const record = tail[index];
    await admission.membership(
      {
        ...(index === 0
          ? {
              revision: target.revision.toString(),
              statementHash: encodeBinary(target.hash),
              ownerKey: encodeBinary(admission.owner),
            }
          : {}),
        statements: record ? [new StatementTransfer(record.bytes, record.hash)] : [],
        more: index + 1 < tail.length,
      },
      index === 0,
    );
  }
}
