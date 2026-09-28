import { externalLink } from './external-link.js';
import './external-link-review.css';

/** A second, deliberate user action navigates; opening this panel never does. */
export function ExternalLinkReview({ destination }: { destination: string }) {
  let url: URL;
  try {
    url = externalLink(destination);
  } catch {
    return <p role="alert">This web destination is unavailable.</p>;
  }
  return (
    <section className="external-link-review">
      <h2>Open an external website?</h2>
      <p>This link leaves your local Office. Only continue if you trust the destination.</p>
      <p>
        Destination: <strong>{url.origin}</strong>
      </p>
      <p className="external-link-url">{url.href}</p>
      <a href={url.href} target="_blank" rel="noopener noreferrer">
        Open website ↗
      </a>
    </section>
  );
}
