#!/usr/bin/env bash
# Glass test-suite: build + unit tests + CLI + synthetic encode + pipeline asserts.
# PRIVACY: encoding is tested ONLY with synthetic sources (gst videotestsrc /
# ffmpeg lavfi). No real screen is ever captured and no portal session is started.
set -euo pipefail
cd "$(dirname "$0")"

export TC="$HOME/.local/toolchains"
export PATH="$HOME/.cargo/bin:$TC/go/go/bin:$TC/lua/bin:$TC/node/bin:$HOME/.local/bin:$PATH"

PASS=0
ok()   { PASS=$((PASS + 1)); printf '  [ok]   %s\n' "$1"; }
fail() { printf '  [FAIL] %s\n' "$1" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "missing required tool: $1"; }

need cargo
need gst-launch-1.0
need ffmpeg
need ffprobe

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "[1] cargo build --release"
if cargo build --release >/dev/null 2>&1; then ok "release build"; else fail "release build"; fi
BIN=./target/release/glass

echo "[2] cargo test (pure helpers + child lifecycle)"
if cargo test >/dev/null 2>&1; then ok "unit tests"; else fail "unit tests"; fi

echo "[3] glass --version"
V="$("$BIN" --version)"
if [ "$V" = "glass 0.1.1" ]; then ok "--version prints '$V'"; else fail "--version got '$V'"; fi

echo "[4] glass --help"
if "$BIN" --help >/dev/null 2>&1; then ok "--help exits 0"; else fail "--help"; fi

echo "[5] GStreamer synthetic encode (videotestsrc -> x264enc -> mp4mux)"
if gst-launch-1.0 -q videotestsrc num-buffers=30 ! x264enc speed-preset=ultrafast ! mp4mux ! filesink location="$TMP/gst.mp4" >/dev/null 2>&1 && [ -s "$TMP/gst.mp4" ]; then
  ok "gst mp4 created"
else
  fail "gst synthetic encode"
fi

echo "[6] ffprobe: gst duration > 0"
D="$(ffprobe -v error -show_entries format=duration -of default=nw=1:nk=1 "$TMP/gst.mp4")"
if awk "BEGIN{exit !($D>0)}"; then ok "gst duration=${D}s"; else fail "gst duration '$D'"; fi

echo "[7] FFmpeg synthetic encode (lavfi testsrc -> libx264)"
if ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc=duration=1:size=320x240:rate=30 -c:v libx264 -pix_fmt yuv420p "$TMP/ff.mp4" >/dev/null 2>&1 && [ -s "$TMP/ff.mp4" ]; then
  ok "ffmpeg mp4 created"
else
  fail "ffmpeg synthetic encode"
fi

echo "[8] ffprobe: ffmpeg duration > 0"
D2="$(ffprobe -v error -show_entries format=duration -of default=nw=1:nk=1 "$TMP/ff.mp4")"
if awk "BEGIN{exit !($D2>0)}"; then ok "ffmpeg duration=${D2}s"; else fail "ffmpeg duration '$D2'"; fi

echo "[9] ffprobe: gst codec is h264"
C="$(ffprobe -v error -select_streams v:0 -show_entries stream=codec_name -of default=nw=1:nk=1 "$TMP/gst.mp4")"
if [ "$C" = "h264" ]; then ok "gst codec=h264"; else fail "gst codec '$C'"; fi

echo "[10] ffprobe: ffmpeg codec is h264"
C2="$(ffprobe -v error -select_streams v:0 -show_entries stream=codec_name -of default=nw=1:nk=1 "$TMP/ff.mp4")"
if [ "$C2" = "h264" ]; then ok "ffmpeg codec=h264"; else fail "ffmpeg codec '$C2'"; fi

echo "[11] Wayland pipeline builder (pipewiresrc, native rate passthrough)"
W="$("$BIN" --dump-pipeline wayland 3840 2160 144 "$TMP/o.mp4")"
if grep -q 'pipewiresrc fd=3' <<<"$W" && grep -q '3840x2160' <<<"$W" && grep -q 'mp4mux' <<<"$W" && ! grep -q 'video/x-raw,framerate' <<<"$W"; then
  ok "wayland pipeline (no strict framerate filter)"
else
  fail "wayland pipeline: $W"
fi

echo "[12] X11 pipeline builder (ximagesrc + Hz)"
X="$("$BIN" --dump-pipeline x11 1920 1080 60 "$TMP/o.mp4")"
if grep -q 'ximagesrc' <<<"$X" && grep -q 'framerate=60/1' <<<"$X"; then
  ok "x11 pipeline"
else
  fail "x11 pipeline: $X"
fi

echo "[13] Windows pipeline builder (gdigrab default + native res + Hz)"
N="$("$BIN" --dump-pipeline windows 2560 1440 144 "$TMP/o.mp4")"
if grep -q 'gdigrab' <<<"$N" && grep -q '2560x1440' <<<"$N" && grep -q 'framerate 144' <<<"$N"; then
  ok "windows pipeline"
else
  fail "windows pipeline: $N"
fi

echo "[14] glass --probe (display + encoder detection, no capture)"
if "$BIN" --probe >/dev/null 2>&1; then ok "--probe exits 0"; else fail "--probe"; fi

echo
echo "ALL CHECKS PASSED ($PASS checks)"