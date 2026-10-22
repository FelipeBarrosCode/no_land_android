// Tauri supplies this at build/dev time. Do not infer native capabilities from
// userAgent: iPad WebViews can identify as macOS.
export const isIOS = import.meta.env.TAURI_ENV_PLATFORM === "ios";
export const isAndroid = import.meta.env.TAURI_ENV_PLATFORM === "android";
export const isMobile = isIOS || isAndroid;
