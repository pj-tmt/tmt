import { canonicalUuid } from '../contracts/record.js';
import { decodeDispatchInput, DISPATCH_MESSAGE_LIMIT } from './dispatch-contract.js';
import type { DispatchInput } from './dispatch-contract.js';

export interface DispatchJournal {
  read(): DispatchInput | undefined;
  save(input: DispatchInput): void;
  clear(operationId?: string): void;
}

/** Tab-local pending intent only. Accepted messages and replies remain in the host database. */
export function createDispatchJournal(
  scope:
    | { kind: 'direct'; recipientId: string; roomId?: string }
    | { kind: 'roster'; roomId: string },
  storage: () => Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> = () => sessionStorage
): DispatchJournal {
  const key =
    scope.kind === 'roster'
      ? `tmt.office.pending-room-request.${canonicalUuid(scope.roomId)}`
      : `tmt.office.pending-request.${canonicalUuid(scope.recipientId)}${scope.roomId ? `.room.${canonicalUuid(scope.roomId)}` : ''}`;
  function admit(value: unknown) {
    const input = decodeDispatchInput(value);
    const matches =
      scope.kind === 'roster'
        ? input.room?.kind === 'roster' && input.room.roomId === scope.roomId
        : (scope.roomId
            ? input.room?.kind === 'direct' && input.room.roomId === scope.roomId
            : !input.room) &&
          input.recipientIds.length === 1 &&
          input.recipientIds[0] === scope.recipientId;
    if (input.kind === 'announcement' || !matches)
      throw new Error('Pending request belongs to a different conversation.');
    return input;
  }
  function read() {
    const encoded = storage().getItem(key);
    if (encoded === null) return undefined;
    if (encoded.length > DISPATCH_MESSAGE_LIMIT * 6 + 4096)
      throw new Error('Pending request exceeds its limit.');
    return admit(JSON.parse(encoded));
  }
  return {
    read,
    save(value) {
      const input = admit(value);
      const current = read();
      if (current && JSON.stringify(current) !== JSON.stringify(input))
        throw new Error('Another pending request must be resolved first.');
      storage().setItem(key, JSON.stringify(input));
    },
    clear(operationId) {
      if (operationId && read()?.operationId !== operationId) return;
      storage().removeItem(key);
    },
  };
}
