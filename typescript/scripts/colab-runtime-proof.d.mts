export function verifyColabApp(options: {
  executable: string;
  version: string;
  expectedApp?: string;
  notices: string;
  tmtExecutable?: string;
}): Promise<void>;
