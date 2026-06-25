#!/bin/sh
set -eu

export HOME="${HOME:-/home/app}"
export NO_AT_BRIDGE="${NO_AT_BRIDGE:-1}"
export WEBKIT_DISABLE_DMABUF_RENDERER="${WEBKIT_DISABLE_DMABUF_RENDERER:-1}"

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-app}"
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-/home/app/.config}"
export XDG_DATA_HOME="${XDG_DATA_HOME:-/home/app/.local/share}"
export XDG_CACHE_HOME="${XDG_CACHE_HOME:-/home/app/.cache}"

mkdir -p \
  "$HOME" \
  "$XDG_RUNTIME_DIR" \
  "$XDG_CONFIG_HOME" \
  "$XDG_DATA_HOME" \
  "$XDG_CACHE_HOME"

chmod 700 "$XDG_RUNTIME_DIR" || true

if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ] && command -v dbus-launch >/dev/null 2>&1; then
  eval "$(dbus-launch --sh-syntax)"
fi

echo "Running as: $(id)"
echo "HOME=$HOME"
echo "XDG_CONFIG_HOME=$XDG_CONFIG_HOME"
echo "XDG_DATA_HOME=$XDG_DATA_HOME"
echo "XDG_CACHE_HOME=$XDG_CACHE_HOME"

if command -v cockpit-tools >/dev/null 2>&1; then
  exec cockpit-tools
fi

if [ -x /usr/bin/cockpit-tools ]; then
  exec /usr/bin/cockpit-tools
fi

echo "Cannot find cockpit-tools executable" >&2
dpkg -L cockpit-tools 2>/dev/null || true
exit 127
