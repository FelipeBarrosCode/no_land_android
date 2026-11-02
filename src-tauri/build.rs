use sha2::{Digest, Sha256};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

fn native_dep_prefixes(target: &str, manifest_dir: &std::path::Path) -> Vec<PathBuf> {
    let mut prefixes = Vec::new();

    if let Ok(prefix) = env::var("NOLAND_NATIVE_DEPS_PREFIX") {
        if !prefix.trim().is_empty() {
            prefixes.push(PathBuf::from(prefix));
        }
    }

    prefixes.push(manifest_dir.join(".native-deps").join(target));
    prefixes
}

fn join_existing<I>(paths: I, separator: &str) -> Option<String>
where
    I: IntoIterator<Item = PathBuf>,
{
    let paths = paths
        .into_iter()
        .filter(|path| path.exists())
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();

    if paths.is_empty() {
        None
    } else {
        Some(paths.join(separator))
    }
}

fn native_env(name: &str) -> Option<std::ffi::OsString> {
    println!("cargo:rerun-if-env-changed={name}");
    env::var_os(name).filter(|value| !value.is_empty())
}

fn android_sdk_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for name in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(path) = native_env(name) {
            roots.push(PathBuf::from(path));
        }
    }
    if let Some(home) = native_env("HOME") {
        let home = PathBuf::from(home);
        if cfg!(target_os = "macos") {
            roots.push(home.join("Library/Android/sdk"));
        } else {
            roots.push(home.join("Android/Sdk"));
        }
    }
    if let Some(local_app_data) = native_env("LOCALAPPDATA") {
        roots.push(PathBuf::from(local_app_data).join("Android/Sdk"));
    }
    roots
}

// SDK package versions must sort numerically (e.g. NDK 29 after NDK 9).
fn android_package_dirs(root: &Path) -> Vec<PathBuf> {
    if root.is_dir() {
        println!("cargo:rerun-if-changed={}", root.display());
    }
    let mut dirs = fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    dirs.sort_by_cached_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .split(['.', '-'])
            .filter_map(|part| part.parse::<u32>().ok())
            .collect::<Vec<_>>()
    });
    dirs.reverse();
    dirs
}

fn native_target_env(name: &str, target: &str) -> Option<std::ffi::OsString> {
    native_env(&format!("{name}_{target}"))
        .or_else(|| native_env(&format!("{name}_{}", target.replace('-', "_"))))
        .or_else(|| native_env(&format!("TARGET_{name}")))
        .or_else(|| native_env(name))
}

fn executable_on_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|dir| dir.join(format!("{name}{}", env::consts::EXE_SUFFIX)))
            .find(|path| path.is_file())
    })
}

fn configure_android_cmake(config: &mut cmake::Config, target: &str) {
    let abi = match target {
        "aarch64-linux-android" => "arm64-v8a",
        "armv7-linux-androideabi" | "thumbv7neon-linux-androideabi" => "armeabi-v7a",
        "i686-linux-android" => "x86",
        "x86_64-linux-android" => "x86_64",
        _ => panic!("unsupported Android native target: {target}"),
    };
    let mut sdk_roots = android_sdk_roots();
    let ndk = [
        "ANDROID_NDK_HOME",
        "NDK_HOME", // Set by Tauri/cargo-mobile when invoking Cargo.
        "ANDROID_NDK_ROOT",
        "NDK_ROOT",
        "TAURI_ANDROID_NDK_HOME",
        "TAURI_ANDROID_NDK_ROOT",
        "CMAKE_ANDROID_NDK",
    ]
    .into_iter()
    .find_map(|name| native_env(name).map(PathBuf::from))
    .or_else(|| {
        sdk_roots.iter().find_map(|sdk| {
            android_package_dirs(&sdk.join("ndk"))
                .into_iter()
                .chain(std::iter::once(sdk.join("ndk-bundle")))
                .find(|ndk| ndk.join("build/cmake/android.toolchain.cmake").is_file())
        })
    })
    .expect("Android NDK not found: set ANDROID_NDK_HOME or NDK_HOME to an installed NDK");
    let toolchain = ndk.join("build/cmake/android.toolchain.cmake");
    assert!(
        toolchain.is_file(),
        "invalid Android NDK {}: missing build/cmake/android.toolchain.cmake",
        ndk.display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        ndk.join("source.properties").display()
    );
    if let Some(packages) = ndk.parent().filter(|path| path.ends_with("ndk")) {
        if let Some(sdk) = packages.parent() {
            sdk_roots.push(sdk.to_path_buf());
        }
    }

    // Respect cmake-rs's target-specific overrides, then discover an SDK CMake
    // installation or PATH executable. No developer-specific SDK/version paths.
    let cmake = native_target_env("CMAKE", target)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            sdk_roots
                .iter()
                .flat_map(|sdk| android_package_dirs(&sdk.join("cmake")))
                .map(|dir| dir.join(format!("bin/cmake{}", env::consts::EXE_SUFFIX)))
                .find(|path| path.is_file())
                .or_else(|| executable_on_path("cmake"))
                .expect("CMake not found: install the Android SDK CMake package or set CMAKE")
        });
    // cmake-rs selects its executable from the process environment, not Config::env.
    env::set_var("CMAKE", &cmake);
    let generator = native_target_env("CMAKE_GENERATOR", target);
    if generator.as_ref().map_or(true, |value| value == "Ninja") {
        let ninja = cmake
            .parent()
            .map(|dir| dir.join(format!("ninja{}", env::consts::EXE_SUFFIX)))
            .filter(|path| path.is_file())
            .or_else(|| executable_on_path("ninja"))
            .expect("Ninja not found: install the Android SDK CMake package or add ninja to PATH");
        config
            .generator("Ninja")
            .define("CMAKE_MAKE_PROGRAM", ninja);
    }
    // The installed/compile SDK level is not the minimum native runtime API.
    let platform = native_env("ANDROID_PLATFORM")
        .or_else(|| native_env("NDK_API_LEVEL"))
        .unwrap_or_else(|| "android-24".into());
    config
        .define("CMAKE_TOOLCHAIN_FILE", toolchain)
        .define("ANDROID_ABI", abi)
        .define("ANDROID_PLATFORM", platform)
        .define("ANDROID_STL", "c++_static")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        // Avoid building/installing unrelated MbedTLS TLS/X.509 libraries.
        .build_target("noland_moonlight");
}

