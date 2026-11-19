# Manual Write Anything (mwa)

웹 제품의 **코드와 실제 화면**으로 한국어 사용자 매뉴얼을 만들고, 제품이 바뀔 때마다 다시 맞추는 도구입니다.

## 하는 일

| 일 | 흐름 |
|---|---|
| **기존 매뉴얼 갱신** | 예전 매뉴얼(PDF·Markdown)을 가져와 → 같은 장면을 지금 화면으로 다시 찍고 → 코드로 내용을 다시 쓰고(목차·양식·말투는 기존 것을 따름) → 사람이 다듬어 저장 |
| **새 매뉴얼 쓰기** | 매뉴얼이 없는 새 기능은 코드를 읽고, 이미 만든 매뉴얼을 본보기로 같은 양식으로 씀 |
| **버전 관리** | 저장할 때마다 버전이 남고, 기존 매뉴얼·이전 버전과 나란히 비교·되돌리기. 문단마다 "왜 이렇게 썼나"(코드 근거)와 "어디서 왔나"(이력)가 남음 |

확장 기능(설정에서 켬): **컨플루언스 Cloud에 게시**. 그 밖에 Word(.docx) 내보내기가 있습니다.

LLM은 API 키가 아니라 **구독으로 로그인된 공식 CLI**(`claude`)를 하위 프로세스로 실행해서 씁니다. 사용량은 구독 한도 안에서 쓰며, 화면의 금액은 정가로 바꿔 본 값입니다.

> 지금은 Next.js(app router) + FastAPI 구조의 제품을 지원합니다. 제품마다 다른 규칙(메뉴 파일, 로그인 방식 등)은 `adapter.toml`로 줍니다.

## 다른 컴퓨터에 설치하기

### 준비물

| 무엇 | 왜 | 설치 |
|---|---|---|
| Rust (stable) | 빌드 | https://rustup.rs |
| Node.js 20 이상 | 화면 빌드, 화면 촬영 | https://nodejs.org |
| Claude Code CLI | 글쓰기·판단 (구독 로그인) | `npm i -g @anthropic-ai/claude-code` 후 `claude` 로 로그인 |
| poppler | 기존 매뉴얼 PDF 읽기 | macOS `brew install poppler`, Ubuntu `apt install poppler-utils` |

### 빌드와 설치

```sh
git clone https://github.com/now1256/manual-write-anything.git
cd manual-write-anything
(cd web && npm ci && npm run build)   # 화면 (엔진에 내장됨)
cargo install --path crates/cli       # ~/.cargo/bin/mwa
```

설치한 `mwa` 하나에 화면과 촬영 스크립트가 모두 들어 있습니다. 촬영 스크립트는 처음 쓸 때 데이터 폴더(macOS `~/Library/Application Support/mwa`, Linux `~/.local/share/mwa`)에 풀리고, 화면의 **"브라우저 설치"**(또는 `mwa doctor`가 알려 주는 단계)가 Playwright와 Chromium을 그 폴더에 받습니다.

### 처음 실행

```sh
mwa -C ~/manuals/acme serve      # 빈 폴더면 프로젝트를 만듭니다 → http://127.0.0.1:7700
```

화면의 **시작하기**가 남은 단계를 순서대로 알려 줍니다.

1. **설정** — 코드(폴더 또는 Git 주소), 제품(어댑터), 촬영할 사이트 주소·계정·비밀번호. 비밀번호와 토큰은 사용자 PC의 자격 파일(`~/.config/mwa/credentials.json`, 0600)에만 저장됩니다.
2. **제품 지도** — 코드에서 메뉴·화면·API·권한을 뽑습니다(LLM 없음).
3. **브라우저 설치·전체 점검** — Node, Playwright, Chromium, Claude 로그인, PDF 도구를 확인합니다.

다른 PC에서 서버의 엔진을 쓰려면 SSH 터널로 붙습니다(엔진은 `127.0.0.1`에만 열립니다).

```sh
ssh -t -L 7700:localhost:7700 <서버> mwa -C <프로젝트> serve
```

## 기능 하나의 매뉴얼 만들기 (화면)

첫 화면의 **매뉴얼 현황** 표에서 기능을 고르면 네 단계 탭이 나옵니다. 표의 "다음 할 일"이 지금 할 단계를 알려 줍니다.

