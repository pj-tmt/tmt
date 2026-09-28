import { createContext } from 'react';
import { defaultCatalog } from '../blocks/block-contract.js';
import type { Furniture } from '../blocks/block-contract.js';
import { BUILTIN_CATALOG, decodePropPack } from '../props/prop-contract.js';
import type { CatalogPack } from '../props/prop-contract.js';
import { decodeAvatarPack } from '../avatars/avatar-contract.js';
import type { CatalogAvatarPack } from '../avatars/avatar-contract.js';
import { decodeAvatarCatalog } from '../avatars/avatar-catalog.js';
import type { AvatarCatalog } from '../avatars/avatar-catalog.js';
import {
  decodeBoardList,
  decodeBoardShow,
  decodeCategoryPage,
  decodeDeleteReceipt,
  decodeEditReceipt,
  decodePostReceipt,
  decodeReplyReceipt,
} from './board-contract.js';
import type {
  BoardCategory,
  BoardCategoryPage,
  BoardCreateReceipt,
  BoardDeleteReceipt,
  BoardEditReceipt,
  BoardListPage,
  BoardShowPage,
} from './board-contract.js';
import {
  decodeProfileProjection,
  decodeProfileMutation,
  decodeProfileSnapshot,
  ProfileConflict,
  ProfileAvatarUnavailable,
} from '../profiles/profile-contract.js';
import type { Profile, ProfilePort } from '../profiles/profile-contract.js';
import {
  decodeWhiteboardDocument,
  decodeWhiteboardReceipt,
  decodeWhiteboardSave,
  whiteboardId,
} from '../whiteboard/document-contract.js';
import type { WhiteboardPort } from '../whiteboard/document-contract.js';
import { uuidIdentifier, canonicalUuid } from '../contracts/record.js';
import { readBoundedBody } from '../transport/response-body.js';
import {
  checkSnapshotImage,
  decodeWhiteboardCapture,
  decodeWhiteboardSnapshot,
  SNAPSHOT_PNG_LIMIT,
} from '../whiteboard/snapshot-contract.js';
import type { WhiteboardSnapshotPort } from '../whiteboard/snapshot-contract.js';
import {
  decodeDispatchInput,
  decodeDispatchReceipt,
  RoomRosterChanged,
  DispatchRejected,
  requestIdentifier,
  matchDispatchReceipt,
} from './dispatch-contract.js';
import type { DispatchPort } from './dispatch-contract.js';
import { decodeRoom, decodeRoomList, decodeRoomWrite } from './room-contract.js';
import type { RoomPort } from './room-contract.js';
import {
  decodeHistoryQuery,
  decodeHistoryPage,
  decodeHistoryDetail,
  REQUEST_HISTORY_PAGE_BYTES,
  REQUEST_DETAIL_BYTES,
} from './request-history-contract.js';
import type { RequestHistoryPort } from './request-history-contract.js';
import {
  decodeWorldSnapshot,
  decodeWorldWrite,
  decodeWorldValidationError,
  WorldConflict,
  WORLD_ENVELOPE_BYTES,
} from '../world-map/world-port.js';
import type { WorldPort } from '../world-map/world-port.js';
import { sameWorld } from '../world-map/world-contract.js';
import { decodeNotebook, NOTEBOOK_ENVELOPE_BYTES } from '../notebooks/notebook-contract.js';
import type { NotebookPort } from '../notebooks/notebook-contract.js';
import {
  decodePropCatalogPage,
  decodePropInstallReceipt,
  propCatalogCursor,
  propDocumentDigest,
  PROP_CATALOG_PAGE_BYTES,
  PROP_FILE_BYTES,
} from '../props/prop-catalog-contract.js';
import type { PropCatalogPort } from '../props/prop-catalog-contract.js';
import { validImmutableArtDigest } from '../rendering/immutable-art-reference.js';

export type BoardAuthorFilter = { kind: 'owner' } | { kind: 'identity'; identityId: string };

export interface LocalRuntime {
  world: WorldPort;
  profiles: ProfilePort;
  avatars: { list(signal?: AbortSignal): Promise<AvatarCatalog> };
  board: LocalBoardPort;
  whiteboards: WhiteboardPort;
  notebooks: NotebookPort;
  propCatalog: PropCatalogPort;
  whiteboardSnapshots: WhiteboardSnapshotPort;
  dispatch: DispatchPort;
  requests: RequestHistoryPort;
  rooms: RoomPort;
  resolveProps(objects: Furniture[], signal?: AbortSignal): Promise<CatalogPack[]>;
  preview(previewId: string): Promise<CatalogPack>;
  avatarPreview(previewId: string): Promise<CatalogAvatarPack>;
  dispose(): void;
}

