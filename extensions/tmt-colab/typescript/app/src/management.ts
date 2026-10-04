import {
  binary,
  decimal,
  encodeBinary,
  equal,
  exactKeys,
  generatedId,
  idList,
  managementInput,
  payload,
  requireValue,
  sign,
  strictJson,
  text,
} from '@tmt/colab-client';
import type { statement } from '@tmt/colab-client';
import { Admission } from './admission.js';
import { Frames } from './frames.js';
import { discover, type Bootstrap, type PageInfo } from './bootstrap.js';
import { jsonResponse, verifyRegistration, type Registration } from './registration.js';

export interface Recipient {
  id: string;
  role: payload.Role;
  pages: string[];
}
export interface Policy extends PageInfo {
  retentionDays: number | null;
  deleted: boolean;
}
export interface ManagementView {
  page: Policy;
  revision: string;
  ownerMember: string;
  members: Recipient[];
  links: Recipient[];
}
export type Selection =
  | { operation: 'page.share'; value: { pageId: string; mode: PageInfo['sharing'] } }
  | { operation: 'page.history'; value: payload.Values['page.history'] }
  | { operation: 'epoch.advance'; value: { pageId: string } }
  | { operation: 'retention.set'; value: payload.Values['retention.set'] }
  | { operation: 'page.archive' | 'page.delete'; value: { pageId: string } }
  | { operation: 'member.add'; value: payload.Values['member.add'] }
  | { operation: 'member.remove'; value: { memberId: string; pages: string[] } }
  | { operation: 'member.role'; value: { memberId: string; pages: string[]; role: payload.Role } }
  | { operation: 'link.add'; value: LinkSelection }
  | {
      operation: 'link.remove';
      value: { linkId: string; pages: string[]; replacement: LinkSelection | null };
    };
export interface LinkSelection {
  linkId: string;
  role: payload.Role;
  pages: string[];
  seed: string;
}
export interface Pending {
  readonly body: string;
  readonly id: string;
  readonly page: string;
  readonly context: Readonly<Policy>;
  readonly operation: Selection['operation'];
  readonly expiresAt: number;
  readonly expectedRevision: string;
  readonly artifact?: { readonly linkId: string; readonly seed: string };
}
export interface Acknowledgment {
  operationId: string;
  membershipHead: { revision: string; statementHash: string };
}
export interface ManagementPort {
  read(page: string, signal?: AbortSignal): Promise<ManagementView>;
  prepare(view: ManagementView, selection: Selection): Promise<Pending>;
  send(pending: Pending): Promise<Acknowledgment>;
  verify(pending: Pending, ack: Acknowledgment, signal?: AbortSignal): Promise<ManagementView>;
}
export class ManagementError extends Error {
  constructor(readonly code: string) {
    super(code);
  }
}

/** Only an admitted contiguous owner chain supplies policy and assignments. */
export function project(page: PageInfo, log: readonly statement.Verified[]): ManagementView {
  requireValue(log.length > 0);
  const policy: Policy = {
    ...page,
    sharing: 'private',
    history: 'shared',
    archived: false,
    retentionDays: 30,
    deleted: false,
  };
  const members = new Map<string, Recipient>(),
    links = new Map<string, Recipient>();
  for (const { payload: p } of log) {
    switch (p.operation) {
      case 'member.add':
        members.set(p.value.memberId, {
          id: p.value.memberId,
          role: p.value.role,
          pages: [...p.value.pages],
        });
        break;
      case 'member.remove':
        members.delete(p.value.memberId);
        break;
      case 'member.role': {
        const m = members.get(p.value.memberId);
        requireValue(m !== undefined);
        m.role = p.value.role;
        break;
      }
      case 'link.add':
        links.set(p.value.linkId, {
          id: p.value.linkId,
          role: p.value.role,
          pages: [...p.value.pages],
        });
        break;
      case 'link.remove':
        links.delete(p.value.linkId);
        break;
      case 'page.share':
        if (p.value.pageId === page.pageId) {
          policy.sharing = p.value.mode;
          policy.epoch = p.value.epoch;
        }
        break;
      case 'page.history':
        if (p.value.pageId === page.pageId) policy.history = p.value.mode;
        break;
      case 'epoch.advance':
        if (p.value.pageId === page.pageId) policy.epoch = p.value.epoch;
        break;
      case 'retention.set':
        if (p.value.pageId === page.pageId) policy.retentionDays = p.value.days;
        break;
      case 'page.delete':
        if (p.value.pageId === page.pageId) policy.deleted = true;
        break;
      case 'page.archive':
        if (p.value.pageId === page.pageId) policy.archived = true;
        break;
    }
  }
  const head = log[log.length - 1].head;
  return {
    page: policy,
    revision: String(head.revision),
    ownerMember: head.ownerMember.id,
    members: [...members.values()].filter(
      (m) => m.id === head.ownerMember.id || m.pages.includes(page.pageId),
    ),
    links: [...links.values()].filter((m) => m.pages.includes(page.pageId)),
  };
}

