/**
 * What the Colab settings page reads: the settings `tmt colab settings --json` prints (opening the
 * browser, the work-in-progress limit, the GitHub repository, page expiry), in the same shape. The
 * page depends only on `ColabSettingsSource`; the default source is a MOCK until Colab serves this
 * read to the owner's browser.
 */
export interface ColabSettingsView {
  /** `serve` and `page create` open the browser. */
  open: boolean;
  /** Assigned todo and in-progress tasks an agent may hold, and the agents with their own limit. */
  wipLimit: { default: number; agents: Readonly<Record<string, number>> };
  /** The repository the GitHub driver pushes tasks to, when one is set. */
  githubRepo?: string;
  /** Days without activity before serve archives a page; null never archives. */
  pageExpiryDays: number | null;
  /** Whether these are sample values, not this space's settings. */
  sample: boolean;
}
export interface ColabSettingsSource {
  read(): Promise<ColabSettingsView>;
}

// MOCK: Colab's defaults, so the page can be built and reviewed before the read exists. Replace with
// the owner read; nothing else in the page changes.
export const mockColabSettingsSource: ColabSettingsSource = {
  async read() {
    return { open: true, wipLimit: { default: 1, agents: {} }, pageExpiryDays: 180, sample: true };
  },
};

/** The source the page reads by default. */
export const colabSettingsSource: ColabSettingsSource = mockColabSettingsSource;
