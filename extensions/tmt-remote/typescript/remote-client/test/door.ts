import assert from 'node:assert/strict';
import {
  createHmac,
  createPublicKey,
  generateKeyPairSync,
  randomBytes,
  sign,
  verify,
  type KeyObject,
} from 'node:crypto';
import {
  enrollmentPossessionSigningBytes,
  enrollmentSigningBytes,
  envelopeSigningBytes,
  responseKeyInput,
  serverProofInput,
  type Envelope,
} from '../src/canonical-bytes.js';
import { DeviceKey, pair, parseLink, type Descriptor } from '../src/device.js';
/**
 * A stand-in door built on node:crypto, independent of the SDK's WebCrypto
 * path: it checks the enrollment MAC and possession signature, answers 202
 * once, then issues a receipt with serverProof and signs session responses.
 */
export const ORIGIN = 'http://127.0.0.1:43210';
export const b64 = (bytes: Uint8Array): string => Buffer.from(bytes).toString('base64url');
export const raw = (text: string): Buffer => Buffer.from(text, 'base64url');
export function publicKeyObject(key: Uint8Array): KeyObject {
  const spki = Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key]);
  return createPublicKey({ key: spki, format: 'der', type: 'spki' });
}
export class Door {
  code = randomBytes(16);
  machine = generateKeyPairSync('ed25519');
  machinePublic = this.machine.publicKey.export({ format: 'der', type: 'spki' }).subarray(12);
  descriptor: Descriptor = {
    profile: 'local-v1',
    binding: 'loopback-http',
    machineId: crypto.randomUUID(),
    windowId: crypto.randomUUID(),
    offerId: crypto.randomUUID(),
    address: `${ORIGIN}/r/${'a'.repeat(16)}`,
    serverChallenge: randomBytes(16).toString('hex'),
  };
  clientId = crypto.randomUUID();
  pending = 1;
  tamper: { proof?: boolean; receiptKey?: boolean; signature?: boolean; correlation?: boolean } =
    {};
  link(): string {
    const symbols = base32(this.code);
    return `${ORIGIN}/pair#${symbols}`;
  }
  fetch = (async (url: string, init: RequestInit): Promise<Response> => {
    const body = JSON.parse(init.body as string) as Record<string, string>;
    return url.endsWith('/pair')
      ? this.pair(body)
      : body.operation === 'session.open'
        ? this.session(body)
        : this.operation(body, init);
  }) as typeof fetch;
  pair(body: Record<string, string>): Response {
    const enrollment = enrollmentSigningBytes({
      profile: 'local-v1',
      machineId: body.machineId!,
      windowId: body.windowId!,
      offerId: body.offerId!,
      serverChallenge: Buffer.from(body.serverChallenge!, 'hex'),
      clientNonce: Buffer.from(body.clientNonce!, 'hex'),
      kind: body.kind as 'browser',
      origin: body.origin!,
      name: body.name!,
      publicKey: raw(body.publicKey!),
    });
    const mac = createHmac('sha256', this.code).update(enrollment).digest();
    assert.equal(body.mac, b64(mac));
    assert.ok(
      verify(
        null,
        enrollmentPossessionSigningBytes(enrollment, mac),
        publicKeyObject(raw(body.publicKey!)),
        raw(body.signature!),
      ),
    );
    if (this.pending-- > 0) return Response.json({ state: 'pending' }, { status: 202 });
    const receipt = Buffer.from(
      JSON.stringify({
        grant: {
          clientId: this.clientId,
          machineId: this.descriptor.machineId,
          profile: 'local-v1',
          publicKey: this.tamper.receiptKey ? b64(randomBytes(32)) : body.publicKey,
          kind: body.kind,
          origin: body.origin,
          name: body.name,
          agents: 'all',
          scopes: ['agents.read'],
          mode: 'direct',
          issuedAtMs: 1,
          expiresAtMs: null,
          revision: 1,
          disabled: false,
        },
        machinePublicKey: b64(this.machinePublic),
      }),
    );
    const responseKey = createHmac('sha256', this.code)
      .update(responseKeyInput(enrollment))
      .digest();
    const proof = createHmac('sha256', responseKey).update(serverProofInput(receipt)).digest();
    if (this.tamper.proof) proof[0]! ^= 1;
    return Response.json({ receipt: b64(receipt), serverProof: b64(proof) });
  }
  async session(body: Record<string, string | number | null>): Promise<Response> {
    const payload = raw(body.payload as string);
    const signed = await envelopeSigningBytes({
      ...(body as unknown as Envelope),
      payload,
    });
    assert.ok(
      verify(null, signed, publicKeyObject(this.devicePublic!), raw(body.signature as string)),
    );
    const sessionId = crypto.randomUUID();
    this.opens++;
    this.sessionId = sessionId;
    this.clientSequence = 1n;
    this.machineSequence = 1n;
    const reply = Buffer.from(
      JSON.stringify({ sessionId, serverTimeMs: 2, grantRevision: 1, expiresAtMs: null }),
    );
    const response = {
      version: 1 as const,
      profile: 'local-v1' as const,
      kind: 'response' as const,
      id: crypto.randomUUID(),
      correlationId: this.tamper.correlation ? crypto.randomUUID() : (body.id as string),
      machineId: this.descriptor.machineId,
      windowId: this.descriptor.windowId,
      clientId: this.clientId,
      sessionId,
      sequence: '1',
      timestampMs: 3,
      origin: ORIGIN,
      operation: 'session.open',
    };
    const signature = sign(
      null,
      await envelopeSigningBytes({ ...response, payload: reply }),
      this.machine.privateKey,
    );
    if (this.tamper.signature) signature[0]! ^= 1;
    return Response.json({ ...response, payload: b64(reply), signature: b64(signature) });
  }
  sessionId = '';
  opens = 0;
  clientSequence = 1n;
  machineSequence = 1n;
  calls: { envelope: Record<string, unknown>; payload: Record<string, unknown>; bytes: string }[] =
    [];
  intents = new Map<string, string>();
  receipts = new Map<string, Record<string, unknown>>();
  agents: unknown[] = [{ id: crypto.randomUUID(), name: 'Agent', presence: 'active' }];
  httpStatus = 200;
  errorBeforeConsume = false;
  forceReplay = false;
  replyState = 'replied';
  final = '';
  managementReply?: (operation: string, payload: Record<string, unknown>) => unknown;
  sendState = 'accepted';
  error?: {
    code: string;
    message: string;
    retryAfterMs?: number;
    limit?: number;
    scope?: string;
    settingsUrl?: string | null;
  };
  afterSign?: (response: Record<string, unknown>) => void;
  mutate?: (response: Record<string, unknown>) => void;
  afterAdoption?: (body: Record<string, unknown>, init: RequestInit) => Promise<void>;
  async operation(body: Record<string, unknown>, init: RequestInit): Promise<Response> {
    const bytes = raw(body.payload as string);
    assert.equal(body.version, 1);
    assert.equal(body.profile, 'local-v1');
    assert.equal(body.kind, 'request');
    assert.equal(body.correlationId, null);
    assert.equal(body.machineId, this.descriptor.machineId);
    assert.equal(body.windowId, this.descriptor.windowId);
    assert.equal(body.clientId, this.clientId);
    assert.equal(body.sessionId, this.sessionId);
    assert.equal(body.origin, ORIGIN);
    assert.equal(init.credentials, 'omit');
    assert.ok(
      verify(
        null,
        await envelopeSigningBytes({ ...(body as unknown as Envelope), payload: bytes }),
        publicKeyObject(this.devicePublic!),
        raw(body.signature as string),
      ),
    );
    const payload = JSON.parse(bytes.toString('utf8')) as Record<string, unknown>;
    this.calls.push({ envelope: body, payload, bytes: bytes.toString('utf8') });
    if (this.forceReplay || body.sequence !== this.clientSequence.toString()) {
      return this.response(body, {
        error: { code: 'REMOTE_REPLAY', message: 'Sequence or busy refusal.' },
      });
    }
    if (this.error && this.errorBeforeConsume) return this.response(body, { error: this.error });
    this.clientSequence++;

    let result: unknown;
    if (this.error) result = { error: this.error };
    else
      switch (body.operation) {
        case 'capabilities':
          assert.deepEqual(payload, {});
          result = {
            version: 1,
            profile: 'local-v1',
            binding: 'loopback-http',
            operations: ['agents.list', 'dispatch.create', 'operation.show', 'result'],
            limits: { inputBytes: 1024, outputBytes: 4096 },
          };
          break;
        case 'dispatch.create': {
          const input = payload.input as {
            operationId: string;
            recipientIds: string[];
            message: string;
            kind: string;
          };
          assert.deepEqual(Object.keys(payload).sort(), [
            'input',
            'operation',
            'originator',
            'version',
          ]);
          assert.equal(payload.version, 1);
          assert.equal(payload.operation, body.operation);
          assert.equal(payload.originator, 'anonymous');
          assert.deepEqual(Object.keys(input).sort(), [
            'kind',
            'message',
            'operationId',
            'recipientIds',
          ]);
          assert.equal(input.operationId, body.id);
          assert.equal(input.kind, 'request');
          assert.equal(input.recipientIds.length, 1);
          if (this.intents.has(input.operationId))
            assert.equal(this.intents.get(input.operationId), bytes.toString('utf8'));
          this.intents.set(input.operationId, bytes.toString('utf8'));
          result = {
            state: this.sendState,
            operationId: body.id,
            ...(this.sendState === 'accepted' ? { requestId: `req_${crypto.randomUUID()}` } : {}),
          };
          result = this.receipts.get(input.operationId) ?? result;
          this.receipts.set(input.operationId, result as Record<string, unknown>);
          break;
        }
        case 'operation.show':
          assert.deepEqual(Object.keys(payload), ['operationId']);
          result = this.receipts.get(payload.operationId as string) ?? {
            error: { code: 'REMOTE_STATE_UNAVAILABLE', message: 'Missing' },
          };
          break;
        case 'result':
          assert.deepEqual(Object.keys(payload), ['requestId']);
          result = {
            state: this.replyState,
            requestId: payload.requestId,
            ...(this.replyState === 'replied' ? { message: this.final } : {}),
          };
          break;
        case 'agents.list':
          assert.deepEqual(payload, {});
          result = { identities: this.agents };
          break;
        case 'remote.settings.show':
        case 'remote.settings.set':
        case 'remote.devices.list':
        case 'remote.devices.rename':
        case 'remote.devices.revoke':
        case 'remote.devices.talk':
        case 'remote.management.operation':
          assert.ok(this.managementReply, 'Management fixture is explicitly configured.');
          result = this.managementReply(body.operation as string, payload);
          break;
        default:
          assert.fail(`Unexpected operation ${String(body.operation)}`);
      }
    await this.afterAdoption?.(body, init);
    return this.response(body, result);
  }
  async response(body: Record<string, unknown>, value: unknown): Promise<Response> {
    const payload = Buffer.from(JSON.stringify(value));
    // Gaps are valid: the machine may also sign journal metadata responses.
    this.machineSequence += 2n;
    const reply: Record<string, unknown> = {
      version: 1,
      profile: 'local-v1',
      kind: 'response',
      id: crypto.randomUUID(),
      correlationId: body.id,
      machineId: this.descriptor.machineId,
      windowId: this.descriptor.windowId,
      clientId: this.clientId,
      sessionId: this.sessionId,
      sequence: this.machineSequence.toString(),
      timestampMs: Date.now(),
      origin: ORIGIN,
      operation: body.operation,
      payload: b64(payload),
    };
    this.mutate?.(reply);
    reply.signature = b64(
      sign(
        null,
        await envelopeSigningBytes({
          ...(reply as unknown as Envelope),
          payload: raw(reply.payload as string),
        }),
        this.machine.privateKey,
      ),
    );
    if (this.tamper.signature) reply.signature = b64(randomBytes(64));
    this.afterSign?.(reply);
    return Response.json(reply, { status: this.httpStatus });
  }
  devicePublic?: Uint8Array;
}
export function base32(code: Uint8Array): string {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  let bits = '';
  for (const byte of code) bits += byte.toString(2).padStart(8, '0');
  return (bits.match(/.{1,5}/g) ?? []).map((g) => alphabet[parseInt(g.padEnd(5, '0'), 2)]).join('');
}
export async function paired(door: Door) {
  const key = await DeviceKey.generate();
  door.devicePublic = key.publicKey();
  const { descriptor, code } = parseLink(door.link(), door.descriptor);
  const result = await pair({
    descriptor,
    code,
    key,
    kind: 'browser',
    origin: ORIGIN,
    name: 'Laptop',
    fetch: door.fetch,
  });
  return { key, result };
}