/** Stop after membership admission. Content, wraps and baselines never enter a decoder. */
export async function managementLog(
  mount: URL,
  boot: Bootstrap,
  page: PageInfo,
  registration: Registration,
  signal?: AbortSignal,
): Promise<readonly statement.Verified[]> {
  const a = new Admission(boot.space, page.pageId, page.epoch, boot.owner, registration);
  await a.restore();
  return new Promise((resolve, reject) => {
    const url = new URL('sync', mount);
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(url, 'colab-sync-v1');
    let stopped = false,
      started = false,
      queued = 0;
    let tasks = Promise.resolve();
    const finish = (error?: Error) => {
      if (stopped) return;
      stopped = true;
      clearTimeout(timer);
      frames.close();
      signal?.removeEventListener('abort', abort);
      socket.onopen = socket.onmessage = socket.onclose = socket.onerror = null;
      socket.close();
      if (error) reject(error);
      else resolve(a.statements());
    };
    const frames = new Frames(a, finish);
    const abort = () => finish(new ManagementError('UNAVAILABLE'));
    const timer = setTimeout(() => finish(new ManagementError('UNAVAILABLE')), 10_000);
    const send = (type: string, fields: Record<string, unknown>) =>
      socket.send(
        JSON.stringify({
          version: 1,
          type,
          space: boot.space,
          page: page.pageId,
          epoch: page.epoch,
          ...fields,
        }),
      );
    socket.onopen = () => {
      if (socket.protocol !== 'colab-sync-v1') return finish(new ManagementError('INVALID'));
      send('hello', {
        device: registration.deviceId,
        membershipRevision: a.head?.revision.toString() ?? '0',
        cursors: [],
      });
    };
    socket.onmessage = (event) => {
      if (stopped) return;
      if (typeof event.data !== 'string' || text(event.data).length > 64 * 1024 || ++queued > 8)
        return finish(new ManagementError('CAPACITY'));
      const raw = event.data;
      tasks = tasks
        .then(async () => {
          if (stopped) return;
          const frame = frames.receive(raw);
          if (!frame) {
            send('ack', { cursors: [] });
            return;
          }
          const v = frame;
          requireValue(
            frame.version === 1 &&
              frame.space === boot.space &&
              frame.page === page.pageId &&
              frame.epoch === page.epoch,
          );
          if (frame.type === 'error') {
            exactKeys(frame, ['version', 'type', 'space', 'page', 'epoch', 'code']);
            throw new ManagementError(String(frame.code));
          }
          const keys = [
            'version',
            'type',
            'space',
            'page',
            'epoch',
            'streams',
            'more',
            started ? 'membership' : 'membershipHead',
          ];
          if (!started) {
            keys.push('baseline');
            if (Object.hasOwn(frame, 'baselineObject')) keys.push('baselineObject');
          }
          exactKeys(v, keys);
          requireValue(
            v.type === 'catchup' &&
              Array.isArray(v.streams) &&
              v.streams.length === 0 &&
              v.more === true,
          );
          const membership = started ? v.membership : v.membershipHead;
          await a.membership(membership, !started);
          started = true;
          if (stopped) return;
          requireValue(
            membership !== null && typeof membership === 'object' && 'more' in membership,
          );
          if (!membership.more) {
            requireValue(a.head !== null && a.head.revision >= decimal(boot.revision));
            finish();
          } else send('ack', { cursors: [] });
        })
        .catch((error) => finish(error instanceof Error ? error : new ManagementError('INVALID')))
        .finally(() => queued--);
    };
    socket.onclose = socket.onerror = () => finish(new ManagementError('UNAVAILABLE'));
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) abort();
  });
}

