#!/usr/bin/env bash
# Builds the macOS app (.app and .dmg) from a clone of this repository. Run on a Mac.
# Needs: Rust (https://rustup.rs), Node.js 20+ (e.g. `brew install node`).
set -euo pipefail
cd "$(dirname "$0")/.."

need() { command -v "$1" >/dev/null 2>&1 || { echo "필요한 프로그램이 없습니다: $1 ($2)"; exit 1; }; }
need cargo "https://rustup.rs 에서 설치"
need npm "brew install node"

echo "[1/5] 화면(web) 빌드"
(cd web && npm ci && npm run build)

echo "[2/5] 엔진(mwa) 빌드"
cargo build --release -p mwa
triple=$(rustc -vV | sed -n 's/^host: //p')
mkdir -p app/src-tauri/binaries
cp target/release/mwa "app/src-tauri/binaries/mwa-$triple"

echo "[3/5] 촬영기 준비 (Playwright + Chromium, 처음 한 번은 브라우저를 내려받습니다)"
rm -rf app/src-tauri/capture
mkdir -p app/src-tauri/capture
cp sidecar/capture/index.mjs sidecar/capture/package.json sidecar/capture/package-lock.json app/src-tauri/capture/
(cd app/src-tauri/capture && npm ci --omit=dev && npx playwright install chromium)

echo "[4/5] 앱 빌드 (Tauri)"
(cd app && npm ci && npm run tauri build)

echo "[5/5] 완료"
ls -1 app/src-tauri/target/release/bundle/dmg/*.dmg
echo "앱: app/src-tauri/target/release/bundle/macos/Manual Write Anything.app"
