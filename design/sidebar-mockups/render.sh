#!/bin/sh
# Renders sidebar.html to PNGs (2x) with headless Chrome. Usage: ./render.sh
set -e
cd "$(dirname "$0")"
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
render() { # name variant theme height
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=2 \
    --window-size="$5","$4" --virtual-time-budget=1500 \
    --screenshot="$PWD/$1.png" "file://$PWD/sidebar.html?v=$2&t=$3" >/dev/null 2>&1
}
render before-light before light 1010 760
render before-dark before dark 1010 760
render a-light a light 760 1040
render a-dark a dark 900 1040
render b-light b light 860 760
render b-dark b dark 990 760
