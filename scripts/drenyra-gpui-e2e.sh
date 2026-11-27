#!/usr/bin/env bash
# End-to-end check of the native Drenyra GPUI window on an isolated virtual X display.
#
# Runs the real binary (built with `--features gpui`) inside its own Xvfb server, drives it with
# xdotool (XTEST input only reaches that display, never the operator's desktop), asserts the
# non-visual outcomes and leaves screenshots for visual review.
#
# Requirements: Xvfb (Arch: xorg-server-xvfb), xdotool, ImageMagick (`import`), a Vulkan driver.
# The agent pane starts `pi --mode rpc` when `pi` is on PATH; no prompt is ever sent, so no
# model is called.
#
# Usage: scripts/drenyra-gpui-e2e.sh [output-dir]
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${1:-$(mktemp -d "${TMPDIR:-/tmp}/drenyra-gpui-e2e.XXXXXX")}
BIN="${CARGO_TARGET_DIR:-$ROOT/target}/debug/drenyra-gpui"
mkdir -p "$OUT/xdg"

for tool in Xvfb xdotool import; do
  command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 2; }
done

cargo build -q --manifest-path "$ROOT/Cargo.toml" -p drenyra-app --features gpui --locked

# First free display number from :90.
DISPLAY_NUM=90
while [ -e "/tmp/.X11-unix/X$DISPLAY_NUM" ] || [ -e "/tmp/.X$DISPLAY_NUM-lock" ]; do
  DISPLAY_NUM=$((DISPLAY_NUM + 1))
done
export DISPLAY=":$DISPLAY_NUM"

Xvfb "$DISPLAY" -screen 0 1600x1000x24 -nolisten tcp >/dev/null 2>&1 &
XVFB_PID=$!
APP_PID=""
# shellcheck disable=SC2329 # invoked by the EXIT trap
cleanup() {
  [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null || true
  kill "$XVFB_PID" 2>/dev/null || true
}
trap cleanup EXIT
sleep 1

# Xvfb has no DRI3, so Vulkan must present through Mesa's software path.
env -u WAYLAND_DISPLAY MESA_VK_WSI_DEBUG=sw XDG_CONFIG_HOME="$OUT/xdg" \
  "$BIN" --window >"$OUT/app.log" 2>&1 &
APP_PID=$!

WIN=""
for _ in $(seq 1 30); do
  WIN=$(xdotool search --name 'Drenyra GPUI' 2>/dev/null | head -1 || true)
  [ -n "$WIN" ] && break
  sleep 1
done
[ -n "$WIN" ] || { echo "FAIL: window never appeared" >&2; exit 1; }
# The first frame is presented on the first configure/expose under the software WSI.
xdotool windowsize "$WIN" 1440 901
xdotool windowfocus "$WIN"
sleep 10 # let the agent start (Ready) when pi is installed

shot() { sleep 0.6; import -window "$WIN" "$OUT/$1.png"; }
key() { xdotool key --clearmodifiers "$@"; sleep 0.35; }
typ() { xdotool type --delay 40 "$1"; sleep 0.3; }
fail=0
check() { if eval "$2"; then echo "PASS: $1"; else echo "FAIL: $1"; fail=1; fi; }

shot 01-start
key ctrl+p; shot 02-approval-blocked; key Escape
for _ in 1 2 3; do key a; key j; done
key ctrl+p; typ "CPA-PERU-001"; shot 03-approval-ready
key Return; shot 04-approved
key ctrl+shift+p; typ "sierra"; shot 05-palette
key Return; sleep 8; shot 06-sierra
key ctrl+b; shot 07-no-sidebar; key ctrl+b
key ctrl+shift+p; typ "andino"; key Return
key i; typ "saldo@caja ñ jk"; shot 08-composer; key Escape
key ctrl+comma; key j; key Return; shot 09-light-settings
key Escape; shot 10-light-main

check "theme saved to settings file" \
  "grep -q 'theme = \"dreamcoder-light\"' '$OUT/xdg/drenyra-gpui/settings.toml'"
CHILDREN=$(pgrep -P "$APP_PID" || true)
key ctrl+q
for _ in $(seq 1 10); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 1; done
check "Ctrl+Q closes the window" "! kill -0 $APP_PID 2>/dev/null"
wait "$APP_PID" && status=0 || status=$?
APP_PID=""
check "exit code 0 (got $status)" "[ $status -eq 0 ]"
for child in $CHILDREN; do
  check "agent process $child exited" "! kill -0 $child 2>/dev/null"
done

echo "screenshots and logs: $OUT"
exit "$fail"
