import document from '../../../../../contracts/commons-preset-v1.json';
import { validLocalLayout } from './block-contract.js';
import type { Furniture } from './block-contract.js';

if (!validLocalLayout(document)) throw new Error('Invalid bundled commons layout.');

/** Fixed public fixtures, not a fallback for the user's saved lobby layout. */
export const COMMONS_FURNITURE: readonly Furniture[] = document.objects;
