import { cpSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const WIREGUARD_DEPENDENCY = "com.wireguard.android:tunnel:1.0.20260102";
export const DESUGAR_DEPENDENCY = "com.android.tools:desugar_jdk_libs:2.1.5";
const managedWireGuardComment = "// Noland native WireGuard tunnel dependency";
const managedDesugarComment = "// Noland Java 17 core library desugaring dependency";
const managedCompatibilityStart = "// Noland Java 17 compatibility start";
const managedCompatibilityEnd = "// Noland Java 17 compatibility end";

function findTopLevelBlock(source, blockName) {
  let depth = 0;
  let state = "code";

  for (let index = 0; index < source.length; index += 1) {
    const char = source[index];
    const next = source[index + 1];

    if (state === "line-comment") {
      if (char === "\n") state = "code";
      continue;
    }
    if (state === "block-comment") {
      if (char === "*" && next === "/") {
        state = "code";
        index += 1;
      }
      continue;
    }
    if (state === "single-quote" || state === "double-quote") {
      if (char === "\\") {
        index += 1;
      } else if (
        (state === "single-quote" && char === "'") ||
        (state === "double-quote" && char === '"')
      ) {
        state = "code";
      }
      continue;
    }
    if (char === "/" && next === "/") {
      state = "line-comment";
      index += 1;
      continue;
    }
    if (char === "/" && next === "*") {
      state = "block-comment";
      index += 1;
      continue;
    }
    if (char === "'") {
      state = "single-quote";
      continue;
    }
    if (char === '"') {
      state = "double-quote";
      continue;
    }
    if (char === "{") {
      depth += 1;
      continue;
    }
    if (char === "}") {
      depth -= 1;
      continue;
    }
    if (depth !== 0 || !source.startsWith(blockName, index)) continue;

    const before = source[index - 1];
    const after = source[index + blockName.length];
    if ((before && /[\w$]/u.test(before)) || (after && /[\w$]/u.test(after))) continue;

    let openBrace = index + blockName.length;
    while (/\s/u.test(source[openBrace] ?? "")) openBrace += 1;
    if (source[openBrace] !== "{") continue;

    let blockDepth = 1;
    let blockState = "code";
    for (let cursor = openBrace + 1; cursor < source.length; cursor += 1) {
      const blockChar = source[cursor];
      const blockNext = source[cursor + 1];
      if (blockState === "line-comment") {
        if (blockChar === "\n") blockState = "code";
        continue;
      }
      if (blockState === "block-comment") {
        if (blockChar === "*" && blockNext === "/") {
          blockState = "code";
          cursor += 1;
        }
        continue;
      }
      if (blockState === "single-quote" || blockState === "double-quote") {
        if (blockChar === "\\") {
          cursor += 1;
        } else if (
          (blockState === "single-quote" && blockChar === "'") ||
          (blockState === "double-quote" && blockChar === '"')
        ) {
          blockState = "code";
        }
        continue;
      }
      if (blockChar === "/" && blockNext === "/") {
        blockState = "line-comment";
        cursor += 1;
      } else if (blockChar === "/" && blockNext === "*") {
        blockState = "block-comment";
        cursor += 1;
      } else if (blockChar === "'") {
        blockState = "single-quote";
      } else if (blockChar === '"') {
        blockState = "double-quote";
      } else if (blockChar === "{") {
        blockDepth += 1;
      } else if (blockChar === "}" && --blockDepth === 0) {
        return { openBrace, closeBrace: cursor };
      }
    }
    throw new Error(`The generated app Gradle ${blockName} block is not balanced.`);
  }

  throw new Error(`The generated app Gradle file has no top-level ${blockName} block.`);
}

function injectManagedLines(source, blockName, lines) {
  const lineEnding = source.includes("\r\n") ? "\r\n" : "\n";
  const block = findTopLevelBlock(source, blockName);
  const closeLineStart = source.lastIndexOf("\n", block.closeBrace - 1) + 1;
  const closeIndent = source.slice(closeLineStart, block.closeBrace);
  const closeIsOnOwnLine = /^\s*$/u.test(closeIndent);
  const insertionPoint = closeIsOnOwnLine ? closeLineStart : block.closeBrace;
  const body = source.slice(block.openBrace + 1, block.closeBrace);
  const existingIndent = body.match(/(?:^|\r?\n)([ \\t]+)\S/u)?.[1];
  const indent = existingIndent ?? `${closeIsOnOwnLine ? closeIndent : ""}    `;
  const prefix = insertionPoint > 0 && source[insertionPoint - 1] !== "\n" ? lineEnding : "";
  const injected = `${prefix}${lines.map((line) => `${indent}${line}`).join(lineEnding)}${lineEnding}`;
  return source.slice(0, insertionPoint) + injected + source.slice(insertionPoint);
}

function removeDependency(source, comment, configuration, coordinate) {
  const escapedComment = comment.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
  const escapedCoordinate = coordinate.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
  return source.replace(
    new RegExp(
      `^[ \\t]*(?:${escapedComment}\\r?\\n[ \\t]*)?` +
        `${configuration}[ \\t]*(?:\\([ \\t]*)?["']${escapedCoordinate.split(":").slice(0, 2).join(":")}:[^"']+["'][ \\t]*\\)?[^\\r\\n]*\\r?\\n?`,
      "gmu",
    ),
    "",
  );
}

export function injectWireGuardDependency(source, dialect = "kotlin") {
  if (dialect !== "kotlin" && dialect !== "groovy") {
    throw new Error(`Unsupported generated app Gradle dialect: ${dialect}.`);
  }

  const managedCompatibility = new RegExp(
    `^[ \\t]*${managedCompatibilityStart.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")}\\r?\\n` +
      `[\\s\\S]*?^[ \\t]*${managedCompatibilityEnd.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")}\\r?\\n?`,
    "gmu",
  );
  let configured = source.replace(managedCompatibility, "");
  configured = removeDependency(
    configured,
    managedWireGuardComment,
    "(?:implementation|api)",
    WIREGUARD_DEPENDENCY,
  );
  configured = removeDependency(
    configured,
    managedDesugarComment,
    "coreLibraryDesugaring",
    DESUGAR_DEPENDENCY,
  );
  let hasCompilerOptionsTarget = false;
  configured = configured
    .replace(
      /^([ \t]*)sourceCompatibility(?:[ \t]*=[ \t]*|[ \t]+)JavaVersion\.VERSION_[A-Z0-9_]+[ \t]*$/gmu,
      "$1sourceCompatibility = JavaVersion.VERSION_17",
    )
    .replace(
      /^([ \t]*)targetCompatibility(?:[ \t]*=[ \t]*|[ \t]+)JavaVersion\.VERSION_[A-Z0-9_]+[ \t]*$/gmu,
      "$1targetCompatibility = JavaVersion.VERSION_17",
    )
    .replace(
      /^([ \t]*)jvmTarget[ \t]*=[ \t]*["'][^"']+["'][ \t]*$/gmu,
      '$1jvmTarget = "17"',
    )
    // CLI 2.12 uses typed compilerOptions, whose explicit target takes precedence
    // over the legacy android.kotlinOptions fallback.
    .replace(
      /^([ \t]*jvmTarget[ \t]*=[ \t]*(?:org\.jetbrains\.kotlin\.gradle\.dsl\.)?JvmTarget\.)JVM_[A-Z0-9_]+\b/gmu,
      (_, prefix) => {
        hasCompilerOptionsTarget = true;
        return `${prefix}JVM_17`;
      },
    )
    .replace(
      /jvmTarget\.set\(([^\r\n]*JvmTarget\.)JVM_[A-Z0-9_]+\)/gu,
      (_, target) => {
        hasCompilerOptionsTarget = true;
        return `jvmTarget.set(${target}JVM_17)`;
      },
    )
    .replace(
      /^([ \t]*)isCoreLibraryDesugaringEnabled[ \t]*=[ \t]*(?:true|false)[ \t]*$/gmu,
      "$1isCoreLibraryDesugaringEnabled = true",
    )
    .replace(
      /^([ \t]*)coreLibraryDesugaringEnabled(?:[ \t]*=[ \t]*|[ \t]+)(?:true|false)[ \t]*$/gmu,
      "$1coreLibraryDesugaringEnabled true",
    );

  const coreDesugaringSetting =
    dialect === "kotlin"
      ? "    isCoreLibraryDesugaringEnabled = true"
      : "    coreLibraryDesugaringEnabled true";
  configured = injectManagedLines(configured, "android", [
    managedCompatibilityStart,
    "compileOptions {",
    "    sourceCompatibility = JavaVersion.VERSION_17",
    "    targetCompatibility = JavaVersion.VERSION_17",
    coreDesugaringSetting,
    "}",
    ...(hasCompilerOptionsTarget ? [] : [
      "kotlinOptions {",
      '    jvmTarget = "17"',
      "}",
    ]),
    managedCompatibilityEnd,
  ]);
  return injectManagedLines(configured, "dependencies", [
    managedWireGuardComment,
    `implementation("${WIREGUARD_DEPENDENCY}")`,
    managedDesugarComment,
    `coreLibraryDesugaring("${DESUGAR_DEPENDENCY}")`,
  ]);
}

function configureAndroidProject() {
  const root = fileURLToPath(new URL("../", import.meta.url));
  const overlay = join(root, "src-tauri", "android");
  const generated = join(root, "src-tauri", "gen", "android");

  if (!existsSync(generated)) {
    throw new Error(
      "Generated Android project not found at src-tauri/gen/android. Run `npm run tauri:android:init` first.",
    );
  }

  if (!existsSync(overlay)) {
    console.log("No maintained src-tauri/android overlay found; generated Android project is unchanged.");
    return;
  }

  cpSync(overlay, generated, {
    recursive: true,
    force: true,
    errorOnExist: false,
  });

  const gradleCandidates = [
    join(generated, "app", "build.gradle.kts"),
    join(generated, "app", "build.gradle"),
  ].filter(existsSync);
  if (gradleCandidates.length !== 1) {
    throw new Error(
      `Expected exactly one generated app Gradle file, found ${gradleCandidates.length}.`,
    );
  }

  const gradlePath = gradleCandidates[0];
  const originalGradle = readFileSync(gradlePath, "utf8");
  const configuredGradle = injectWireGuardDependency(
    originalGradle,
    gradlePath.endsWith(".kts") ? "kotlin" : "groovy",
  );
  if (configuredGradle !== originalGradle) writeFileSync(gradlePath, configuredGradle);

  console.log(
    `Applied the Android overlay and ensured Java/Kotlin JVM 17, core library desugaring, ${WIREGUARD_DEPENDENCY}, and ${DESUGAR_DEPENDENCY} in the generated app Gradle file.`,
  );
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) configureAndroidProject();
