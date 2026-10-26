#!/bin/sh
set -eu
lightos_application_environment="$(dirname -- "$0")/../application-environment.sh"
. "$lightos_application_environment"
unset lightos_application_environment
if [ "$#" -eq 0 ]; then exit 2; fi
case $1 in
    /snap/*|*/snap|snap|*/flatpak|flatpak)
        printf '%s\n' 'LightOS: this virtual-session adapter requires a native browser installation; sandboxed launchers need a separate adapter' >&2
        exit 1 ;;
esac
lightos_explicit_platform=0
lightos_explicit_profile=0
for lightos_argument in "$@"; do
    case $lightos_argument in --ozone-platform|--ozone-platform=*|--ozone-platform-hint|--ozone-platform-hint=*) lightos_explicit_platform=1 ;; esac
    case $lightos_argument in --user-data-dir|--user-data-dir=*) lightos_explicit_profile=1 ;; esac
done
lightos_browser=${LIGHTOS_BROWSER_DESKTOP_ID:-${1##*/}}
unset LIGHTOS_BROWSER_DESKTOP_ID
lightos_browser=${lightos_browser%.desktop}
case $lightos_browser in
    google-chrome-beta) lightos_browser=google-chrome-beta ;;
    google-chrome-unstable) lightos_browser=google-chrome-unstable ;;
    google-chrome|google-chrome-stable|chrome) lightos_browser=google-chrome ;;
    google-chrome*) ;;
    chromium|chromium-browser) lightos_browser=chromium ;;
    chromium*) ;;
    brave|brave-browser|brave-browser-stable) lightos_browser=BraveSoftware/Brave-Browser ;;
    brave*beta) lightos_browser=BraveSoftware/Brave-Browser-Beta ;;
    brave*nightly) lightos_browser=BraveSoftware/Brave-Browser-Nightly ;;
    brave*) ;;
    microsoft-edge|microsoft-edge-stable) lightos_browser=microsoft-edge ;;
    microsoft-edge*) ;;
    vivaldi|vivaldi-stable) lightos_browser=vivaldi ;;
    vivaldi*|opera*|helium*) ;;
    *) printf '%s\n' 'LightOS: unsupported browser launch adapter' >&2; exit 1 ;;
esac
if [ "$lightos_explicit_profile" = 0 ]; then
    set -- "$@" "--user-data-dir=$CHROME_CONFIG_HOME/$lightos_browser"
fi
if [ "${LIGHTOS_BROWSER_PRIVATE:-0}" = 1 ]; then
    unset LIGHTOS_BROWSER_PRIVATE
    set -- "$@" --incognito
fi
if [ "${LIGHTOS_SESSION_INPUT_METHOD:-}" = fcitx5 ]; then
    lightos_explicit_ime=0
    for lightos_argument in "$@"; do
        case $lightos_argument in --enable-wayland-ime|--disable-wayland-ime|--wayland-text-input-version|--wayland-text-input-version=*) lightos_explicit_ime=1 ;; esac
    done
    if [ "$lightos_explicit_ime" = 0 ]; then
        set -- "$@" --enable-wayland-ime --wayland-text-input-version=3
    fi
    unset lightos_explicit_ime
fi
unset lightos_argument lightos_browser
# Packaged Chrome can prepend host chrome-flags.conf (including forced X11).
# An owned Wayland desktop chooses its native display unless explicitly asked otherwise.
if [ "$lightos_explicit_platform" = 0 ]; then
    exec "$@" --ozone-platform=wayland
fi
exec "$@"
