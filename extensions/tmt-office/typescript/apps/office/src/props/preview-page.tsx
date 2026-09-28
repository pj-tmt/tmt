import { useContext } from 'react';
import { LocalRuntimeContext } from '../local/local-runtime.js';
import type { LocalRuntime } from '../local/local-runtime.js';
import { usePreview } from '../local/use-preview.js';
import { IndexedProp } from './indexed-prop.js';
import { PROP_DIRECTIONS } from './prop-contract.js';

const requestPreview = (runtime: LocalRuntime, id: string) => runtime.preview(id);

export function PropPreviewPage({ previewId }: { previewId: string }) {
  const runtime = useContext(LocalRuntimeContext);
  const state = usePreview(runtime, previewId, requestPreview);
  if (state.kind === 'failed')
    return <p role="alert">This prop preview is unavailable or expired.</p>;
  if (state.kind === 'loading') return <p>Loading prop preview…</p>;
  const catalog = state.value;
  return (
    <section className="prop-preview">
      <p className="eyebrow">DATA-ONLY PROP PREVIEW</p>
      <h1>{catalog.pack.label}</h1>
      <p>
        {catalog.pack.credit} · {catalog.pack.license}
      </p>
      <div className="prop-preview-grid">
        {catalog.pack.props.map((prop) => (
          <article key={prop.key}>
            <h2>{prop.label}</h2>
            {(prop.frames ? [0, 1, 2, 3] : [0]).map((rotation) => (
              <figure key={rotation}>
                <svg
                  viewBox={`0 0 ${rotation % 2 ? prop.footprint.height : prop.footprint.width} ${rotation % 2 ? prop.footprint.width : prop.footprint.height}`}
                  role="img"
                  aria-label={`${prop.label}${prop.frames ? ` · ${PROP_DIRECTIONS[rotation]}` : ''}`}
                >
                  <IndexedProp pack={catalog.pack} prop={prop} rotation={rotation} />
                </svg>
                {prop.frames && <figcaption>{PROP_DIRECTIONS[rotation]}</figcaption>}
              </figure>
            ))}
            <p>
              {prop.footprint.width} × {prop.footprint.height} tiles
            </p>
          </article>
        ))}
      </div>
    </section>
  );
}