fn link_android_compiler_runtime(target: &str) {
    // Rust uses -nodefaultlibs, so Clang does not automatically supply the CPU
    // detection state required by Moonlight's x86 __builtin_cpu_supports calls.
    let compiler = cc::Build::new().target(target).get_compiler();
    let output = compiler
        .to_command()
        .arg("--print-libgcc-file-name")
        .output()
        .expect("failed to query the Android compiler runtime archive");
    assert!(
        output.status.success(),
        "Android compiler runtime query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let archive = PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("Android compiler runtime path is not UTF-8")
            .trim(),
    );
    assert!(
        archive.is_file(),
        "Android compiler runtime archive does not exist: {}",
        archive.display()
    );
    let library = archive
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("lib"))
        .and_then(|name| name.strip_suffix(".a"))
        .expect("Android compiler runtime must be a static lib*.a archive");
    println!(
        "cargo:rustc-link-search=native={}",
        archive.parent().unwrap().display()
    );
    println!("cargo:rustc-link-lib=static={library}");
}

fn main() {
    println!("cargo:rustc-check-cfg=cfg(noland_feedback_native_android)");
    println!("cargo:rerun-if-env-changed=NOLAND_FEEDBACK_API_URL");
    println!("cargo:rerun-if-env-changed=NOLAND_SKIP_APP_BUILD_RS");
    println!("cargo:rerun-if-env-changed=NOLAND_SKIP_NATIVE_BUILD");

    if env::var("NOLAND_SKIP_APP_BUILD_RS").ok().as_deref() == Some("1") {
        return;
    }

    let target = env::var("TARGET").expect("TARGET is not set");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let is_android = target_os == "android";
    let is_ios = target_os == "ios";
    let is_macos = target_os == "macos";
    let is_linux = target_os == "linux";
    let is_windows = target_os == "windows";
    let is_desktop = is_macos || is_linux || is_windows;

    if is_android {
        embed_android_remote_agent_sources();
    }

    if is_ios {
        println!("cargo:rerun-if-changed=apple/NolandPlatform.m");
        cc::Build::new()
            .file("apple/NolandPlatform.m")
            .flag("-fobjc-arc")
            .compile("noland_ios_platform");
        println!("cargo:rustc-link-lib=framework=UIKit");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=z");
        // Swift bridge functions are resolved when Xcode links the final app.
        println!("cargo:rustc-link-arg-cdylib=-Wl,-undefined,dynamic_lookup");
    }

    if matches!(env::var("NOLAND_SKIP_NATIVE_BUILD").as_deref(), Ok("1")) {
        if target.ends_with("apple-darwin") {
            cc::Build::new()
                .file("src/moonlight/platform/macos_display_detect.m")
                .flag("-fobjc-arc")
                .compile("noland_macos_display_detect");
            println!("cargo:rustc-link-lib=framework=AppKit");
            println!("cargo:rustc-link-lib=framework=ApplicationServices");
        }
        write_static_noland_moonlight_bindings();
        tauri_build::build();
        return;
    }

    if is_desktop {
        prepare_gstreamer_bundle(&target).expect("failed to prepare bundled GStreamer runtime");
        ensure_managed_sidecar_bundle_artifacts()
            .expect("failed to prepare managed tool sidecar bundle artifacts");
    }

    let native_root = PathBuf::from("native");
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"));
    let wrapper_header = native_root.join("noland-moonlight/include/noland_moonlight.h");

    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        native_root.join("CMakeLists.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/CMakeLists.txt")
            .display()
    );
    println!("cargo:rerun-if-changed={}", wrapper_header.display());
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_moonlight.c")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_controller_manager.c")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_controller_manager.h")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_video_renderer.c")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_video_renderer.h")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_latency_telemetry.c")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_latency_telemetry.h")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_frame_deadline_policy.c")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_frame_deadline_policy.h")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("moonlight-common-c/src/RtpVideoQueue.c")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_audio_renderer.h")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_video_renderer_macos.m")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_video_renderer_linux.c")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_video_renderer_windows.cpp")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_desktop_input_sdl.c")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_audio_renderer_sdl.c")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native_root
            .join("noland-moonlight/src/noland_audio_renderer_macos.m")
            .display()
    );
    for source in [
        "noland-moonlight/src/noland_video_renderer_ios.m",
        "noland-moonlight/src/noland_stream_controls_ios.m",
        "noland-moonlight/src/noland_stream_controls_ios.h",
        "noland-moonlight/src/noland_keyboard_ios.h",
        "noland-moonlight/src/noland_audio_renderer_ios.m",
        "noland-moonlight/src/noland_microphone_ios.m",
        "noland-moonlight/src/noland_input_ios.c",
        "noland-moonlight/src/noland_video_renderer_android.c",
        "noland-moonlight/src/noland_audio_renderer_android.c",
        "noland-moonlight/src/noland_input_android.c",
        "moonlight-common-c/CMakeLists.txt",
    ] {
        println!(
            "cargo:rerun-if-changed={}",
            native_root.join(source).display()
        );
    }

    println!(
        "cargo:rerun-if-changed={}",
        PathBuf::from("src/moonlight/platform/macos_stream_input.m").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        PathBuf::from("src/mic_client/macos_permissions.m").display()
    );

    let mut cmake_config = cmake::Config::new(&native_root);
    cmake_config
        .define("BUILD_NOLAND_MOONLIGHT_HARNESS", "OFF")
        .define("BUILD_NOLAND_MOONLIGHT_TESTS", "OFF");

    if is_android {
        configure_android_cmake(&mut cmake_config, &target);
    }

    if is_macos || is_windows || is_linux {
        if is_macos {
            let arch = if target.starts_with("x86_64-") {
                "x86_64"
            } else if target.starts_with("aarch64-") {
                "arm64"
            } else {
                ""
            };
            if !arch.is_empty() {
                cmake_config.define("CMAKE_OSX_ARCHITECTURES", arch);
            }
            if let Ok(deployment_target) = env::var("MACOSX_DEPLOYMENT_TARGET") {
                if !deployment_target.trim().is_empty() {
                    cmake_config.define("CMAKE_OSX_DEPLOYMENT_TARGET", deployment_target);
                }
            }
        }

        let prefixes = native_dep_prefixes(&target, &manifest_dir);
        if let Some(primary_prefix) = prefixes.iter().find(|prefix| prefix.exists()) {
            cmake_config.define("NOLAND_NATIVE_PREFIX", primary_prefix.display().to_string());
            if is_windows {
                cmake_config.define("OPENSSL_ROOT_DIR", primary_prefix.display().to_string());
            }
        }
        if let Some(prefix_path) = join_existing(prefixes.clone(), ";") {
            cmake_config.define("CMAKE_PREFIX_PATH", prefix_path);
        }
        if let Some(pkg_config_path) = join_existing(
            prefixes
                .iter()
                .flat_map(|prefix| {
                    [
                        prefix.join("lib/pkgconfig"),
                        prefix.join("lib64/pkgconfig"),
                        prefix.join("share/pkgconfig"),
                    ]
                })
                .collect::<Vec<_>>(),
            ":",
        ) {
            let merged = match env::var("PKG_CONFIG_PATH") {
                Ok(existing) if !existing.trim().is_empty() => {
                    format!("{pkg_config_path}:{existing}")
                }
                _ => pkg_config_path,
            };
            cmake_config.env("PKG_CONFIG_PATH", merged);
        }
    }

    if is_windows {
        cmake_config.profile("Release");
    }

    let dst = cmake_config.build();

    let lib_dir = dst.join("lib");
    let static_lib_dir = dst.join("lib/static");
    let moonlight_common_lib_dir = dst.join("build/moonlight-common-c");
    let enet_lib_dir = dst.join("build/moonlight-common-c/enet");
    let mobile_opus_lib_dir = dst.join("build/_deps/opus-build");
    let mobile_mbedtls_lib_dir = dst.join("build/_deps/mbedtls-build/library");
    let windows_config = if is_windows { Some("Release") } else { None };
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!(
        "cargo:rustc-link-search=native={}",
        static_lib_dir.display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        moonlight_common_lib_dir.display()
    );
    println!("cargo:rustc-link-search=native={}", enet_lib_dir.display());
    if is_ios || is_android {
        println!(
            "cargo:rustc-link-search=native={}",
            mobile_opus_lib_dir.display()
        );
        println!(
            "cargo:rustc-link-search=native={}",
            mobile_mbedtls_lib_dir.display()
        );
    }
    if is_android {
        println!(
            "cargo:rustc-link-search=native={}",
            dst.join("build/noland-moonlight").display()
        );
    }
    if let Some(config) = windows_config {
        println!(
            "cargo:rustc-link-search=native={}",
            lib_dir.join(config).display()
        );
        println!(
            "cargo:rustc-link-search=native={}",
            static_lib_dir.join(config).display()
        );
        println!(
            "cargo:rustc-link-search=native={}",
            moonlight_common_lib_dir.join(config).display()
        );
        println!(
            "cargo:rustc-link-search=native={}",
            enet_lib_dir.join(config).display()
        );
    }
    println!("cargo:rustc-link-lib=static=noland_moonlight");
    println!("cargo:rustc-link-lib=static=moonlight-common-c");
    println!("cargo:rustc-link-lib=static=enet");
    if is_ios || is_android {
        println!("cargo:rustc-link-lib=static=opus");
        println!("cargo:rustc-link-lib=static=mbedcrypto");
    }
    if is_android {
        if matches!(
            target.as_str(),
            "x86_64-linux-android" | "i686-linux-android"
        ) {
            link_android_compiler_runtime(&target);
        }
        // Only the actual Android native runtime path enables contribution capabilities.
        println!("cargo:rustc-cfg=noland_feedback_native_android");
        for library in ["android", "log", "mediandk", "OpenSLES", "m"] {
            println!("cargo:rustc-link-lib={library}");
        }
    }
    if is_ios {
        println!("cargo:rustc-link-lib=framework=UIKit");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rustc-link-lib=framework=CoreVideo");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
        println!("cargo:rustc-link-lib=framework=GameController");
        println!("cargo:rustc-link-lib=framework=CoreHaptics");
    }
    if is_macos {
        cc::Build::new()
            .file("src/moonlight/platform/macos_stream_input.m")
            .flag("-fobjc-arc")
            .compile("noland_macos_stream_input");
        cc::Build::new()
            .file("src/mic_client/macos_permissions.m")
            .flag("-fobjc-arc")
            .compile("noland_macos_permissions");

        for prefix in native_dep_prefixes(&target, &manifest_dir) {
            println!(
                "cargo:rustc-link-search=native={}",
                prefix.join("opt/openssl@3/lib").display()
            );
            println!(
                "cargo:rustc-link-search=native={}",
                prefix.join("lib").display()
            );
        }

        println!("cargo:rustc-link-lib=dylib=crypto");
        println!("cargo:rustc-link-lib=dylib=opus");
        println!("cargo:rustc-link-lib=dylib=SDL2");
        // Dev binaries stage project-native dylibs beside the executable. Keep
        // the bundle Frameworks rpath added by Tauri and add this deterministic
        // local fallback so libopus/SDL2 survive Frameworks directory refreshes.
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");
        stage_macos_dev_runtime_dylibs(&target, &manifest_dir)
            .expect("failed to stage macOS dev runtime dylibs");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rustc-link-lib=framework=CoreVideo");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
    }
    if is_windows {
        for prefix in native_dep_prefixes(&target, &manifest_dir) {
            println!(
                "cargo:rustc-link-search=native={}",
                prefix.join("lib").display()
            );
        }

        println!("cargo:rustc-link-lib=static=opus");
        println!("cargo:rustc-link-lib=static=SDL2-static");
        for library in [
            "advapi32", "bcrypt", "d3d11", "dinput8", "dxgi", "dxguid", "gdi32", "imm32", "mf",
            "mfplat", "mfuuid", "ole32", "oleaut32", "setupapi", "shell32", "user32", "uuid",
            "version", "winmm",
        ] {
            println!("cargo:rustc-link-lib={library}");
        }
    }
    if is_linux {
        for prefix in native_dep_prefixes(&target, &manifest_dir) {
            println!(
                "cargo:rustc-link-search=native={}",
                prefix.join("lib").display()
            );
            println!(
                "cargo:rustc-link-search=native={}",
                prefix.join("lib64").display()
            );
        }

        println!("cargo:rustc-link-lib=dylib=crypto");
        println!("cargo:rustc-link-lib=static=opus");
        println!("cargo:rustc-link-lib=static=SDL2");
        for library in [
            "dl",
            "m",
            "pthread",
            "rt",
            "gstreamer-1.0",
            "gstapp-1.0",
            "gstvideo-1.0",
            "gobject-2.0",
            "glib-2.0",
        ] {
            println!("cargo:rustc-link-lib={library}");
        }
    }

    write_static_noland_moonlight_bindings();

    tauri_build::build()
}

