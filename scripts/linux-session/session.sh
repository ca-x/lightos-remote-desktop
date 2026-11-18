#!/bin/sh
# Own only this instance's virtual display, bus, audio and selected desktop.
set -eu
payload=${LIGHTOS_REMOTE_DESKTOP_PAYLOAD_DIR:-/opt/lightos/remote-desktop}
program=${LIGHTOS_REMOTE_DESKTOP_PROGRAM:-$payload/bin/lightos-remoted}
original_path=$PATH
unset PIPEWIRE_CONFIG_DIR PIPEWIRE_MODULE_DIR SPA_PLUGIN_DIR
unset LIGHTOS_REMOTE_DESKTOP_NESTED_RESIZE_HELPER
unset LIGHTOS_REMOTE_DESKTOP_GNOME_X11
runtime=${XDG_RUNTIME_DIR:?}
# Physical discovery must never inherit this service's private display or bus.
export LIGHTOS_REMOTE_DESKTOP_PHYSICAL_RUNTIME_DIR="/run/user/$(id -u)"
mkdir -p "$runtime"
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}" "${XDG_CACHE_HOME:-$HOME/.cache}"
chmod 700 "$runtime"
exec 8>"$runtime/session.lock"
flock -n 8 || { echo 'Another LightOS remote desktop owns this runtime directory' >&2; exit 1; }

session=${LIGHTOS_REMOTE_DESKTOP_SESSION_ID:-${LIGHTOS_REMOTE_DESKTOP_DESKTOP_ENV:-}}
command=${LIGHTOS_REMOTE_DESKTOP_SESSION:-}
case "$session:$command" in
  *[Nn]iri*)
    unset DISPLAY WAYLAND_DISPLAY LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR
    if [ -z "${WLR_RENDERER:-}" ]; then
      if [ -r /dev/dri/renderD128 ] && [ -w /dev/dri/renderD128 ]; then export WLR_RENDERER=gles2; else export WLR_RENDERER=pixman; fi
    fi
    export LIGHTOS_REMOTE_DESKTOP_RUNTIME_DIR=${LIGHTOS_REMOTE_DESKTOP_RUNTIME_DIR:-/run/lrd-session}
    if [ "${LIGHTOS_REMOTE_DESKTOP_SESSION_ONLY:-0}" = 1 ]; then
      flock -u 8; exec 8>&-
      exec "$program" desktop-session --compositor niri --runtime-dir "$runtime"
    fi
    exec "$program" serve --compositor niri
    ;;
  *[Hh]yprland*)
    # The native backend owns its Cage/Hyprland session and cleans it up itself.
    unset DISPLAY WAYLAND_DISPLAY
    case "$(readlink -f /usr/lib/libaquamarine.so.14 2>/dev/null || true)" in
      */libaquamarine.so.0.15.0|*/libaquamarine.so.0.15.1) export LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR="$payload/compat/aquamarine" ;;
    esac
    if [ -z "${WLR_RENDERER:-}" ]; then
      if [ -r /dev/dri/renderD128 ] && [ -w /dev/dri/renderD128 ]; then
        export WLR_RENDERER=gles2
      else
        export WLR_RENDERER=pixman
      fi
    fi
    export LIGHTOS_REMOTE_DESKTOP_RUNTIME_DIR=${LIGHTOS_REMOTE_DESKTOP_RUNTIME_DIR:-/run/lrd-session}
    if [ "${LIGHTOS_REMOTE_DESKTOP_SESSION_ONLY:-0}" = 1 ]; then
      flock -u 8; exec 8>&-
      exec "$program" desktop-session --runtime-dir "$runtime"
    fi
    exec "$program" serve
    ;;
esac
[ -n "$command" ] || { echo 'No installed desktop session is selected' >&2; exit 1; }

