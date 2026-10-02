import { describe, expect, it } from 'vite-plus/test';
import { assertBenchmarkHelp } from '../support/performance-contract.mjs';

const commands = ['talk', 'reply', 'result', 'identity', 'config'];
const inventory = commands.map((command) => `  ${command} <arguments>  Description`).join('\n');

describe('performance help evidence', () => {
  it.each(['COMMANDS', 'Commands:'])(
    'accepts %s without a renderer-specific heading',
    (heading) => {
      expect(() => assertBenchmarkHelp(`${heading}\n${inventory}\n`)).not.toThrow();
    }
  );

  it.each(commands)('rejects a missing %s command even with the old heading', (missing) => {
    const incomplete = commands
      .filter((command) => command !== missing)
      .map((command) => `  ${command} <arguments>`)
      .join('\n');
    expect(() => assertBenchmarkHelp(`TALK OPTIONS\n${incomplete}\n`)).toThrow(
      `Missing help command: ${missing}`
    );
  });

  it.each(['', 'talk reply result identity config', '  talkative reply result identity config'])(
    'rejects non-command output %j',
    (stdout) => {
      expect(() => assertBenchmarkHelp(stdout)).toThrow('Missing help command: talk');
    }
  );
});