fn embed_android_remote_agent_sources() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"));

    embed_android_source_archive(
        "state-agent-source.tar.gz",
        &[(
            "state-agent",
            manifest_dir.join("../state-agent"),
            &[
                "Cargo.toml",
                "Cargo.lock",
                "rust-toolchain.toml",
                "crates",
                "scripts",
                "systemd",
            ],
        )],
    );
    embed_android_source_archive(
        "network-agent-source.tar.gz",
        &[
            (
                "network-agent",
                manifest_dir.join("../network-agent"),
                &["Cargo.toml", "Cargo.lock", "src"],
            ),
            (
                "network-contracts",
                manifest_dir.join("../network-contracts"),
                &["Cargo.toml", "Cargo.lock", "src"],
            ),
        ],
    );
    embed_android_source_archive(
        "mic-receiver-source.tar.gz",
        &[(
            "vm-cloud-mic-agent",
            manifest_dir.join("../vm-cloud-mic-agent"),
            &["Cargo.toml", "Cargo.lock", "src"],
        )],
    );
}

fn embed_android_source_archive(output_name: &str, roots: &[(&str, PathBuf, &[&str])]) {
    let output = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is not set")).join(output_name);
    let archive_file = fs::File::create(&output).unwrap_or_else(|error| {
        panic!("failed to create embedded source archive {output_name}: {error}")
    });
    let encoder = flate2::write::GzEncoder::new(archive_file, flate2::Compression::best());
    let mut archive = tar::Builder::new(encoder);

    for (archive_root, source_root, entries) in roots {
        archive
            .append_dir(archive_root, source_root)
            .unwrap_or_else(|error| panic!("failed to archive {archive_root}: {error}"));
        for relative in *entries {
            let source = source_root.join(relative);
            println!("cargo:rerun-if-changed={}", source.display());
            let destination = Path::new(archive_root).join(relative);
            if source.is_dir() {
                archive
                    .append_dir_all(&destination, &source)
                    .unwrap_or_else(|error| {
                        panic!("failed to archive {}: {error}", source.display())
                    });
            } else {
                archive
                    .append_path_with_name(&source, &destination)
                    .unwrap_or_else(|error| {
                        panic!("failed to archive {}: {error}", source.display())
                    });
            }
        }
    }

    archive
        .into_inner()
        .unwrap_or_else(|error| panic!("failed to finish {output_name} tar archive: {error}"))
        .finish()
        .unwrap_or_else(|error| panic!("failed to finish {output_name} gzip archive: {error}"));
}

