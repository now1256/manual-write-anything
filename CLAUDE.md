# mwa — Manual Write Anything

웹 제품의 한국어 사용자 매뉴얼을 코드와 실제 화면에서 생성·갱신하는 도구(Rust 엔진 + React 화면 + Playwright 촬영기).
이 저장소는 **공개**입니다. 특정 회사·제품의 정보는 저장소에 두지 않습니다.

## 세션 시작할 때 읽는 순서

`docs/internal/`은 저장소에 올라가지 않는 비공개 작업 문서입니다(있는 환경에서만).

1. `docs/internal/STATUS.md` — 현재 마일스톤, 다음 작업, 막힌 것
2. 현재 마일스톤 파일 `docs/internal/milestones/Mn-*.md` — 작업 목록과 "읽을 것"
3. 마일스톤의 "읽을 것"에 적힌 명세만 읽습니다. 나머지 명세는 필요할 때만.

전체를 미리 읽지 않습니다. `docs/internal/reports/*.html`(사람용 스냅샷)과 `docs/internal/archive/`(폐기 초안)는 필요할 때만 grep.

## 문서 지도

| 파일 | 내용 | 공개 |
|---|---|---|
| `docs/architecture.md` | 원칙, 흐름, crate 구조, 작업 공간 형식, 역할 | ○ |
| `docs/adapter.md` | 제품 어댑터(`adapter.toml`) 형식 | ○ |
| `docs/cli-flags.md` | 설치 버전 `--help`로 확인한 CLI 플래그 | ○ |
| `docs/internal/decisions.md` | 확정 결정 D-xxx, 열린 질문 Q-x (충돌 시 가장 우선) | × |
| `docs/internal/specs/features/*.md` | 기능별 명세 F0–F13, FS, FB | × |
| `docs/internal/specs/product-facts.md` | 대상 제품의 코드 사실과 경로 (사용 전 원본 재확인) | × |
| `docs/internal/milestones/ROADMAP.md` | 마일스톤 통과 기준 | × |

명세와 다르게 구현해야 하면: 먼저 `decisions.md`에 새 D-xxx를 쓰고, 해당 명세를 고친 뒤 구현합니다.

## 절대 규칙

- **공개 파일에 회사·제품 고유 정보를 쓰지 않음.** 서버 주소, 계정, 도메인, 내부 함수·경로 이름, 보안 관련 사실은 `docs/internal/`과 프로젝트 폴더(`mwa.toml`, `adapter.toml`)에만. 테스트·예시는 가상 제품(acme)으로.
- **대상 제품 코드는 읽기 전용.**
- **공유 개발 서버에 쓰기 요청 금지.** 로그인·세션 연장 외 비-GET `/api/*` 금지. `*/test`, `*/run`, `*/sync`도 쓰기로 취급.
- **로그인 실패 재시도 금지.** 제품이 몇 번 실패로 계정을 잠글 수 있음. 실패하면 그 프로파일의 자동 로그인을 멈춤.
- **구독 OAuth 토큰을 직접 읽거나 쓰지 않음.** 구독 모델은 공식 CLI(`claude`, `codex`, `opencode`)를 하위 프로세스로만 실행.
- **비밀값(비밀번호, 토큰, API 키)은 저장소와 `mwa.toml`에 두지 않음.**
- CLI 플래그는 기억으로 쓰지 말고 `docs/cli-flags.md`를 따름.

## 코드 규칙

- 저장소 루트가 cargo workspace, crate는 `crates/`, 바이너리 `mwa`.
- 제품 전용 지식은 바이너리에 넣지 않고 프로젝트의 `adapter.toml`(형식은 `docs/adapter.md`)과 설정에만.
- 마일스톤에 없는 crate·기능을 미리 만들지 않음.
- 식별자·코드 주석은 영어, 문서와 사용자 출력은 한국어(합니다체).

## 검증

```
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cd web && npm run typecheck && npm run build   # 웹 화면 (mwa 바이너리에 내장되므로 cargo build 전에)
```
웹 화면 실제 확인: `mwa serve --addr 127.0.0.1:7713` 후 `cd web && MWA_URL=http://127.0.0.1:7713 npm run e2e -- <출력폴더>` (실제 Claude 호출 1회, 스크린샷 4장).
작업마다 마일스톤 파일에 적힌 "확인" 명령을 실제로 실행하고 결과를 보고합니다.

## 세션 끝낼 때

1. 마일스톤 파일 체크박스 갱신
2. `docs/internal/STATUS.md`의 다음 작업·막힌 것·최근 세션 한 줄 갱신
3. 새 결정이나 사실 정정은 `docs/internal/decisions.md` / `product-facts.md`에 기록
