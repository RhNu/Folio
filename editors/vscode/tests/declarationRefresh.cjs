const assert = require('node:assert/strict');
const { test } = require('node:test');
const { DeclarationRefresh } = require('../out/declarationRefresh.js');

test('a burst of project generations requires only one refresh', () => {
  const refresh = new DeclarationRefresh();
  assert.equal(refresh.request(4), true);
  assert.equal(refresh.request(5), true);
  assert.equal(refresh.request(5), false);
  assert.equal(refresh.request(3), false);
  assert.equal(refresh.take(), true);
  assert.equal(refresh.take(), false);
  assert.equal(refresh.request(6), true);
  assert.equal(refresh.take(), true);
});

test('reconnect discards pending work and accepts restarted generations', () => {
  const refresh = new DeclarationRefresh();
  refresh.request(20);
  refresh.reset();
  assert.equal(refresh.take(), false);
  assert.equal(refresh.request(1), true);
  assert.equal(refresh.take(), true);
  assert.equal(refresh.request(), true);
  assert.equal(refresh.take(), true);
});

test('malformed generations cannot suppress a later valid refresh', () => {
  const refresh = new DeclarationRefresh();
  for (const generation of [-1, 1.5, NaN, Infinity, '3', null]) {
    assert.equal(refresh.request(generation), false);
  }
  assert.equal(refresh.take(), false);
  assert.equal(refresh.request(0), true);
  assert.equal(refresh.take(), true);
});
