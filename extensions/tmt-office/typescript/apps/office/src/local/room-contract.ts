import { boundedText, canonicalUuid, exactRecord } from '../contracts/record.js';
import { DISPATCH_RECIPIENT_LIMIT } from './dispatch-contract.js';

export interface MeetingRoom {
  id: string;
  name: string;
  revision: number;
  retired: boolean;
  memberIds: string[];
}
export interface RoomWrite {
  expectedRevision: number;
  name: string;
  memberIds: string[];
}
export interface RoomPort {
  list(signal?: AbortSignal): Promise<MeetingRoom[]>;
  save(id: string, input: RoomWrite, signal?: AbortSignal): Promise<MeetingRoom>;
  retire(id: string, expectedRevision: number, signal?: AbortSignal): Promise<MeetingRoom>;
}
function members(value: unknown): string[] {
  if (!Array.isArray(value) || value.length > DISPATCH_RECIPIENT_LIMIT)
    throw new Error('A room supports up to 64 members.');
  return [...new Set(value.map(canonicalUuid))].sort();
}
function name(value: unknown): string {
  if (!boundedText(value, 80) || !/[^\p{White_Space}]/u.test(value))
    throw new Error('Use a room name up to 80 UTF-8 bytes without control characters.');
  return value;
}
export function decodeRoom(value: unknown): MeetingRoom {
  const room = exactRecord(
    value,
    ['id', 'name', 'revision', 'retired', 'memberIds'],
    'meeting room'
  );
  if (!Number.isSafeInteger(room.revision) || (room.revision as number) < 1)
    throw new Error('Invalid room revision.');
  if (typeof room.retired !== 'boolean') throw new Error('Invalid room retirement state.');
  const memberIds = members(room.memberIds);
  if (JSON.stringify(memberIds) !== JSON.stringify(room.memberIds))
    throw new Error('Invalid room member order.');
  return {
    id: canonicalUuid(room.id),
    name: name(room.name),
    revision: room.revision as number,
    retired: room.retired,
    memberIds,
  };
}
export function decodeRoomList(value: unknown): MeetingRoom[] {
  if (!Array.isArray(value)) throw new Error('Invalid room list.');
  const rooms = value.map(decodeRoom);
  if (rooms.some((room) => room.retired)) throw new Error('Retired room in active list.');
  if (new Set(rooms.map((room) => room.id)).size !== rooms.length)
    throw new Error('Duplicate room.');
  return rooms;
}
export function decodeRoomWrite(value: unknown): RoomWrite {
  const input = exactRecord(value, ['expectedRevision', 'name', 'memberIds'], 'room update');
  if (
    !Number.isSafeInteger(input.expectedRevision) ||
    (input.expectedRevision as number) < 0 ||
    (input.expectedRevision as number) >= Number.MAX_SAFE_INTEGER
  )
    throw new Error('Invalid room revision.');
  return {
    expectedRevision: input.expectedRevision as number,
    name: name(input.name),
    memberIds: members(input.memberIds),
  };
}