fn write_static_noland_moonlight_bindings() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is not set"));
    fs::write(
        out_dir.join("noland_moonlight_bindings.rs"),
        static_noland_moonlight_bindings(),
    )
    .expect("failed to write noland moonlight bindings");
}

fn static_noland_moonlight_bindings() -> &'static str {
    r#"pub type nl_result_t = ::std::os::raw::c_uint;
pub const nl_result_NL_RESULT_OK: nl_result_t = 0;
pub const nl_result_NL_RESULT_INVALID_ARGUMENT: nl_result_t = 1;
pub const nl_result_NL_RESULT_OUT_OF_MEMORY: nl_result_t = 2;
pub const nl_result_NL_RESULT_NOT_READY: nl_result_t = 3;
pub const nl_result_NL_RESULT_INVALID_STATE: nl_result_t = 4;
pub const nl_result_NL_RESULT_QUEUE_EMPTY: nl_result_t = 5;

#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct nl_performance_stats_t {
    pub incoming_fps: f64,
    pub decoded_fps: f64,
    pub submitted_fps: f64,
    pub video_mbps: f64,
    pub missing_frames_percent: f64,
    pub host_processing_ms: f64,
    pub reassembly_ms: f64,
    pub decode_ms: f64,
    pub render_queue_ms: f64,
    pub width: u32,
    pub height: u32,
    pub video_format: u32,
    pub samples: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct nl_microphone_statistics_t {
    pub captured_samples: u64,
    pub encoded_packets: u64,
    pub sent_bytes: u64,
    pub dropped_samples: u64,
    pub rtcp_reports: u64,
    pub network_errors: u64,
    pub queue_depth_samples: u32,
    pub running: u8,
    pub suspended: u8,
    pub muted: u8,
}

pub type nl_stream_state_t = ::std::os::raw::c_uint;
pub const nl_stream_state_NL_STREAM_STATE_IDLE: nl_stream_state_t = 0;
pub const nl_stream_state_NL_STREAM_STATE_STARTING: nl_stream_state_t = 1;
pub const nl_stream_state_NL_STREAM_STATE_STREAMING: nl_stream_state_t = 2;
pub const nl_stream_state_NL_STREAM_STATE_STOPPING: nl_stream_state_t = 3;

