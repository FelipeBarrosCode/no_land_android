#!/usr/bin/env python3
"""Host-only MediaCodec fault-injection regressions; no SDK, app build, or device needed."""

import argparse
import os
from pathlib import Path
import shlex
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sanitize", action="store_true", help="Enable AddressSanitizer and UBSan")
    parser.add_argument("--repeat", type=int, default=1, help="Repeat lifecycle/concurrency regressions")
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error("--repeat must be positive")

    tests = Path(__file__).resolve().parent
    module = tests.parent
    with tempfile.TemporaryDirectory(prefix="noland-video-tests-") as temporary:
        executable = Path(temporary) / "android_video_renderer_test"
        command = shlex.split(os.environ.get("CC", "cc")) + [
            "-std=c11", "-Wall", "-Wextra", "-Werror", "-g", "-O1", "-pthread",
            "-D_POSIX_C_SOURCE=200809L", "-DNOLAND_ANDROID_VIDEO_TEST",
            "-I" + str(module / "include"),
            "-I" + str(module.parent / "moonlight-common-c" / "src"),
        ]
        if args.sanitize:
            command += ["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]
        command += [str(tests / "android_video_renderer_test.c"), "-o", str(executable)]
        subprocess.run(command, check=True)
        for _ in range(args.repeat):
            subprocess.run([str(executable)], check=True, timeout=10)


if __name__ == "__main__":
    main()
