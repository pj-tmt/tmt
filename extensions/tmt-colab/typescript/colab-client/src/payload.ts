/** Owner-statement syntax only. Callers default absent page.history to shared and apply
 * the 64-most-recent-epochs cap, bounded sorted unique wrap lists and atomic joins. */
import {
  binary,
  decimal,
  exactKeys,
  generatedId,
  idList,
  namespace,
  requireValue,
  text,
  time,
} from './bytes.js';
import { validEdPoint } from './crypto.js';
import { strictJson } from './json.js';
import * as streamCut from './stream-cut.js';
import * as wrap from './wrap.js';
export const MAX_BYTES = 768 * 1024;
export type HistoryMode = 'shared' | 'current';
export type Role = 'viewer' | 'commenter' | 'editor';
export interface Cut {
  pageId: string;
  epoch: string;
  namespace: string;
  cut: string;
}
export interface Baseline {
  pageId: string;
  epoch: string;
  sourceDigest: string;
  baselineCommitment: string;
  title: string;
  objectEnvelopeHash: string;
  membershipRevision: string;
}
export interface PublishedKey {
  epoch: string;
  key: string;
}
export interface Values {
  'member.add': { memberId: string; role: Role; signKey: string; encKey: string; pages: string[] };
  'member.remove': { memberId: string; cuts: Cut[] };
  'member.role': { memberId: string; role: Role; cuts: Cut[] };
  'link.add': {
    linkId: string;
    role: Role;
    linkSignKey: string;
    linkEncKey: string;
    pages: string[];
  };
  'link.remove': { linkId: string; cuts: Cut[] };
  'device.revoke': { deviceId: string; cuts: Cut[] };
  'bridge.add': { machineId: string; machineSignKey: string; encKey: string; pages: string[] };
  'epoch.advance': {
    pageId: string;
    epoch: string;
    cuts: Cut[];
    baseline: Baseline;
    wraps: wrap.Envelope[];
  };
  'page.share': {
    pageId: string;
    mode: 'private' | 'link' | 'public';
    epoch: string;
    publishedKeys?: PublishedKey[];
  };
  'page.history': { pageId: string; mode: HistoryMode };
  'retention.set': { pageId: string; days: number | null };
  'page.archive': { pageId: string };
  'page.delete': { pageId: string };
}
export type Payload = { [K in keyof Values]: { operation: K; value: Values[K] } }[keyof Values];
function string(v: Record<string, unknown>, name: string): string {
  requireValue(typeof v[name] === 'string');
  return v[name];
}
function id(v: Record<string, unknown>, name: string): void {
  generatedId(string(v, name));
}
function key(v: Record<string, unknown>, name: string, signing = false): void {
  const bytes = binary(v[name], 32, 32);
  if (signing) requireValue(validEdPoint(bytes));
}
function role(v: Record<string, unknown>): void {
  requireValue(['viewer', 'commenter', 'editor'].includes(string(v, 'role')));
}
function list(value: unknown, max: number): unknown[] {
  requireValue(Array.isArray(value) && value.length <= max);
  return value;
}
function pages(value: unknown): void {
  const ids = list(value, 256);
  requireValue(ids.every((id) => typeof id === 'string'));
  idList(ids as string[]);
}
function before(a: (string | bigint)[], b: (string | bigint)[]): boolean {
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return a[i] < b[i];
  return false;
}
function cuts(value: unknown): void {
  let previous: (string | bigint)[] | undefined;
  for (const item of list(value, 512)) {
    exactKeys(item, ['pageId', 'epoch', 'namespace', 'cut']);
    id(item, 'pageId');
    const epoch = decimal(string(item, 'epoch')),
      ns = string(item, 'namespace');
    namespace(ns);
    const cut = streamCut.decode(binary(item.cut, 1024));
    requireValue(cut.namespace === ns);
    const order = [string(item, 'pageId'), epoch, ns, cut.streamId];
    requireValue(previous === undefined || before(previous, order));
    previous = order;
  }
}
export function validateBaseline(value: unknown): asserts value is Baseline {
  exactKeys(value, [
    'pageId',
    'epoch',
    'sourceDigest',
    'baselineCommitment',
    'title',
    'objectEnvelopeHash',
    'membershipRevision',
  ]);
  id(value, 'pageId');
  decimal(string(value, 'epoch'));
  decimal(string(value, 'membershipRevision'));
  string(value, 'title');
  for (const name of ['sourceDigest', 'baselineCommitment', 'objectEnvelopeHash']) key(value, name);
}
export function decodeBaseline(raw: Uint8Array): Baseline {
  const v = strictJson(raw, MAX_BYTES, true);
  validateBaseline(v);
  return v;
}
export function decode(operation: string, raw: Uint8Array): Payload {
  const v = strictJson(raw, MAX_BYTES, true);
  switch (operation) {
    case 'member.add':
      exactKeys(v, ['memberId', 'role', 'signKey', 'encKey', 'pages']);
      id(v, 'memberId');
      role(v);
      key(v, 'signKey', true);
      key(v, 'encKey');
      pages(v.pages);
      break;
    case 'member.remove':
      exactKeys(v, ['memberId', 'cuts']);
      id(v, 'memberId');
      cuts(v.cuts);
      break;
    case 'member.role':
      exactKeys(v, ['memberId', 'role', 'cuts']);
      id(v, 'memberId');
      role(v);
      cuts(v.cuts);
      break;
    case 'link.add':
      exactKeys(v, ['linkId', 'role', 'linkSignKey', 'linkEncKey', 'pages']);
      id(v, 'linkId');
      role(v);
      key(v, 'linkSignKey', true);
      key(v, 'linkEncKey');
      pages(v.pages);
      break;
    case 'link.remove':
      exactKeys(v, ['linkId', 'cuts']);
      id(v, 'linkId');
      cuts(v.cuts);
      break;
    case 'device.revoke':
      exactKeys(v, ['deviceId', 'cuts']);
      id(v, 'deviceId');
      cuts(v.cuts);
      break;
    case 'bridge.add':
      exactKeys(v, ['machineId', 'machineSignKey', 'encKey', 'pages']);
      id(v, 'machineId');
      key(v, 'machineSignKey', true);
      key(v, 'encKey');
      pages(v.pages);
      break;
    case 'epoch.advance': {
      exactKeys(v, ['pageId', 'epoch', 'cuts', 'baseline', 'wraps']);
      id(v, 'pageId');
      decimal(string(v, 'epoch'));
      cuts(v.cuts);
      validateBaseline(v.baseline);
      requireValue(v.baseline.pageId === v.pageId && v.baseline.epoch === v.epoch);
      let previous: string[] | undefined;
      v.wraps = list(v.wraps, 512).map((value) => {
        const envelope = wrap.Envelope.fromJson(text(JSON.stringify(value))),
          h = envelope.header();
        requireValue(
          h.page === v.pageId &&
            h.epoch === v.epoch &&
            h.membershipRevision === (v.baseline as Baseline).membershipRevision,
        );
        const order = [h.recipientKind, h.recipientId];
        requireValue(previous === undefined || before(previous, order));
        previous = order;
        return envelope;
      });
      break;
    }
    case 'page.share': {
      requireValue(v !== null && typeof v === 'object' && !Array.isArray(v));
      const record = v as Record<string, unknown>;
      exactKeys(
        record,
        Object.hasOwn(record, 'publishedKeys')
          ? ['pageId', 'mode', 'epoch', 'publishedKeys']
          : ['pageId', 'mode', 'epoch'],
      );
      id(record, 'pageId');
      const epoch = decimal(string(record, 'epoch')),
        mode = string(record, 'mode');
      requireValue(['private', 'link', 'public'].includes(mode));
      let prior = 0n,
        current = false;
      const keys = Object.hasOwn(record, 'publishedKeys') ? list(record.publishedKeys, 64) : [];
      for (const item of keys) {
        exactKeys(item, ['epoch', 'key']);
        const n = decimal(string(item, 'epoch'));
        requireValue(n > prior && n <= epoch);
        key(item, 'key');
        prior = n;
        current = n === epoch;
      }
      requireValue(mode === 'public' ? current : keys.length === 0);
      break;
    }
    case 'page.history':
      exactKeys(v, ['pageId', 'mode']);
      id(v, 'pageId');
      requireValue(['shared', 'current'].includes(string(v, 'mode')));
      break;
    case 'retention.set':
      exactKeys(v, ['pageId', 'days']);
      id(v, 'pageId');
      if (v.days !== null) {
        requireValue(typeof v.days === 'number');
        time(v.days);
        requireValue(v.days > 0);
      }
      break;
    case 'page.archive':
    case 'page.delete':
      exactKeys(v, ['pageId']);
      id(v, 'pageId');
      break;
    default:
      throw new Error('Unknown colab-v1 operation');
  }
  // All discriminant-specific types above are checked before exposing the typed value.
  return { operation, value: v } as Payload;
}
