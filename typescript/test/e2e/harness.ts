export { E2EFixture, withE2EFixture } from './harness/fixture.js';
export type {
  CliResult,
  MockEvent,
  MockPane,
  CliRunOptions,
  CliProcess,
  MetadataBarrierOptions,
  E2EFixtureOptions,
} from './harness/types.js';
export { readCompleteJsonLines } from './harness/readiness.js';
export { processState } from './harness/cleanup.js';
