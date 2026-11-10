import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { runInNewContext } from 'node:vm';

test('copy migration decodes HTML entities exactly once, without double unescaping', () => {
  const script = readFileSync(new URL('../feedback-copy-localization.mjs', import.meta.url), 'utf8');
  const declaration = script.match(/^const normalized = .+;$/m)?.[0];
  assert.ok(declaration);
  const normalize = runInNewContext(`${declaration}\nnormalized;`);
  assert.equal(normalize('  A &gt; B &lt; C &amp; &quot;D&quot;  '), 'A > B < C & "D"');
  assert.equal(normalize('&amp;quot; &amp;lt; &amp;amp;'), '&quot; &lt; &amp;');
});
