import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/storageAllocation.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022,
} }).outputText;
const exports = {};
vm.runInNewContext(compiled, { exports });
const { storageAllocationLimit, isValidStorageAllocation } = exports;

test('selected instance capacity is floored and capped, never rounded up', () => {
  assert.equal(storageAllocationLimit(80.9), 80);
  assert.equal(storageAllocationLimit(20000), 10000);
  for (const value of [NaN, Infinity, -1]) assert.equal(storageAllocationLimit(value), 0);
});

test('allocation requires a whole number between 30 GB and the selected card limit', () => {
  for (const input of ['', ' ', 'NaN', 'Infinity', '-1', '0', '29', '80.5', '81']) {
    assert.equal(isValidStorageAllocation(input, 80), false, input);
  }
  for (const input of ['30', '50', '80']) assert.equal(isValidStorageAllocation(input, 80), true);
  assert.equal(isValidStorageAllocation('30', 29), false);
});
