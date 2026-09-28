import type { BlockPort } from '../blocks/block-contract.js';
import type { PairingCapabilities } from '../pairing/pairing-contract.js';

export const SPACE_PAGE_SIZE = 20;
export interface AgentSpace {
  principalUid: string;
  installationId: string;
  identityId: string;
  blockId: string;
  capabilities: PairingCapabilities;
  enabled: boolean;
  expiresAtMs: number;
  replacedByPairingId?: string;
}
export interface SpacePage {
  entries: AgentSpace[];
  next: string | null;
}
export interface SpacePort {
  list(worldId: string, ownerUid: string, after?: string): Promise<SpacePage>;
  block(blockId: string): BlockPort;
}
