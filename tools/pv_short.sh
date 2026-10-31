#!/bin/sh
# Build the 10 s ONE CUT short: events JSON -> sound -> sync check -> MP4 + looping GIF. Usage: tools/pv_short.sh [OUTDIR]
set -e
out="${1:-out}"; mkdir -p "$out"
cargo build --release -q
bin=./target/release/ascii-shader
"$bin" --story pvshort --film-events "$out/pv-short.json" 2>/dev/null
python3 tools/pv_short_audio.py "$out/pv-short.json" "$out/pv-short.wav"
python3 tools/pv_short_check.py "$out/pv-short.json" "$out/pv-short.wav"
"$bin" --story pvshort --film "$out/pv-short.mp4" --audio "$out/pv-short.wav" 2>/dev/null
ffmpeg -y -loglevel error -i "$out/pv-short.mp4" -vf "fps=15,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=64[p];[b][p]paletteuse=dither=none" -loop 0 "$out/pv-short.gif"
ls -l "$out/pv-short.mp4" "$out/pv-short.gif"
