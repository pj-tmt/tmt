// How each native product's GitHub release is published. The CLI release is
// the repository's "latest" release, so
// `releases/latest/download/install.sh` always reaches the CLI installer.
// Extension releases share the repository and must never become latest.
// Alpha status lives in the version and title, not in GitHub's prerelease flag.
// `tmt upgrade` checks the flag with `Product::accepts_prerelease_flag` in
// tmt-core; a Rust test in native_install/release_tests.rs reads this table, so
// publishing a flag the updater refuses fails there.

const PRODUCTS = {
  cli: { tagPrefix: 'v', prerelease: false, latest: true },
  office: { tagPrefix: 'tmt-office-v', prerelease: true, latest: false },
  squad: { tagPrefix: 'tmt-squad-v', prerelease: true, latest: false },
};

/** The publication settings for one product; unknown products are refused. */
export function releasePolicy(product) {
  const policy = PRODUCTS[product];
  if (!policy) throw new Error(`Unknown native product: ${product}`);
  return { product, ...policy };
}

/** `gh release create` flags that apply the policy to a draft. */
export function releaseFlags(product) {
  const policy = releasePolicy(product);
  return [...(policy.prerelease ? ['--prerelease'] : []), `--latest=${policy.latest}`];
}

/**
 * After publication: the repository's latest release must be a CLI tag, so the
 * one-line installer URL resolves to a CLI `install.sh`.
 */
export function checkLatestTag(tag) {
  const owner = Object.values(PRODUCTS).find(
    (policy) => tag.startsWith(policy.tagPrefix) && /^\d/.test(tag.slice(policy.tagPrefix.length))
  );
  if (owner !== PRODUCTS.cli) {
    throw new Error(
      `The latest release ${tag} is not a CLI release; install.sh would not resolve.`
    );
  }
  return true;
}