export interface BoardListInput {
  category: BoardCategory;
  view?: 'recent' | 'updated';
  author?: BoardAuthorFilter;
  sinceMs?: number;
  limit?: number;
  cursor?: string;
}

export interface LocalBoardPort {
  categories(input?: { limit?: number; cursor?: string }): Promise<BoardCategoryPage>;
  list(input: BoardListInput): Promise<BoardListPage>;
  show(input: {
    threadId: string;
    replyLimit?: number;
    replyCursor?: string;
  }): Promise<BoardShowPage>;
  post(input: {
    category: BoardCategory;
    title: string;
    body: string;
    operationId: string;
  }): Promise<BoardCreateReceipt>;
  reply(input: {
    threadId: string;
    body: string;
    operationId: string;
  }): Promise<BoardCreateReceipt>;
  edit(input: {
    entryId: string;
    title?: string;
    body?: string;
    ifRevision: number;
    operationId: string;
  }): Promise<BoardEditReceipt>;
  delete(input: {
    entryId: string;
    ifRevision: number;
    moderate: boolean;
    operationId: string;
  }): Promise<BoardDeleteReceipt>;
}

export const LocalRuntimeContext = createContext<LocalRuntime | undefined>(undefined);

export class LocalHttpError extends Error {
  constructor(
    readonly status: number,
    readonly code?: string
  ) {
    super(
      code ? `Local Office request failed (${code}).` : `Local Office request failed (${status}).`
    );
  }
}

