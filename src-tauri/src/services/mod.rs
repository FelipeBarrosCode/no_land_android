#[cfg(target_os = "android")]
pub mod android_vpn;
pub mod app_config;
pub mod app_context;
pub mod audio_latency;
pub mod clipboard;
pub mod cloudflare_turn;
pub mod connection_manager;
pub mod diagnostics;
pub mod display_profile;
pub mod health_check;
pub mod instance_lifecycle;
pub mod instance_manager;
#[cfg(target_os = "ios")]
mod ios_network;
#[cfg(target_os = "ios")]
pub mod ios_platform;
#[cfg(target_os = "ios")]
pub mod ios_vpn;
pub mod launch_library;
pub mod lifecycle_agent;
pub mod location;
pub mod mic_passthrough;
pub mod mic_receiver;
pub mod moonlight;
pub mod network_agent;
pub mod network_control;
pub mod nvidia_headless;
pub mod offer_selector;
pub mod orchestration;
pub mod os_detection;
mod package_manager;

pub mod post_wireguard_setup;
pub mod reboot_helper;
pub mod remote_display;
#[cfg_attr(
    any(target_os = "ios", target_os = "android"),
    path = "remote_exec_ios.rs"
)]
pub mod remote_exec;
pub mod shared_storage;
#[cfg_attr(target_os = "ios", path = "sleep_inhibit_ios.rs")]
pub mod sleep_inhibit;
pub mod software_artwork;
pub mod ssh_keys;
pub mod state_secrets;
pub mod state_store;
pub mod sunshine;
pub mod vast_api;
pub mod wireguard;
mod wireguard_mtu;