children=""
cleanup() {
  status=$?
  trap - EXIT INT TERM HUP
  if [ "$status" -ne 0 ]; then
    for log in "$runtime"/*.log; do
      if [ -f "$log" ]; then
        printf '\n[%s]\n' "$log" >&2
        tail -n 40 "$log" >&2 || true
      fi
    done
  fi
  for pid in $children; do kill -TERM "-$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true; done
  for pid in $children; do wait "$pid" 2>/dev/null || true; done
  exit "$status"
}
trap cleanup EXIT
trap 'exit 0' INT TERM HUP
start() {
  log=$1; shift
  setsid "$@" >"$runtime/$log" 2>&1 &
  child=$!
  children="$child $children"
}
wait_socket() {
  socket=$1; pid=$2; count=0
  while [ ! -S "$socket" ]; do
    kill -0 "$pid" 2>/dev/null || { echo "Desktop dependency exited; see $runtime" >&2; return 1; }
    count=$((count + 1))
    [ "$count" -lt 240 ] || { echo "Desktop socket did not become ready: $socket" >&2; return 1; }
    sleep 0.25
  done
}
start_nested_weston() {
  renderer=$1
  help=$(weston --help 2>&1 || true)
  if printf '%s' "$help" | grep -q -- --renderer; then
    set -- --backend=x11 --shell=kiosk-shell.so --renderer="$renderer"
  else
    set -- --backend=x11-backend.so --shell=kiosk-shell.so
    if [ "$renderer" = pixman ]; then set -- "$@" --use-pixman; fi
  fi
  start parent-display.log weston "$@" --socket=lightos-parent --width="$width" --height="$height" --idle-time=0 --no-config
  parent=$child
  wait_socket "$runtime/lightos-parent" "$parent"
  export WAYLAND_DISPLAY=lightos-parent
  export LIGHTOS_REMOTE_DESKTOP_NESTED_RESIZE_HELPER="$payload/resize-nested.sh"
  export LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_KIND=weston
  export LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_PID="$parent"
  export LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_DISPLAY="$DISPLAY"
  export LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_AUTHORITY="$XAUTHORITY"
  export LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_WAYLAND="$runtime/lightos-parent"
}
export DISPLAY=${LIGHTOS_REMOTE_DESKTOP_VIRTUAL_DISPLAY:-:20}
export XAUTHORITY="$runtime/Xauthority"
export PULSE_SERVER="unix:$runtime/pulse/native"
export PULSE_RUNTIME_PATH="$runtime/pulse"
export DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus"
unset WAYLAND_DISPLAY DBUS_SESSION_BUS_PID SESSION_MANAGER
screen=${LIGHTOS_REMOTE_DESKTOP_SCREEN:-1920x1080x24}
size=${screen%x*}
width=${size%x*}
height=${size#*x}
case "$width:$height" in *[!0-9:]*|:*|*:) echo 'Invalid remote desktop screen size' >&2; exit 1;; esac
virtual_dynamic=1
allocation_screen="3840x2160x${screen##*x}"
case "$session" in wayland:*) virtual_dynamic=0; allocation_screen="$screen" ;; esac
case "$session:$command" in
  wayland:*gnome*|wayland:*GNOME*|wayland:*ubuntu*|wayland:*plasma*|wayland:*kde*|wayland:*KDE*) allocation_screen="3840x2160x${screen##*x}" ;;
esac
cookie=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
touch "$XAUTHORITY"
chmod 600 "$XAUTHORITY"
xauth -f "$XAUTHORITY" add "$DISPLAY" . "$cookie"
start xvfb.log Xvfb "$DISPLAY" -screen 0 "$allocation_screen" -auth "$XAUTHORITY" -nolisten tcp -noreset
xvfb=$child
count=0
until xdpyinfo >/dev/null 2>&1; do
  kill -0 "$xvfb" 2>/dev/null || { cat "$runtime/xvfb.log" >&2; exit 1; }
  count=$((count + 1)); [ "$count" -lt 80 ] || exit 1
  sleep 0.25
done
x11_output=screen
case "$session" in
  x11:gnome*|x11:ubuntu*)
    export LIGHTOS_REMOTE_DESKTOP_GNOME_X11=1
    # Xephyr re-allocates live GLX drawables on RandR resize. Xvfb remains a
    # fixed private parent, so resizing never touches a host display or VT.
    xset s off -dpms >/dev/null 2>&1 || true
    parent=$xvfb
    parent_display=$DISPLAY
    display_number=${DISPLAY#:}; display_number=${display_number%%.*}
    case "$display_number" in ''|*[!0-9]*) echo 'Invalid virtual display number' >&2; exit 1 ;; esac
    export DISPLAY=:$((display_number + 1))
    xauth -f "$XAUTHORITY" add "$DISPLAY" . "$cookie"
    start xephyr.log env DISPLAY="$parent_display" Xephyr "$DISPLAY" -screen 3840x2160 -resizeable -auth "$XAUTHORITY" -nolisten tcp -noreset
    xvfb=$child
    count=0
    until xdpyinfo >/dev/null 2>&1; do
      kill -0 "$xvfb" 2>/dev/null || { cat "$runtime/xephyr.log" >&2; exit 1; }
      count=$((count + 1)); [ "$count" -lt 80 ] || exit 1
      sleep 0.25
    done
    x11_output=default
    ;;
esac
if [ "$virtual_dynamic" = 1 ] && [ "${width}x${height}" != 3840x2160 ]; then
  # Xephyr also advertises an automatic 0 Hz mode for each window size. An
  # identical custom 0 Hz mode makes older Mutter lose the active logical
  # monitor when returning to the initial resolution. Give ours valid timing.
  pixel_clock=$(python3 -c 'import sys; print(int(sys.argv[1])*int(sys.argv[2])*60/1000000)' "$width" "$height")
  xrandr --newmode LIGHTOS-INITIAL "$pixel_clock" "$width" "$width" "$width" "$width" "$height" "$height" "$height" "$height"
  xrandr --addmode "$x11_output" LIGHTOS-INITIAL
  xrandr --output "$x11_output" --mode LIGHTOS-INITIAL
fi
xset s off -dpms >/dev/null 2>&1 || true
set -- --session --nofork --nopidfile --address="$DBUS_SESSION_BUS_ADDRESS"
set -- "$@" --systemd-activation
start session-bus.log dbus-daemon "$@"
bus=$child
wait_socket "$runtime/bus" "$bus"
mkdir -p "$runtime/pulse"
start pipewire.log pipewire
pipewire=$child
wait_socket "$runtime/pipewire-0" "$pipewire"
if command -v pipewire-pulse >/dev/null 2>&1; then
  start audio-server.log pipewire-pulse
else
  # The socket is inside this service's mode-0700 runtime, not a host listener.
  start audio-server.log pulseaudio --daemonize=no --exit-idle-time=-1 --use-pid-file=no --log-target=stderr -n \
    --load="module-native-protocol-unix socket=$runtime/pulse/native auth-anonymous=1"
fi
pulse=$child
wait_socket "$runtime/pulse/native" "$pulse"
pactl load-module module-null-sink sink_name=lightos_remote >/dev/null
export PULSE_SINK=lightos_remote
export PULSE_SOURCE=lightos_remote.monitor

# Keep GNOME/Plasma services on an owned user manager, apart from physical logins.
export XDG_SESSION_TYPE=x11
manager=""
desktop_path="$original_path"
case "$session:$command" in
  *gnome*|*GNOME*|*ubuntu*) export XDG_CURRENT_DESKTOP=GNOME ;;
  *plasma*|*kde*|*KDE*)
    export XDG_CURRENT_DESKTOP=KDE
    # The managed session already has a connection/loading UI. KSplash can
    # still own input after KWin starts, so do not present it as a ready desktop.
    for config_writer in kwriteconfig6 kwriteconfig5; do
      command -v "$config_writer" >/dev/null 2>&1 || continue
      "$config_writer" --file "$XDG_CONFIG_HOME/ksplashrc" --group KSplash --key Engine None
      break
    done
    ;;
  *xfce*|*XFCE*) export XDG_CURRENT_DESKTOP=XFCE ;;
esac
units="$runtime/systemd/user"
mkdir -p "$units"
# This session already owns its audio server and policy daemon. Starting
# distribution socket units would replace the live private Unix sockets.
for unit in pipewire.service pipewire.socket pipewire-pulse.service pipewire-pulse.socket wireplumber.service pulseaudio.service pulseaudio.socket; do
  ln -sf /dev/null "$units/$unit"
done
printf '[Unit]\nDescription=LightOS private desktop session\n' > "$units/default.target"
case "$session" in
  wayland:*)
    export XDG_SESSION_TYPE=wayland
    kwin_mode=""
    case "$session:$command" in
      *xfce*|*XFCE*|*labwc*)
        # A service-owned headless output can follow the browser's size and
        # scale. A nested Wayland window is constrained by its parent shell.
        unset WAYLAND_DISPLAY
        export WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1
        ;;
      *plasma*|*kde*|*KDE*)
        render_node=""
        for node in /dev/dri/renderD*; do
          if [ -r "$node" ] && [ -w "$node" ]; then render_node="$node"; break; fi
        done
        [ -n "$render_node" ] || { echo 'KDE Wayland needs an accessible render node for its private OpenGL display.' >&2; exit 1; }
        command -v labwc >/dev/null 2>&1 || { echo 'KDE Wayland requires the system labwc compositor; reinstall LightOS Remote Desktop to install the missing dependency.' >&2; exit 1; }
        mkdir -p "$runtime/parent-labwc"
        cat > "$runtime/parent-labwc/rc.xml" <<'PARENT_LABWC'
<?xml version="1.0"?>
<labwc_config>
  <windowRules>
    <windowRule identifier="*">
      <action name="SetDecorations" decorations="none"/>
      <action name="Maximize"/>
    </windowRule>
  </windowRules>
</labwc_config>
PARENT_LABWC
        : > "$runtime/parent-labwc/autostart"
        start parent-display.log env WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1 WLR_RENDERER=gles2 WLR_RENDER_DRM_DEVICE="$render_node" labwc -C "$runtime/parent-labwc"
        parent=$child
        wait_socket "$runtime/wayland-0" "$parent"
        export WAYLAND_DISPLAY=wayland-0
        wlr-randr --output HEADLESS-1 --custom-mode "${width}x${height}@60Hz"
        export LIGHTOS_REMOTE_DESKTOP_NESTED_RESIZE_HELPER="$payload/resize-nested.sh"
        export LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_KIND=wlroots
        export LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_PID="$parent"
        export LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_WAYLAND="$runtime/wayland-0"
        ;;
      *)
        start_nested_weston pixman
        ;;
    esac
    export XDG_CURRENT_DESKTOP=$(printf '%s' "${session#wayland:}" | tr '[:lower:]' '[:upper:]')
    case "$session:$command" in
      *gnome*|*GNOME*|*ubuntu*)
        export XDG_CURRENT_DESKTOP=GNOME
        # Keep the input adapter in this virtual session's data directories.
        # It exports focused app IDs only; no physical desktop settings change.
        adapter="$runtime/gnome-data/gnome-shell/extensions/lightos-input@lightos"
        mkdir -p "$adapter" "$runtime/gnome-data/gnome-shell/modes"
        python3 - "$payload/gnome-input" "$adapter" <<'PY_ADAPTER'
import json,pathlib,re,shutil,subprocess,sys
source=pathlib.Path(sys.argv[1]);target=pathlib.Path(sys.argv[2])
version=subprocess.check_output(['/usr/bin/gnome-shell','--version'],text=True)
major=re.search(r'([0-9]+)\.',version)
if not major:raise RuntimeError('Could not identify GNOME Shell version')
major=int(major.group(1))
shutil.copyfile(source/('extension.js' if major>=45 else 'legacy.js'),target/'extension.js')
(target/'metadata.json').write_text(json.dumps({'uuid':'lightos-input@lightos','name':'LightOS input context','description':'Focused application identity for the private remote desktop session','shell-version':[str(major)],'session-modes':['user','lightos-remote']}))
PY_ADAPTER
        cp "$payload/gnome-input/lightos-remote.json" "$runtime/gnome-data/gnome-shell/modes/lightos-remote.json"
        export XDG_DATA_DIRS="$runtime/gnome-data:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
        shell_help=$(/usr/bin/gnome-shell --help 2>&1)
        if printf '%s' "$shell_help" | grep -q -- --devkit; then
          if [ ! -x /usr/libexec/mutter-devkit ] && [ ! -x /usr/lib/mutter-devkit ]; then
            echo 'GNOME requires its matching mutter-dev-bin (Debian/Ubuntu) or mutter-devkit (Arch) desktop package for a virtual session.' >&2
            exit 1
          fi
          nested=--devkit
          # GNOME 49 replaced the nested output with a devkit-owned virtual
          # monitor. Resizing its viewer/Weston no longer resizes that monitor.
          # Keep the failure scoped to this versioned path, not all desktops.
          export LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_KIND=gnome-devkit
        else
          nested=--nested
        fi
        for base in /usr/lib/systemd/user /lib/systemd/user; do
          for name in org.gnome.Shell@.service org.gnome.Shell@wayland.service; do
            source="$base/$name"
            [ -f "$source" ] || continue
            launch=$(sed -n 's/^ExecStart=//p' "$source" | head -n 1)
            [ -n "$launch" ] || continue
            mkdir -p "$units/$name.d"
            printf '[Service]\nExecStart=\nExecStart=%s %s --wayland --mode=lightos-remote\n' "$launch" "$nested" > "$units/$name.d/90-lightos-virtual.conf"
          done
        done
        export MUTTER_DEBUG_DUMMY_MODE_SPECS="${width}x${height}"
        if /usr/bin/gnome-session --help 2>&1 | grep -q -- --no-reexec; then command="$command --no-reexec"; fi
        ;;
      *xfce*|*XFCE*|*labwc*)
        # The selected Xfce session uses the distribution's compositor.
        ;;
      *plasma*|*kde*|*KDE*)
        export XDG_CURRENT_DESKTOP=KDE
        # Older KWin admits privileged protocols through desktop-file metadata.
        # Register only our executable, only in this virtual session's search path.
        mkdir -p "$runtime/kde-data/applications"
        cat > "$runtime/kde-data/applications/lightos-remoted.desktop" <<KDE_REMOTE
[Desktop Entry]
Type=Application
Name=LightOS Remote Desktop
Exec="$program"
NoDisplay=true
X-KDE-Wayland-Interfaces=org_kde_kwin_fake_input,zkde_screencast_unstable_v1,org_kde_plasma_window_management
KDE_REMOTE
        export XDG_DATA_DIRS="$runtime/kde-data:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
        # The shake effect paints a second cursor as desktop content even in
        # metadata-only screencasts. Disable it only in our private profile.
        for config_writer in kwriteconfig6 kwriteconfig5; do
          command -v "$config_writer" >/dev/null 2>&1 || continue
          "$config_writer" --file "$XDG_CONFIG_HOME/kwinrc" --group Plugins --key shakecursorEnabled false
          break
        done
        for base in /usr/lib/systemd/user /lib/systemd/user; do
          source="$base/plasma-kwin_wayland.service"
          [ -f "$source" ] || continue
          launch=$(sed -n 's/^ExecStart=//p' "$source" | head -n 1)
          [ -n "$launch" ] || continue
          mkdir -p "$units/plasma-kwin_wayland.service.d"
          # Ubuntu gives KWin a file capability for realtime scheduling. The
          # private compositor needs no elevated capabilities; retaining them
          # prevents its same-user capture service from verifying /proc/PID/exe.
          printf '[Service]\nNoNewPrivileges=yes\nExecStart=\nExecStart=%s %s --width %s --height %s\n' "$launch" "$kwin_mode" "$width" "$height" > "$units/plasma-kwin_wayland.service.d/90-lightos-virtual.conf"
        done
        ;;

    esac
    ;;
esac
for systemd in /usr/lib/systemd/systemd /lib/systemd/systemd; do
  [ -x "$systemd" ] || continue
  start user-manager.log env PATH="$original_path" SYSTEMD_UNIT_PATH="$units:" "$systemd" --user --log-target=console
  manager=$child
  wait_socket "$runtime/systemd/private" "$manager"
  /usr/bin/systemctl --user start dbus.socket
  /usr/bin/systemctl --user import-environment DISPLAY XAUTHORITY WAYLAND_DISPLAY XDG_SESSION_TYPE XDG_CURRENT_DESKTOP XDG_CONFIG_HOME XDG_CACHE_HOME XDG_DATA_DIRS XDG_RUNTIME_DIR PULSE_SERVER DBUS_SESSION_BUS_ADDRESS
  break
done
mkdir -p "$runtime/audio-config/wireplumber/wireplumber.conf.d"
cp "$payload/config/90-lightos-audio.conf" "$runtime/audio-config/wireplumber/wireplumber.conf.d/90-lightos.conf"
if wireplumber --help 2>&1 | grep -q -- --profile; then
  start wireplumber.log env XDG_CONFIG_HOME="$runtime/audio-config" wireplumber --profile=lightos
else
  mkdir -p "$runtime/audio-config/wireplumber/main.lua.d" "$runtime/audio-config/wireplumber/bluetooth.lua.d"
  printf '%s\n' 'if alsa_monitor then alsa_monitor.enabled = false end' 'if v4l2_monitor then v4l2_monitor.enabled = false end' > "$runtime/audio-config/wireplumber/main.lua.d/99-lightos.lua"
  printf '%s\n' 'if bluez_monitor then bluez_monitor.enabled = false end' > "$runtime/audio-config/wireplumber/bluetooth.lua.d/99-lightos.lua"
  start wireplumber.log env XDG_CONFIG_HOME="$runtime/audio-config" wireplumber
fi
wireplumber=$child

# Desktop and application processes retain the instance's loader, libraries and PATH.
start session.log env PATH="$desktop_path" /bin/sh -c "$command"
desktop=$child
if [ "$XDG_SESSION_TYPE" = wayland ]; then
  command -v gdbus >/dev/null 2>&1 || { echo 'The private desktop readiness helper gdbus is missing; reinstall the remote desktop runtime.' >&2; exit 1; }
  count=0
  while :; do
    selected_socket=""
    for socket in "$runtime"/wayland-*; do
      [ -S "$socket" ] || continue
      [ "$socket" != "${LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_WAYLAND:-}" ] || continue
      selected_socket=$socket
    done
    [ -z "$selected_socket" ] || break
    kill -0 "$desktop" 2>/dev/null || { cat "$runtime/session.log" >&2; exit 1; }
    count=$((count + 1)); [ "$count" -lt 480 ] || { echo 'Selected Wayland desktop did not become ready' >&2; exit 1; }
    sleep 0.25
  done
  compositor_pid=$(python3 - "$selected_socket" <<'PY_COMPOSITOR_PEER'
import socket,struct,sys
with socket.socket(socket.AF_UNIX) as connection:
    connection.connect(sys.argv[1])
    print(struct.unpack('3i',connection.getsockopt(socket.SOL_SOCKET,socket.SO_PEERCRED,12))[0])
PY_COMPOSITOR_PEER
)
  export WAYLAND_DISPLAY="$selected_socket"
  # Plasma can create a session bus distinct from its launcher's bus. Admit
  # the actual compositor before querying readiness or activating applications.
  DBUS_SESSION_BUS_ADDRESS=$(python3 "$payload/sync-wayland-activation.py" "$runtime" "$selected_socket" "$compositor_pid")
  export DBUS_SESSION_BUS_ADDRESS
fi
# Capture the actual GNOME compositor; its parent window is not its clipboard
# or cursor source. The binary verifies the private runtime, service and peer PID.
case "$session:$command" in
  wayland:*gnome*|wayland:*GNOME*|wayland:*ubuntu*)
    count=0
    until gdbus call --session --dest org.gnome.SessionManager --object-path /org/gnome/SessionManager --method org.gnome.SessionManager.IsSessionRunning 2>/dev/null | grep -q true; do
      kill -0 "$desktop" 2>/dev/null || { echo 'GNOME session exited during startup' >&2; exit 1; }
      count=$((count + 1)); [ "$count" -lt 80 ] || { echo 'GNOME session startup did not complete' >&2; exit 1; }
      sleep 0.25
    done
    set -- serve --wayland-socket "$selected_socket" --virtual-wayland-server-pid "$compositor_pid"
    ;;
  wayland:*plasma*|wayland:*kde*|wayland:*KDE*|wayland:*xfce*|wayland:*XFCE*|wayland:*labwc*)
    count=0
    if [ "$XDG_CURRENT_DESKTOP" = KDE ]; then
    until gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetNameOwner org.kde.KWin >/dev/null 2>&1 \
      && gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetNameOwner org.kde.plasmashell >/dev/null 2>&1; do
      kill -0 "$desktop" 2>/dev/null || { echo 'KDE session exited during startup' >&2; exit 1; }
      count=$((count + 1)); [ "$count" -lt 80 ] || { echo 'KDE session startup did not complete' >&2; exit 1; }
      sleep 0.25
    done
    fi
    set -- serve --wayland-socket "$selected_socket" --virtual-wayland-server-pid "$compositor_pid"
    ;;
  wayland:*)
    echo 'Selected Wayland desktop requires a native capture backend with independent cursor support.' >&2
    exit 1
    ;;
  *)
    case "$session:$command" in
      x11:*gnome*|x11:*GNOME*|x11:*ubuntu*)
        count=0
        until gdbus call --session --dest org.gnome.SessionManager --object-path /org/gnome/SessionManager --method org.gnome.SessionManager.IsSessionRunning 2>/dev/null | grep -q true; do
          kill -0 "$desktop" 2>/dev/null || { echo 'GNOME X11 session exited during startup' >&2; exit 1; }
          count=$((count + 1)); [ "$count" -lt 80 ] || { echo 'GNOME X11 session startup did not complete' >&2; exit 1; }
          sleep 0.25
        done
        ;;
    esac
    set -- serve --x11-display "$DISPLAY" --xauthority "$XAUTHORITY" --virtual-x11-server-pid "$xvfb"
    if [ "$virtual_dynamic" = 1 ]; then set -- "$@" --virtual-x11-dynamic-modes; fi
    ;;
esac
if [ "${LIGHTOS_REMOTE_DESKTOP_SESSION_ONLY:-0}" = 1 ]; then
  # Only publish session coordinates; the parent owns authentication and media.
  python3 - "$runtime" "$@" <<'PY_READY'
import json,os,pathlib,sys
runtime=pathlib.Path(sys.argv[1])
value={'version':1,'arguments':sys.argv[2:], 'environment':{key:os.environ[key] for key in (
 'PULSE_SERVER','PULSE_SOURCE','DBUS_SESSION_BUS_ADDRESS','LIGHTOS_REMOTE_DESKTOP_GNOME_X11',
 'LIGHTOS_REMOTE_DESKTOP_NESTED_RESIZE_HELPER','LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_KIND',
 'LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_PID','LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_DISPLAY',
 'LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_AUTHORITY','LIGHTOS_REMOTE_DESKTOP_RESIZE_PARENT_WAYLAND') if key in os.environ}}
temporary=runtime/'installed-session.pending'
with temporary.open('x') as output:
 os.chmod(temporary,0o600);json.dump(value,output)
os.replace(temporary,runtime/'installed-session.json')
PY_READY
  transport=$desktop
else
  start remote-desktop.log "$program" "$@"
  transport=$child
fi
while kill -0 "$transport" 2>/dev/null; do
  for pid in "$desktop" "$xvfb" "$bus" "$pipewire" "$pulse" "$wireplumber" ${parent:-} ${manager:-}; do
    kill -0 "$pid" 2>/dev/null || { echo 'Owned desktop dependency exited' >&2; exit 1; }
  done
  if [ "$XDG_SESSION_TYPE" = wayland ]; then
    ready=0
    for socket in "$runtime"/wayland-*; do [ -S "$socket" ] && ready=1; done
    [ "$ready" = 1 ] || { echo 'Selected Wayland compositor exited' >&2; exit 1; }
  fi
  sleep 1
done
wait "$transport"