export function newLink(role: payload.Role, pages: string[]): LinkSelection {
  const seed = crypto.getRandomValues(new Uint8Array(32));
  try {
    return { linkId: crypto.randomUUID(), role, pages: [...pages], seed: encodeBinary(seed) };
  } finally {
    seed.fill(0);
  }
}
/** Validate selections before the async signing boundary; no owner-computed fields. */
export function selectionBytes(view: ManagementView, selection: Selection): Uint8Array {
  const value = selection.value,
    page = view.page.pageId;
  const keys = {
    'page.share': ['pageId', 'mode'],
    'page.history': ['pageId', 'mode'],
    'epoch.advance': ['pageId'],
    'retention.set': ['pageId', 'days'],
    'page.archive': ['pageId'],
    'page.delete': ['pageId'],
    'member.add': ['memberId', 'role', 'signKey', 'encKey', 'pages'],
    'member.remove': ['memberId', 'pages'],
    'member.role': ['memberId', 'pages', 'role'],
    'link.add': ['linkId', 'role', 'pages', 'seed'],
    'link.remove': ['linkId', 'pages', 'replacement'],
  };
  requireValue(!view.page.deleted);
  exactKeys(value, keys[selection.operation]);
  if ('pageId' in value) requireValue(value.pageId === page);
  if ('pages' in value) {
    idList(value.pages);
    requireValue(value.pages.length > 0 && value.pages.includes(page));
    value.pages.forEach(generatedId);
  }
  switch (selection.operation) {
    case 'retention.set':
      requireValue(
        selection.value.days === null ||
          (Number.isSafeInteger(selection.value.days) && selection.value.days > 0),
      );
      break;
    case 'page.archive':
      requireValue(!view.page.archived);
      break;
    case 'member.add':
      payload.decode(selection.operation, text(JSON.stringify(value)));
      break;
    case 'member.remove':
    case 'member.role': {
      const v = selection.value,
        member = view.members.find((m) => m.id === v.memberId);
      requireValue(
        member !== undefined &&
          member.id !== view.ownerMember &&
          JSON.stringify(member.pages) === JSON.stringify(v.pages),
      );
      if (selection.operation === 'member.role')
        requireValue(['viewer', 'commenter', 'editor'].includes(selection.value.role));
      break;
    }
    case 'link.add':
      validateLink(selection.value);
      break;
    case 'link.remove': {
      const v = selection.value,
        link = view.links.find((m) => m.id === v.linkId);
      requireValue(link !== undefined && JSON.stringify(link.pages) === JSON.stringify(v.pages));
      if (v.replacement) {
        validateLink(v.replacement);
        requireValue(
          v.replacement.linkId !== v.linkId &&
            JSON.stringify(v.replacement.pages) === JSON.stringify(v.pages),
        );
      }
      break;
    }
    case 'page.share':
      requireValue(['private', 'link', 'public'].includes(selection.value.mode));
      break;
    case 'page.history':
      requireValue(['shared', 'current'].includes(selection.value.mode));
      break;
  }
  const bytes = text(JSON.stringify(value));
  requireValue(bytes.length <= 16 * 1024);
  return bytes;
}
function validateLink(value: LinkSelection) {
  exactKeys(value, ['linkId', 'role', 'pages', 'seed']);
  generatedId(value.linkId);
  idList(value.pages);
  value.pages.forEach(generatedId);
  requireValue(['viewer', 'commenter', 'editor'].includes(value.role));
  binary(value.seed, 32, 32);
}

