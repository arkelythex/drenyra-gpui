#!/usr/bin/env bash
# End-to-end check of the native Drenyra GPUI window on an isolated virtual X display.
#
# Runs the real binary (built with `--features gpui`) inside its own Xvfb server, drives it with
# xdotool (XTEST input only reaches that display, never the operator's desktop), asserts the
# non-visual outcomes and leaves screenshots for visual review.
#
# Requirements: Xvfb (Arch: xorg-server-xvfb), xdotool, ImageMagick (`import`, `compare`), a
# Vulkan driver.
# The agent pane starts `pi --mode rpc` when `pi` is on PATH; no prompt is ever sent, so no
# model is called.
#
# Usage: scripts/drenyra-gpui-e2e.sh [output-dir]
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${1:-$(mktemp -d "${TMPDIR:-/tmp}/drenyra-gpui-e2e.XXXXXX")}
BIN="${CARGO_TARGET_DIR:-$ROOT/target}/debug/drenyra-gpui"
mkdir -p "$OUT/xdg"

for tool in Xvfb xdotool import compare; do
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
# Mouse wheel down N notches at window-relative X Y (XTEST on this Xvfb display only).
wheel() {
  xdotool mousemove --window "$WIN" "$1" "$2"; sleep 0.2
  xdotool click --repeat "$3" --delay 80 5; sleep 0.4
}
# True when the WxH+X+Y region differs between two screenshots (compare exits 1 on a difference).
# shellcheck disable=SC2329 # invoked through check's eval
region_changed() {
  local rc=0
  compare -metric AE "$OUT/$1.png[$3]" "$OUT/$2.png[$3]" null: 2>/dev/null || rc=$?
  [ "$rc" -eq 1 ]
}

shot 01-start
key ctrl+p; shot 02-approval-blocked; key Escape
for _ in 1 2 3; do key a; key j; done
key ctrl+p; typ "CPA-PERU-001"; shot 03-approval-ready
key Return; shot 04-approved
# Re-approval: the same ID is refused, a different one replaces it explicitly.
key ctrl+p; typ "CPA-PERU-001"; key Return; shot 04b-same-approver-refused; key Escape
key ctrl+p; typ "CPA-PERU-002"; shot 04c-replace-dialog; key Return; shot 04d-replaced
# Ledger, Evidence and CLI panes (each split opens the next kind), then back to one pane.
key ctrl+d; key ctrl+d; key ctrl+d; shot 04e-ledger-evidence-cli
# Scroll: in this layout (Ledger bottom left, Evidence and CLI stacked bottom right) the Evidence
# and CLI bodies overflow; the wheel must reveal older events and the flag reference while the
# pane headers and footnotes stay put.
EVIDENCE_REGION=350x75+640+528
CLI_REGION=350x75+640+735
wheel 815 580 6; wheel 815 780 6; wheel 445 700 6; shot 04e1-panes-scrolled
check "mouse wheel scrolls the Evidence pane" \
  "region_changed 04e-ledger-evidence-cli 04e1-panes-scrolled $EVIDENCE_REGION"
check "mouse wheel scrolls the CLI pane" \
  "region_changed 04e-ledger-evidence-cli 04e1-panes-scrolled $CLI_REGION"
# Focus returns to the agent after each close, so step to the next pane before closing it.
key ctrl+w; key ctrl+Tab; key ctrl+w; key ctrl+Tab; key ctrl+w; shot 04e2-agent-only
# Restart: kill Andino's agent, then Ctrl+Shift+R must start a new process.
OLD_AGENT=$(pgrep -P "$APP_PID" | head -1 || true)
if [ -n "$OLD_AGENT" ]; then
  kill "$OLD_AGENT"; sleep 2; shot 04f-agent-stopped
  key ctrl+shift+r; sleep 6; shot 04g-agent-restarted
  NEW_AGENT=$(pgrep -P "$APP_PID" | head -1 || true)
  check "restart started a new agent ($OLD_AGENT -> ${NEW_AGENT:-none})" \
    "[ -n '$NEW_AGENT' ] && [ '$NEW_AGENT' != '$OLD_AGENT' ]"
else
  echo "SKIP: restart check (no agent process; is pi installed?)"
fi
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
