#!/usr/bin/env python3
"""Device-free, offline Kotlin regression runner. Never invokes Gradle or downloads.

Uses kotlinc on PATH (or KOTLINC), otherwise an existing Gradle compiler cache.
All generated classes live in a temporary directory removed on exit.
"""
import glob
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


HERE = Path(__file__).resolve().parent
STREAM = HERE.parent / "app/src/main/java/com/noland/connect/stream"
CACHE = Path.home() / ".gradle/caches/modules-2/files-2.1"


def cached(group, artifact, version="*"):
    return sorted(glob.glob(str(CACHE / group / artifact / version / "*/*.jar")))


def main():
    java = str(Path(os.environ["JAVA_HOME"]) / "bin/java") if os.environ.get("JAVA_HOME") else shutil.which("java")
    if not java:
        raise SystemExit("A JDK and kotlinc (or a cached Kotlin compiler) are required.")
    kotlinc = os.environ.get("KOTLINC") or shutil.which("kotlinc")
    if kotlinc:
        stdlib = Path(kotlinc).resolve().parent.parent / "lib/kotlin-stdlib.jar"
        if not stdlib.is_file():
            raise SystemExit("Cannot locate kotlin-stdlib.jar next to kotlinc; use a standard Kotlin distribution.")
        compile_command = [kotlinc]
        libraries = [str(stdlib)]
    else:
        compilers = cached("org.jetbrains.kotlin", "kotlin-compiler-embeddable")
        # Numeric ordering avoids treating 1.9 as newer than 1.10 or 2.0.
        compilers.sort(key=lambda p: tuple(int(n) for n in Path(p).parents[1].name.split(".") if n.isdigit()))
        if not compilers:
            raise SystemExit("kotlinc not found and no cached compiler available. Install Kotlin, then rerun this script.")
        compiler = compilers[-1]
        version = Path(compiler).parents[1].name
        stdlibs = cached("org.jetbrains.kotlin", "kotlin-stdlib", version)
        if not stdlibs:
            raise SystemExit("Cached compiler has no matching stdlib; install kotlinc to run offline.")
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
        compile_command = [java, "-Xmx256m", "-cp", os.pathsep.join(dependencies), "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler"]
        libraries = [stdlibs[0]] + cached("org.jetbrains", "annotations")
        print("Using cached Kotlin " + version + " (no downloads)", flush=True)
    with tempfile.TemporaryDirectory(prefix="noland-touch-tests-") as output:
        jar = str(Path(output) / "tests.jar")
        subprocess.run(compile_command + [
            "-no-stdlib", "-no-reflect", "-classpath", os.pathsep.join(libraries),
            str(STREAM / "StreamTouchController.kt"), str(HERE / "StreamTouchControllerTest.kt"), "-d", jar,
        ], check=True)
        subprocess.run([java, "-cp", os.pathsep.join([jar] + libraries),
                        "com.noland.connect.stream.StreamTouchControllerTestKt"], check=True)


if __name__ == "__main__":
    main()
