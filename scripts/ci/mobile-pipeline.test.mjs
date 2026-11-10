import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const workflow = name => readFileSync(new URL(`../../.github/workflows/${name}.yml`, import.meta.url), 'utf8');

test('shared CI checks every script suite, localization and runtime contracts', () => {
  const ci = workflow('ci');
  assert.match(ci, /workflow_call:/);
  assert.match(ci, /os: \[ubuntu-24\.04, macos-latest\]/);
  assert.match(ci, /run: npm run test:scripts/);
  assert.match(ci, /run: npm run i18n:check/);
  assert.match(ci, /runtime-contracts\/Cargo\.toml/);
  assert.match(ci, /needs: \[script-tests, typecheck-build, rust-format, rust-check, unit-tests, security\]/);
});

test('mobile releases cannot accidentally invoke the inherited six-desktop builder', () => {
  const release = workflow('release');
  assert.match(release, /workflow_call:/);
  assert.match(release, /uses: .*\/ci\.yml/);
  assert.match(release, /needs: checks\n    uses: .*\/_tests\.yml/);
  assert.doesNotMatch(release + workflow('nightly'), /_build\.yml|signed-six-platform|git tag/);
});

test('publishing requires successful release validation and both Android ABIs', () => {
  const android = workflow('android');
  assert.match(android, /uses: .*\/release\.yml/);
  assert.match(android, /needs: \[version, validation, build\]/);
  assert.match(android, /needs\.validation\.result == 'success'/);
  assert.match(android, /github\.event_name == 'push' \|\| inputs\.publish/);
  assert.match(android, /abi: arm64/);
  assert.match(android, /abi: x86_64/);
  assert.match(workflow('nightly'), /publish: false/);
});
