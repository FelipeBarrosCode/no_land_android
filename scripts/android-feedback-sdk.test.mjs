import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src-tauri/android/app/src/main/java/com/noland/connect/feedback/FeedbackNativeBridge.kt', import.meta.url), 'utf8');

test('protected storage uses public Android SDK file APIs and supports legacy atomic backups', () => {
  assert.doesNotMatch(source, /\b(?:target|accountTarget)\.exists\(\)/);
  assert.ok(source.includes('target.baseFile.exists() || File("${target.baseFile.path}.bak").exists()'));
  assert.ok(source.includes('key(anyExistingBlob, alias)'));
  assert.ok(source.includes('hasStoredBlob(accountTarget)'));
});

test('protected writes keep directory fsync without unavailable O_DIRECTORY constant', () => {
  assert.doesNotMatch(source, /OsConstants\.O_DIRECTORY/);
  assert.ok(source.includes('android.system.OsConstants.O_RDONLY, 0)'));
  assert.ok(source.includes('android.system.Os.fsync(parent)'));
  assert.ok(source.includes('android.system.Os.close(parent)'));
  assert.ok(source.includes('stream.fd.sync()'));
});
