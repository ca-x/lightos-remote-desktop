export function isAndroidPlatform() {
  const browser = navigator as Navigator & { userAgentData?: { platform?: string } };
  return /\bAndroid\b/i.test(browser.userAgentData?.platform ?? browser.platform)
    || /\bAndroid\b/i.test(browser.userAgent);
}

// Match WebShell's OS classification. Touch capability also exists on desktop
// Windows/Linux machines and does not select the mobile interface.
export function isMobilePlatform() {
  const browser = navigator as Navigator & { userAgentData?: { platform?: string } };
  const platform = browser.userAgentData?.platform || browser.platform || "";
  const userAgent = browser.userAgent || "";
  return isAndroidPlatform()
    || /\b(iPhone|iPad|iPod)\b/i.test(platform) || /\b(iPhone|iPad|iPod)\b/i.test(userAgent)
    || (/\bMac/i.test(platform) && (browser.maxTouchPoints ?? 0) > 1);
}

export function isDesktopClient() {
  return /\b(?:LazyCatClientDesktop|lzc-client-desktop)\//i.test(navigator.userAgent || "");
}

export function trackpadClickFeedback() {
  const host = window as Window & {
    lzc_vibrate?: { Vibrate?: (level: number) => void };
    webkit?: { messageHandlers?: { VibrateDevice?: { postMessage: (message: { duration: number }) => unknown } } };
  };
  try {
    if (host.lzc_vibrate?.Vibrate) {
      host.lzc_vibrate.Vibrate(0);
      return;
    }
    const ios = host.webkit?.messageHandlers?.VibrateDevice;
    if (ios) {
      void Promise.resolve(ios.postMessage({ duration: 0 })).catch(() => {});
      return;
    }
  } catch {}
  try { navigator.vibrate?.(10); } catch {}
}
