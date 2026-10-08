import { describe, expect, it } from 'vite-plus/test';
import { attachment, binary, strictJson, text } from '@tmt/colab-client';
import corpus from '../../../contracts/vectors/attachment-v1.json';
import { FrozenAttachmentUpload, readPolicy, uploadPolicy } from '../src/attachment-channel.js';

describe('frozen attachment object-channel bindings', () => {
  for (const answer of corpus.channelPolicies) {
    it(`${answer.name} uses the exact shared minimal policy and immutable ciphertext`, async () => {
      const value = corpus.cases.find((c) => c.name === `${answer.name}-asset`)!;
      const descriptor = attachment.attachmentDescriptor(strictJson(text(value.input), 2048));
      expect(await uploadPolicy(descriptor)).toEqual(text(answer.upload));
      expect(
        readPolicy(descriptor, answer.peerEpoch, attachment.attachmentSelector(answer.reference)),
      ).toEqual(text(answer.read));
      const bytes = binary(
        'payload' in value ? value.payload! : '',
        attachment.ATTACHMENT_PAYLOAD_BYTES,
      );
      const frozen = await FrozenAttachmentUpload.capture(
        descriptor,
        `v1:${'ab'.repeat(32)}`,
        '11111111-1111-4111-8111-111111111111',
        bytes,
      );
      const first = frozen.policy();
      first.fill(0);
      expect(frozen.policy()).toEqual(text(answer.upload));
      descriptor.filename = 'changed draft';
      const copy = frozen.descriptor;
      copy.objectId = 'ef'.repeat(32);
      expect(frozen.descriptor.filename).toBe('cat 🐈.bin');
      expect(frozen.descriptor.objectId).not.toBe(copy.objectId);
      const part = frozen.part(0);
      expect(part).toEqual(bytes);
      part.fill(0);
      expect(frozen.part(0)).toEqual(bytes);
      expect(() => frozen.part(1)).toThrow();
      expect(() => frozen.part(-1)).toThrow();
      expect(() => frozen.part(0.5)).toThrow();
      const bad = bytes.slice();
      bad[0] ^= 1;
      await expect(
        FrozenAttachmentUpload.capture(frozen.descriptor, frozen.base, frozen.transferId, bad),
      ).rejects.toThrow();
      bytes.fill(0);
      expect(frozen.part(0)).not.toEqual(bytes);
    });
  }
});