pub type nl_event_kind_t = ::std::os::raw::c_uint;
pub const nl_event_kind_NL_EVENT_NONE: nl_event_kind_t = 0;
pub const nl_event_kind_NL_EVENT_STATE_CHANGED: nl_event_kind_t = 1;
pub const nl_event_kind_NL_EVENT_CONNECTED: nl_event_kind_t = 2;
pub const nl_event_kind_NL_EVENT_STOPPED: nl_event_kind_t = 3;
pub const nl_event_kind_NL_EVENT_SURFACE_ATTACHED: nl_event_kind_t = 4;
pub const nl_event_kind_NL_EVENT_SURFACE_DETACHED: nl_event_kind_t = 5;
pub const nl_event_kind_NL_EVENT_ERROR: nl_event_kind_t = 6;
pub const nl_event_kind_NL_EVENT_STAGE_STARTING: nl_event_kind_t = 7;
pub const nl_event_kind_NL_EVENT_STAGE_COMPLETE: nl_event_kind_t = 8;
pub const nl_event_kind_NL_EVENT_STAGE_FAILED: nl_event_kind_t = 9;
pub const nl_event_kind_NL_EVENT_TERMINATED: nl_event_kind_t = 10;
pub const nl_event_kind_NL_EVENT_VIDEO_FRAME: nl_event_kind_t = 11;

pub type nl_surface_type_t = ::std::os::raw::c_uint;
pub const nl_surface_type_NL_SURFACE_TYPE_UNKNOWN: nl_surface_type_t = 0;
pub const nl_surface_type_NL_SURFACE_WINDOWS_HWND: nl_surface_type_t = 1;
pub const nl_surface_type_NL_SURFACE_MACOS_NSVIEW: nl_surface_type_t = 2;
pub const nl_surface_type_NL_SURFACE_X11_WINDOW: nl_surface_type_t = 3;
pub const nl_surface_type_NL_SURFACE_WAYLAND_SURFACE: nl_surface_type_t = 4;
pub const nl_surface_type_NL_SURFACE_IOS_UIVIEW: nl_surface_type_t = 5;
pub const nl_surface_type_NL_SURFACE_ANDROID_NATIVE_WINDOW: nl_surface_type_t = 6;

pub type nl_pacing_mode_t = ::std::os::raw::c_uint;
pub const nl_pacing_mode_NL_PACING_MODE_OFF: nl_pacing_mode_t = 0;
pub const nl_pacing_mode_NL_PACING_MODE_AUTOMATIC: nl_pacing_mode_t = 1;
pub const nl_pacing_mode_NL_PACING_MODE_SOFTWARE: nl_pacing_mode_t = 2;
pub const nl_pacing_mode_NL_PACING_MODE_HARDWARE_MULTIPLE: nl_pacing_mode_t = 3;

pub type nl_frame_buffer_mode_t = ::std::os::raw::c_uint;
pub const nl_frame_buffer_mode_NL_FRAME_BUFFER_MODE_OFF: nl_frame_buffer_mode_t = 0;
pub const nl_frame_buffer_mode_NL_FRAME_BUFFER_MODE_ONE_FRAME: nl_frame_buffer_mode_t = 1;
pub const nl_frame_buffer_mode_NL_FRAME_BUFFER_MODE_TWO_FRAMES: nl_frame_buffer_mode_t = 2;
pub const nl_frame_buffer_mode_NL_FRAME_BUFFER_MODE_THREE_FRAMES: nl_frame_buffer_mode_t = 3;

pub type nl_remote_stream_mode_t = ::std::os::raw::c_uint;
pub const nl_remote_stream_mode_NL_REMOTE_STREAM_MODE_AUTO: nl_remote_stream_mode_t = 0;
pub const nl_remote_stream_mode_NL_REMOTE_STREAM_MODE_FORCE_REMOTE: nl_remote_stream_mode_t = 1;
pub const nl_remote_stream_mode_NL_REMOTE_STREAM_MODE_FORCE_LOCAL: nl_remote_stream_mode_t = 2;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct nl_latency_config_t {
    pub telemetry_enabled: u8,
    pub adaptive_late_frame_drop_enabled: u8,
    pub decoder_backpressure_policy_enabled: u8,
    pub auto_reconnect_on_unexpected_termination: u8,
    pub vsync_enabled: u8,
    pub pacing_mode: nl_pacing_mode_t,
    pub frame_buffer_mode: nl_frame_buffer_mode_t,
    pub remote_stream_mode: nl_remote_stream_mode_t,
    pub remote_packet_size: u32,
    pub late_frame_tolerance_us: u32,
}

