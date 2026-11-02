#!/usr/bin/env node
// Scoped, syntax-only copy migration. Existing translations remain untouched.
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import ts from "typescript";
const root = new URL("../", import.meta.url);
const write = process.argv.includes("--write");
const paths = [...readdirSync(new URL("src/features/feedback/", root)).filter((name) => name.endsWith(".tsx")).map((name) => `src/features/feedback/${name}`), "src/features/dashboard/DashboardScreen.tsx", "src/features/servers/ServerPickerModal.tsx", "src/features/settings/SettingsScreen.tsx"];
const english = JSON.parse(readFileSync(new URL("src/locales/en-US.json", root), "utf8"));
const copy = new Map();
const attributes = new Set(["title", "placeholder", "aria-label", "alt", "data-text", "loadingText", "label", "description", "topic"]);
const setters = new Set(["setError", "setNotice", "setActionMessage"]);
const visible = (text) => /[A-Za-zÀ-ÿ]/u.test(text);
const normalized = (text) => text.replace(/&gt;/g, ">").replace(/&lt;/g, "<").replace(/&amp;/g, "&").replace(/&quot;/g, '"').replace(/\s+/g, " ").trim();
function keyFor(message) {
  const existing = Object.entries(english).find(([, value]) => value === message)?.[0];
  if (existing) return existing;
  const key = `feedback.${createHash("sha256").update(message).digest("hex").slice(0, 16)}`;
  copy.set(key, message);
  return key;
}
let changed = 0;
for (const path of paths) {
  let text = readFileSync(new URL(path, root), "utf8");
  const ast = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const edits = [];
  function replaceString(node, jsx = false) {
    const message = normalized(node.text);
    if (!visible(message)) return;
    const expression = `translate(${JSON.stringify(keyFor(message))})`;
    edits.push([node.getStart(ast), node.getEnd(), jsx ? `{${expression}}` : expression]);
  }
  function visit(node) {
    if (ts.isJsxText(node)) {
      const raw = node.getText(ast);
      const message = normalized(raw);
      if (visible(message)) edits.push([node.getStart(ast), node.getEnd(), `${raw.match(/^\s*/u)?.[0] ?? ""}{translate(${JSON.stringify(keyFor(message))})}${raw.match(/\s*$/u)?.[0] ?? ""}`]);
    } else if (ts.isJsxAttribute(node) && attributes.has(node.name.getText(ast)) && node.initializer) {
      if (ts.isStringLiteral(node.initializer)) { replaceString(node.initializer, true); return; }
      if (ts.isJsxExpression(node.initializer) && node.initializer.expression && ts.isTemplateExpression(node.initializer.expression)) {
        const template = node.initializer.expression;
        let message = template.head.text;
        const values = [];
        template.templateSpans.forEach((span, index) => { message += `{value${index}}${span.literal.text}`; values.push(`value${index}: ${span.expression.getText(ast)}`); });
        edits.push([template.getStart(ast), template.getEnd(), `translate(${JSON.stringify(keyFor(message))}, { ${values.join(", ")} })`]);
        return;
      }
    } else if (ts.isCallExpression(node) && setters.has(node.expression.getText(ast)) && node.arguments[0] && ts.isStringLiteral(node.arguments[0])) {
      replaceString(node.arguments[0]);
      return;
    } else if (ts.isStringLiteral(node) && ts.isConditionalExpression(node.parent) && (node.parent.whenTrue === node || node.parent.whenFalse === node)) {
      let parent = node.parent;
      while (parent && !ts.isJsxExpression(parent) && !ts.isJsxAttribute(parent)) parent = parent.parent;
      const attribute = parent && ts.isJsxExpression(parent) && ts.isJsxAttribute(parent.parent) ? parent.parent : parent && ts.isJsxAttribute(parent) ? parent : null;
      if (parent && (!attribute || attributes.has(attribute.name.getText(ast)))) replaceString(node);
    }
    ts.forEachChild(node, visit);
  }
  visit(ast);
  if (!edits.length) continue;
  changed += 1;
  for (const [start, end, replacement] of edits.sort((a, b) => b[0] - a[0])) text = text.slice(0, start) + replacement + text.slice(end);
  if (!text.includes('from "../../lib/i18n"')) text = `import { translate } from "../../lib/i18n";\n${text}`;
  if (write) writeFileSync(new URL(path, root), text);
}
if (write) {
  for (const name of readdirSync(new URL("src/locales/", root)).filter((name) => name.endsWith(".json"))) {
    const url = new URL(`src/locales/${name}`, root);
    const messages = JSON.parse(readFileSync(url, "utf8"));
    for (const [key, value] of copy) messages[key] ??= value;
    writeFileSync(url, `${JSON.stringify(messages, null, 2)}\n`);
  }
}
console.log(`${write ? "Localized" : "Would localize"} ${changed} frontend files; ${copy.size} new copy keys with English fallbacks.`);
if (!write && changed) process.exitCode = 1;
