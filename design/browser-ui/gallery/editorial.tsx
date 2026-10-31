import { Pair, Section } from './kit.js';

/** Every element the editorial layer sets, as an author page or a blog post writes it: plain HTML in
 * one `.tmt-ui-prose` container, with no class on the elements themselves. */
function ProseSample() {
  return (
    <article className="tmt-ui-prose" data-testid="prose">
      <h1>Editorial layer</h1>
      <p>
        Long-form text reads on one type ramp. A paragraph sits a block gap from the next, a{' '}
        <a href="#editorial">link</a> keeps its underline, and <code>inline code</code> takes the
        code surface. <small>Small text is a step down.</small>
      </p>
      <h2>Rhythm and lists</h2>
      <ul>
        <li>List items sit a step apart.</li>
        <li>
          Nested text wraps anywhere: <code>tmt colab page check --strict</code>
        </li>
      </ul>
      <ol>
        <li>First</li>
        <li>Second</li>
      </ol>
      <blockquote>A quote keeps a rule on its leading edge and secondary ink.</blockquote>
      <h3>Tables and figures</h3>
      <figure>
        <table>
          <thead>
            <tr>
              <th>Role</th>
              <th>Size</th>
              <th>Use</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>h2</td>
              <td>22px</td>
              <td>Section heading</td>
            </tr>
            <tr>
              <td>body</td>
              <td>15px</td>
              <td>Prose</td>
            </tr>
          </tbody>
        </table>
        <figcaption>Figure 1. A plain table in a figure, with its caption under it.</figcaption>
      </figure>
      <h4>Code</h4>
      <pre>
        <code>{`tmt colab page create --title Notes --layout column --file page.html\n# A long line wraps rather than scrolling sideways, so a phone never pans the block.`}</code>
      </pre>
      <hr />
      <p>
        Tags: <span className="tmt-ui-tag">Private</span>{' '}
        <a className="tmt-ui-tag" href="#editorial">
          design
        </a>
      </p>
    </article>
  );
}

export function EditorialSection() {
  return (
    <Section id="editorial">
      <Pair title="Prose · every element">{() => <ProseSample />}</Pair>
    </Section>
  );
}