#[repr(C)]
pub struct nl_runtime_t {
    _unused: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct nl_start_request_t {
    pub host_id: *const ::std::os::raw::c_char,
    pub app_id: u32,
    pub session_url: *const ::std::os::raw::c_char,
    pub host_address: *const ::std::os::raw::c_char,
    pub server_app_version: *const ::std::os::raw::c_char,
    pub server_gfe_version: *const ::std::os::raw::c_char,
    pub server_codec_mode_support: i32,
    pub width: i32,
    pub height: i32,
    pub fps: i32,
    pub bitrate_kbps: i32,
    pub packet_size: i32,
    pub streaming_remotely: i32,
    pub audio_configuration: i32,
    pub audio_target_buffer_ms: u32,
    pub audio_maximum_buffer_ms: u32,
    pub supported_video_formats: i32,
    pub client_refresh_rate_x100: i32,
    pub color_space: i32,
    pub color_range: i32,
    pub encryption_flags: i32,
    pub remote_input_aes_key: [i8; 16],
    pub remote_input_aes_iv: [i8; 16],
    pub session_generation: u64,
    pub latency_config: nl_latency_config_t,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct nl_surface_descriptor_t {
    pub surface_type: nl_surface_type_t,
    pub window_handle: *mut ::std::ffi::c_void,
    pub display_handle: *mut ::std::ffi::c_void,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct nl_event_t {
    pub kind: nl_event_kind_t,
    pub state: nl_stream_state_t,
    pub code: i32,
    pub session_generation: u64,
    pub message: [::std::os::raw::c_char; 256],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct nl_stats_t {
    pub state: nl_stream_state_t,
    pub start_count: u64,
    pub stop_count: u64,
    pub surface_attach_count: u64,
    pub surface_detach_count: u64,
    pub dropped_event_count: u64,
    pub last_width: u32,
    pub last_height: u32,
    pub has_surface: u8,
    pub estimated_rtt_ms: u32,
    pub estimated_rtt_variance_ms: u32,
    pub has_estimated_rtt: u8,
    pub video_setup_count: u64,
    pub video_frame_count: u64,
    pub video_frame_event_count: u64,
    pub coalesced_video_frame_event_count: u64,
    pub renderer_ready: u8,
    pub video_session_active: u8,
    pub renderer_submitted_frame_count: u64,
    pub renderer_decoded_output_frame_count: u64,
    pub renderer_dropped_frame_count: u64,
    pub audio_init_count: u64,
    pub audio_sample_count: u64,
    pub mouse_move_count: u64,
    pub mouse_position_count: u64,
    pub mouse_button_count: u64,
    pub keyboard_event_count: u64,
    pub controller_arrival_count: u64,
    pub controller_state_count: u64,
    pub last_video_frame_number: i32,
    pub last_video_frame_type: i32,
    pub last_video_frame_length: i32,
    pub last_video_host_processing_latency: u16,
    pub last_video_receive_time_us: u64,
    pub last_video_enqueue_time_us: u64,
    pub last_video_presentation_time_us: u64,
    pub last_video_rtp_timestamp: u32,
    pub last_video_hdr_active: u8,
    pub last_video_colorspace: u8,
    pub session_generation: u64,
    pub video_packets_interval: u32,
    pub fec_packets_interval: u32,
    pub fec_recoveries_interval: u32,
    pub fec_failures_interval: u32,
    pub out_of_sequence_packets_interval: u32,
    pub invalid_packets_interval: u32,
    pub invalid_fec_packets_interval: u32,
    pub pending_core_video_frames: i32,
    pub decoder_queue_depth: u16,
    pub render_queue_depth: u16,
    pub average_decode_pipeline_us: u64,
    pub average_render_queue_dwell_us: u64,
    pub late_frame_count: u64,
    pub adaptive_stale_drop_count: u64,
    pub pacer_backlog_drop_count: u64,
    pub renderer_error_drop_count: u64,
    pub maximum_lateness_us: u64,
    pub decoder_backpressure_time_us: u64,
    pub last_drop_lateness_us: u64,
    pub rendered_fps_x100: u32,
    pub consecutive_late_frames: u32,
    pub late_tolerance_us: u32,
    pub decoder_backpressured: u8,
    pub smoothing_queue_depth: u8,
    pub smoothing_queue_capacity: u8,
    pub max_smoothing_queue_depth: u8,
    pub smoothing_overflow_drops: u64,
    pub smoothing_underflow_repeats: u64,
    pub smoothing_reserve_budget_us: u64,
    pub frame_timing_ring_count: u32,
    pub reconnect_attempt_count: u64,
    pub reconnect_success_count: u64,
    pub resolved_remote_stream_mode: nl_remote_stream_mode_t,
    pub requested_packet_size: u32,
    pub stream_fps: u32,
    pub client_refresh_rate_x100: u32,
    pub configured_pacing_mode: nl_pacing_mode_t,
    pub effective_pacing_mode: nl_pacing_mode_t,
}

unsafe extern "C" {
    pub fn nl_runtime_create(output: *mut *mut nl_runtime_t) -> nl_result_t;
    pub fn nl_runtime_destroy(runtime: *mut nl_runtime_t);
    pub fn nl_runtime_version_string() -> *const ::std::os::raw::c_char;
    pub fn nl_get_launch_query_parameters() -> *const ::std::os::raw::c_char;
    pub fn nl_runtime_smoke_test() -> i32;
    pub fn nl_sizeof_start_request() -> usize;
    pub fn nl_sizeof_event() -> usize;
    pub fn nl_sizeof_stats() -> usize;
    pub fn nl_runtime_start(runtime: *mut nl_runtime_t, request: *const nl_start_request_t) -> nl_result_t;
    pub fn nl_runtime_request_stop(runtime: *mut nl_runtime_t) -> nl_result_t;
    pub fn nl_runtime_attach_surface(runtime: *mut nl_runtime_t, surface: *const nl_surface_descriptor_t) -> nl_result_t;
    pub fn nl_runtime_detach_surface(runtime: *mut nl_runtime_t) -> nl_result_t;
    pub fn nl_runtime_poll_event(runtime: *mut nl_runtime_t, output: *mut nl_event_t) -> nl_result_t;
    pub fn nl_runtime_read_stats(runtime: *mut nl_runtime_t, output: *mut nl_stats_t) -> nl_result_t;
    pub fn nl_runtime_read_performance(runtime: *mut nl_runtime_t, output: *mut nl_performance_stats_t);
    pub fn nl_sizeof_performance_stats() -> usize;
    pub fn nl_runtime_set_overlay_text(runtime: *mut nl_runtime_t, text: *const ::std::os::raw::c_char);
    pub fn nl_runtime_record_reconnect_result(runtime: *mut nl_runtime_t, attempt_started: bool, succeeded: bool);
    pub fn nl_desktop_input_install(surface: *const nl_surface_descriptor_t) -> ::std::os::raw::c_int;
    pub fn nl_desktop_input_uninstall();
    pub fn nl_desktop_input_set_capture_active(active: bool, mode: ::std::os::raw::c_int) -> ::std::os::raw::c_int;
    pub fn nl_send_relative_mouse(runtime: *mut nl_runtime_t, delta_x: i16, delta_y: i16) -> nl_result_t;
    pub fn nl_send_absolute_mouse(runtime: *mut nl_runtime_t, x: i16, y: i16, reference_width: i16, reference_height: i16) -> nl_result_t;
    pub fn nl_send_mouse_button(runtime: *mut nl_runtime_t, button: u8, pressed: bool) -> nl_result_t;
    pub fn nl_send_vertical_scroll(runtime: *mut nl_runtime_t, amount: i16, high_resolution: bool) -> nl_result_t;
    pub fn nl_send_horizontal_scroll(runtime: *mut nl_runtime_t, amount: i16, high_resolution: bool) -> nl_result_t;
    pub fn nl_send_keyboard(runtime: *mut nl_runtime_t, virtual_key: u16, pressed: bool, modifiers: u8) -> nl_result_t;
    pub fn nl_send_controller_arrival(runtime: *mut nl_runtime_t, controller_number: u8, active_gamepad_mask: u16, controller_type: u8, supported_button_flags: u32, capabilities: u16) -> nl_result_t;
    pub fn nl_send_controller_state(runtime: *mut nl_runtime_t, controller_number: i16, active_gamepad_mask: i16, button_flags: i32, left_trigger: u8, right_trigger: u8, left_stick_x: i16, left_stick_y: i16, right_stick_x: i16, right_stick_y: i16) -> nl_result_t;
    pub fn nl_send_utf8_text(runtime: *mut nl_runtime_t, text: *const ::std::os::raw::c_char, length: u32) -> nl_result_t;
    pub fn nl_send_touch(runtime: *mut nl_runtime_t, event_type: u8, pointer_id: u32, x: f32, y: f32, pressure: f32, rotation: u16) -> nl_result_t;
    pub fn nl_send_controller(runtime: *mut nl_runtime_t, controller_number: u16, active_gamepad_mask: u16, button_flags: u32, left_trigger: u8, right_trigger: u8, left_stick_x: i16, left_stick_y: i16, right_stick_x: i16, right_stick_y: i16) -> nl_result_t;
    pub fn nl_release_all_input(runtime: *mut nl_runtime_t) -> nl_result_t;
    pub fn nl_microphone_start(host: *const ::std::os::raw::c_char, rtp_port: u16, rtcp_port: u16, local_rtcp_port: u16, ssrc: u32, sequence_offset: u16, timestamp_offset: u32, bitrate_bps: u32, frame_ms: u32) -> ::std::os::raw::c_int;
    pub fn nl_microphone_stop();
    pub fn nl_microphone_set_muted(muted: bool);
    pub fn nl_microphone_set_bitrate(bitrate_bps: u32) -> ::std::os::raw::c_int;
    pub fn nl_microphone_get_statistics(statistics: *mut nl_microphone_statistics_t);
}
"#
}

fn ensure_managed_sidecar_bundle_artifacts() -> io::Result<()> {
    println!("cargo:rerun-if-env-changed=TAURI_ENV_TARGET_TRIPLE");
    println!("cargo:rerun-if-env-changed=PROFILE");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let binaries_dir = manifest_dir.join("binaries");
    fs::create_dir_all(&binaries_dir)?;

    let target_triple = env::var("TAURI_ENV_TARGET_TRIPLE")
        .or_else(|_| env::var("TARGET"))
        .unwrap_or_else(|_| format!("{}-{}", env::consts::ARCH, env::consts::OS));
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let is_release = profile == "release";
    let target_is_windows = target_triple.contains("windows");

    ensure_staged_bundle_binary(
        &binaries_dir,
        "noland-mic-sender",
        &target_triple,
        target_is_windows,
        is_release,
        true,
        true,
        "run the Tauri build through the npm wrapper so the mic sidecar is staged first",
    )?;
    ensure_staged_bundle_binary(
        &binaries_dir,
        "noland-net-helper",
        &target_triple,
        target_is_windows,
        is_release,
        true,
        false,
        "run the Tauri build through the npm wrapper so the embedded GotaTun helper is staged first",
    )?;
    let helper_name = if target_is_windows {
        format!("noland-net-helper-{target_triple}.exe")
    } else {
        format!("noland-net-helper-{target_triple}")
    };
    let helper_path = binaries_dir.join(helper_name);
    println!("cargo:rerun-if-changed={}", helper_path.display());
    let helper_bytes = fs::read(helper_path)?;
    let helper_sha256 = format!("{:x}", Sha256::digest(helper_bytes));
    println!("cargo:rustc-env=NOLAND_NET_HELPER_SHA256={helper_sha256}");
    if target_is_windows && is_release {
        let wintun = binaries_dir.join(format!("wintun-{target_triple}.dll"));
        if !wintun.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "missing packaged Wintun adapter library '{}'; run the managed-tool staging step first",
                    wintun.display()
                ),
            ));
        }
        let wintun_license = binaries_dir.join("wintun-LICENSE.txt");
        if !wintun_license.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "missing packaged Wintun license '{}'; run the managed-tool staging step first",
                    wintun_license.display()
                ),
            ));
        }
    }
    ensure_staged_bundle_binary(
        &binaries_dir,
        "ssh",
        &target_triple,
        target_is_windows,
        is_release,
        true,
        false,
        "run the Tauri build through the npm wrapper so the bundled OpenSSH client is staged first",
    )?;
    ensure_staged_bundle_binary(
        &binaries_dir,
        "scp",
        &target_triple,
        target_is_windows,
        is_release,
        true,
        false,
        "run the Tauri build through the npm wrapper so the bundled OpenSSH scp client is staged first",
    )?;
    ensure_staged_bundle_binary(
        &binaries_dir,
        "ssh-keygen",
        &target_triple,
        target_is_windows,
        is_release,
        true,
        false,
        "run the Tauri build through the npm wrapper so the bundled OpenSSH keygen client is staged first",
    )?;

    Ok(())
}

