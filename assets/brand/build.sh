#!/usr/bin/env bash
# Rebuild the brand assets from the generated sources in source/.
# Needs ImageMagick 7 (`magick`). Outputs land next to this script and in the
# web assets the daemon embeds (crates/taskstaka/assets).
#
#   logo.png      full scene (source/logo-1.jpeg), transparent, 1024 square
#   logo-alt.png  the same from source/logo-2.jpeg (two falling cubes)
#   mark.png      the pyramid and the falling cube only, no ground cubes
#   favicon.ico, apple-touch-icon.png, icon-192.png, icon-512.png   from mark.png
#
# The sources are generated pixel art and not on a clean pixel grid, so they are
# cleaned at full size (JPEG noise snapped to 14 colours, backdrop made
# transparent) rather than reduced to a pixel grid.
set -euo pipefail
cd "$(dirname "$0")"
WEB=../../crates/taskstaka/assets
BG='#0f1012'
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT

clean() { # src dst
  magick "$1" +dither -colors 14 -alpha set -fuzz 4% -fill none \
    -draw 'color 5,5 floodfill' -draw 'color 2040,5 floodfill' \
    -draw 'color 5,2040 floodfill' -draw 'color 2040,2040 floodfill' "$2"
}
square() { # src dst: trim, pad 6%, centre on a transparent square, 1024px
  magick "$1" -trim +repage "$tmp/t.png"
  local w h m
  w=$(magick identify -format %w "$tmp/t.png"); h=$(magick identify -format %h "$tmp/t.png")
  m=$(( (w > h ? w : h) * 106 / 100 ))
  magick "$tmp/t.png" -background none -gravity center -extent "${m}x${m}" -filter Lanczos -resize 1024x1024 "$2"
}
on_dark() { # size mark-percent dst: the mark centred on the dark background
  magick -size "$1x$1" "xc:$BG" \( mark.png -filter Lanczos -resize "$(( $1 * $2 / 100 ))x$(( $1 * $2 / 100 ))" \) \
    -gravity center -composite "$3"
}

clean source/logo-1.jpeg "$tmp/a.png"
clean source/logo-2.jpeg "$tmp/b.png"
square "$tmp/a.png" logo.png
square "$tmp/b.png" logo-alt.png

# The mark: drop the three cubes resting on the ground in source 1 (they are at
# fixed positions in the 2048px source) by keeping only the pyramid's region.
magick "$tmp/a.png" \( -size 2048x2048 xc:none -fill white \
  -draw 'rectangle 0,0 1469,1319' -draw 'rectangle 561,1320 1099,2047' \
  -draw 'rectangle 1100,1320 1469,1439' \) -compose DstIn -composite "$tmp/p.png"
square "$tmp/p.png" mark.png

magick mark.png -filter Lanczos -resize 256x256 "$tmp/m256.png"
magick "$tmp/m256.png" -define icon:auto-resize=48,32,16 favicon.ico
on_dark 180 80 apple-touch-icon.png
on_dark 192 80 icon-192.png
on_dark 512 80 icon-512.png

# What the daemon serves.
cp favicon.ico apple-touch-icon.png "$WEB/"
magick mark.png -filter Lanczos -resize 128x128 "$WEB/mark.png"
echo "brand assets rebuilt"
