#!/usr/bin/env python3
"""Compile the production release function with recording transport stubs, without an SDK."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

source = Path(__file__).resolve().parents[1] / "src" / "noland_moonlight.c"
text = source.read_text()
start = text.index("nl_result_t nl_release_all_input(")
opening = text.index("{", start)
depth = 1
end = opening + 1
while depth:
    depth += (text[end] == "{") - (text[end] == "}")
    end += 1
production = text[start:end]

fixture = r'''
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>
typedef int nl_result_t;
#define NL_RESULT_OK 0
#define NL_RESULT_INVALID_ARGUMENT -1
#define BUTTON_ACTION_RELEASE 1
#define KEY_ACTION_UP 1
#define LI_TOUCH_EVENT_CANCEL_ALL 1
#define LI_ROT_UNKNOWN 0
struct nl_runtime {
    bool connected;
    bool pressed_mouse_buttons[6];
    bool pressed_keys[256];
    uint16_t active_gamepad_mask;
};
typedef struct nl_runtime nl_runtime_t;
static int pad_calls, key_calls, mouse_calls, touch_calls;
static unsigned masks[16];
static bool nl_runtime_can_send_input(nl_runtime_t* r) { return r->connected; }
static int LiSendMouseButtonEvent(int action, int button) {
    assert(action == BUTTON_ACTION_RELEASE && button == 1); mouse_calls++; return 0;
}
static int LiSendKeyboardEvent(short key, int action, int modifiers) {
    assert(key == 65 && action == KEY_ACTION_UP && modifiers == 0); key_calls++; return 0;
}
static int LiSendMultiControllerEvent(short slot, short mask, int buttons,
    int lt, int rt, int lx, int ly, int rx, int ry) {
    assert(slot == 0 || slot == 2);
    assert(!(buttons || lt || rt || lx || ly || rx || ry));
    masks[pad_calls++] = (uint16_t)mask; return 0;
}
static int LiSendTouchEvent(int event, int id, int x, int y, int p, int a, int b, int rotation) {
    assert(event == LI_TOUCH_EVENT_CANCEL_ALL); touch_calls++; return 0;
}
'''
checks = r'''
int main(void) {
    assert(nl_release_all_input(NULL) == NL_RESULT_INVALID_ARGUMENT);
    nl_runtime_t runtime = {0};
    runtime.connected = true;
    runtime.active_gamepad_mask = 5;
    runtime.pressed_keys[65] = true;
    runtime.pressed_mouse_buttons[1] = true;
    assert(nl_release_all_input(&runtime) == NL_RESULT_OK);
    assert(key_calls == 1 && mouse_calls == 1 && touch_calls == 1 && pad_calls == 2);
    assert(!runtime.pressed_keys[65] && !runtime.pressed_mouse_buttons[1]);
#if defined(__ANDROID__) || defined(NOLAND_IOS_RENDERER)
    assert(masks[0] == 5 && masks[1] == 5 && runtime.active_gamepad_mask == 5);
#else
    assert(masks[0] == 0 && masks[1] == 0 && runtime.active_gamepad_mask == 0);
#endif
    int previous = pad_calls;
    runtime.connected = false;
    assert(nl_release_all_input(&runtime) == NL_RESULT_OK);
    assert(previous == pad_calls && key_calls == 1 && mouse_calls == 1 && touch_calls == 1);
    return 0;
}
'''
compiler = os.environ.get("CC") or shutil.which("clang") or shutil.which("cc")
if not compiler:
    raise SystemExit("A C compiler is required (CC, clang, or cc)")
with tempfile.TemporaryDirectory(prefix="noland-input-release-") as directory:
    root = Path(directory)
    test = root / "release.c"
    test.write_text(fixture + production + checks)
    for name, defines in (("android", ["-D__ANDROID__"]), ("ios", ["-DNOLAND_IOS_RENDERER"]), ("desktop", [])):
        binary = root / name
        subprocess.run([compiler, "-std=c11", "-Wall", "-Wextra", "-Wno-unused-parameter", "-Werror",
                        *defines, str(test), "-o", str(binary)], check=True)
        subprocess.run([str(binary)], check=True)
        print(f"PASS {name}: production input release balances held controls and preserves platform topology")
