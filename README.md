# Manual Write Anything (mwa)

웹 제품의 **코드와 실제 화면**에서 한국어 사용자 매뉴얼을 만들고, 제품이 바뀌면 다시 맞춰 주는 도구입니다.

- 코드에서 메뉴·화면·API·권한을 결정적으로 뽑아 "제품 지도"를 만듭니다. LLM은 쓰지 않습니다.
- 기능별로 코드에서 사실을 뽑고, 사실마다 인용한 근거 줄을 검사한 뒤, 근거가 있는 문장만으로 매뉴얼을 씁니다.
- Playwright로 실제 화면을 찍습니다. 대화상자나 상세 화면은 접근성 트리를 보고 클릭 순서를 계획해 찍고, 한 번 성공한 순서는 LLM 없이 다시 찍습니다.
- 데이터가 없는 개발 서버에서는 프론트 타입에 맞춘 데모 데이터를 브라우저 안에서 응답해 화면을 채웁니다. 서버로는 쓰기 요청을 보내지 않습니다.
- LLM은 API 키가 아니라 **구독으로 로그인된 공식 CLI**(`claude`)를 하위 프로세스로 실행해서 씁니다.

> 상태: 초기 개발 중입니다. 지금은 Next.js(app router) + FastAPI 구조의 제품을 지원합니다.

## 구성

| 부분 | 위치 | 설명 |
|---|---|---|
| 엔진 | `crates/` (Rust) | 명령줄 `mwa`, 제품 지도, 매뉴얼 생성·검증, 촬영 제어, 화면용 서버(`mwa serve`) |
| 화면 | `web/` (React) | 대화, 기능 목록, 매뉴얼 미리보기, 스크린샷. `mwa serve`가 내장해서 제공 |
| 촬영기 | `sidecar/capture/` (Node + Playwright) | 엔진이 작업 JSON을 넘기면 화면을 찍어 결과를 돌려줌 |

데스크톱 앱(Tauri)으로 묶어 엔진을 앱 안에서 돌리거나, SSH로 원격 서버의 엔진에 붙는 방식을 준비하고 있습니다.

## 준비물

- Rust (stable, edition 2024)
- Node.js 20 이상
- [Claude Code](https://claude.com/claude-code) CLI 설치 후 `claude`로 로그인
- 촬영용 브라우저: `cd sidecar/capture && npm ci && npx playwright install chromium`

## 빌드

```sh
cd web && npm ci && npm run build && cd ..   # 화면을 먼저 빌드 (엔진에 내장됨)
cargo build --release                        # target/release/mwa
```

## 사용

```sh
mwa -C ~/manuals/acme init            # 프로젝트 폴더와 mwa.toml을 만듭니다
# mwa.toml: [[source]] 코드 위치, [product], [profiles], [capture]를 채웁니다
# adapter.toml: 제품 규칙 (docs/adapter.md)

mwa -C ~/manuals/acme doctor          # 설치·로그인·소스 점검
mwa -C ~/manuals/acme auth set manual # 촬영 계정 비밀번호 (사용자 PC의 자격 파일에만 저장)
mwa -C ~/manuals/acme map             # 제품 지도
mwa -C ~/manuals/acme shot menus      # 메뉴 첫 화면 촬영
mwa -C ~/manuals/acme gen <기능 키>    # 매뉴얼 생성·검증
mwa -C ~/manuals/acme world gen <기능 키>               # 데모 데이터 (선택)
mwa -C ~/manuals/acme shot detail --only <기능 키> --world  # "캡처 필요" 화면 촬영
mwa -C ~/manuals/acme serve           # 화면: http://127.0.0.1:7700
```

`mwa serve`는 `127.0.0.1`에만 열립니다. 다른 PC에서 쓰려면 SSH 터널을 씁니다.

```sh
ssh -L 7700:localhost:7700 <서버> mwa -C <프로젝트> serve
```

## 안전 원칙

- 대상 제품의 코드는 읽기만 합니다.
- 공유 서버에는 로그인·세션 연장 외의 쓰기 요청을 보내지 않습니다. 촬영 중 화면이 보내는 쓰기 요청은 브라우저에서 막거나 데모 응답으로 처리합니다.
- 로그인은 1회만 시도하고 실패하면 자동 로그인을 멈춥니다.
- 비밀번호·토큰은 저장소와 `mwa.toml`에 두지 않습니다. 구독 OAuth 토큰은 읽지 않고 공식 CLI만 실행합니다.

## 문서

- [docs/architecture.md](docs/architecture.md): 설계 개요
- [docs/adapter.md](docs/adapter.md): 제품 어댑터 형식
- [docs/cli-flags.md](docs/cli-flags.md): 사용하는 Claude CLI 플래그
