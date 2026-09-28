import { render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import vectors from '../../../../../contracts/external-link-vectors.json';
import definition from '../../../../../contracts/link-extension-v1.json';
import { externalLink } from './external-link.js';
import { decodeExtensionDefinition, decodeResourceBinding } from './extension-contract.js';
import { activateExtension, bindWorldExtension } from './extension-binding.js';
import { ExternalLinkReview } from './external-link-review.js';
import { defaultCatalog } from '../blocks/block-contract.js';
import type { WorldObject } from '../world-map/world-contract.js';

it.each(vectors)('web destination agrees with native admission: $name', ({ url, valid }) => {
  if (valid)
    expect(decodeResourceBinding({ kind: 'external-link', url })).toEqual({
      kind: 'external-link',
      url,
    });
  else expect(() => decodeResourceBinding({ kind: 'external-link', url })).toThrow();
});

it('bounds UTF-8 bytes, not UTF-16 units, without truncating or fetching a destination', () => {
  const base = 'https://example.com/';
  expect(() => externalLink(base + 'a'.repeat(2048 - base.length))).not.toThrow();
  expect(() => externalLink(base + 'a'.repeat(2049 - base.length))).toThrow();
  expect(() => externalLink(base + 'é'.repeat(1100))).toThrow();
});

it('keeps a link inert without a handler or with invalid data; activation only calls its host review', () => {
  const object: WorldObject = {
    id: '10000000-0000-4000-8000-000000000001',
    kind: 'decoration',
    surface: { type: 'floor' },
    placement: { ...definition.appearance, x: 1, y: 1, rotation: 0 },
    extension: {
      definition: definition.id,
      binding: { kind: 'external-link', url: 'https://example.com/docs' },
    },
  };
  const handler = vi.fn();
  const defs = [decodeExtensionDefinition(definition)];
  const entry = bindWorldExtension(object, defs, defaultCatalog(), { 'link.open': handler })!;
  expect(handler).not.toHaveBeenCalled();
  expect(activateExtension([entry], object.id)).toBe(true);
  expect(handler).toHaveBeenCalledExactlyOnceWith(object.extension!.binding);
  const unavailable = bindWorldExtension(object, defs, defaultCatalog(), {})!;
  expect(activateExtension([unavailable], object.id)).toBe(false);
  const unsafe = bindWorldExtension(
    {
      ...object,
      extension: {
        definition: definition.id,
        binding: { kind: 'external-link', url: 'javascript:alert(1)' },
      },
    },
    defs,
    defaultCatalog(),
    { 'link.open': handler }
  )!;
  expect(unsafe.unavailable).toBe('Resource binding is invalid.');
  expect(activateExtension([unsafe], object.id)).toBe(false);
  expect(handler).toHaveBeenCalledTimes(1);
});

it('shows the canonical origin and a no-opener destination without navigating on render', () => {
  const open = vi.spyOn(window, 'open');
  const fetch = vi.spyOn(globalThis, 'fetch');
  const view = render(<ExternalLinkReview destination="HTTPS://EXAMPLE.COM/docs?q=office#wall" />);
  expect(screen.getByText('https://example.com')).toBeDefined();
  const link = screen.getByRole('link', { name: 'Open website ↗' });
  expect(link.getAttribute('href')).toBe('https://example.com/docs?q=office#wall');
  expect(link.getAttribute('target')).toBe('_blank');
  expect(link.getAttribute('rel')).toBe('noopener noreferrer');
  expect(open).not.toHaveBeenCalled();
  expect(fetch).not.toHaveBeenCalled();
  view.rerender(<ExternalLinkReview destination="https://user:secret@example.com" />);
  expect(screen.queryByRole('link')).toBeNull();
  expect(screen.getByRole('alert')).toHaveProperty(
    'textContent',
    'This web destination is unavailable.'
  );
});
