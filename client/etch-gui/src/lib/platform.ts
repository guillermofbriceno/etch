export const PLATFORM_WINDOWS = navigator.userAgent.includes('Windows');
export const PLATFORM_MACOS = navigator.userAgent.includes('Macintosh');
export const PLATFORM_LINUX = !PLATFORM_WINDOWS && !PLATFORM_MACOS;
