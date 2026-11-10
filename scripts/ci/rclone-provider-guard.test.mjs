import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

test('merged storage changes retain exactly one exported provider argument guard', () => {
  const source = path => readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');
  const profile = source('state-agent/crates/noland-rclone-adapter/src/profile.rs');
  assert.equal([...profile.matchAll(/pub fn provider_arg_is_allowed\(/g)].length, 1);
  assert.match(source('state-agent/crates/noland-rclone-adapter/src/lib.rs'), /provider_arg_is_allowed/);
  const storage = source('state-agent/crates/noland-storage/src/rclone.rs');
  assert.match(storage, /noland_rclone_adapter::provider_arg_is_allowed/);
  assert.doesNotMatch(storage, /push_upload_options\(/);
  assert.match(storage, /fn push_backend_options\(/);
});
