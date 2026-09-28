import { decodeWhiteboardScene } from './scene-contract.js';
import type { WhiteboardScene } from './scene-contract.js';
import { snapshotHistory } from '../editor/snapshot-history.js';
import type { SnapshotHistory } from '../editor/snapshot-history.js';

export type WhiteboardHistory = SnapshotHistory<WhiteboardScene>;
const history = snapshotHistory<WhiteboardScene>(decodeWhiteboardScene);
export const createHistory = history.create;
export const currentScene = history.current;
export const commitScene = history.commit;
export const undoScene = history.undo;
export const redoScene = history.redo;
