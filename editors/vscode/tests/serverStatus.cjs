const assert = require('node:assert/strict');
const { test } = require('node:test');
const { ServerStatusTracker } = require('../out/serverStatus.js');

test('project progress remains authoritative until the matching generation is ready', () => {
  const tracker = new ServerStatusTracker();
  assert.equal(tracker.accept({ state: 'loading', phase: 'Declarations', message: 'Reading APIs',
    completed: 12, total: 40, generation: 7 }), true);
  assert.equal(tracker.loading, true);
  assert.match(tracker.presentation.label, /Reading APIs.*12\/40/);
  assert.equal(tracker.accept({ state: 'ready', phase: 'Ready', message: 'Ready', generation: 6 }), false);
  assert.equal(tracker.loading, true);
  assert.equal(tracker.accept({ state: 'ready', phase: 'Ready', message: 'Ready', generation: 7 }), true);
  assert.equal(tracker.loading, false);
  assert.match(tracker.presentation.tooltip, /Ready/);
});

test('reconnecting accepts a restarted generation and error clears request suppression', () => {
  const tracker = new ServerStatusTracker();
  tracker.accept({ state: 'ready', phase: 'Ready', message: 'Ready', generation: 20 });
  tracker.reset();
  assert.equal(tracker.current, undefined);
  assert.equal(tracker.accept({ state: 'loading', phase: 'Project', message: 'Loading', generation: 0 }), true);
  assert.equal(tracker.accept({ state: 'error', phase: 'Project', message: 'Cannot read manifest', generation: 0 }), true);
  assert.equal(tracker.loading, false);
  assert.match(tracker.presentation.tooltip, /Cannot read manifest/);
});

test('malformed progress does not corrupt the current status', () => {
  const tracker = new ServerStatusTracker();
  const valid = { state: 'loading', phase: 'Project', message: 'Loading', generation: 1 };
  tracker.accept(valid);
  for (const invalid of [null, {}, { ...valid, generation: -1 }, { ...valid, state: 'connected' },
    { ...valid, total: '40' }, { ...valid, completed: -1 }]) {
    assert.equal(tracker.accept(invalid), false);
  }
  assert.equal(tracker.current, valid);
});
