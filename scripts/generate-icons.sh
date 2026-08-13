#!/bin/sh
set -eu

PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SOURCE_ICON="$PROJECT_DIR/src-tauri/icons/fips-mac-mark.svg"
OUTPUT_DIR="$PROJECT_DIR/src-tauri/icons"
ICON_WORK_DIR=$(mktemp -d)
ICONSET_DIR="$ICON_WORK_DIR/FIPS.iconset"

cleanup() {
  case "$ICON_WORK_DIR" in
    "${TMPDIR:-/tmp}"/*|/var/folders/*) rm -R -- "$ICON_WORK_DIR" ;;
  esac
}
trap cleanup EXIT HUP INT TERM

command -v magick >/dev/null 2>&1 || {
  echo "ImageMagick is required (brew install imagemagick)." >&2
  exit 1
}
command -v iconutil >/dev/null 2>&1 || {
  echo "iconutil is required and ships with macOS." >&2
  exit 1
}

mkdir -p "$ICONSET_DIR"
magick -background none "$SOURCE_ICON" -resize 1024x1024 -depth 8 \
  "PNG32:$ICON_WORK_DIR/master.png"

magick "$ICON_WORK_DIR/master.png" -resize 32x32 -depth 8 "PNG32:$OUTPUT_DIR/32x32.png"
magick "$ICON_WORK_DIR/master.png" -resize 64x64 -depth 8 "PNG32:$OUTPUT_DIR/64x64.png"
magick "$ICON_WORK_DIR/master.png" -resize 128x128 -depth 8 "PNG32:$OUTPUT_DIR/128x128.png"
magick "$ICON_WORK_DIR/master.png" -resize 256x256 -depth 8 "PNG32:$OUTPUT_DIR/128x128@2x.png"
magick "$ICON_WORK_DIR/master.png" -resize 512x512 -depth 8 "PNG32:$OUTPUT_DIR/icon.png"

for icon_size in 16 32 128 256 512; do
  retina_size=$((icon_size * 2))
  magick "$ICON_WORK_DIR/master.png" -resize "${icon_size}x${icon_size}" -depth 8 \
    "PNG32:$ICONSET_DIR/icon_${icon_size}x${icon_size}.png"
  magick "$ICON_WORK_DIR/master.png" -resize "${retina_size}x${retina_size}" -depth 8 \
    "PNG32:$ICONSET_DIR/icon_${icon_size}x${icon_size}@2x.png"
done

iconutil -c icns "$ICONSET_DIR" -o "$OUTPUT_DIR/icon.icns"
magick "$ICON_WORK_DIR/master.png" \
  -define icon:auto-resize=256,128,64,48,32,16 "$OUTPUT_DIR/icon.ico"

echo "Generated application icons from $SOURCE_ICON"
