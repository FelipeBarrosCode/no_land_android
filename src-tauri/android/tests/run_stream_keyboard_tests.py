#!/usr/bin/env python3
"""Offline keyboard-only validation; no Gradle, downloads, or gesture harness edits.

Compiles the complete View against the installed Android SDK, then extracts its pure
bookkeeping class for device-free JVM tests. Android framework/IME lifecycle behavior
requires a device; android.jar only provides compile-time stubs.
"""
import glob
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "app/src/main/java/com/noland/connect/stream/StreamKeyboardView.kt"
CACHE = Path.home() / ".gradle/caches/modules-2/files-2.1"


def cached(group, artifact, version="*"):
    return sorted(glob.glob(str(CACHE / group / artifact / version / "*/*.jar")))


def main():
    java = str(Path(os.environ["JAVA_HOME"]) / "bin/java") if os.environ.get("JAVA_HOME") else shutil.which("java")
    if not java:
        raise SystemExit("A JDK is required.")
    kotlinc = os.environ.get("KOTLINC") or shutil.which("kotlinc")
    if kotlinc:
        stdlib = Path(kotlinc).resolve().parent.parent / "lib/kotlin-stdlib.jar"
        if not stdlib.is_file():
            raise SystemExit("Cannot locate kotlin-stdlib.jar next to kotlinc.")
        compiler = [kotlinc]
        libraries = [str(stdlib)]
    else:
        compilers = cached("org.jetbrains.kotlin", "kotlin-compiler-embeddable")
        compilers.sort(key=lambda p: tuple(int(n) for n in Path(p).parents[1].name.split(".") if n.isdigit()))
        if not compilers:
            raise SystemExit("Install kotlinc or populate an existing Gradle Kotlin compiler cache.")
        version = Path(compilers[-1]).parents[1].name
        stdlibs = cached("org.jetbrains.kotlin", "kotlin-stdlib", version)
        if not stdlibs:
            raise SystemExit("No matching cached Kotlin stdlib.")
        dependencies = [compilers[-1], stdlibs[0]]
        for group, artifact in [
            ("org.jetbrains.kotlin", "kotlin-script-runtime"),
            ("org.jetbrains.kotlin", "kotlin-daemon-embeddable"),
            ("org.jetbrains.kotlin", "kotlin-reflect"),
            ("org.jetbrains.intellij.deps", "trove4j"),
            ("org.jetbrains.kotlinx", "kotlinx-coroutines-core-jvm"),
            ("org.jetbrains", "annotations"),
        ]:
            matches = cached(group, artifact, version) or cached(group, artifact)
            if matches:
                dependencies.append(matches[-1])
        compiler = [java, "-Xmx256m", "-cp", os.pathsep.join(dependencies), "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler"]
        libraries = [stdlibs[0]] + cached("org.jetbrains", "annotations")
        print("Using cached Kotlin " + version + " (no downloads)", flush=True)

    sdk = Path(os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT") or
               Path.home() / "Library/Android/sdk")
    platforms = list((sdk / "platforms").glob("android-*/android.jar"))
    platforms.sort(key=lambda p: int(p.parent.name.split("-")[-1]))
    if not platforms:
        raise SystemExit("Set ANDROID_HOME to an installed Android SDK (API 30+).")
    flags = ["-no-stdlib", "-no-reflect", "-jvm-target", "1.8"]
    with tempfile.TemporaryDirectory(prefix="noland-keyboard-tests-") as temporary:
        output = Path(temporary)
        subprocess.run(compiler + flags + ["-classpath", os.pathsep.join(libraries + [str(platforms[-1])]),
                       str(SOURCE), "-d", str(output / "view.jar")], check=True)
        print("Complete StreamKeyboardView compiled against " + platforms[-1].parent.name, flush=True)
        source = SOURCE.read_text()
        pure = output / "StreamKeyboardCommits.kt"
        pure.write_text("package com.noland.connect.stream\n\n" +
                        source[source.index("internal class StreamKeyboardCommits {"):])
        tests = output / "tests.jar"
        subprocess.run(compiler + flags + ["-classpath", os.pathsep.join(libraries), str(pure),
                       str(HERE / "StreamKeyboardCommitsTest.kt"), "-d", str(tests)], check=True)
        subprocess.run([java, "-cp", os.pathsep.join([str(tests)] + libraries),
                        "com.noland.connect.stream.StreamKeyboardCommitsTestKt"], check=True)


if __name__ == "__main__":
    main()
