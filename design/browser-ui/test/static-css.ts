import { format } from 'vite-plus/fmt';
// @ts-expect-error The generator is a plain development script.
import { authoredStaticCss } from '../scripts/generate-static-css.mjs';

/** Source-level selector/token assertions are independent of distribution serialization.
 * Generation checks inspect shipped bytes; browser equivalence tests compare actual paint. */
const result = await format('static.css', authoredStaticCss, {
  singleQuote: true,
  printWidth: 100,
});
if (result.errors.length) throw new Error(JSON.stringify(result.errors));
export const staticCss = result.code;
