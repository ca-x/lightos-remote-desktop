#!/bin/sh
# Called only by an explicit install/reinstall action, never by instance sync.
set -eu
check=0
plan=0
session_only=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --check) check=1; shift ;;
    --plan) plan=1; shift ;;
    --session-only) session_only=1; shift ;;
    *) break ;;
  esac
done
payload=${1:?program staging directory required}
shift
if [ "$(cat /proc/1/comm 2>/dev/null || true)" != systemd ] || ! command -v systemctl >/dev/null 2>&1; then
  echo 'This image must run systemd before the remote desktop service can be installed.' >&2
  exit 1
fi
if [ "$session_only" = 0 ] && [ -r /etc/lightos/remote-desktop.env ]; then . /etc/lightos/remote-desktop.env; fi
session=${LIGHTOS_REMOTE_DESKTOP_SESSION_ID:-${LIGHTOS_REMOTE_DESKTOP_DESKTOP_ENV:-}}
desktop=${LIGHTOS_REMOTE_DESKTOP_SESSION:-}
required="python3 dbus-daemon dbus-update-activation-environment pactl pipewire wireplumber flock setsid timeout"
need_xephyr=0
need_wayland=0
need_labwc=0
need_devkit=0
case "$session:$desktop" in
  *[Nn]iri*) required="$required cage niri pipewire-pulse"; native=1 ;;
  *[Hh]yprland*) required="$required cage pipewire-pulse"; native=1 ;;
  *)
    native=0
    required="$required Xvfb xauth xdpyinfo xrandr xset xsetroot xdotool gdbus"
    # Explicit service preparation covers the available desktop sessions too,
    # so a later switch does not require selecting it first and reinstalling.
    # The caller supplies supported sessions, including Bookworm filtering.
    for candidate in "$session" "$@"; do
      case "$candidate" in
        x11:gnome*|x11:ubuntu*) need_xephyr=1 ;;
        wayland:*) need_wayland=1 ;;
      esac
      case "$candidate" in
        wayland:*plasma*|wayland:*kde*|wayland:*KDE*|wayland:*xfce*|wayland:*XFCE*|wayland:*labwc*) need_labwc=1 ;;
      esac
    done
    case "$session:$desktop" in
      wayland:*plasma*|wayland:*kde*|wayland:*KDE*|wayland:*xfce*|wayland:*XFCE*|wayland:*labwc*) need_labwc=1 ;;
    esac
    [ "$need_xephyr" = 0 ] || required="$required Xephyr"
    [ "$need_wayland" = 0 ] || required="$required weston wl-copy wl-paste"
    [ "$need_labwc" = 0 ] || required="$required labwc wlr-randr"
    case "$session:$desktop" in
      wayland:*gnome*|wayland:*GNOME*|wayland:*ubuntu*)
        if gnome-shell --help 2>&1 | grep -q -- --devkit; then need_devkit=1; fi
        ;;
    esac
    ;;
esac
missing=0
gpu_vendor=""
for vendor_file in /sys/class/drm/renderD*/device/vendor; do
  [ "$session_only" = 0 ] || break
  [ -r "$vendor_file" ] || continue
  case "$(cat "$vendor_file")" in
    0x8086) gpu_vendor=intel; break ;;
    0x1002) gpu_vendor=amd ;;
  esac
done
driver_missing=0
case "$gpu_vendor" in
  intel)
    if [ ! -f /usr/lib/dri/iHD_drv_video.so ] && [ ! -f /usr/lib/x86_64-linux-gnu/dri/iHD_drv_video.so ]; then driver_missing=1; fi
    ;;
  amd)
    if [ ! -f /usr/lib/dri/radeonsi_drv_video.so ] && [ ! -f /usr/lib/x86_64-linux-gnu/dri/radeonsi_drv_video.so ]; then driver_missing=1; fi
    ;;
esac
if [ "$driver_missing" = 1 ]; then missing=1; fi
for program in $required; do
  if ! command -v "$program" >/dev/null 2>&1; then
    echo "Missing remote desktop dependency: $program" >&2
    missing=1
  fi
done
if [ "$need_devkit" = 1 ] && [ ! -x /usr/libexec/mutter-devkit ] && [ ! -x /usr/lib/mutter-devkit ]; then
  echo 'Missing remote desktop dependency: mutter-dev-bin (Debian/Ubuntu) or mutter-devkit (Arch)' >&2
  missing=1
