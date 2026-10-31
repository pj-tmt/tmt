/** A realistic script-free design proposal, shared by the catalog and its document regression. */
export const designDocument = `<style>
@layer tmt-starter {
  body { font: 15px/1.5 var(--tmt-ui-font-body); }
  h2, h3 { margin: 0 0 8px; font-weight: 660; line-height: 1.2; }
  h2 { font-size: 22px; }
  p { margin: 0 0 16px; }
  .proposal-paper { padding-block: var(--tmt-ui-space-l); max-width: 760px; margin-inline: auto; display: flex; flex-direction: column; gap: 16px; }
  .proposal-controls { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; padding: 16px; border: 1px dashed var(--tmt-ui-surface-rule); border-radius: var(--tmt-ui-radius); }
  .proposal-controls > tmt-ui-tabs { flex: 1 1 100%; }
  .proposal-note { color: var(--tmt-ui-color-secondary); font-size: 12px; }
}
</style>
<section><div class="proposal-paper">
<section><h2>Shortcut dialog — proposal</h2><p>The list keeps its header. On phones, sorting stays beside the category filters.</p></section>
<section class="proposal-controls">
  <tmt-ui-menu label="Sort" value="recent"><span slot="item" data-value="recent">Recently updated</span><span slot="item" data-value="title">Title</span></tmt-ui-menu>
  <tmt-ui-tabs label="Category" value="all" variant="flat" wrap>
    <section slot="panel" data-value="all" data-label="All"><p>Five shortcuts, with one design review waiting for you.</p></section>
    <section slot="panel" data-value="waiting" data-label="Needs you"><p>Review the shortcut editor before it ships.</p></section>
    <section slot="panel" data-value="done" data-label="Done"><p>Two shortcuts have been reviewed.</p></section>
  </tmt-ui-tabs>
</section>
<section><p class="proposal-note">Live menu and tabs: the same controls used by the app, with state local to this document.</p><p>Open the proposed shortcut editor and try changing its name:</p>
  <tmt-ui-dialog title="Manage shortcuts" trigger="Manage shortcuts">
    <div slot="body"><tmt-ui-field label="Name" value="Design reviews"></tmt-ui-field><tmt-ui-checkbox label="Follow updates"></tmt-ui-checkbox></div>
    <p slot="actions">Press Escape to close. Reopening keeps your draft; this proposal does not save changes.</p>
  </tmt-ui-dialog>
</section>
<section><p class="proposal-note">Try the prototype without changing your saved shortcuts.</p></section>
</div></section>`;
