#!/usr/bin/env bash

set -eu

ffmpeg -i "$1" -c:v libx264 -crf 28 -preset slow -pix_fmt yuv420p \
    -vf "scale=1920:-2,fps=30" -an -movflags +faststart "${1%.*}.mp4"