export class ManagementClient implements ManagementPort {
  constructor(
    readonly mount: URL,
    readonly registration: Registration,
  ) {}
  async snapshot(signal?: AbortSignal, context?: { page: string; exclude?: boolean }) {
    const boot = await discover(
      this.mount,
      (space, owner) => verifyRegistration(this.registration, space, owner),
      signal,
    );
    const page = context?.exclude
      ? boot.pages.find((p) => p.pageId !== context.page)
      : (boot.pages.find((p) => p.pageId === context?.page) ?? boot.pages[0]);
    const log = page ? await managementLog(this.mount, boot, page, this.registration, signal) : [];
    return { boot, log };
  }
  async read(id: string, signal?: AbortSignal): Promise<ManagementView> {
    const { boot, log } = await this.snapshot(signal, { page: id });
    const page = boot.pages.find((p) => p.pageId === id);
    if (!page || !log.length) throw new ManagementError('DENIED');
    const view = project(page, log);
    requireValue(!view.page.deleted && view.page.epoch === page.epoch);
    return view;
  }
  async prepare(view: ManagementView, selection: Selection): Promise<Pending> {
    const selected = structuredClone(selection),
      context = Object.freeze({ ...view.page }),
      expectedRevision = view.revision;
    const bytes = selectionBytes(view, selected),
      id = crypto.randomUUID(),
      issuedAt = Date.now(),
      cert = this.registration.chain.certificate(),
      expiresAt = Math.min(issuedAt + 600000, cert.expiresAt);
    requireValue(cert.issuedAt <= issuedAt && cert.expiresAt > issuedAt);
    const request = await managementInput({
      space: cert.space,
      page: context.pageId,
      expectedRevision,
      operationId: id,
      operation: selected.operation,
      payload: bytes,
      senderDevice: this.registration.deviceId,
      issuedAt,
      expiresAt,
    });
    const signature = await sign(this.registration.keys.sign, request);
    const link =
      selected.operation === 'link.add'
        ? selected.value
        : selected.operation === 'link.remove'
          ? selected.value.replacement
          : null;
    return Object.freeze({
      id,
      page: context.pageId,
      context,
      operation: selected.operation,
      expiresAt,
      expectedRevision,
      body: JSON.stringify({
        request: encodeBinary(request),
        payload: encodeBinary(bytes),
        signature: encodeBinary(signature),
      }),
      ...(link ? { artifact: Object.freeze({ linkId: link.linkId, seed: link.seed }) } : {}),
    });
  }
  async send(pending: Pending): Promise<Acknowledgment> {
    if (Date.now() >= pending.expiresAt) throw new ManagementError('EXPIRED');
    let response: Response;
    try {
      response = await fetch(new URL('api/management', this.mount), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: pending.body,
        signal: AbortSignal.timeout(10_000),
      });
    } catch {
      throw new ManagementError('UNKNOWN');
    }
    // Parse bounded error bodies as well; never expose server text or request secrets.
    let v;
    try {
      v = await jsonResponse(new Response(response.body, { status: 200 }), 8192);
    } catch {
      throw new ManagementError('UNKNOWN');
    }
    if (!response.ok) {
      let code: string;
      try {
        exactKeys(v, ['code']);
        const status = {
          INVALID: 400,
          DENIED: 403,
          EXPIRED: 403,
          CONFLICT: 409,
          STALE_HEAD: 409,
          CAPACITY: 503,
          UNAVAILABLE: 503,
        };
        requireValue(
          typeof v.code === 'string' &&
            Object.hasOwn(status, v.code) &&
            status[v.code as keyof typeof status] === response.status,
        );
        code = v.code;
      } catch {
        throw new ManagementError('UNKNOWN');
      }
      throw new ManagementError(code);
    }
    try {
      exactKeys(v, ['operationId', 'membershipHead']);
      exactKeys(v.membershipHead, ['revision', 'statementHash']);
      requireValue(v.operationId === pending.id && typeof v.membershipHead.revision === 'string');
      decimal(v.membershipHead.revision);
      binary(v.membershipHead.statementHash, 32, 32);
    } catch {
      throw new ManagementError('UNKNOWN');
    }
    return v as unknown as Acknowledgment;
  }
  async verify(
    pending: Pending,
    ack: Acknowledgment,
    signal?: AbortSignal,
  ): Promise<ManagementView> {
    requireValue(ack.operationId === pending.id);
    const deleting = pending.operation === 'page.delete';
    const { boot, log } = await this.snapshot(signal, { page: pending.page, exclude: deleting });
    if (deleting && !log.length) throw new ManagementError('AWAITING_VERIFICATION');
    requireValue(log.length > 0);
    const committed = log.find((v) => v.header.revision === ack.membershipHead.revision);
    requireValue(
      committed !== undefined &&
        equal(committed.head.hash, binary(ack.membershipHead.statementHash, 32, 32)),
    );
    const request = strictJson(
      binary((JSON.parse(pending.body) as { payload: string }).payload, 16384),
      16384,
      true,
    );
    requireValue(request !== null && typeof request === 'object');
    const selected = request as Record<string, unknown>;
    const changes = log.filter(
      (v) =>
        v.head.revision > decimal(pending.expectedRevision) &&
        v.head.revision <= decimal(ack.membershipHead.revision),
    );
    requireValue(
      changes.some(
        (v) =>
          v.payload.operation === pending.operation &&
          Object.entries(selected).every(
            ([k, value]) =>
              ['seed', 'replacement'].includes(k) ||
              (k === 'pages' && !('pages' in v.payload.value)) ||
              JSON.stringify((v.payload.value as unknown as Record<string, unknown>)[k]) ===
                JSON.stringify(value),
          ),
      ),
    );
    if (pending.artifact)
      requireValue(
        changes.some(
          (v) =>
            v.payload.operation === 'link.add' &&
            v.payload.value.linkId === pending.artifact!.linkId,
        ),
      );
    const page = boot.pages.find((p) => p.pageId === pending.page);
    if (deleting) {
      requireValue(page === undefined);
      const view = project(pending.context, log);
      requireValue(view.page.deleted);
      return view;
    }
    requireValue(page !== undefined);
    const view = project(page, log);
    requireValue(boot.pages.some((p) => p.pageId === pending.page && p.epoch === view.page.epoch));
    return view;
  }
}
