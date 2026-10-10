import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test, vi } from 'vite-plus/test';
import { parseFirestoreLayers, firestoreSpecs, firestoreReasons } from '../src/firestore.js';
import { parsePublishedLimits } from '../src/budget.js';
import { renderFirestore } from '../src/firestore-page.js';
import type { FirestoreSettingsView } from '../src/management.js';

export const publishedFixture = () =>
  JSON.parse(
    readFileSync(
      new URL(
        '../../../rust/tmt-remote/tests/fixtures/firestore_budget/limits-member.json',
        import.meta.url,
      ),
      'utf8',
    ),
  );
export const enabledFixture = () => [
  {
    layer: 'sharing',
    state: 'enabled',
    prerequisites: ['project', 'sign-in', 'rules', 'plan-tier', 'quota'].map((item) => ({
      item,
      state: 'enabled',
    })),
  },
  {
    layer: 'operations',
    state: 'not-enabled',
    prerequisites: [
      { item: 'plan-tier', state: 'enabled' },
      { item: 'support', state: 'not-enabled', reason: 'not-implemented' },
    ],
  },
  {
    layer: 'attachments',
    state: 'not-enabled',
    prerequisites: [
      { item: 'support', state: 'not-enabled', reason: 'not-implemented' },
      ...['project', 'rules', 'quota'].map((item) => ({ item, state: 'enabled' })),
    ],
  },
];
// Independent expected vocabulary and copy; not derived from the product lookup.
const cases = [
  ['project', 'not-configured', 'No Firebase project is set up.', true],
  ['project', 'access-lost', 'tmt can no longer reach the Firebase project.', true],
  ['project', 'not-checked', 'The Firebase project has not been checked.', false],
  ['sign-in', 'provider-disabled', 'Sign-in is not enabled for the project.', true],
  [
    'sign-in',
    'permission-missing',
    "This account isn't allowed to turn on sign-in for the project.",
    false,
  ],
  ['sign-in', 'not-checked', 'Sign-in has not been checked.', false],
  ['rules', 'not-deployed', 'Firestore rules are not deployed.', true],
  ['rules', 'out-of-date', 'Firestore rules are out of date.', true],
  ['rules', 'partial', 'Firestore rules are only partly deployed.', true],
  ['rules', 'not-checked', 'Firestore rules have not been checked.', false],
  ['plan-tier', 'not-checked', 'The Firebase plan has not been checked.', false],
  ['quota', 'headroom-low', "Today's free Firebase quota is almost used up.", false],
  ['quota', 'exhausted', "Today's free Firebase quota is used up.", false],
  ['quota', 'not-checked', 'The free Firebase quota has not been checked.', false],
] as const;
// A bounded DOM stand-in, matching the browser-entry tests; browser geometry is checked separately.
class FirestoreDomNode {
  private content = '';
  writes = 0;
  get textContent() {
    return this.content;
  }
  set textContent(value: string) {
    this.content = value;
    this.writes++;
  }
  dataset: Record<string, string> = {};
  children: FirestoreDomNode[] = [];
  attributes: Record<string, string> = {};
  id = '';
  className = '';
  scope = '';
  constructor(
    readonly tagName: string,
    text = '',
  ) {
    this.textContent = text;
  }
  append(...nodes: FirestoreDomNode[]) {
    this.children.push(...nodes);
  }
  replaceChildren() {
    this.children = [];
    this.textContent = '';
  }
  setAttribute(name: string, value: string) {
    this.attributes[name] = value;
  }
  text(): string {
    return this.textContent + this.children.map((child) => child.text()).join('');
  }
  find(tag: string): FirestoreDomNode[] {
    return [
      ...(this.tagName === tag ? [this] : []),
      ...this.children.flatMap((child) => child.find(tag)),
    ];
  }
}
function render(
  layers: unknown,
  budget = publishedFixture(),
  announcement = new FirestoreDomNode('p'),
) {
  vi.stubGlobal('document', {
    getElementById: (id: string) => {
      assert.equal(id, 'firestore-announcement');
      return announcement;
    },
    createElement: (tag: string) => new FirestoreDomNode(tag),
    createTextNode: (text: string) => new FirestoreDomNode('#text', text),
  });
  const target = new FirestoreDomNode('div');
  const view = {
    firestoreLayers: parseFirestoreLayers(layers),
    firestoreBudget: parsePublishedLimits(budget),
  } as FirestoreSettingsView;
  renderFirestore(target as unknown as HTMLElement, 'confirmed', view);
  return target;
}
test('every visible sharing prerequisite has exact state, fixed sentence and only the allowed command', () => {
  try {
    for (const [item, reason, sentence, next] of cases) {
      const layers = enabledFixture();
      const state = reason === 'not-checked' ? 'unknown' : 'not-enabled';
      const prerequisite = layers[0]!.prerequisites.find((p) => p.item === item)!;
      Object.assign(prerequisite, {
        state,
        reason,
        ...(next ? { next: 'tmt remote deploy firestore' } : {}),
      });
      layers[0]!.state = state;
      const target = render(layers);
      const row = target.find('li').find((p) => p.dataset.item === item)!;
      assert.ok(row.text().includes(state === 'unknown' ? 'Not checked' : 'Not enabled'));
      assert.ok(row.text().includes(sentence));
      assert.equal(row.find('code').length, next ? 1 : 0);
      assert.ok(!row.text().includes(reason));
      assert.equal(
        target.find('h3').filter((h) => h.text().includes('Not available in this release.')).length,
        2,
      );
      // Unsupported layers contain no rows/commands even when another prerequisite is off.
      for (const layer of target.find('section').filter((s) => s.dataset.layer !== 'sharing')) {
        assert.equal(layer.find('li').length, 0);
        assert.equal(layer.find('code').length, 0);
      }
    }
    const target = render(enabledFixture());
    for (const row of target.find('li')) assert.ok(row.text().includes('Enabled'));
  } finally {
    vi.unstubAllGlobals();
  }
});
test('strict readiness rejects arbitrary copy, extra keys, wrong order, illegal reasons and inconsistent aggregates', () => {
  assert.deepEqual(parseFirestoreLayers([]), []);
  for (const mutate of [
    (v: any) => {
      v[0].message = 'TOKEN_CANARY';
    },
    (v: any) => {
      v[0].state = 'not-enabled';
    },
    (v: any) => {
      v[0].prerequisites.reverse();
    },
    (v: any) => {
      Object.assign(v[0].prerequisites[0], { state: 'not-enabled', reason: 'paid-plan-required' });
      v[0].state = 'not-enabled';
    },
    (v: any) => {
      Object.assign(v[0].prerequisites[2], {
        state: 'not-enabled',
        reason: 'partial',
        next: 'firebase deploy',
      });
      v[0].state = 'not-enabled';
    },
    (v: any) => {
      Object.assign(v[0].prerequisites[2], {
        state: 'unknown',
        reason: 'partial',
        next: 'tmt remote deploy firestore',
      });
      v[0].state = 'unknown';
    },
  ]) {
    const value = enabledFixture();
    mutate(value);
    assert.throws(() => parseFirestoreLayers(value), /Invalid Firestore/);
  }
  const free = enabledFixture();
  Object.assign(free[1]!.prerequisites[0]!, { state: 'not-enabled', reason: 'paid-plan-required' });
  assert.equal(parseFirestoreLayers(free)[1]!.prerequisites[0]!.reason, 'paid-plan-required');
});
test('dated budget equals the native golden and renders received date/values, never usage', () => {
  try {
    const limits = publishedFixture();
    assert.deepEqual(parsePublishedLimits(limits), limits);
    let target = render(enabledFixture(), limits);
    assert.equal(target.find('tbody')[0]!.children.length, 10);
    assert.ok(target.text().includes('50,000 per day'));
    assert.ok(target.text().includes('70%') && target.text().includes('90%'));
    assert.ok(target.text().includes("Remote can't see your actual usage"));
    limits.readOn = '2026-09-01';
    limits.limits.readsPerDay = 30000;
    target = render(enabledFixture(), limits);
    assert.ok(target.text().includes('Published limits as of 2026-09-01'));
    assert.ok(target.text().includes('30,000 per day'));
    for (const change of [
      (v: any) => {
        v.usage = 70;
      },
      (v: any) => {
        v.guard.warnPercent = 95;
      },
      (v: any) => {
        v.limits.readsPerDay = 0;
      },
      (v: any) => {
        v.limits.secret = 'TOKEN_CANARY';
      },
    ]) {
      const bad = publishedFixture();
      change(bad);
      assert.throws(() => parsePublishedLimits(bad));
    }
    assert.ok(render([]).text().includes('Firestore is not configured.'));
  } finally {
    vi.unstubAllGlobals();
  }
});

