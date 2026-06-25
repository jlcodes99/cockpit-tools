#!/bin/sh
set -eu

export DISPLAY="${DISPLAY:-:1}"
export HOME="${HOME:-/home/app}"
export NO_AT_BRIDGE="${NO_AT_BRIDGE:-1}"
export WEBKIT_DISABLE_DMABUF_RENDERER="${WEBKIT_DISABLE_DMABUF_RENDERER:-1}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-app}"

mkdir -p "$HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR" || true

if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ] && command -v dbus-launch >/dev/null 2>&1; then
  eval "$(dbus-launch --sh-syntax)"
fi

Xvfb "$DISPLAY" -screen 0 "${XVFB_WHD:-1280x800x24}" -nolisten tcp >/tmp/xvfb.log 2>&1 &
sleep 1

fluxbox >/tmp/fluxbox.log 2>&1 &
sleep 1

x11vnc \
  -display "$DISPLAY" \
  -forever \
  -shared \
  -nopw \
  -localhost \
  -rfbport 5900 \
  >/tmp/x11vnc.log 2>&1 &

websockify \
  --web=/usr/share/novnc/ \
  0.0.0.0:6080 \
  localhost:5900 \
  >/tmp/websockify.log 2>&1 &

echo "noVNC is available on http://127.0.0.1:6080/vnc.html"
echo "Starting Cockpit Tools..."

exec /bin/sh /usr/local/bin/run-cockpit-tools
