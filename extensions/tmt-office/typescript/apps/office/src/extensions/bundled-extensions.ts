import discussion from '../../../../../contracts/discussion-extension-v1.json' with { type: 'json' };
import whiteboard from '../../../../../contracts/whiteboard-extension-v1.json' with { type: 'json' };
import broadcaster from '../../../../../contracts/broadcaster-extension-v1.json' with { type: 'json' };
import link from '../../../../../contracts/link-extension-v1.json' with { type: 'json' };
import notebook from '../../../../../contracts/notebook-extension-v1.json' with { type: 'json' };
import { decodeExtensionDefinition } from './extension-contract.js';

export const DISCUSSION_EXTENSION = decodeExtensionDefinition(discussion);
export const WHITEBOARD_EXTENSION = decodeExtensionDefinition(whiteboard);
export const BROADCASTER_EXTENSION = decodeExtensionDefinition(broadcaster);
export const NOTEBOOK_EXTENSION = decodeExtensionDefinition(notebook);
export const BUNDLED_EXTENSIONS = [
  DISCUSSION_EXTENSION,
  WHITEBOARD_EXTENSION,
  BROADCASTER_EXTENSION,
  decodeExtensionDefinition(link),
  NOTEBOOK_EXTENSION,
];
