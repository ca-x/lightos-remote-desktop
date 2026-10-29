// Match WebShell's OS classification. Touch capability also exists on desktop
// Windows/Linux machines and does not select the mobile interface.
export function isMobilePlatform() {
  const browser = navigator as Navigator & { userAgentData?: { platform?: string } };
  const platform = browser.userAgentData?.platform || browser.platform || "";
  const userAgent = browser.userAgent || "";
  return /\bAndroid\b/i.test(platform) || /\bAndroid\b/i.test(userAgent)
    || /\b(iPhone|iPad|iPod)\b/i.test(platform) || /\b(iPhone|iPad|iPod)\b/i.test(userAgent)
    || (/\bMac/i.test(platform) && (browser.maxTouchPoints ?? 0) > 1);
}

export function isDesktopClient() {
  return /\b(?:LazyCatClientDesktop|lzc-client-desktop)\//i.test(navigator.userAgent || "");
}
