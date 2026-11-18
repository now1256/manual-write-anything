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

echo "[5/5] dmg 만들기"
# Built with hdiutil directly: Tauri's dmg step styles the window through Finder
# (AppleScript), which times out without Automation permission.
bundle=app/src-tauri/target/release/bundle
app_path="$bundle/macos/Manual Write Anything.app"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' app/src-tauri/Cargo.toml | head -1)
arch=$(uname -m)
mkdir -p "$bundle/dmg"
dmg="$bundle/dmg/Manual-Write-Anything_${version}_${arch}.dmg"
stage=$(mktemp -d)
cp -R "$app_path" "$stage/"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -volname "Manual Write Anything" -srcfolder "$stage" -ov -format UDZO "$dmg" >/dev/null
rm -rf "$stage"

echo "완료"
echo "앱: $app_path"
echo "dmg: $dmg"