fn ensure_staged_bundle_binary(
    binaries_dir: &Path,
    stem: &str,
    target_triple: &str,
    uses_exe_suffix: bool,
    is_release: bool,
    required_in_release: bool,
    allow_debug_placeholder: bool,
    release_hint: &str,
) -> io::Result<()> {
    let staged_name = if uses_exe_suffix {
        format!("{stem}-{target_triple}.exe")
    } else {
        format!("{stem}-{target_triple}")
    };
    let staged_path = binaries_dir.join(staged_name);

    if staged_path.exists() {
        return Ok(());
    }

    if is_release && required_in_release {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "missing packaged managed tool sidecar '{}'; {}",
                staged_path.display(),
                release_hint,
            ),
        ));
    }

    if !allow_debug_placeholder {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "missing required managed tool sidecar '{}'; {}",
                staged_path.display(),
                release_hint,
            ),
        ));
    }

    write_debug_sidecar_placeholder(&staged_path, target_triple.contains("windows"))
}

fn write_debug_sidecar_placeholder(path: &Path, target_is_windows: bool) -> io::Result<()> {
    if target_is_windows {
        fs::write(
            path,
            b"@echo off\r\necho noland-mic-sender debug placeholder. Run npm run tauri:dev or npm run prepare:mic-sidecar before launching the app.\r\nexit /b 1\r\n",
        )?;
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::write(
            path,
            b"#!/bin/sh\necho 'noland-mic-sender debug placeholder. Run npm run tauri:dev or npm run prepare:mic-sidecar before launching the app.' >&2\nexit 1\n",
        )?;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)?;
        return Ok(());
    }

    #[cfg(not(unix))]
    {
        fs::write(
            path,
            b"noland-mic-sender debug placeholder. Run npm run tauri:dev or npm run prepare:mic-sidecar before launching the app.\n",
        )?;
        Ok(())
    }
}