fi
if [ "$session_only" = 0 ]; then
  if ! "$payload/bin/lightos-remoted" --version >/dev/null 2>&1; then missing=1; fi
  if ldd "$payload/bin/lightos-codec-ffmpeg" 2>&1 | grep -q 'not found'; then missing=1; fi
fi
if ! command -v pipewire-pulse >/dev/null 2>&1 && ! command -v pulseaudio >/dev/null 2>&1; then
  echo 'Missing remote desktop dependency: private PulseAudio server' >&2
  missing=1
fi
# Read-only probes never run a package manager or codec worker.
if [ "$check" = 1 ]; then exit "$missing"; fi
if [ "$missing" = 0 ] && [ "$plan" = 1 ]; then
  echo 'Remote desktop system dependencies are already available.'
  exit 0
fi
if [ "$missing" = 0 ]; then
  if [ "$session_only" = 0 ]; then "$payload/bin/lightos-codec-ffmpeg" </dev/null; fi
  echo 'Remote desktop system dependencies are already available.'
  exit 0
fi
echo 'Installing missing dependencies for the requested remote desktop service.'
if command -v apt-get >/dev/null 2>&1; then
  export DEBIAN_FRONTEND=noninteractive
  if [ "$plan" = 0 ]; then apt-get update; fi
  packages="python3 dbus-x11 pulseaudio-utils pipewire wireplumber libxkbcommon0 libstdc++6 libpipewire-0.3-0 libpulse0 libwayland-client0 libva2 libva-drm2 libdrm2"
  if [ "$driver_missing" = 1 ]; then
    case "$gpu_vendor" in intel) packages="$packages intel-media-va-driver" ;; amd) packages="$packages mesa-va-drivers" ;; esac
  fi
  if [ "$native" = 1 ]; then
    packages="$packages cage pipewire-pulse"
  else
    packages="$packages xvfb xauth x11-utils x11-xserver-utils xdotool libglib2.0-bin"
    [ "$need_xephyr" = 0 ] || packages="$packages xserver-xephyr"
    [ "$need_wayland" = 0 ] || packages="$packages weston wl-clipboard wlr-randr"
    [ "$need_labwc" = 0 ] || packages="$packages labwc wlr-randr"
    [ "$need_devkit" = 0 ] || packages="$packages mutter-dev-bin"
    # Keep an existing system audio stack; do not replace PulseAudio with PipeWire.
    if ! command -v pipewire-pulse >/dev/null 2>&1 && ! command -v pulseaudio >/dev/null 2>&1; then packages="$packages pulseaudio"; fi
  fi
  set --
  for package in $packages; do
    status=$(dpkg-query -W -f='${Status}' "$package" 2>/dev/null || true)
    [ "$status" = 'install ok installed' ] || set -- "$@" "$package"
  done
  if [ "$plan" = 1 ]; then
    if [ "$#" -gt 0 ]; then apt-get -s install --no-install-recommends --no-upgrade --no-remove "$@"; fi
    exit 0
  fi
  if [ "$#" -gt 0 ]; then apt-get install -y --no-install-recommends --no-upgrade --no-remove "$@"; fi
