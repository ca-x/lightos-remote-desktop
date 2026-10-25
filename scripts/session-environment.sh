#!/bin/sh
set -eu
lightos_application_environment="$(dirname -- "$0")/application-environment.sh"
if [ -f "$lightos_application_environment" ]; then
    . "$lightos_application_environment"
fi
unset lightos_application_environment
dbus-update-activation-environment \
    "DISPLAY=${DISPLAY:-}" \
    "WAYLAND_DISPLAY=$WAYLAND_DISPLAY" \
    "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR" \
    "XDG_STATE_HOME=${XDG_STATE_HOME:-}" \
    "OMARCHY_PATH=${OMARCHY_PATH:-}" \
    "PIPEWIRE_RUNTIME_DIR=${PIPEWIRE_RUNTIME_DIR:-}" \
    "DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS" \
    "HYPRLAND_INSTANCE_SIGNATURE=$HYPRLAND_INSTANCE_SIGNATURE" \
    "XDG_CURRENT_DESKTOP=${XDG_CURRENT_DESKTOP:-Hyprland}" \
    "XDG_SESSION_TYPE=wayland" \
    "PATH=$PATH" \
    "CHROME_CONFIG_HOME=${CHROME_CONFIG_HOME:-}" \
    "CHROME_USER_DATA_DIR=" \
    "PULSE_SERVER=${PULSE_SERVER:-}" \
    "PULSE_CLIENTCONFIG=${PULSE_CLIENTCONFIG:-}"
printf '%s\n' "${DISPLAY:-}" > "$1.pending"
mv -f -- "$1.pending" "$1"