fn prepare_gstreamer_bundle(target: &str) -> io::Result<()> {
    println!("cargo:rerun-if-env-changed=NOLAND_GSTREAMER_FRAMEWORK");

    if !target.ends_with("apple-darwin") {
        return Ok(());
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let bundled_framework = manifest_dir
        .join("bundled")
        .join("macos")
        .join("GStreamer.framework");

    if has_macos_gstreamer_framework(&bundled_framework) {
        return Ok(());
    }

    if let Some(source) = resolve_macos_gstreamer_framework_source() {
        if let Some(parent) = bundled_framework.parent() {
            fs::create_dir_all(parent)?;
        }
        if bundled_framework.exists() {
            fs::remove_dir_all(&bundled_framework)?;
        }
        copy_dir_all(&source, &bundled_framework)?;
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "missing staged macOS GStreamer.framework; run node scripts/bootstrap-native-deps.mjs --target <triple> before building",
    ))
}

fn resolve_macos_gstreamer_framework_source() -> Option<PathBuf> {
    env::var("NOLAND_GSTREAMER_FRAMEWORK")
        .ok()
        .map(PathBuf::from)
        .filter(|path| has_macos_gstreamer_framework(path))
}

fn stage_macos_dev_runtime_dylibs(target: &str, manifest_dir: &Path) -> io::Result<()> {
    let Some(prefix) = native_dep_prefixes(target, manifest_dir)
        .into_iter()
        .find(|prefix| prefix.join("lib").is_dir())
    else {
        return Ok(());
    };

    let lib_dir = prefix.join("lib");
    let Some(target_dir) = cargo_target_profile_dir()? else {
        return Ok(());
    };
    let destinations = [target_dir.clone(), target_dir.join("deps")];
    let patterns = ["libopus", "libSDL2", "libcrypto", "libssl"];

    for entry in fs::read_dir(&lib_dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !patterns.iter().any(|prefix| name.starts_with(prefix)) {
            continue;
        }
        if !(name.ends_with(".dylib") || file_type.is_symlink()) {
            continue;
        }
        for destination in &destinations {
            fs::create_dir_all(destination)?;
            stage_runtime_entry(&path, destination.join(name))?;
        }
    }

    Ok(())
}

fn stage_runtime_entry(source: &Path, destination: PathBuf) -> io::Result<()> {
    match fs::symlink_metadata(&destination) {
        Ok(_) => fs::remove_file(&destination)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        let link_target = fs::read_link(source)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&link_target, &destination)?;
        #[cfg(not(unix))]
        {
            let resolved = source
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(link_target);
            fs::copy(resolved, &destination)?;
        }
        return Ok(());
    }
    fs::copy(source, &destination)?;
    Ok(())
}

fn cargo_target_profile_dir() -> io::Result<Option<PathBuf>> {
    let out_dir = PathBuf::from(
        env::var("OUT_DIR").map_err(|error| io::Error::new(io::ErrorKind::NotFound, error))?,
    );
    for ancestor in out_dir.ancestors() {
        let Some(name) = ancestor.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if matches!(name, "debug" | "release") {
            return Ok(Some(ancestor.to_path_buf()));
        }
    }
    Ok(None)
}

fn has_macos_gstreamer_framework(path: &Path) -> bool {
    [
        path.join("Versions/Current/lib/GStreamer"),
        path.join("Versions/Current/lib/libgstreamer-1.0.dylib"),
        path.join("Versions/Current/lib/libgstreamer-1.0.0.dylib"),
        path.join("Versions/1.0/lib/GStreamer"),
        path.join("Versions/1.0/lib/libgstreamer-1.0.dylib"),
        path.join("Versions/1.0/lib/libgstreamer-1.0.0.dylib"),
        path.join("Versions/Current/Libraries/GStreamer"),
        path.join("Versions/1.0/Libraries/GStreamer"),
    ]
    .iter()
    .any(|candidate| candidate.is_file())
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let destination = dst.join(entry.file_name());
        if destination.exists() {
            if destination.is_dir() {
                fs::remove_dir_all(&destination)?;
            } else {
                fs::remove_file(&destination)?;
            }
        }
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &destination)?;
        } else if file_type.is_symlink() {
            let Ok(resolved) = entry.path().canonicalize() else {
                continue;
            };
            if resolved.is_dir() {
                copy_dir_all(&resolved, &destination)?;
            } else {
                fs::copy(&resolved, &destination)?;
            }
        } else {
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}