elif command -v pacman >/dev/null 2>&1; then
  [ "$plan" = 0 ] || { echo 'Package simulation is currently supported on Debian/Ubuntu only.' >&2; exit 1; }
  # Share the desktop install lock with desktop/package installation. Never
  # remove pacman's lock: another package transaction may still own it.
  if command -v flock >/dev/null 2>&1; then
    exec 8>/run/lightos-desktop-install.lock
    echo 'Waiting for the desktop package installation lock...'
    flock -w "${LIGHTOS_REMOTE_DESKTOP_PACKAGE_LOCK_TIMEOUT:-600}" 8 || {
      echo 'Timed out waiting for another desktop package installation.' >&2
      exit 1
    }
  fi
  wait_for_pacman() {
    waited=0
    while [ -e /var/lib/pacman/db.lck ]; do
      if [ "$waited" = 0 ]; then echo 'Waiting for the running pacman transaction...'; fi
      if [ "$waited" -ge "${LIGHTOS_REMOTE_DESKTOP_PACKAGE_LOCK_TIMEOUT:-600}" ]; then
        echo 'Timed out waiting for the pacman database lock.' >&2
        return 1
      fi
      sleep 1
      waited=$((waited + 1))
    done
  }
  arch_work=$(mktemp -d)
  trap 'rm -rf "$arch_work"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  run_arch_transaction() {
    attempt=1
    while true; do
      wait_for_pacman || return "$?"
      : > "$arch_work/status"
      # POSIX sh reports tee's status for a pipeline. Record pacman's status
      # independently so a failed installation can never look successful.
      if (
        if LC_ALL=C "$@"; then
          printf '0\n' > "$arch_work/status"
        else
          printf '%s\n' "$?" > "$arch_work/status"
        fi
      ) 2>&1 | tee "$arch_work/download.log"; then
        :
      else
        echo 'Unable to record the package installation output.' >&2
        return 1
      fi
      package_status=$(cat "$arch_work/status")
      case "$package_status" in
        0) return 0 ;;
        ''|*[!0-9]*) echo 'Package transaction ended without an exit status.' >&2; return 1 ;;
      esac
      # Missing versions need the archive immediately. Retry only retrieval
      # failures; signatures, conflicts and installation errors stay failures.
      if [ "$attempt" -ge 3 ] ||
         ! grep -q 'failed retrieving file' "$arch_work/download.log" ||
         grep -Eq 'returned error: (404|410)' "$arch_work/download.log"; then
        return "$package_status"
      fi
      attempt=$((attempt + 1))
      echo "Retrying package downloads (attempt $attempt/3)."
      sleep 2
    done
  }
  wait_for_pacman
  packages="python dbus libpulse pipewire wireplumber libxkbcommon wayland gcc-libs libva libdrm"
  if [ "$driver_missing" = 1 ]; then
    case "$gpu_vendor" in intel) packages="$packages intel-media-driver" ;; amd) packages="$packages libva-mesa-driver" ;; esac
  fi
  if [ "$native" = 1 ]; then
    packages="$packages cage pipewire-pulse"
  else
    packages="$packages xorg-server-xvfb xorg-xauth xorg-xdpyinfo xorg-xrandr xorg-xset xorg-xsetroot xdotool glib2"
    [ "$need_xephyr" = 0 ] || packages="$packages xorg-server-xephyr"
    [ "$need_wayland" = 0 ] || packages="$packages weston wl-clipboard wlr-randr"
    [ "$need_labwc" = 0 ] || packages="$packages labwc wlr-randr"
    [ "$need_devkit" = 0 ] || packages="$packages mutter-devkit"
    if ! command -v pipewire-pulse >/dev/null 2>&1 && ! command -v pulseaudio >/dev/null 2>&1; then packages="$packages pipewire-pulse"; fi
  fi
  set --
  for package in $packages; do pacman -Q "$package" >/dev/null 2>&1 || set -- "$@" "$package"; done
  # Use the instance's current package database. A service install must not
  # initiate a whole-system upgrade or a partial database refresh.
  if [ "$#" -gt 0 ]; then
    # A snapshot mirror can lose an older package while the instance still
    # uses that database. Keep exact versions and normal signature checks.
    package_plan=$(LC_ALL=C pacman -Sp --needed --print-format '%n %f' "$@")
    if run_arch_transaction pacman -S --needed --noconfirm "$@"; then
      :
    else
      status=$?
      if ! grep -q 'failed retrieving file' "$arch_work/download.log"; then exit "$status"; fi
      echo 'Retrying the exact missing package versions from Arch Linux Archive.'
      set --
      while read -r package filename; do
        case "$package" in ''|*[!a-zA-Z0-9@._+-]*) echo 'Invalid package archive plan' >&2; exit 1 ;; esac
        case "$filename" in "$package"-*.pkg.tar.*) ;; *) echo 'Invalid package archive filename' >&2; exit 1 ;; esac
        case "$filename" in */*|*'..'*) echo 'Invalid package archive path' >&2; exit 1 ;; esac
        first=${package%${package#?}}
        set -- "$@" "https://archive.archlinux.org/packages/$first/$package/$filename"
      done <<EOF
$package_plan
EOF
      [ "$#" -gt 0 ] || { echo 'The package archive plan is empty.' >&2; exit 1; }
      run_arch_transaction pacman -U --needed --noconfirm "$@"
    fi
  fi
else
  echo 'Install the missing desktop dependencies with this system package manager before retrying.' >&2
  exit 1
fi
for program in $required; do
  command -v "$program" >/dev/null 2>&1 || { echo "Missing remote desktop dependency: $program" >&2; exit 1; }
done
if [ "$session_only" = 0 ]; then
  "$payload/bin/lightos-remoted" --version
  "$payload/bin/lightos-codec-ffmpeg" </dev/null
fi
if [ "$need_devkit" = 1 ] && [ ! -x /usr/libexec/mutter-devkit ] && [ ! -x /usr/lib/mutter-devkit ]; then
  echo 'The installed GNOME development kit is unavailable' >&2
  exit 1
fi