test('SDK readiness vocabulary mechanically equals the Rust-owned table fixture', () => {
  const fixture = JSON.parse(
    readFileSync(
      new URL(
        '../../../rust/tmt-remote/tests/fixtures/firestore_readiness/table.json',
        import.meta.url,
      ),
      'utf8',
    ),
  );
  assert.deepEqual(
    firestoreSpecs.map(([layer, items]) => ({ layer, items })),
    fixture.layers.map(({ layer, items }: { layer: string; items: string[] }) => ({
      layer,
      items,
    })),
  );
  const reasons = Object.entries(firestoreReasons).flatMap(([item, rows]) =>
    Object.entries(rows).map(([reason, [sentence, next]]) => ({
      item,
      reason,
      sentence,
      next: next ? 'tmt remote deploy firestore' : null,
    })),
  );
  assert.deepEqual(reasons, fixture.reasons);
  for (const spec of fixture.layers) {
    if (!spec.items.includes('plan-tier')) continue;
    const layers = enabledFixture();
    const layer = layers.find((layer) => layer.layer === spec.layer)!;
    Object.assign(
      layer.prerequisites.find((item) => item.item === 'plan-tier')!,
      {
        state: 'not-enabled',
        reason: 'paid-plan-required',
      },
    );
    layer.state = 'not-enabled';
    if (spec.requiresPaidPlan) parseFirestoreLayers(layers);
    else assert.throws(() => parseFirestoreLayers(layers));
  }
});

