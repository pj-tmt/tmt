import { render } from '@testing-library/react';
import { expect, it } from 'vitest';
import { BUILTIN_PACK } from './prop-contract.js';
import type { PropDefinition } from './prop-contract.js';
import { PropThumbnail } from './prop-thumbnail.js';

it.each([false, true])(
  'fits visible pixels with the correct index width (directional: %s)',
  (directional) => {
    const pixels = directional ? ['00000000', '00010100', '00000000'] : ['0000', '0110', '0000'];
    const prop: PropDefinition = {
      key: 'padded',
      label: 'Padded art',
      footprint: directional ? { width: 8, height: 6 } : { width: 4, height: 3 },
      ...(directional
        ? { frames: [pixels, pixels, pixels, pixels] as [string[], string[], string[], string[]] }
        : { pixels }),
    };
    const before = structuredClone(prop);
    const { container } = render(<PropThumbnail pack={BUILTIN_PACK} prop={prop} />);
    expect(container.querySelector('svg')?.getAttribute('viewBox')).toBe(
      directional ? '2 2 4 2' : '1 1 2 1'
    );
    expect(container.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
    expect(container.querySelectorAll('path')).toHaveLength(1);
    expect(prop).toEqual(before);
  }
);

it('keeps a finite thumbnail frame for transparent admitted art', () => {
  const prop: PropDefinition = {
    key: 'empty',
    label: 'Empty',
    footprint: { width: 2, height: 1 },
    pixels: ['00'],
  };
  const { container } = render(<PropThumbnail pack={BUILTIN_PACK} prop={prop} />);
  expect(container.querySelector('svg')?.getAttribute('viewBox')).toBe('0 0 2 1');
  expect(container.querySelectorAll('path')).toHaveLength(0);
});
