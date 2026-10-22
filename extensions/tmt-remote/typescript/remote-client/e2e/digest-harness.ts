/** Test-only entry: the Digest page on its real markup, with the sample or the product source. */
import { digestPage } from '../src/digest-page.js';
import { mockDigestSource } from '../src/digest-mock.js';
import { digestSource, type DigestSource } from '../src/digest-source.js';

const params = new URLSearchParams(location.search);
declare global {
  interface Window {
    digestReads: number;
    digestRoute: AbortController;
  }
}

if (params.has('refresh')) {
  // A source that counts its reads: the second read is the same report a minute later (only its
  // own clock moved), the third has less allowance left, the fourth cannot read, the fifth is the
  // third again. `route.abort()` stops the refresh.
  window.digestReads = 0;
  window.digestRoute = new AbortController();
  const source: DigestSource = {
    async report(range) {
      window.digestReads += 1;
      if (window.digestReads === 4)
        return {
          range,
          sample: false,
          unavailable: 'Your machine did not answer. This page tries again shortly.',
        };
      const report = await mockDigestSource.report(range);
      const document = structuredClone(report.document!);
      document.observedAtMs += window.digestReads * 60_000;
      if (window.digestReads >= 3) for (const limit of document.limits) limit.usedPercent = 90;
      return { ...report, document };
    },
  };
  digestPage(source, window.digestRoute.signal);
} else {
  digestPage(params.has('product') ? digestSource : mockDigestSource);
}
