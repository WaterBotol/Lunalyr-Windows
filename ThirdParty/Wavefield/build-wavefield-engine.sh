#!/bin/bash
set -euo pipefail
project="$(cd -- "$(dirname -- "$0")/.." && pwd)"
revision=3858a657190ebd09bee704caa094e7e1fd3c6cc2
source_repo="${1:?Pass a local clone of https://github.com/niko-dellic/wavefield}"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

# Build the exact upstream revision using its checked-in npm lock.
git -C "$source_repo" archive "$revision" | tar -xf - -C "$stage"
cp "$project"/ThirdParty/Wavefield/lunalyr-{engine.ts,vite.config.mjs} "$stage/"
(
  cd "$stage"
  # The upstream lock omits optional peer entries required by recent npm.
  npm ci --legacy-peer-deps --ignore-scripts --no-audit --no-fund
  printf '%s\n' '{"extends":"./tsconfig.json","include":["src","lunalyr-engine.ts"]}' > tsconfig.lunalyr.json
  ./node_modules/.bin/tsc --project tsconfig.lunalyr.json
  npm run test:analysis
  ./node_modules/.bin/vite build --config lunalyr-vite.config.mjs
)
python3 - "$stage" "$project/Sources/Resources" <<'PY'
import json
from pathlib import Path
import sys

stage, resources = map(Path, sys.argv[1:])
script = (stage / "dist/wavefield.js").read_text()
# Prevent embedded text in the bundle from terminating the script element.
script = script.replace("</script", r"<\/script")
(resources / "WavefieldEngine.html").write_text(
    '<!doctype html>\n<meta charset="utf-8">\n'
    '<meta http-equiv="Content-Security-Policy" '
    'content="default-src \'none\'; script-src \'unsafe-inline\'; style-src \'unsafe-inline\'">\n'
    '<style>html,body,canvas{margin:0;width:100%;height:100%;overflow:hidden;display:block;background:#000}</style>\n'
    '<canvas></canvas>\n<script>\n' + script + '\n</script>\n'
)
presets = [
    {"id": path.stem, "name": json.loads(path.read_text())["name"]}
    for path in sorted((stage / "src/templates").glob("*.json"))
]
presets.sort(key=lambda preset: (preset["id"] != "default", preset["name"].casefold()))
(resources / "WavefieldPresets.json").write_text(json.dumps(presets, indent=2) + "\n")
PY

# The IIFE includes these two dependencies. The original app shell and its
# other runtime packages are outside the entry graph.
{
  printf '%s\n\n' 'Three.js'
  cat "$stage/node_modules/three/LICENSE"
  printf '\n%s\n\n' 'postprocessing'
  cat "$stage/node_modules/postprocessing/LICENSE.md"
} > "$project/Sources/Resources/Wavefield-Web-Dependencies-LICENSE.txt"
