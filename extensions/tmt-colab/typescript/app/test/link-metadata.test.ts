import { describe, expect, it, vi } from 'vite-plus/test';
import { canonical, digest, encodeBinary, metadataRequest, text } from '@tmt/colab-client';
import {
  buildMetadataRequest,
  MetadataRequests,
  type MetadataInput,
} from '../src/link-metadata.js';
import type { JsonValue, OwnState } from '../src/fold-protocol.js';

const writer = '00000000-0000-4000-8000-000000000001';
const owner = '00000000-0000-4000-8000-000000000002';
const input: MetadataInput = {
  action: 'creator.set',
  body: { selector: 'gui-3' },
  target: { creator: null, creationRecipient: null },
};
const revision = `v1:${'a'.repeat(64)}`;
async function receipt(
  request: metadataRequest.MetadataRequest,
  result: metadataRequest.EditResult['result'] = {
    operationId: request.operationId,
    state: 'committed',
    revision,
  },
): Promise<OwnState> {
  const value = metadataRequest.editResult({
    version: 1,
    kind: 'edit-result',
    writer,
    operationId: request.operationId,
    requestSha256: encodeBinary(await digest(text(canonical(request)))),
    result,
  });
  return {
    [owner]: {
      threads: {},
      intents: {},
      replies: {},
      messages: {
        [metadataRequest.outcomeKey(writer, request.operationId)]: value as unknown as JsonValue,
      },
    },
  };
}

describe('editor metadata request lifetime', () => {
  it('freezes the target and validates the closed request before any transport', () => {
    const mutable = structuredClone(input);
    const request = buildMetadataRequest(mutable);
    mutable.target.creator = { changed: true };
    expect(request.target.creator).toBeNull();
    expect(metadataRequest.metadataRequest(request, 'intents', request.operationId)).toEqual(
      request,
    );
    expect(buildMetadataRequest(input).operationId).not.toBe(request.operationId);
    expect(() => buildMetadataRequest({ ...input, body: { selector: ' ' } })).toThrow();
  });
  it.each(['committed', 'unchanged'] as const)(
    'observes an owner %s receipt arriving during append',
    async (state) => {
      const sent: metadataRequest.MetadataRequest[] = [];
      const requests = new MetadataRequests('space/page/1', writer, async (request) => {
        sent.push(request);
        requests.show(
          await receipt(request, { operationId: request.operationId, state, revision }),
          () => true,
        );
        return 'sent';
      });
      const outcome = await requests.send(input);
      expect(outcome).toEqual({ operationId: sent[0].operationId, state: 'applied', revision });
      expect(await requests.check(outcome.operationId)).toEqual(outcome);
      expect(sent).toHaveLength(1);
    },
  );
  it('returns the stable owner refusal without changing the requested target', async () => {
    const requests = new MetadataRequests('space/page/1', writer, async (request) => {
      requests.show(
        await receipt(request, {
          operationId: request.operationId,
          state: 'rejected',
          code: 'COLAB_IDENTITY_UNKNOWN',
          message: 'Agent not found',
        }),
        () => true,
      );
      return 'sent';
    });
    expect(await requests.send(input)).toMatchObject({
      state: 'refused',
      code: 'COLAB_IDENTITY_UNKNOWN',
      message: 'Agent not found',
    });
    expect(input.target.creator).toBeNull();
  });
  it('retains an uncertain append and checks its receipt without sending again', async () => {
    const sent: metadataRequest.MetadataRequest[] = [];
    const requests = new MetadataRequests(
      'space/page/1',
      writer,
      async (request) => {
        sent.push(request);
        return 'unconfirmed';
      },
      0,
    );
    const initial = await requests.send(input);
    expect(initial.state).toBe('unconfirmed');
    expect((await requests.check(initial.operationId)).state).toBe('unconfirmed');
    requests.show(await receipt(sent[0]), () => true);
    expect(await requests.check(initial.operationId)).toMatchObject({ state: 'applied', revision });
    expect(sent).toHaveLength(1);
  });
  it('never accepts a link-authored receipt or one bound to another request', async () => {
    let request!: metadataRequest.MetadataRequest;
    const requests = new MetadataRequests(
      'space/page/1',
      writer,
      async (value) => {
        request = value;
        return 'sent';
      },
      0,
    );
    const initial = await requests.send(input);
    const own = await receipt(request);
    requests.show(own, () => false);
    expect((await requests.check(initial.operationId)).state).toBe('unconfirmed');
    const key = metadataRequest.outcomeKey(writer, request.operationId);
    const forged = structuredClone(own);
    (forged[owner].messages[key] as Record<string, JsonValue>).requestSha256 = encodeBinary(
      new Uint8Array(32),
    );
    requests.show(forged, () => true);
    expect((await requests.check(initial.operationId)).state).toBe('unconfirmed');
    requests.show(own, () => true);
    expect((await requests.check(initial.operationId)).state).toBe('applied');
  });
  it('restores status to a new subscriber and observes a late receipt without another send', async () => {
    const sent: metadataRequest.MetadataRequest[] = [];
    const requests = new MetadataRequests('space/page/1', writer, async (request) => {
      sent.push(request);
      return 'unconfirmed';
    });
    const initial = await requests.send(input);
    const oldListener = vi.fn();
    const dispose = requests.subscribe(oldListener);
    expect(requests.snapshot()?.outcome).toEqual(initial);
    dispose();
    const replacement = vi.fn();
    const unsubscribe = requests.subscribe(replacement);
    requests.show(await receipt(sent[0]), () => true);
    await vi.waitFor(() => expect(requests.snapshot()?.outcome?.state).toBe('applied'));
    expect(requests.snapshot()?.request.operationId).toBe(initial.operationId);
    expect(replacement).toHaveBeenCalled();
    expect(oldListener).not.toHaveBeenCalled();
    expect(sent).toHaveLength(1);
    unsubscribe();
    requests.end();
  });
  it('ends outstanding observations on page/epoch replacement and ignores late receipts', async () => {
    let request!: metadataRequest.MetadataRequest;
    const requests = new MetadataRequests('space/page/1', writer, async (value) => {
      request = value;
      return 'unconfirmed';
    });
    const initial = await requests.send(input);
    const pending = requests.check(initial.operationId);
    requests.end();
    expect((await pending).state).toBe('ended');
    requests.show(await receipt(request), () => true);
    expect((await requests.check(initial.operationId)).state).toBe('ended');
    expect((await requests.send(input)).state).toBe('ended');
  });
  it('distinguishes confirmed non-submission and unexpected append uncertainty', async () => {
    const refused = new MetadataRequests('scope', writer, async () => 'refused');
    const outcome = await refused.send(input);
    expect(outcome.state).toBe('not-sent');
    expect(await refused.check(outcome.operationId)).toEqual(outcome);
    const uncertain = new MetadataRequests('scope', writer, async () => {
      throw new Error('lost reply');
    });
    expect((await uncertain.send(input)).state).toBe('unconfirmed');
  });
});
