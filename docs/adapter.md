# 제품 어댑터 (`adapter.toml`)

mwa는 특정 제품의 규칙을 바이너리에 넣지 않습니다. 제품마다 다른 것(메뉴 설정 파일 위치, 프록시 함수 이름, 권한 사전, 로그인 방식, 화면 관례)은 프로젝트 폴더의 `adapter.toml`에 적고, `mwa.toml`에서 가리킵니다.

```toml
# mwa.toml
[product]
name = "Acme Console"
adapter = "adapter.toml"   # 프로젝트 폴더 기준 경로
frontend = "frontend"      # [[source]] 이름
backend = "backend"
```

보통은 직접 쓰지 않습니다. 화면의 **시작하기 → 코드 읽고 제품 파악**(또는 `mwa adapter`)이 코드를 읽어 프로젝트 폴더에 `adapter.toml`을 만들고 제품 지도로 확인합니다. 이 문서는 그 결과를 고치거나 직접 쓸 때 봅니다.

지금 지원하는 구조는 **Next.js(app router) 프론트 + FastAPI 백엔드**입니다. 다른 구조는 파서를 추가해야 합니다.

## 예시

```toml
name = "acme"

# 메뉴: nav_file에서 export된 메뉴 배열을 읽습니다.
nav_file = "src/app/(app)/_components/sidebar/menu.ts"
app_dir = "src/app"
api_routes_dir = "src/app/api"
alias_prefix = "@/"                    # import "@/src/x" → <frontend>/src/x
stop_dirs = ["src/lib/contexts"]       # 화면별 코드 범위를 셀 때 들어가지 않는 공용 폴더
client_call_fns = ["apiFetch", "fetch", "EventSource"]   # 첫 인자가 "/api/..."인 호출

# 권한: 백엔드의 권한 이름 사전과 서버 검사 함수
permissions_file = "lib/permissions.py"
permission_labels_dict = "PERMISSION_NAMES"
non_licensable_set = "FREE_MENUS"
enforcement_fn = "needs_permission"
backend_code_dir = "lib"

[[nav_sections]]
export = "MAIN_NAV"

[[nav_sections]]
export = "ADMIN_NAV"
license_menu = "설정"      # 이 묶음의 메뉴가 속하는 라이선스 메뉴(그룹이 없을 때 이름표로도 씀)

# 프론트 API 라우트가 백엔드로 넘기는 방식 (해당하는 것만)
[[proxy_rules]]
kind = "sink"              # callUpstream(req, "/backend/path", init)
func = "callUpstream"
path_arg = 1
init_arg = 2

[[proxy_rules]]
kind = "factory"           # const fwd = forwarder("/prefix"); fwd(req, { path, method })
factory = "forwarder"
opts_arg = 1
path_prop = "path"

[[proxy_rules]]
kind = "prefixed_wrapper"  # const helper = (req, path, opts) => inner("/prefix", req, path, opts)
inner = "proxyQuery"
path_arg = 1
opts_arg = 2

[[proxy_rules]]
kind = "direct_fetch"      # fetch(`${BACKEND_ORIGIN}/backend/path`, init)
func = "fetch"
base_const = "BACKEND_ORIGIN"

# 로그인: 제품 화면이 쓰는 경로를 브라우저처럼 1회만 호출합니다 (실패해도 재시도하지 않음)
[auth]
login_path = "/api/auth/login"
me_path = "/api/auth/me"
logout_path = "/api/auth/logout"
session_cookie = "acme_session"
# 화면이 로그인할 때 보내는 JSON 본문의 키 이름. 반드시 로그인 호출 코드(body)에서 확인해 적습니다.
# 다르면 제품이 형식 오류(400)로 거부하고, mwa는 계정 보호를 위해 그 계정의 자동 로그인을 멈춥니다.
user_field = "userId"                         # 기본 "username"
password_field = "password"                   # 기본 "password"
keepalive_paths = ["/api/auth/session/touch"] # 공유 서버에서도 허용하는 비-GET (세션 연장만)
first_login_flag = "/isFirstLogin"            # JSON 포인터: true면 비밀번호 변경 필요
error_title = "/message/title"
error_info = "/message/info"
me_data = "/data"

# 촬영 관례
[ui]
domain_storage_key = "acme:tenant"   # [capture] domain을 넣을 sessionStorage 키
mask_buttons = ["사용자 메뉴"]         # 사진에서 가릴 버튼(접근성 이름)
toast_selector = "[data-sonner-toast]"
toast_type_attr = "data-type"
read_posts = ["/api/reports/preview"] # 읽기만 하는 POST(미리보기·검색). 사람이 코드를 확인하고 넣음(자동 생성은 비워 둠). 이것 말고 비-GET은 모두 차단
```

테스트용 가상 제품 전체는 `crates/productmap/tests/fixtures/mini/`에 있습니다.