export function startLocalRuntime(location: Location): LocalRuntime {
  const token = new URLSearchParams(location.hash.slice(1)).get('token');
  history.replaceState(null, '', `${location.pathname}${location.search}`);
  if (!token || !/^[A-Za-z0-9_-]{43}$/.test(token)) throw new Error('Office session is missing.');
  const controllers = new Set<AbortController>();
  let disposed = false;
  async function request<T>(
    path: string,
    init: RequestInit | undefined,
    consume: (response: Response) => Promise<T>,
    lifetime?: AbortSignal,
    contentType = 'application/json'
  ): Promise<T> {
    const controller = new AbortController();
    const abort = () => controller.abort();
    if (lifetime?.aborted || disposed) controller.abort();
    else lifetime?.addEventListener('abort', abort, { once: true });
    const timeout = window.setTimeout(abort, 5_000);
    controllers.add(controller);
    try {
      const response = await fetch(path, {
        ...init,
        signal: controller.signal,
        headers: {
          Authorization: `Bearer ${token}`,
          ...(init?.body ? { 'Content-Type': contentType } : {}),
        },
      });
      return await consume(response);
    } finally {
      window.clearTimeout(timeout);
      lifetime?.removeEventListener('abort', abort);
      controllers.delete(controller);
    }
  }
  function jsonRequest(
    path: string,
    init?: RequestInit,
    lifetime?: AbortSignal,
    maxBytes?: number
  ) {
    return request(
      path,
      init,
      async (response) => {
        const value: unknown =
          maxBytes === undefined
            ? await response.json()
            : JSON.parse(
                new TextDecoder('utf-8', { fatal: true }).decode(
                  await readBoundedBody(response, maxBytes)
                )
              );
        return { response, value };
      },
      lifetime
    );
  }
  function snapshotImageRequest(id: string, body?: Blob, lifetime?: AbortSignal) {
    const path = `/api/v1/local/whiteboard-snapshots/${uuidIdentifier(id)}/image`;
    if (body) checkSnapshotImage(body);
    return request(
      path,
      body ? { method: 'PUT', body } : undefined,
      async (response) => {
        if (!response.ok) checked(response, await response.json(), () => undefined);
        if (response.headers.get('Content-Type')?.split(';')[0]?.trim() !== 'image/png') {
          await response.body?.cancel();
          throw new Error('Unexpected snapshot image response.');
        }
        const bytes = await readBoundedBody(response, SNAPSHOT_PNG_LIMIT);
        return checkSnapshotImage(new Blob([bytes], { type: 'image/png' }));
      },
      lifetime,
      'image/png'
    );
  }
  function checked<T>(response: Response, value: unknown, decode: (value: unknown) => T): T {
    if (!response.ok) {
      const code =
        value && typeof value === 'object' && !Array.isArray(value)
          ? (value as Record<string, unknown>).error
          : undefined;
      throw new LocalHttpError(response.status, typeof code === 'string' ? code : undefined);
    }
    return decode(value);
  }
  async function jsonMutation<T>(
    path: string,
    method: 'POST' | 'PUT' | 'DELETE',
    body: unknown,
    decode: (value: unknown) => T,
    lifetime?: AbortSignal,
    maxBytes?: number
  ): Promise<T> {
    const { response, value } = await jsonRequest(
      path,
      {
        method,
        body: JSON.stringify(body),
      },
      lifetime,
      maxBytes
    );
    return checked(response, value, decode);
  }
  async function catalogFor(objects: Furniture[], lifetime?: AbortSignal): Promise<CatalogPack[]> {
    const digests = Array.from(
      new Set(
        objects
          .map((item) => item.prop.split('/')[0]!)
          .filter((digest) => !BUILTIN_CATALOG.some((entry) => entry.digest === digest))
      )
    );
    const catalog = defaultCatalog();
    // The endpoint accepts at most 16 unique digests per request. One overview
    // shares these resolutions across rooms rather than starting room listeners.
    for (let offset = 0; offset < digests.length; offset += 16) {
      catalog.push(...(await resolveDigests(digests.slice(offset, offset + 16), lifetime)));
    }
    return catalog;
  }
  async function resolveDigests(digests: string[], lifetime?: AbortSignal): Promise<CatalogPack[]> {
    const { response, value } = await jsonRequest(
      '/api/v1/local/props/resolve',
      { method: 'POST', body: JSON.stringify({ digests }) },
      lifetime,
      PROP_FILE_BYTES * 6 * digests.length + 16 * 1024
    );
    return checked(response, value, (candidate) => {
      if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate))
        throw new Error('Invalid prop catalog.');
      const record = candidate as Record<string, unknown>;
      if (
        Object.keys(record).sort().join(',') !== 'catalogRevision,packs,unavailable' ||
        !Number.isSafeInteger(record.catalogRevision) ||
        Number(record.catalogRevision) < 0 ||
        !Array.isArray(record.packs) ||
        !Array.isArray(record.unavailable)
      )
        throw new Error('Invalid prop catalog.');
      const packs = record.packs.map((entry) => {
        if (!entry || typeof entry !== 'object' || Array.isArray(entry))
          throw new Error('Invalid prop catalog entry.');
        const item = entry as Record<string, unknown>;
        if (
          Object.keys(item).sort().join(',') !== 'digest,pack' ||
          typeof item.digest !== 'string' ||
          !validImmutableArtDigest(item.digest)
        )
          throw new Error('Invalid prop catalog entry.');
        return { digest: item.digest, pack: decodePropPack(item.pack) };
      });
      if (!record.unavailable.every((digest): digest is string => validImmutableArtDigest(digest)))
        throw new Error('Invalid unavailable prop catalog entry.');
      const returned = [...packs.map((pack) => pack.digest), ...record.unavailable];
      if (
        new Set(returned).size !== returned.length ||
        returned.length !== digests.length ||
        returned.some((digest) => !digests.includes(digest))
      )
        throw new Error('Invalid prop catalog resolution.');
      return packs;
    });
  }
  const board: LocalBoardPort = {
    categories(input = {}) {
      return jsonMutation('/api/v1/local/board/categories/list', 'POST', input, decodeCategoryPage);
    },
    list(input) {
      return jsonMutation('/api/v1/local/board/threads/list', 'POST', input, decodeBoardList);
    },
    show(input) {
      return jsonMutation('/api/v1/local/board/threads/show', 'POST', input, decodeBoardShow);
    },
    post(input) {
      return jsonMutation('/api/v1/local/board/threads', 'POST', input, decodePostReceipt);
    },
    reply({ threadId, ...input }) {
      return jsonMutation(
        `/api/v1/local/board/threads/${encodeURIComponent(threadId)}/replies`,
        'POST',
        input,
        decodeReplyReceipt
      );
    },
    edit({ entryId, ...input }) {
      return jsonMutation(
        `/api/v1/local/board/entries/${encodeURIComponent(entryId)}`,
        'PUT',
        input,
        decodeEditReceipt
      );
    },
    delete({ entryId, ...input }) {
      return jsonMutation(
        `/api/v1/local/board/entries/${encodeURIComponent(entryId)}`,
        'DELETE',
        input,
        decodeDeleteReceipt
      );
    },
  };
  const propCatalog: PropCatalogPort = {
    list(cursor, signal) {
      return jsonMutation(
        '/api/v1/local/props/list',
        'POST',
        cursor === undefined ? {} : { cursor: propCatalogCursor(cursor) },
        decodePropCatalogPage,
        signal,
        PROP_CATALOG_PAGE_BYTES
      );
    },
    async load(digest, signal) {
      if (!validImmutableArtDigest(digest)) throw new Error('Invalid prop digest.');
      const packs = await resolveDigests([digest], signal);
      if (packs.length !== 1) throw new Error('Artwork is missing or unavailable.');
      return packs[0]!;
    },
    async install(input, signal) {
      // Snapshot caller-owned values before asynchronous hashing; retries supply this same intent.
      const intent = { expectedRevision: input.expectedRevision, document: input.document };
      const digest = await propDocumentDigest(intent);
      const receipt = await jsonMutation(
        '/api/v1/local/props/install',
        'POST',
        intent,
        decodePropInstallReceipt,
        signal,
        4096
      );
      if (
        receipt.digest !== digest ||
        (receipt.changed
          ? receipt.revision !== intent.expectedRevision + 1
          : receipt.revision !== intent.expectedRevision &&
            receipt.revision !== intent.expectedRevision + 1)
      )
        throw new Error('Unexpected prop install receipt.');
      return receipt;
    },
  };
  const notebooks: NotebookPort = {
    async read(identityId, signal) {
      const { response, value } = await jsonRequest(
        `/api/v1/local/notebooks/${canonicalUuid(identityId)}`,
        undefined,
        signal,
        NOTEBOOK_ENVELOPE_BYTES
      );
      const notebook = checked(response, value, decodeNotebook);
      if (notebook.identityId !== identityId) throw new Error('Unexpected notebook identity.');
      return notebook;
    },
  };
  const whiteboards: WhiteboardPort = {
    async show(id, signal) {
      const { response, value } = await jsonRequest(
        `/api/v1/local/whiteboards/${whiteboardId(id)}`,
        undefined,
        signal
      );
      const document = checked(response, value, decodeWhiteboardDocument);
      if (document.id !== id) throw new Error('Unexpected whiteboard document.');
      return document;
    },
    async save(id, input, signal) {
      const path = `/api/v1/local/whiteboards/${whiteboardId(id)}`;
      const intent = decodeWhiteboardSave(input);
      const receipt = await jsonMutation(path, 'PUT', intent, decodeWhiteboardReceipt, signal);
      if (
        receipt.documentId !== id ||
        receipt.operationId !== intent.operationId ||
        receipt.revision !== intent.expectedRevision + (receipt.changed ? 1 : 0)
      )
        throw new Error('Unexpected whiteboard receipt.');
      return receipt;
    },
  };
  const whiteboardSnapshots: WhiteboardSnapshotPort = {
    async capture(documentId, input, signal) {
      const id = whiteboardId(documentId);
      const intent = decodeWhiteboardCapture(input);
      const captured = await jsonMutation(
        `/api/v1/local/whiteboards/${id}/snapshots`,
        'POST',
        intent,
        decodeWhiteboardSnapshot,
        signal
      );
      if (
        captured.id !== intent.operationId ||
        captured.documentId !== id ||
        captured.documentRevision !== intent.expectedRevision ||
        captured.annotation !== intent.annotation ||
        JSON.stringify(captured.selectedElementIds) !== JSON.stringify(intent.selectedElementIds)
      )
        throw new Error('Unexpected whiteboard snapshot.');
      return captured;
    },
    async show(id, signal) {
      const { response, value } = await jsonRequest(
        `/api/v1/local/whiteboard-snapshots/${uuidIdentifier(id)}`,
        undefined,
        signal
      );
      const captured = checked(response, value, decodeWhiteboardSnapshot);
      if (captured.id !== id) throw new Error('Unexpected whiteboard snapshot.');
      return captured;
    },
    async attachImage(id, image, signal) {
      return snapshotImageRequest(id, image, signal);
    },
    async image(id, signal) {
      return snapshotImageRequest(id, undefined, signal);
    },
  };
  const dispatch: DispatchPort = {
    async send(input, signal) {
      const intent = decodeDispatchInput(input);
      const receipt = await jsonMutation(
        '/api/v1/local/dispatch',
        'POST',
        intent,
        decodeDispatchReceipt,
        signal
      ).catch((error: unknown) => {
        if (
          error instanceof LocalHttpError &&
          error.status === 409 &&
          error.code === 'ROOM_ROSTER_CHANGED'
        )
          throw new RoomRosterChanged();
        if (error instanceof LocalHttpError) {
          if (error.status === 409 && error.code === 'ROOM_RECIPIENT_NOT_MEMBER')
            throw new DispatchRejected('roomMembership');
          if (error.status === 400 && error.code === 'DISPATCH_INVALID')
            throw new DispatchRejected('invalid');
          if (error.status === 409 && error.code === 'DISPATCH_IDEMPOTENCY_CONFLICT')
            throw new DispatchRejected('operationConflict');
        }
        throw error;
      });
      return matchDispatchReceipt(receipt, intent);
    },
  };
  const requests: RequestHistoryPort = {
    async list(query, signal) {
      const intent = decodeHistoryQuery(query);
      return jsonMutation(
        '/api/v1/local/requests/list',
        'POST',
        intent,
        (value) => decodeHistoryPage(value, intent),
        signal,
        REQUEST_HISTORY_PAGE_BYTES
      );
    },
    async show(id, signal) {
      const requestId = requestIdentifier(id);
      const detail = await jsonMutation(
        '/api/v1/local/requests/show',
        'POST',
        { requestId },
        decodeHistoryDetail,
        signal,
        REQUEST_DETAIL_BYTES
      );
      if (detail.requestId !== requestId) throw new Error('Unexpected request detail.');
      return detail;
    },
    async receipt(operationId, signal) {
      canonicalUuid(operationId);
      try {
        const receipt = await jsonMutation(
          '/api/v1/local/dispatch/show',
          'POST',
          { operationId },
          decodeDispatchReceipt,
          signal,
          16_384
        );
        if (receipt.operationId !== operationId) throw new Error('Unexpected dispatch receipt.');
        return receipt;
      } catch (error) {
        if (
          error instanceof LocalHttpError &&
          error.status === 404 &&
          error.code === 'DISPATCH_NOT_FOUND'
        )
          return null;
        throw error;
      }
    },
  };
  const rooms: RoomPort = {
    async list(signal) {
      const { response, value } = await jsonRequest('/api/v1/local/rooms', undefined, signal);
      return checked(response, value, decodeRoomList);
    },
    async save(id, input, signal) {
      const path = `/api/v1/local/rooms/${canonicalUuid(id)}`;
      const intent = decodeRoomWrite(input);
      const room = await jsonMutation(path, 'PUT', intent, decodeRoom, signal);
      if (
        room.id !== id ||
        room.retired ||
        room.name !== intent.name ||
        JSON.stringify(room.memberIds) !== JSON.stringify(intent.memberIds) ||
        (room.revision !== intent.expectedRevision && room.revision !== intent.expectedRevision + 1)
      )
        throw new Error('Unexpected room receipt.');
      return room;
    },
    async retire(id, expectedRevision, signal) {
      if (
        !Number.isSafeInteger(expectedRevision) ||
        expectedRevision < 1 ||
        expectedRevision > Number.MAX_SAFE_INTEGER
      )
        throw new Error('Invalid room revision.');
      const room = await jsonMutation(
        `/api/v1/local/rooms/${canonicalUuid(id)}/retire`,
        'POST',
        { expectedRevision },
        decodeRoom,
        signal
      );
      if (
        room.id !== id ||
        !room.retired ||
        (room.revision !== expectedRevision && room.revision !== expectedRevision + 1)
      )
        throw new Error('Unexpected room retirement receipt.');
      return room;
    },
  };
  const world: WorldPort = {
    async show(signal) {
      const { response, value } = await jsonRequest(
        '/api/v1/local/world',
        undefined,
        signal,
        WORLD_ENVELOPE_BYTES
      );
      return checked(response, value, decodeWorldSnapshot);
    },
    async save(input, signal) {
      const intent = decodeWorldWrite(input);
      const { response, value } = await jsonRequest(
        '/api/v1/local/world',
        { method: 'PUT', body: JSON.stringify(intent) },
        signal,
        WORLD_ENVELOPE_BYTES
      );
      if (
        response.status === 409 &&
        value &&
        typeof value === 'object' &&
        'error' in value &&
        value.error === 'WORLD_REVISION_CONFLICT'
      )
        throw new WorldConflict();
      if (
        response.status === 400 &&
        value &&
        typeof value === 'object' &&
        'error' in value &&
        value.error === 'WORLD_INVALID' &&
        'issues' in value
      )
        throw decodeWorldValidationError(value);
      const result = checked(response, value, decodeWorldSnapshot);
      if (
        result.revision !== intent.expectedRevision + Number(result.changed) ||
        result.legacyBasis !== null ||
        !sameWorld(result.layout, intent.layout)
      )
        throw new Error('Unexpected Office world save result.');
      return result;
    },
  };
  const profiles: ProfilePort = {
    async list(signal) {
      const { response, value } = await jsonRequest('/api/v1/local/profiles', undefined, signal);
      if (!response.ok) throw new LocalHttpError(response.status);
      if (!Array.isArray(value)) throw new Error('Invalid local profile projection.');
      return value.map(decodeProfileProjection);
    },
    async show(identityId) {
      const { response, value } = await jsonRequest(
        `/api/v1/local/profiles/${encodeURIComponent(identityId)}`
      );
      return checked(response, value, decodeProfileSnapshot);
    },
    async apply(identityId: string, expectedRevision: number, profile: Profile) {
      const { response, value } = await jsonRequest(
        `/api/v1/local/profiles/${encodeURIComponent(identityId)}`,
        {
          method: 'PUT',
          body: JSON.stringify({ expectedRevision, profile }),
        }
      );
      if (response.status === 409 && (value as { error?: unknown })?.error === 'AVATAR_UNAVAILABLE')
        throw new ProfileAvatarUnavailable();
      if (response.status === 409) throw new ProfileConflict();
      return checked(response, value, decodeProfileMutation);
    },
  };
  const avatars = {
    async list(signal?: AbortSignal) {
      const { response, value } = await jsonRequest(
        '/api/v1/local/avatar-catalog',
        undefined,
        signal
      );
      return checked(response, value, decodeAvatarCatalog);
    },
  };
  return {
    world,
    board,
    whiteboards,
    notebooks,
    propCatalog,
    whiteboardSnapshots,
    dispatch,
    requests,
    rooms,
    profiles,
    avatars,
    resolveProps: catalogFor,
    async preview(previewId) {
      if (!/^[A-Za-z0-9_-]{43}$/.test(previewId)) throw new Error('Invalid prop preview ID.');
      const { response, value } = await jsonRequest(
        `/api/v1/local/prop-previews/${encodeURIComponent(previewId)}`
      );
      return checked(response, value, (candidate) => {
        if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate))
          throw new Error('Invalid prop preview.');
        const record = candidate as Record<string, unknown>;
        if (
          Object.keys(record).sort().join(',') !== 'digest,pack' ||
          typeof record.digest !== 'string' ||
          !/^sha256:[0-9a-f]{64}$/.test(record.digest)
        )
          throw new Error('Invalid prop preview.');
        return { digest: record.digest, pack: decodePropPack(record.pack) };
      });
    },
    async avatarPreview(previewId) {
      if (!/^[A-Za-z0-9_-]{43}$/.test(previewId)) throw new Error('Invalid avatar preview ID.');
      const { response, value } = await jsonRequest(
        `/api/v1/local/avatar-previews/${encodeURIComponent(previewId)}`
      );
      return checked(response, value, (candidate) => {
        if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate))
          throw new Error('Invalid avatar preview.');
        const record = candidate as Record<string, unknown>;
        if (
          Object.keys(record).sort().join(',') !== 'digest,pack' ||
          typeof record.digest !== 'string' ||
          !/^sha256:[0-9a-f]{64}$/.test(record.digest)
        )
          throw new Error('Invalid avatar preview.');
        return { digest: record.digest, pack: decodeAvatarPack(record.pack) };
      });
    },
    dispose() {
      disposed = true;
      for (const controller of controllers) controller.abort();
      controllers.clear();
    },
  };
}
