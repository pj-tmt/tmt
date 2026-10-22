/** Test-only entry: the Digest page on its real markup, with the sample or the product source. */
import { digestPage } from '../src/digest-page.js';
import { mockDigestSource } from '../src/digest-mock.js';
import { digestSource } from '../src/digest-source.js';

digestPage(new URLSearchParams(location.search).has('product') ? digestSource : mockDigestSource);
