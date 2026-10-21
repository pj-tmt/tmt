import {
  copy,
  decimal,
  frame,
  generatedId,
  requireValue,
  spaceId,
  text,
  time,
  type Bytes,
} from './bytes.js';
import { digest, hmac, strictVerify, validEdPoint } from './crypto.js';
export interface SignIn {
  codeId: string;
  space: string;
  device: string;
  signingKey: Uint8Array;
  encryptionKey: Uint8Array;
  nonce: Uint8Array;
}
export function signinInput(v: SignIn): Bytes {
  generatedId(v.codeId);
  spaceId(v.space);
  generatedId(v.device);
  requireValue(validEdPoint(v.signingKey));
  return frame(
    text('tmt-colab-signin-v1'),
    text('1'),
    text(v.codeId),
    text(v.space),
    text(v.device),
    copy(v.signingKey, 32),
    copy(v.encryptionKey, 32),
    copy(v.nonce, 16),
  );
}
export function signinPossessionInput(v: SignIn): Bytes {
  return frame(text('tmt-colab-signin-possession-v1'), text('1'), signinInput(v));
}
export async function signinProof(code: Uint8Array, v: SignIn): Promise<Bytes> {
  return hmac(copy(code, 16), signinInput(v));
}
export async function verifySignin(
  code: Uint8Array,
  v: SignIn,
  proof: Uint8Array,
  signature: Uint8Array,
): Promise<boolean> {
  const input = signinInput(v),
    key = copy(code, 16),
    mac = copy(proof),
    sig = copy(signature),
    publicKey = copy(v.signingKey, 32);
  if (mac.length !== 32) return false;
  const native = await crypto.subtle.importKey(
    'raw',
    key,
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['verify'],
  );
  return (
    (await crypto.subtle.verify('HMAC', native, mac, input)) &&
    (await strictVerify(
      publicKey,
      sig,
      frame(text('tmt-colab-signin-possession-v1'), text('1'), input),
    ))
  );
}
const OPERATIONS = [
  'member.add',
  'member.remove',
  'member.role',
  'link.add',
  'link.remove',
  'device.revoke',
  'bridge.add',
  'epoch.advance',
  'page.share',
  'page.history',
  'retention.set',
  'page.archive',
  'page.unarchive',
  'page.delete',
];
export function operation(value: string): void {
  requireValue(OPERATIONS.includes(value));
}
/** Display-only operations change owner-store state without a log statement; only management requests name them. */
const DISPLAY_ONLY_OPERATIONS = ['page.projects', 'project.manage'];
/** The page a space-level request names: the frame binds one page and a project change has none. Only `project.manage` may name it. */
export const SPACE_SCOPE_PAGE = '00000000-0000-4000-8000-000000000000';
export function managementOperation(value: string): void {
  requireValue(OPERATIONS.includes(value) || DISPLAY_ONLY_OPERATIONS.includes(value));
}
/** A log operation names the membership head it was prepared against; a display-only one has none, so its signed field is empty. */
function managementRevision(operationName: string, value: string): void {
  if (DISPLAY_ONLY_OPERATIONS.includes(operationName)) requireValue(value === '');
  else decimal(value);
}
export interface Management {
  space: string;
  page: string;
  expectedRevision: string;
  operationId: string;
  operation: string;
  payload: Uint8Array;
  senderDevice: string;
  issuedAt: number;
  expiresAt: number;
}
/** Caller validates the operation's user-selected payload schema before building signed bytes. */
export async function managementInput(v: Management): Promise<Bytes> {
  spaceId(v.space);
  for (const id of [v.page, v.operationId, v.senderDevice]) generatedId(id);
  managementOperation(v.operation);
  managementRevision(v.operation, v.expectedRevision);
  requireValue((v.operation === 'project.manage') === (v.page === SPACE_SCOPE_PAGE));
  time(v.issuedAt);
  time(v.expiresAt);
  requireValue(
    v.expiresAt > v.issuedAt && v.expiresAt - v.issuedAt <= 600000 && v.payload.length <= 16384,
  );
  const prefix = [v.space, v.page, v.expectedRevision, v.operationId, v.operation].map(text);
  const suffix = [v.senderDevice, v.issuedAt.toString(), v.expiresAt.toString()].map(text);
  const payload = copy(v.payload);
  return frame(
    text('tmt-colab-management-v1'),
    text('1'),
    ...prefix,
    await digest(payload),
    ...suffix,
  );
}
