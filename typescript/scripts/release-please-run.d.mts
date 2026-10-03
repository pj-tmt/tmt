import type {
  GitHub,
  Manifest,
} from '../../.github/release-please/node_modules/release-please/build/src/index.js';
import type { ComponentMap } from './ci-scope.mjs';

export function loadPinnedReleasePlease(): typeof import('../../.github/release-please/node_modules/release-please/build/src/index.js');

export function assertReleasePleaseApi(api: unknown): void;
export function attributeReleaseConsumption(
  github: GitHub,
  components: ComponentMap['components']
): GitHub;

export function executeReleasePlease(
  manifest: Pick<
    Manifest,
    'createPullRequests' | 'buildPullRequests' | 'createReleases' | 'buildReleases'
  >,
  command: string,
  live: boolean
): Promise<unknown>;

export function preserveUnchangedReleasePullRequests(
  github: GitHub,
  fileNotFoundError: typeof import('../../.github/release-please/node_modules/release-please/build/src/index.js').Errors.FileNotFoundError
): GitHub;

export function holdTaglessDraftCandidates(manifest: Manifest, heldPaths: unknown): Manifest;
