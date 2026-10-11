Linux packages use the distribution OpenSSH client through staged wrapper binaries.

macOS and Windows release builds may populate this directory with the runtime
libraries required by their managed OpenSSH binaries. Keep this tracked file so
the Tauri resource glob remains valid in clean checkouts and test builds.
