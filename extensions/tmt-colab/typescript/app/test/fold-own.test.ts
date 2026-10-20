import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import { Admission } from '../src/admission.js';
import { Objects } from '../src/objects.js';
import type { Registration } from '../src/registration.js';

vi.mock('../src/storage.js', () => ({ record: async () => undefined }));
vi.stubGlobal('navigator', { locks: { request: async (_key: string, fn: () => unknown) => fn() } });

// The same frozen bytes native `src/fold_vectors.rs` replays at model level: each case is a
// catch-up of anchor frames and fold frames, and marks the ones both sides admit and refuse.
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/fold-v1.json', import.meta.url), 'utf8'),
);
interface Row {
  name: string;
  log: string;
  frames: { streamId: string; namespace: string; checkpoint: unknown; anchor?: unknown }[];
  expect: string;
}
async function admission(log: string): Promise<Admission> {
  const statements: string[] = v.logs[log];
  const last = c.statement.Envelope.fromJson(c.binary(statements.at(-1)!, 32 * 1024));
  const a = new Admission(
    v.spaceId,
    v.pageId,
    v.epoch,
    c.binary(v.ownerKey, 32, 32),
    {} as Registration,
    {
      linkId: '70000000-0000-4000-8000-000000000001',
      principal: 'principal',
      expiresAt: Number.MAX_SAFE_INTEGER,
    },
  );
  await a.membership(
    {
      revision: String(statements.length),
      statementHash: c.encodeBinary(await last.hash()),
      ownerKey: v.ownerKey,
      statements,
      more: false,
    },
    true,
  );
  a.root = await crypto.subtle.importKey('raw', c.binary(v.epochSecret, 32, 32), 'HKDF', false, [
    'deriveBits',
  ]);
  await a.chains(v.chains);
  return a;
}
/** Every frame in catch-up order; the first refusal ends the replay, as it ends a catch-up. */
async function replay(row: Row) {
  const objects = new Objects(await admission(row.log));
  const admitted: { namespace: string; writer: string }[] = [];
  let refused: unknown;
  for (const frame of row.frames) {
    try {
      const { checkpoints } = await objects.streams([frame]);
      admitted.push(...checkpoints);
    } catch (error) {
      refused = error;
      break;
    }
  }
  return { admitted, refused };
}

for (const row of v.cases as Row[]) {
  it(`${row.expect === 'admit' ? 'admits' : 'rejects'}: ${row.name}`, async () => {
    const { admitted, refused } = await replay(row);
    const folds = row.frames.filter((f) => f.checkpoint !== null);
    if (row.expect === 'admit') {
      expect(refused).toBeUndefined();
      // The writer of a folded record is the covered device, never the signer.
      expect(admitted).toHaveLength(folds.length);
      for (const [i, fold] of folds.entries())
        expect(admitted[i]).toMatchObject({ namespace: fold.namespace, writer: fold.streamId });
    } else {
      // A refusal of the model, not a crash of the replay.
      expect((refused as Error).message).toMatch(
        /^(Invalid colab-v1 value or proof|Fresh membership catchup required)$/,
      );
      // Nothing from a refused fold is applied.
      expect(admitted).toHaveLength(0);
    }
  });
}

it('refuses a fold header as a thirteen-field header, the way a reader of today does', () => {
  const header = c.binary(v.oldReader.header, 4096);
  expect(v.oldReader).toMatchObject({ fields: 14, readerFields: 13 });
  expect(() => c.fields(header, v.oldReader.readerFields)).toThrow();
  expect(c.fields(header, v.oldReader.fields)).toHaveLength(v.oldReader.fields);
  expect(c.decodeHeader(header)).toMatchObject({
    context: { kind: 'fold' },
    coveredDevice: v.cases[0].frames[0].streamId,
  });
});
