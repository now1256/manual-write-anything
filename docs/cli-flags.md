# 에이전트 CLI 플래그 (실측 기록)

> 설치된 버전의 `--help`와 실제 실행으로 확인한 것만 적습니다. 버전이 바뀌면 다시 확인합니다.

## Claude Code 2.1.326 (`~/.local/bin/claude`) — 2026-11-17 확인

### 사용하는 호출 형태

```
claude -p \
  --input-format stream-json --output-format stream-json --verbose \
  --model <alias|full-id> \
  --tools "" \
  --no-session-persistence --strict-mcp-config --setting-sources "" \
  --disable-slash-commands --permission-prompts none \
  --system-prompt <text> \
  --json-schema <schema-json>
# 작업 폴더(cwd) = 빈 임시 폴더, stdin = 사용자 메시지 JSONL
```

| 플래그 | 의미 | 확인 |
|---|---|---|
| `-p` | 비대화 실행 | 실행 |
| `--input-format stream-json` | stdin으로 `{"type":"user","message":{"role":"user","content":[...]}}` 한 줄. 이미지 블록(base64) 가능 | 40×20 빨간 PNG → `{"color":"red"}` |
| `--output-format stream-json` | stream-json 입력에는 **필수**(아니면 `Error: --input-format=stream-json requires output-format=stream-json.`). `--verbose` 필요 | 실행 |
| `--output-format json` | 텍스트 프롬프트용 단일 결과. 이번 설계에서는 stream-json으로 통일 | 실행 |
| `--json-schema` | 결과의 `structured_output` 필드에 스키마 검증된 객체. `result`에는 문자열 사본 | `{"answer":2}` |
| `--tools ""` | 내장 도구 없음. init 이벤트의 tools가 `["StructuredOutput"]`만 남음 | init 이벤트 |
| `--model` | 별칭(`haiku`, `sonnet`, `opus`, `fable`) 또는 전체 id | `haiku` → `claude-haiku-5-5` |
| `--system-prompt` | 기본 시스템 프롬프트 대체 | 실행 |
| `--setting-sources ""` | user/project/local 설정 무시 | 실행 |
| `--include-partial-messages` | stream-json 출력에 `stream_event`(`content_block_delta`/`text_delta`) 줄을 추가합니다. 대화 화면 스트리밍에 씁니다 | "한 문장으로 인사해줘" → 조각 5개 수신 |
| `--max-budget-usd` | 호출당 비용 상한(-p 전용). 구독에서는 의미 약함, 미사용 | help만 |
| `--bare` | **사용 불가**: OAuth를 읽지 않아 구독 로그인이 안 됨(API 키 전용) | help |

### 출력 이벤트 (stream-json, 한 줄에 하나)

- `system/init`: `model`, `tools`, `apiKeySource`(`"none"` = 구독 로그인), `claude_code_version`
- `rate_limit_event`: `rate_limit_info.status`(`allowed` 등), `rateLimitType`(예 `seven_day`), `resetsAt`(epoch 초), `unifiedWindows.<창>.utilization`(0~1)
- `result`: `is_error`, `subtype`, `structured_output`, `result`, `total_cost_usd`, `usage`, `modelUsage.<모델>`, `terminal_reason`, `api_error_status`, `duration_ms`, `num_turns`

### 주의

- **오류 판정은 `is_error`로 합니다.** 없는 모델일 때 `subtype`은 `"success"`인데 `is_error: true`, `terminal_reason: "api_error"`, `api_error_status: 404`, 종료 코드 1이었습니다.
- 텍스트 프롬프트를 인자로 줄 때 stdin을 닫지 않으면 3초 대기 경고가 나옵니다. stream-json 방식은 stdin에 메시지를 쓰고 닫습니다.
- 스키마를 쓰면 `num_turns`가 2(StructuredOutput 도구 호출 포함)입니다.
- 빈 작업 폴더에서 `--setting-sources ""`로 실행했을 때 사용자 CLAUDE.md 내용이 보이지 않음을 모델 응답으로 확인했습니다(약한 확인).
- `total_cost_usd`는 구독에서도 정가 기준 추정값이 나옵니다. 실제 청구가 아니라 한도 소모 지표로만 씁니다.

## Codex CLI, OpenCode

- 2026-11-17 현재 이 서버에 미설치. 설치 후 같은 방식으로 확인해 이 파일에 추가합니다. 그 전까지 어댑터는 가짜 실행 파일로만 테스트합니다.

## 로그인 (install.sh)

`claude auth --help`로 확인 (2026-11-19).

| 명령 | 쓰임 |
|---|---|
| `claude auth status --json` | 로그인 여부 확인. `loggedIn`(true/false)만 봅니다 |
| `claude auth login --claudeai` | 구독 계정으로 로그인 (기본값). `--console`은 API 과금이라 쓰지 않습니다 |

