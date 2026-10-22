#!/usr/bin/env python3
"""Offline hardware-input JVM tests; no Gradle, device, network or whole build.

Compiles the COMPLETE production StreamHardwareInput.kt against an installed android.jar.
Only the pure state/math engine executes (Android SDK methods are not mocked or invoked).
GamepadState is extracted verbatim from VirtualGamepadView.kt, never redefined by tests.
Uses kotlinc/KOTLINC or the existing Gradle compiler cache. Outputs are temporary.
"""
import glob
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
STREAM = HERE.parent / "app/src/main/java/com/noland/connect/stream"
CACHE = Path.home() / ".gradle/caches/modules-2/files-2.1"


def cached(group, artifact, version="*"):
    return sorted(glob.glob(str(CACHE / group / artifact / version / "*/*.jar")))


def android_jar():
    explicit = os.environ.get("ANDROID_JAR")
    if explicit:
        if not Path(explicit).is_file():
            raise SystemExit("ANDROID_JAR is not an existing file.")
        return explicit
    roots = [os.environ.get("ANDROID_HOME"), os.environ.get("ANDROID_SDK_ROOT"),
             str(Path.home() / "Library/Android/sdk"), str(Path.home() / "Android/Sdk")]
    candidates = [jar for root in roots if root
                  for jar in Path(root).glob("platforms/android-*/android.jar")
                  if jar.parent.name.removeprefix("android-").isdigit()]
    if not candidates:
        raise SystemExit("An installed Android SDK is required; set ANDROID_JAR or ANDROID_HOME. No downloads attempted.")
    return str(max(candidates, key=lambda jar: int(jar.parent.name.removeprefix("android-"))))


def main():
    java = str(Path(os.environ["JAVA_HOME"]) / "bin/java") if os.environ.get("JAVA_HOME") else shutil.which("java")
    if not java:
        raise SystemExit("An existing JDK is required.")
    sdk = android_jar()
    kotlinc = os.environ.get("KOTLINC") or shutil.which("kotlinc")
    if kotlinc:
        stdlib = Path(kotlinc).resolve().parent.parent / "lib/kotlin-stdlib.jar"
        if not stdlib.is_file():
            raise SystemExit("Cannot locate kotlin-stdlib.jar next to kotlinc; use a standard distribution.")
        compiler_command = [kotlinc]
        libraries = [str(stdlib)]
    else:
        compilers = cached("org.jetbrains.kotlin", "kotlin-compiler-embeddable")
        compilers.sort(key=lambda p: tuple(int(n) for n in Path(p).parents[1].name.split(".") if n.isdigit()))
        if not compilers:
            raise SystemExit("No kotlinc or cached Kotlin compiler; install Kotlin to run. No downloads attempted.")
        compiler = compilers[-1]
        version = Path(compiler).parents[1].name
        stdlibs = cached("org.jetbrains.kotlin", "kotlin-stdlib", version)
        if not stdlibs:
            raise SystemExit("Cached Kotlin compiler requires a matching cached stdlib.")
        dependencies = [compiler, stdlibs[0]]
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
        compiler_command = [java, "-Xmx256m", "-cp", os.pathsep.join(dependencies),
                            "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler"]
        libraries = [stdlibs[0]] + cached("org.jetbrains", "annotations")
        print("Using cached Kotlin " + version + " (no downloads)", flush=True)
    source = (STREAM / "VirtualGamepadView.kt").read_text()
    state = re.search(r"(?ms)^data class GamepadState\(.*?^}", source)
    if state is None:
        raise SystemExit("Could not find the production GamepadState declaration; update this runner's extraction.")
    with tempfile.TemporaryDirectory(prefix="noland-hardware-pure-") as temp:
        state_file = Path(temp) / "GamepadState.kt"
        state_file.write_text("package com.noland.connect.stream\n\n" + state.group() + "\n")
        output = str(Path(temp) / "tests.jar")
        classpath = os.pathsep.join(libraries + [sdk])
        subprocess.run(compiler_command + [
            "-no-stdlib", "-no-reflect", "-classpath", classpath,
            str(STREAM / "StreamHardwareInput.kt"), str(state_file),
            str(HERE / "StreamHardwareInputPureTest.kt"), "-d", output,
        ], check=True, timeout=90)
        subprocess.run([java, "-cp", os.pathsep.join([output, classpath]),
                        "com.noland.connect.stream.StreamHardwareInputPureTestKt"], check=True, timeout=30)


if __name__ == "__main__":
    main()
