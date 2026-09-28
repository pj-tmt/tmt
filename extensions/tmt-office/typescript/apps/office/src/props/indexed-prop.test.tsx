import { render } from '@testing-library/react';
import { expect, it } from 'vitest';
import { readRasterCells } from '../../../../../../../typescript/test/support/indexed-raster.js';
import { BUILTIN_PACK, indexedProp } from './prop-contract.js';
import { IndexedProp } from './indexed-prop.js';

it('renders all indexed pixels as inert SVG paths', () => {
  const prop = indexedProp(BUILTIN_PACK, 'desk')!;
  const view = render(
    <svg>
      <IndexedProp pack={BUILTIN_PACK} prop={prop} />
    </svg>
  );
  const expected = prop.pixels!.flatMap((row, y) =>
    Array.from(row).flatMap((index, x) =>
      index === '0'
        ? []
        : [[String(x), String(y), BUILTIN_PACK.palette[Number.parseInt(index, 16)]!]]
    )
  );
  expect(readRasterCells(view.container)).toEqual(expected);
  expect(view.container.querySelector('script')).toBeNull();
  expect(view.container.querySelector('image')).toBeNull();
});
