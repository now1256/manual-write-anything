# 아키텍처

> 설계 개요입니다. 일부 구성 요소(explore, export 등)는 아직 계획 단계입니다.

## 원칙

- **조율은 Rust 상태 머신이 합니다.** LLM은 정해진 역할의 한 번짜리 판단에만 들어갑니다.
- **사실은 코드에서 결정적으로 뽑습니다(제품 지도).** LLM에는 기능 조각만 보냅니다. 동작 서술은 실행으로 확인된 것만 단정합니다.
- **싼 모델이 일하고, 다른 계열 상위 모델이 판정만 합니다.**
- **제품 맞춤 지식은 프로젝트의 어댑터 파일(`adapter.toml`)과 설정에만 둡니다.** 바이너리에는 특정 제품의 규칙이 들어가지 않습니다. 형식은 [adapter.md](adapter.md).

## 전체 흐름

```
① 소스 연결(FS) → ② 제품 지도(F1) → ③ 양식 대화(F4) → ④ 생성: 사실(F5)·촬영(F6, F3 상태 준비)·작성(F7)
→ ⑤ 검증(F8) → ⑥ 대화 편집(F9) → ⑦ 승인=git 커밋, 내보내기(F10)
⑧ 자동 갱신(F11): 새 커밋 → 지도 diff → 영향 블록만 ④~⑦
전제: 실행 환경 등급(F0), 접속 프로파일(F2), 모델 백엔드(FB), 안전·비용(F13). 부가: QA(F12)
```

## 저장소 구조

```
manual-write-anything/
├─ CLAUDE.md
├─ Cargo.toml                 cargo workspace (M0에서 생성)
├─ crates/
│  ├─ protocol   이벤트, 작업, 설정 타입 (의존성 최소)
│  ├─ core       작업 그래프 상태 머신, 예산, 재시도, 실행 기록(SQLite)
│  ├─ backends   Backend 트레잇 + cli:claude-code / cli:codex / cli:opencode / api:anthropic / api:openai-compat / ollama  (FB)
│  ├─ source     소스 연결: 로컬 경로 / 원격 미러 / 업로드 → SHA 고정 체크아웃 (FS)
│  ├─ productmap 메뉴 설정·권한 사전·라우트 파서, 응답 형태 추출, 어댑터 로더 (F1)
│  ├─ facts      사실 추출, 해시 캐시 (F5)
│  ├─ manual     블록 모델, 양식 스키마, 편집 연산, 결정적 표 (F4, F7, F9)
│  ├─ capture    Playwright 사이드카 제어, 세션, 촬영 명세, 가림, 콜아웃 (F2, F3, F6)
│  ├─ explore    agent-browser 탐색 → Playwright 로케이터 변환 (F6)
│  ├─ verify     규칙 검사, 검증자 루브릭 (F8)
│  ├─ export     PDF, Confluence, 코퍼스 (F10)
│  ├─ tui        대화·검토 화면 (ratatui)
│  └─ cli        clap 진입점: init / doctor / map / gen / review / export / watch / exec
├─ sidecar/playwright/        Node 사이드카 (connectOverCDP, getByRole 재생·촬영)
└─ docs/
```

## 브라우저 계층

- 탐색: Vercel agent-browser(접근성 트리 `@ref`, `connect`로 기존 Chrome CDP 연결). 스크린샷을 모델에 보내지 않음.
- 기록: 고른 `@ref` → 역할·이름·범위로 Playwright 시맨틱 로케이터 변환. 상자 일치 + 후보 1개일 때만 저장.
- 재생·본 촬영: Playwright 하나(Node 사이드카). chromiumoxide는 쓰지 않음.
- 공유 스택: 로그인·세션 연장·로그아웃 외 비-GET `/api/*`는 `page.route`로 차단.

## 매뉴얼 작업 공간 (생성물, 별도 git 저장소)

```
manual-workspace/
├─ mwa.toml                 대상, 소스, 계정 참조, 역할별 백엔드, 예산
├─ template/schema.yaml      양식 (절 순서, 블록 종류, 문체)
├─ map/product-map.json      제품 지도
├─ facts/<feature>.jsonl     사실 (출처: 파일:줄 / 화면 / 실행)
├─ manual/<feature>.md       본문 + 블록 주석 앵커
├─ manual/blocks.json        블록 사이드카 (id, 해시, owner gen|human|locked, 근거)
├─ shots/<feature>/*.yaml|png 촬영 명세, 승인 이미지
├─ baseline/                 승인 시점 접근성 스냅샷, 픽셀 해시
└─ .mwa/                    캐시, 실행 기록, 소스 미러 (git 제외)
```

블록 주석 예: `<!-- blk:user-create-03 owner=gen facts=f81,f82 h=3fa2 -->`

## 역할과 모델 (기본값, mwa.toml로 변경)

| 역할 | 기본 | 병렬 |
|---|---|---|
| 대화·양식 추출·편집 | writer (중간) | 단일 |
| 사실 추출·화면 탐색·캡션 확인 | worker (싼) | 파일/계정별 |
| 절 작성 | writer | 기능별 |
| 검증 | verifier (상위, writer와 다른 계열) | 기능 묶음 |
| 규칙 검사 | 코드 | 전부 |

검증은 결정적 규칙 검사(근거 사실 id, 화면 라벨) 뒤에 상위 모델이 블록별로 판정합니다.
