#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = new URL('../../', import.meta.url);
function discover(directory) {
  const url = new URL(`${directory}/`, root);
  if (!existsSync(url)) return [];
  return readdirSync(url, { withFileTypes: true }).flatMap(entry => {
    const path = `${directory}/${entry.name}`;
    return entry.isDirectory() ? discover(path) : entry.name.endsWith('.test.mjs') ? [path] : [];
  });
}
const files = [...discover('scripts'), ...discover('src/features/feedback')].sort();
if (!files.length) throw new Error('No script regression tests were discovered');
console.log(`Running ${files.length} no-build suites on ${process.platform}/${process.arch}`);
const result = spawnSync(process.execPath, ['--test', '--test-concurrency=2', ...files], { cwd: fileURLToPath(root), stdio: 'inherit' });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
