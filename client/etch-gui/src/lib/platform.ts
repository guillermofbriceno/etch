// Platform detection via user-agent. Reliable in Tauri desktop WebViews
// where the string is controlled by the WebView implementation (WebView2,
// WebKitGTK), not user-configurable.
export const PLATFORM_WINDOWS = navigator.userAgent.includes('Windows');
export const PLATFORM_MACOS = navigator.userAgent.includes('Macintosh');
export const PLATFORM_LINUX =
    navigator.userAgent.includes('Linux') && !navigator.userAgent.includes('Android');
