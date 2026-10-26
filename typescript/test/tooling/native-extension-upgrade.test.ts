import { existsSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';

vi.mock('../../scripts/native-artifact-policy.mjs', () => ({
  selectNativeArtifact: (manifest: string) => ({
    version: manifest === 'previous' ? '0.1.0-alpha.1' : '0.1.0-alpha.2',
  }),
  withNativeArtifact: async (
    _archive: string,
    _artifact: unknown,
    proof: (directory: string) => Promise<void>
  ) => proof('/synthetic/archive'),
}));
vi.mock('../../scripts/native-runtime-proof.mjs', () => ({
  assertMacOsArchitecture: vi.fn(),
}));

const { verifyExtensionUpgrade } = await import(
  new URL('../../scripts/verify-native-extension-upgrade.mjs', import.meta.url).href
);

it.each(['ETIMEDOUT', 'EACCES'])(
  'passes a bounded driver timeout and preserves %s with sandbox cleanup',
  async (code) => {
    const failure = Object.assign(new Error('driver command failed'), { code });
    let root = '';
    const runCommand = vi.fn(
      (
        _executable: string,
        args: string[],
        options: {
          cwd: string;
          env: NodeJS.ProcessEnv;
          timeoutMs: number;
          expectedStatus: number;
        }
      ) => {
        root = options.cwd;
        expect(args.slice(0, 3)).toEqual(['extension', 'install', 'colab']);
        expect(options.timeoutMs).toBe(60_000);
        expect(options.expectedStatus).toBe(0);
        expect(options.env.HOME).toBe(root);
        expect(options.env.PATH).toBe('');
        expect(existsSync(root)).toBe(true);
        throw failure;
      }
    );
    const architecture = process.arch === 'arm64' ? 'aarch64' : 'x86_64';
    const platform = process.platform === 'darwin' ? 'apple-darwin' : 'unknown-linux-musl';
    await expect(
      verifyExtensionUpgrade(
        {
          product: 'colab',
          target: `${architecture}-${platform}`,
          archive: 'current.tar.gz',
          manifest: 'current',
          'previous-archive': 'previous.tar.gz',
          'previous-manifest': 'previous',
          'driver-archive': 'cli.tar.gz',
          'driver-manifest': 'driver',
        },
        runCommand
      )
    ).rejects.toBe(failure);
    expect(runCommand).toHaveBeenCalledOnce();
    expect(root).not.toBe('');
    expect(existsSync(root)).toBe(false);
  }
);
