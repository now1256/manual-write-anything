#!/usr/bin/env bash
# mwa 설치: 빌드 → 촬영 브라우저 → (선택) 서비스 등록 → 사내망 주소 안내.
# 사용법: 저장소 폴더에서 ./install.sh  (다시 실행하면 새 코드로 다시 설치합니다)
set -euo pipefail

cd "$(dirname "$0")"
REPO="$(pwd)"

say()  { printf '\n\033[1m▶ %s\033[0m\n' "$*"; }
ok()   { printf '  ✓ %s\n' "$*"; }
fail() { printf '  ✗ %s\n' "$*" >&2; exit 1; }
ask()  { local q="$1" d="$2" a; read -r -p "  $q [$d]: " a </dev/tty || true; echo "${a:-$d}"; }

# ---------------------------------------------------------------- 준비물
say "준비물 확인"
need() { command -v "$1" >/dev/null 2>&1; }

need git || fail "git이 없습니다. 먼저 설치하세요 (Ubuntu: sudo apt install git)"
ok "git"

if ! need cargo; then
  [ -x "$HOME/.cargo/bin/cargo" ] && export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! need cargo; then
  echo "  Rust가 없습니다. 설치합니다 (rustup, 사용자 폴더에만 설치)."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  export PATH="$HOME/.cargo/bin:$PATH"
fi
ok "Rust $(rustc --version | cut -d' ' -f2)"

need node || fail "Node.js 20 이상이 필요합니다. https://nodejs.org 또는 패키지 관리자로 설치하세요"
NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]')"
[ "$NODE_MAJOR" -ge 20 ] || fail "Node.js $NODE_MAJOR 입니다. 20 이상이 필요합니다"
ok "Node.js $(node --version)"

if need pdftotext; then ok "poppler (PDF 읽기)"
else echo "  ! poppler가 없습니다. 기존 매뉴얼 PDF를 읽으려면 설치하세요 (Ubuntu: sudo apt install poppler-utils, macOS: brew install poppler)"; fi

if ! need claude; then
  echo "  Claude Code CLI가 없습니다. 설치합니다 (npm -g)."
  npm i -g @anthropic-ai/claude-code || fail "설치하지 못했습니다. 직접 실행하세요: npm i -g @anthropic-ai/claude-code"
fi
ok "Claude Code CLI"

# Logged in already? The official CLI says so; its tokens are never read here.
LOGGED_IN=no
claude auth status --json 2>/dev/null \
  | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{try{process.exit(JSON.parse(s).loggedIn?0:1)}catch{process.exit(1)}})' \
  && LOGGED_IN=yes
if [ "$LOGGED_IN" = yes ]; then
  ok "Claude 로그인됨"
elif [ "$(ask 'Claude 구독 로그인을 지금 할까요? 나중에 해도 됩니다 (y/n)' n)" = y ]; then
  claude auth login --claudeai </dev/tty && LOGGED_IN=yes && ok "Claude 로그인됨"
fi

# ---------------------------------------------------------------- 빌드
say "화면 빌드"
(cd web && npm ci --no-audit --no-fund && npm run build) >/dev/null
ok "web/dist"

say "엔진 빌드·설치 (처음엔 몇 분 걸립니다)"
cargo install --locked --path crates/cli --root "$HOME/.cargo" --force >/dev/null 2>&1 \
  || cargo install --path crates/cli --root "$HOME/.cargo" --force
MWA="$HOME/.cargo/bin/mwa"
ok "$MWA"

# ---------------------------------------------------------------- 프로젝트
say "프로젝트"
# Other projects are added and switched from the screen's top bar.
PROJECT="$HOME/manuals/project"
ok "$PROJECT"
PORT="$(ask '열 포트' 7000)"
"$MWA" -C "$PROJECT" browser
ok "촬영 브라우저"

# ---------------------------------------------------------------- 서비스 (선택)
SERVICE=no
if need systemctl && [ "$(ask '서버가 켜질 때 자동으로 띄울까요? (y/n)' y)" = y ]; then
  UNIT="[Unit]
Description=mwa — Manual Write Anything
After=network-online.target

[Service]
ExecStart=$MWA -C $PROJECT serve --addr 0.0.0.0:$PORT
Restart=on-failure
Environment=PATH=$HOME/.cargo/bin:$(dirname "$(command -v node)"):$(dirname "$(command -v claude)"):/usr/local/bin:/usr/bin:/bin
"
  if [ "$(id -u)" = 0 ]; then
    printf '%s\nUser=root\n\n[Install]\nWantedBy=multi-user.target\n' "$UNIT" > /etc/systemd/system/mwa.service
    systemctl daemon-reload && systemctl enable --now mwa.service && systemctl restart mwa.service
  else
    mkdir -p "$HOME/.config/systemd/user"
    printf '%s\n[Install]\nWantedBy=default.target\n' "$UNIT" > "$HOME/.config/systemd/user/mwa.service"
    systemctl --user daemon-reload && systemctl --user enable --now mwa.service && systemctl --user restart mwa.service
    # Keeps the service running after this user logs out (may need sudo).
    loginctl enable-linger "$USER" 2>/dev/null || echo "  ! 로그아웃 뒤에도 띄우려면: sudo loginctl enable-linger $USER"
  fi
  SERVICE=yes
  ok "서비스 mwa 등록"
fi

# ---------------------------------------------------------------- 안내
IP="$( (hostname -I 2>/dev/null || ipconfig getifaddr en0 2>/dev/null) | awk '{print $1}')"
say "끝났습니다"
echo "  접속 주소: http://${IP:-<서버 IP>}:$PORT   (같은 사내망에서, 접속 비밀번호 없음)"
if [ "$SERVICE" = no ]; then
  echo "  띄우기:   $MWA -C $PROJECT serve --addr 0.0.0.0:$PORT"
fi
echo
if [ "$LOGGED_IN" = no ]; then
  echo "  Claude 로그인이 아직입니다. 이 서버에서 한 번: claude auth login"
  echo "  (구독 계정 — 모든 사용자의 작업이 이 계정으로 돕니다. 로그인 전에는 매뉴얼 쓰기·장면 설명이 안 됩니다)"
fi
echo "  화면의 '시작하기'에서 코드 위치·제품 어댑터·촬영할 사이트를 설정합니다."
echo "  방화벽이 있으면 $PORT 포트를 사내망에 열어 주세요 (Ubuntu: sudo ufw allow $PORT/tcp)."
