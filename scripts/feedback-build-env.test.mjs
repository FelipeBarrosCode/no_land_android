import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';

const directory = new URL('../.github/workflows/', import.meta.url);
const workflows = readdirSync(directory).filter((name) => name.endsWith('.yml'));

test('every distributable app build receives the feedback API secret at compile time', () => {
  let builds = 0;
  for (const name of workflows) {
    const source = readFileSync(new URL(name, directory), 'utf8');
    if (!/(?:npm run tauri:(?:ios:)?build|npx tauri android build)/.test(source)) continue;
    builds++;
    const globalEnv = source.match(/^env:\n((?:[ \t]+[^\n]*\n|\n)*)/m)?.[1];
    assert.ok(globalEnv, `${name}: workflow-level build environment exists`);
    assert.match(globalEnv, /^  NOLAND_FEEDBACK_API_URL: \$\{\{ secrets\.NOLAND_FEEDBACK_API_URL \}\}$/m, name);
  }
  assert.ok(builds > 0, 'at least one app build was checked');
});

test('Android rehearsals inherit build secrets without invoking desktop signing or publication', () => {
  const release = readFileSync(new URL('release.yml', directory), 'utf8');
  const nightly = readFileSync(new URL('nightly.yml', directory), 'utf8');
  assert.match(release, /uses: \.\/\.github\/workflows\/ci\.yml/);
  assert.match(release, /uses: \.\/\.github\/workflows\/_tests\.yml/);
  assert.match(nightly, /uses: \.\/\.github\/workflows\/android\.yml/);
  assert.match(nightly, /publish: false/);
  assert.match(nightly, /secrets: inherit/);
  assert.doesNotMatch(release + nightly, /uses: .*\/_build\.yml|action-gh-release|app-store\.yml/);
});