test('unchanged Firestore projection preserves Details and only changed settled state announces', () => {
  try {
    const announcement = new FirestoreDomNode('p');
    const target = render(enabledFixture(), publishedFixture(), announcement);
    const details = target.find('details')[0]!;
    details.setAttribute('open', '');
    const writes = announcement.writes;
    const view = {
      firestoreLayers: parseFirestoreLayers(enabledFixture()),
      firestoreBudget: parsePublishedLimits(publishedFixture()),
    } as FirestoreSettingsView;
    renderFirestore(target as unknown as HTMLElement, 'confirmed', view);
    assert.equal(target.find('details')[0], details);
    assert.equal(details.attributes.open, '');
    assert.equal(announcement.writes, writes);
    renderFirestore(target as unknown as HTMLElement, 'checking');
    assert.equal(announcement.writes, writes);
    renderFirestore(target as unknown as HTMLElement, 'confirmed', view);
    assert.equal(announcement.writes, writes, 'same settled state is quiet after checking');
    renderFirestore(target as unknown as HTMLElement, 'unconfirmed');
    assert.equal(announcement.textContent, 'Firestore setup could not be confirmed.');
    assert.equal(announcement.writes, writes + 1);
    renderFirestore(target as unknown as HTMLElement, 'unconfirmed');
    assert.equal(announcement.writes, writes + 1);
  } finally {
    vi.unstubAllGlobals();
  }
});
