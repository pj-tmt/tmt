import { useContext } from 'react';
import { LocalRuntimeContext } from '../local/local-runtime.js';
import type { LocalRuntime } from '../local/local-runtime.js';
import { usePreview } from '../local/use-preview.js';
import { Avatar } from '../profiles/avatar.js';
import { avatarRaster } from './avatar-contract.js';

const requestPreview = (runtime: LocalRuntime, id: string) => runtime.avatarPreview(id);

const neutralAppearance = {
  hairStyle: 'bald',
  hairColor: 'ink',
  skinTone: 'light',
  shirtColor: 'blue',
  shirtMark: '',
} as const;

export function AvatarPreviewPage({ previewId }: { previewId: string }) {
  const runtime = useContext(LocalRuntimeContext);
  const state = usePreview(runtime, previewId, requestPreview);
  if (state.kind === 'failed')
    return <p role="alert">This avatar preview is unavailable or expired.</p>;
  if (state.kind === 'loading') return <p>Loading avatar preview…</p>;
  const catalog = state.value;
  return (
    <section className="avatar-preview">
      <p className="eyebrow">DATA-ONLY AVATAR PREVIEW</p>
      <h1>{catalog.pack.label}</h1>
      <p>
        {catalog.pack.credit} · {catalog.pack.license}
      </p>
      <div className="avatar-preview-grid">
        {catalog.pack.avatars.map((avatar) => (
          <article key={avatar.key}>
            <svg viewBox="-7 -8 14 16" role="img" aria-label={avatar.label}>
              <Avatar
                appearance={neutralAppearance}
                name={avatar.key}
                displayLabel={avatar.label}
                customArt={avatarRaster(catalog.pack, avatar)}
              />
            </svg>
          </article>
        ))}
      </div>
    </section>
  );
}