| 단계 | 하는 일 |
|---|---|
| **① 기존 매뉴얼** | PDF·MD 파일을 올리거나 고릅니다. "장면 설명 만들기"로 그림마다 어떤 화면·상태인지 읽어 둡니다. 모델 구성(단계별 모델·특징)도 여기 있습니다 |
| **② 그림 고르기** | 기존 그림과 같은 장면을 지금 화면으로 찍고(같은 영역·같은 비율), 장면마다 쓸 그림을 고릅니다: 자동 / 지금 화면 / 기존 그림 / 올린 그림. 못 찍은 장면은 이유와 "어디서 안 됐나"가 남고, 그림을 직접 올릴 수 있습니다 |
| **③ 매뉴얼 만들기** | **코드 읽기**(코드에서 사실을 뽑음) → **매뉴얼 쓰기**(코드 사실로 다시 씀, 기존 매뉴얼은 양식의 본보기. 다 쓴 뒤 사용자 입장에서 한 번 교정). 기존 매뉴얼이 없는 기능은 그림이 들어갈 곳에 **그림 자리**(찍을 장면)를 남기고, ④에서 "그림 자리 찍기"로 지금 화면을 찍어 넣습니다. 제품이 바뀌면 다시 읽고 "제안 받기"로 바뀐 곳만 고칩니다 |
| **④ 편집·버전** | 컨플루언스식 편집기(`/` 메뉴, 표·패널·상태 라벨, 그림 크기). "확인 필요" 메모와 아직 못 찍은 그림 자리는 편집기에만 보이고 Word·컨플루언스에는 나가지 않습니다. 오른쪽 **도우미**에 말로 요청하면 문서 전체를 보고 고칠 곳을 찾아 미리 보여 줍니다. 문단 옆 **?**에서 근거, **기록**에서 버전별로 바꾼 이유, 버전 비교·되돌리기 |

## 명령줄

화면에서 하는 일은 모두 명령으로도 됩니다(`mwa --help`).

```sh
mwa -C <프로젝트> doctor                     # 설치·로그인·소스 점검
mwa -C <프로젝트> map                        # 제품 지도
mwa -C <프로젝트> ref import <파일·폴더>       # 기존 매뉴얼 가져오기 (PDF·MD)
mwa -C <프로젝트> ref describe <문서 id>       # 그림마다 장면 설명
mwa -C <프로젝트> shot ref --only <기능 키>    # 기존 매뉴얼 장면 다시 찍기 (--scene <그림>: 하나만)
mwa -C <프로젝트> gen <기능 키>               # 코드 읽기 (사실 추출)
mwa -C <프로젝트> doc write <기능 키>         # 매뉴얼 쓰기 (③과 같음)
mwa -C <프로젝트> shot doc --only <기능 키>    # 쓴 매뉴얼의 그림 자리 찍기 (--scene <블록 id,…>: 일부만)
mwa -C <프로젝트> doc update <기능 키>        # 코드 기준 갱신 제안
mwa -C <프로젝트> serve                      # 화면
```

## 프로젝트 폴더

| 위치 | 내용 |
|---|---|
| `mwa.toml` | 설정 (비밀값 없음). `[extensions] confluence = true`로 확장 켜기 |
| `adapter.toml` | 제품 규칙 (형식: `docs/adapter.md`) |
| `map/` | 제품 지도 |
| `reference/<문서>/` | 가져온 기존 매뉴얼 (글, 그림, 쪽 이미지, 장면 설명) |
| `shots/` | 찍은 화면 (`reference/<기능>/`: 기존 매뉴얼 장면) |
| `manual/<기능>.json` | 코드 읽기 결과 (코드 사실, 화면의 "코드 사실" 탭) |
| `manual/docs/<기능>/` | 매뉴얼 문서: `doc.json`(현재), `v<n>.json`(버전), `provenance.json`(이력), `grounding.json`(근거), `assets/`(올린 그림) |

## 안전 원칙

- 대상 제품의 코드는 읽기만 합니다.
- 공유 서버에는 로그인·세션 연장 외의 쓰기 요청을 보내지 않습니다. 촬영 중 화면이 보내는 쓰기 요청은 브라우저에서 막습니다.
- 로그인은 1회만 시도하고, 실패하면 그 계정의 자동 로그인을 멈춥니다.
- 비밀번호·토큰은 저장소와 `mwa.toml`에 두지 않습니다. 구독 OAuth 토큰은 읽지 않고 공식 CLI만 실행합니다.
- 매뉴얼에는 사용자가 보고 하는 일만 씁니다. 서버의 권한 검사 같은 보안 내부 사항은 쓰지 않습니다.

## 문서

- [docs/architecture.md](docs/architecture.md): 설계 개요
- [docs/adapter.md](docs/adapter.md): 제품 어댑터 형식
- [docs/cli-flags.md](docs/cli-flags.md): 사용하는 Claude CLI 플래그
- 데스크톱 앱(macOS, 선택): `./app/build-mac.sh` → `app/src-tauri/target/release/bundle/dmg/`
