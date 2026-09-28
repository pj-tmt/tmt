import {
  decodeDispatchInput,
  DispatchRejected,
  RoomRosterChanged,
  matchDispatchReceipt,
} from './dispatch-contract.js';
import type {
  DispatchInput,
  DispatchPort,
  DispatchReceipt,
  DispatchRoom,
} from './dispatch-contract.js';
import type { MeetingRoom } from './room-contract.js';
import type { IdentityChoice } from '../profiles/identity-choice.js';
import type { DispatchJournal } from './dispatch-journal.js';
interface SendPreview {
  text: string;
  recipients: IdentityChoice[];
  room?: MeetingRoom;
  roomStale?: boolean;
  review?: DispatchInput;
  busy: boolean;
  attempted: boolean;
  rejected?: boolean;
  receipt?: DispatchReceipt;
  error?: string;
  recoveryBlocked?: boolean;
}

interface DispatchRecovery {
  journal: DispatchJournal;
  lookup(operationId: string, signal?: AbortSignal): Promise<DispatchReceipt | null>;
}

/** A mounted composition owns one frozen audience and one operation across uncertain retries. */
export function createDispatchComposerState(
  port: DispatchPort,
  composition: {
    kind: 'request' | 'announcement';
    message(text: string): string;
    recipients?: IdentityChoice[];
    context?: Extract<DispatchRoom, { kind: 'direct' }>;
  },
  operationId: () => string = () => crypto.randomUUID(),
  recovery?: DispatchRecovery
) {
  let snapshot: SendPreview = empty();
  const listeners = new Set<() => void>();
  const lifetime = new AbortController();
  let disposed = false;
  function empty(): SendPreview {
    return {
      text: '',
      recipients: composition.recipients?.map((item) => ({ ...item })) ?? [],
      busy: false,
      attempted: false,
    };
  }
  try {
    const pending = recovery?.journal.read();
    if (pending)
      snapshot = {
        ...snapshot,
        text: pending.message,
        review: pending,
        attempted: true,
        recipients: pending.recipientIds.map((id) => ({
          id,
          name: composition.recipients?.find((item) => item.id === id)?.name ?? id,
        })),
        error: 'An earlier send was not confirmed. Check its receipt or retry the same request.',
      };
  } catch {
    snapshot = {
      ...snapshot,
      recoveryBlocked: true,
      error:
        'Pending request recovery is unavailable. Nothing new was sent. Discard the local pending record explicitly before composing again.',
    };
  }
  function publish(next: SendPreview) {
    if (disposed) return;
    snapshot = next;
    for (const listener of listeners) listener();
  }
  function accepted(receipt: DispatchReceipt, input: DispatchInput) {
    if (disposed) return;
    matchDispatchReceipt(receipt, input);
    let error: string | undefined;
    try {
      recovery?.journal.clear(input.operationId);
    } catch {
      error =
        'Request accepted, but the local pending record could not be cleared. Reopening will check the same receipt.';
    }
    publish({ ...snapshot, busy: false, receipt, error });
  }
  return {
    kind: composition.kind,
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    change(text: string, recipients: IdentityChoice[]) {
      if (disposed || snapshot.review || snapshot.recoveryBlocked) return;
      publish({
        ...snapshot,
        text,
        recipients: recipients.map((item) => ({ ...item })),
        error: snapshot.roomStale ? snapshot.error : undefined,
      });
    },
    chooseRoom(room: MeetingRoom | undefined, recipients: IdentityChoice[]) {
      if (composition.context)
        throw new Error('A direct conversation cannot adopt a room-wide audience.');
      if (disposed || snapshot.review || snapshot.recoveryBlocked) return;
      publish({
        ...snapshot,
        room: room ? { ...room, memberIds: [...room.memberIds] } : undefined,
        recipients: recipients.map((item) => ({ ...item })),
        roomStale: false,
        error: undefined,
      });
    },
    review() {
      if (disposed || snapshot.review || snapshot.recoveryBlocked) return;
      try {
        if (!snapshot.text.trim()) throw new Error('Add a message before reviewing.');
        if (snapshot.roomStale) throw new RoomRosterChanged();
        const input = decodeDispatchInput({
          kind: composition.kind,
          operationId: operationId(),
          recipientIds: snapshot.recipients.map((item) => item.id),
          message: composition.message(snapshot.text),
          ...(snapshot.room
            ? {
                room: {
                  kind: 'roster',
                  roomId: snapshot.room.id,
                  revision: snapshot.room.revision,
                },
              }
            : composition.context
              ? { room: composition.context }
              : {}),
        });
        if (
          snapshot.room &&
          JSON.stringify(input.recipientIds) !== JSON.stringify(snapshot.room.memberIds)
        )
          throw new Error('Use the complete room roster before reviewing.');
        const names = new Map(snapshot.recipients.map((item) => [item.id, item.name]));
        publish({
          ...snapshot,
          recipients: input.recipientIds.map((id) => ({ id, name: names.get(id)! })),
          review: input,
          error: undefined,
        });
      } catch (error) {
        publish({
          ...snapshot,
          error: error instanceof Error ? error.message : 'Invalid message.',
        });
      }
    },
    edit() {
      if (disposed || snapshot.attempted) return;
      publish({ ...snapshot, review: undefined, error: undefined });
    },
    async send() {
      if (
        disposed ||
        !snapshot.review ||
        snapshot.busy ||
        snapshot.receipt ||
        snapshot.rejected ||
        snapshot.recoveryBlocked
      )
        return;
      const input = snapshot.review;
      try {
        recovery?.journal.save(input);
      } catch {
        publish({
          ...snapshot,
          error:
            'Could not preserve this pending request in the browser tab. No new send was attempted. Free browser storage or retry.',
        });
        return;
      }
      publish({ ...snapshot, busy: true, attempted: true, error: undefined });
      try {
        const receipt = await port.send(input, lifetime.signal);
        accepted(receipt, input);
      } catch (error) {
        if (error instanceof DispatchRejected) {
          // Keep earlier uncertainty intact: a later rejection cannot cancel queued work.
          publish({ ...snapshot, busy: false, rejected: true, error: error.message });
          return;
        }
        if (error instanceof RoomRosterChanged) {
          let recoveryBlocked = false;
          try {
            recovery?.journal.clear(input.operationId);
          } catch {
            recoveryBlocked = true;
          }
          publish({
            ...snapshot,
            busy: false,
            attempted: false,
            review: undefined,
            roomStale: true,
            recoveryBlocked,
            error: recoveryBlocked
              ? 'The room changed, but its pending record could not be cleared. Discard that local record before composing again.'
              : error.message,
          });
          return;
        }
        publish({
          ...snapshot,
          busy: false,
          error:
            'Send was not confirmed. It may already be queued. Retry keeps the same recipients, message and operation ID.',
        });
      }
    },
    async recover() {
      if (
        !recovery ||
        disposed ||
        !snapshot.review ||
        !snapshot.attempted ||
        snapshot.busy ||
        snapshot.receipt
      )
        return;
      const input = snapshot.review;
      publish({ ...snapshot, busy: true, error: undefined });
      try {
        const receipt = await recovery.lookup(input.operationId, lifetime.signal);
        if (receipt) accepted(receipt, input);
        else
          publish({
            ...snapshot,
            busy: false,
            error:
              'No acceptance receipt was found. Retry keeps the original message, audience and operation ID.',
          });
      } catch {
        publish({
          ...snapshot,
          busy: false,
          error:
            'Could not check the earlier send. Its outcome is still unknown; the original request is preserved.',
        });
      }
    },
    discard() {
      if (disposed || snapshot.busy) return;
      try {
        recovery?.journal.clear();
      } catch {
        publish({
          ...snapshot,
          error: 'Could not clear the local pending record. No new request was started.',
        });
        return;
      }
      publish(empty());
    },
    dispose() {
      disposed = true;
      lifetime.abort();
      listeners.clear();
    },
  };
}
export type DispatchComposerState = ReturnType<typeof createDispatchComposerState>;
