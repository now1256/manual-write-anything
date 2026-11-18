use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mwa_core::{Dispatcher, RunLog};
use mwa_protocol::{Config, Request, Role};
use mwa_source::{Snapshot, snapshot_local};
use serde_json::{Value, json};

mod projects;
mod settings;
mod tui_host;

const CONFIG_FILE: &str = "mwa.toml";
const STATE_DIR: &str = ".mwa";
const TEMPLATE: &str = include_str!("../templates/mwa.toml");
const DEFAULT_SYSTEM: &str =
    "You are a precise assistant. Follow the requested output format exactly.";
/// CLI agents mwa knows how to drive or plans to; shown by `doctor` even when unconfigured.
const KNOWN_CLIS: [(&str, &str); 3] = [
    ("claude", "cli:claude-code"),
    ("codex", "cli:codex (어댑터 미구현)"),
    ("opencode", "cli:opencode (어댑터 미구현)"),
];

#[derive(Parser)]
#[command(
    name = "mwa",
    version,
    about = "Manual Write Anything — 코드와 실제 화면에서 사용자 매뉴얼을 만듭니다"
)]
struct Cli {
    /// 매뉴얼 작업 공간 폴더 (mwa.toml 위치)
    #[arg(short = 'C', long, global = true, default_value = ".")]
    workspace: PathBuf,
    /// 생략하면 대화형 화면(TUI)을 엽니다
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// 대화형 화면(TUI)을 엽니다. `mwa`만 입력해도 같습니다
    Ui {
        /// 화면을 그리지 않고 첫 화면을 텍스트로 출력합니다 (예: 120x36)
        #[arg(long, value_name = "WxH")]
        snapshot: Option<String>,
        /// --snapshot 에서 첫 기능의 상세를 펼칩니다
        #[arg(long, requires = "snapshot")]
        feature: bool,
    },
    /// 데스크톱 화면용 서버를 127.0.0.1에서 엽니다. 맥에서는 SSH 터널로 접속합니다
    Serve {
        #[arg(long, default_value = "127.0.0.1:7700")]
        addr: std::net::SocketAddr,
        /// 개발용: 내장 화면 대신 이 폴더(web/dist)의 화면을 매번 읽습니다. 화면만 바뀌면 새로고침으로 반영됩니다
        #[arg(long, value_name = "DIR")]
        web_dir: Option<PathBuf>,
    },
    /// 작업 공간을 만들고 소스의 현재 커밋을 기록합니다
    Init,
    /// 설치·로그인·소스 상태를 점검합니다
    Doctor {
        /// 백엔드 최소 호출을 건너뜁니다
        #[arg(long)]
        no_call: bool,
    },
    /// 코드에서 제품 지도(map/product-map.json)를 만듭니다. LLM을 쓰지 않습니다
    Map {
        /// OpenAPI URL 또는 파일 (설정의 product.openapi 대신)
        #[arg(long)]
        openapi: Option<String>,
        /// OpenAPI 없이 만듭니다 (백엔드 오퍼레이션 매칭 생략)
        #[arg(long, conflicts_with = "openapi")]
        no_openapi: bool,
    },
    /// 제품 화면을 촬영합니다 (공유 서버에는 쓰기 요청을 보내지 않습니다)
    Shot {
        /// menus: 메뉴 첫 화면 전체 / detail: 매뉴얼의 "캡처 필요" 화면(대화상자·상세, --only로 기능 지정)
        #[arg(value_parser = ["menus", "detail"])]
        what: String,
        /// 같은 화면을 몇 번 찍어 흔들림을 비교할지
        #[arg(long, default_value_t = 2)]
        runs: u32,
        /// 일부 기능만 (기능 키, 쉼표로 구분)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// 기능 API 응답을 데모 데이터(world/world.json)로 채웁니다. 먼저 mwa world gen
        #[arg(long)]
        world: bool,
        /// detail: 저장된 촬영 계획을 버리고 새로 계획합니다
        #[arg(long)]
        replan: bool,
    },
    /// 기능 하나의 매뉴얼을 만듭니다 (사실 추출 → 작성 → 검증 → 기존 매뉴얼 비교)
    Gen {
        /// 기능 키 (예: user-account). 지도에서 확인
        feature: String,
        /// 양식을 배울 견본 매뉴얼(Markdown). 없으면 저장된 양식(template/schema.json)을 씁니다
        #[arg(long)]
        template: Option<PathBuf>,
        /// 비교할 기존 매뉴얼(Markdown)
        #[arg(long)]
        compare: Option<PathBuf>,
        /// LLM 없이 코드 조각과 화면 라벨만 보여줍니다
        #[arg(long)]
        slice_only: bool,
    },
    /// 제품 코드 소스(로컬 폴더·git)를 관리합니다
    Sources {
        #[command(subcommand)]
        action: SourcesCmd,
    },
    /// 상세 촬영용 가상 세계 데이터
    World {
        #[command(subcommand)]
        action: WorldCmd,
    },
    /// 제품 화면 로그인 계정(프로파일)의 비밀번호를 관리합니다
    Auth {
        #[command(subcommand)]
        action: AuthCmd,
    },
    /// 역할에 배정된 모델을 한 번 호출하고 결과를 JSON으로 출력합니다
    Exec {
        #[arg(long)]
        role: Role,
        /// 출력 JSON 스키마 (JSON 문자열 또는 @파일)
        #[arg(long)]
        schema: Option<String>,
        /// 함께 보낼 이미지 (여러 번 지정 가능)
        #[arg(long = "image")]
        images: Vec<PathBuf>,
        #[arg(long, default_value = DEFAULT_SYSTEM)]
        system: String,
        prompt: String,
    },
}

#[derive(Subcommand)]
enum SourcesCmd {
    /// git으로 지정한 소스를 클론하거나 최신으로 맞춥니다 (mwa가 관리하는 사본이라 로컬 변경은 버립니다)
    Sync,
}

#[derive(Subcommand)]
enum WorldCmd {
    /// 기능이 부르는 API의 응답 형태를 프론트 타입에서 뽑습니다 (world/shapes/<기능>.json). LLM을 쓰지 않습니다
    Shapes {
        /// 기능 키 (쉼표로 구분)
        #[arg(required = true, value_delimiter = ',')]
        features: Vec<String>,
    },
    /// 데모 데이터(world/world.json)를 만듭니다. 상류 기능부터 순서대로 적으세요 (예: data-studio,data-dataset-forge)
    Gen {
        /// 기능 키 (쉼표로 구분, 적은 순서대로 만듭니다)
        #[arg(required = true, value_delimiter = ',')]
        features: Vec<String>,
        /// 응답 형태가 그대로여도 다시 만듭니다
        #[arg(long)]
        force: bool,
    },
    /// 저장된 데모 데이터를 지금 코드의 응답 형태로 다시 검사합니다 (LLM 없음)
    Check {
        /// 기능 키 (쉼표로 구분, 없으면 데모 데이터에 있는 기능 전체)
        #[arg(value_delimiter = ',')]
        features: Vec<String>,
    },
}

#[derive(Subcommand)]
enum AuthCmd {
    /// 프로파일 비밀번호를 저장합니다 (입력은 화면에 보이지 않습니다)
    Set {
        profile: String,
        /// 비밀번호를 표준입력 한 줄에서 읽습니다 (터미널이 없을 때)
        #[arg(long)]
        stdin: bool,
    },
    /// 프로파일과 저장 상태를 보여줍니다 (비밀번호는 출력하지 않습니다)
    List,
    /// 로그인을 딱 1번 시도해 확인하고 바로 로그아웃합니다. 실패하면 자동 로그인을 막습니다
    Check { profile: String },
    /// 저장된 비밀번호를 지웁니다
    Forget { profile: String },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        None => ui(&cli.workspace, None, false).await,
        Some(cmd) => run_cmd(&cli.workspace, cmd).await,
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("오류: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run_cmd(ws: &Path, cmd: Cmd) -> Result<ExitCode> {
    let cli_workspace = ws;
    match cmd {
        Cmd::Ui { snapshot, feature } => ui(cli_workspace, snapshot, feature).await,
        Cmd::Init => init(cli_workspace),
        Cmd::Serve { addr, web_dir } => serve(cli_workspace, addr, web_dir).await,
        Cmd::Doctor { no_call } => doctor(cli_workspace, no_call).await,
        Cmd::Map {
            openapi,
            no_openapi,
        } => map(cli_workspace, openapi, no_openapi).await,
        Cmd::Auth { action } => auth(cli_workspace, action).await,
        Cmd::Sources {
            action: SourcesCmd::Sync,
        } => {
            let cfg = load_config(cli_workspace)?;
            let lines = sync_sources(&cfg, |l| println!("{l}"))?;
            let failed = lines.iter().any(|l| l.contains("실패"));
            for l in lines {
                println!("{l}");
            }
            Ok(if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Cmd::World {
            action: WorldCmd::Shapes { features },
        } => world_shapes(cli_workspace, &features),
        Cmd::World {
            action: WorldCmd::Gen { features, force },
        } => world_gen(cli_workspace, &features, force).await,
        Cmd::World {
            action: WorldCmd::Check { features },
        } => world_check(cli_workspace, &features),
        Cmd::Gen {
            feature,
            template,
            compare,
            slice_only,
        } => {
            let cfg = load_config(cli_workspace)?;
            let read = |p: Option<PathBuf>| -> Result<Option<(String, String)>> {
                p.map(|p| {
                    let text = std::fs::read_to_string(&p)
                        .with_context(|| format!("{} 을 읽을 수 없습니다", p.display()))?;
                    Ok((
                        p.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        text,
                    ))
                })
                .transpose()
            };
            // Without --compare, use the existing manual configured in [manual.compare].
            let compare = compare.or_else(|| cfg.manual.compare.get(&feature).map(PathBuf::from));
            let (template, compare) = (read(template)?, read(compare)?);
            if slice_only {
                return gen_slice_only(cli_workspace, &cfg, &feature);
            }
            let run = generate_manual(cli_workspace, &cfg, &feature, template, compare, |l| {
                println!("{l}")
            })
            .await?;
            for l in mwa_manual::pipeline::summary(&run) {
                println!("{l}");
            }
            println!(
                "저장: {}",
                cli_workspace.join(format!("manual/{feature}.md")).display()
            );
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Shot {
            what,
            runs,
            only,
            world,
            replan,
        } => {
            let cfg = load_config(cli_workspace)?;
            if what == "detail" {
                anyhow::ensure!(!only.is_empty(), "detail은 --only <기능 키>가 필요합니다");
                let ok = capture_detail(cli_workspace, &cfg, &only, world, replan, runs, |l| {
                    println!("{l}")
                })
                .await?;
                return Ok(if ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                });
            }
            let m = capture_menus(cli_workspace, &cfg, runs, &only, world, |line| {
                println!("{line}")
            })
            .await?;
            for l in m.summary() {
                println!("{l}");
            }
            println!(
                "저장: {}",
                cli_workspace.join(mwa_capture::menus::MANIFEST).display()
            );
            Ok(if m.shots.iter().all(|s| s.ok) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Cmd::Exec {
            role,
            schema,
            images,
            system,
            prompt,
        } => exec(cli_workspace, role, schema, images, &system, &prompt).await,
    }
}

async fn serve(
    ws: &Path,
    addr: std::net::SocketAddr,
    web_dir: Option<PathBuf>,
) -> Result<ExitCode> {
    let host = std::sync::Arc::new(tui_host::CliHost::open(ws)?);
    let registry = projects::Registry::new(&projects::data_dir());
    let ws_abs = std::fs::canonicalize(ws).unwrap_or_else(|_| ws.to_path_buf());
    if let Err(e) = registry.touch(&ws_abs, None) {
        eprintln!("경고: 프로젝트 목록에 등록하지 못했습니다: {e}");
    }
    let hub = mwa_server::Hub::new(host, Some(std::sync::Arc::new(registry)));
    let listener = mwa_server::bind(addr).await?;
    let port = listener.local_addr()?.port();
    println!("mwa serve: http://{addr}");
    println!(
        "맥에서 접속: ssh -L {port}:localhost:{port} <사용자>@<서버> 후 브라우저로 http://localhost:{port}"
    );
    if let Some(d) = &web_dir {
        println!("화면 폴더(개발 모드): {}", d.display());
    }
    mwa_server::serve_listener(listener, hub, web_dir).await?;
    Ok(ExitCode::SUCCESS)
}

async fn ui(ws: &Path, snapshot: Option<String>, feature: bool) -> Result<ExitCode> {
    let host = tui_host::CliHost::open(ws)?;
    match snapshot {
        Some(size) => {
            let (w, h) = size
                .split_once('x')
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                .context("--snapshot 형식은 120x36 처럼 씁니다")?;
            print!("{}", mwa_tui::snapshot(&host, w, h, feature));
        }
        None => mwa_tui::run(std::sync::Arc::new(host)).await?,
    }
    Ok(ExitCode::SUCCESS)
}

fn load_config(ws: &Path) -> Result<Config> {
    let (cfg, warnings) = load_config_quiet(ws)?;
    for w in warnings {
        eprintln!("경고: {w}");
    }
    Ok(cfg)
}

/// Config plus warnings, without printing (the TUI must not write to the terminal).
fn load_config_quiet(ws: &Path) -> Result<(Config, Vec<String>)> {
    let path = ws.join(CONFIG_FILE);
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "{} 을 읽을 수 없습니다. 먼저 `mwa init`을 실행하세요",
            path.display()
        )
    })?;
    let (mut cfg, warnings) =
        Config::parse(&text).with_context(|| format!("{} 설정 오류", path.display()))?;
    let ws_abs = std::fs::canonicalize(ws).unwrap_or_else(|_| ws.to_path_buf());
    cfg.resolve_paths(&ws_abs);
    Ok((cfg, warnings))
}

fn open_dispatcher(ws: &Path, cfg: Config) -> Result<Dispatcher> {
    let log = RunLog::open(&ws.join(STATE_DIR))?;
    let d = Dispatcher::new(cfg, log);
    d.prune_log()?;
    Ok(d)
}

/// Clones or updates every `[[source]]` that has `git`; one result line per source.
fn sync_sources(cfg: &Config, progress: impl Fn(String)) -> Result<Vec<String>> {
    let git: Vec<_> = cfg.sources.iter().filter(|s| s.git.is_some()).collect();
    anyhow::ensure!(
        !git.is_empty(),
        "git으로 지정한 소스가 없습니다. mwa.toml의 [[source]]에 git = \"주소\"를 적으세요"
    );
    let mut out = Vec::new();
    for s in git {
        let url = s.git.as_deref().expect("filtered");
        progress(format!("{}: {url} 동기화 중…", s.name));
        out.push(
            match mwa_source::sync_git(url, s.branch.as_deref(), &s.path) {
                Ok(msg) => format!("{}: {msg}", s.name),
                Err(e) => format!("{}: 실패 — {e}", s.name),
            },
        );
    }
    Ok(out)
}

fn snapshots(cfg: &Config) -> Vec<(String, Result<Snapshot, String>)> {
    cfg.sources
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                snapshot_local(&s.name, &s.path).map_err(|e| e.to_string()),
            )
        })
        .collect()
}

fn print_sources(snaps: &[(String, Result<Snapshot, String>)]) -> bool {
    let (lines, ok) = source_lines(snaps);
    for l in lines {
        println!("{l}");
    }
    ok
}

fn source_lines(snaps: &[(String, Result<Snapshot, String>)]) -> (Vec<String>, bool) {
    let mut all_ok = true;
    let mut out = vec!["소스".to_string()];
    for (name, snap) in snaps {
        match snap {
            Ok(s) => {
                let state = if s.is_preview() {
                    format!("미리보기 (커밋 안 한 변경 {}개)", s.dirty_entries)
                } else {
                    "깨끗함".to_string()
                };
                out.push(format!(
                    "  {name:<14} OK    {}  {}  {state}",
                    &s.sha[..12.min(s.sha.len())],
                    s.branch.as_deref().unwrap_or("(detached)"),
                ));
            }
            Err(e) => {
                all_ok = false;
                out.push(format!("  {name:<14} 실패  {e}"));
            }
        }
    }
    (out, all_ok)
}

fn init(ws: &Path) -> Result<ExitCode> {
    std::fs::create_dir_all(ws)?;
    let cfg_path = ws.join(CONFIG_FILE);
    if cfg_path.exists() {
        println!("{} 이 이미 있어 그대로 씁니다", cfg_path.display());
    } else {
        std::fs::write(&cfg_path, TEMPLATE)?;
        println!("{} 을 만들었습니다", cfg_path.display());
    }
    let gitignore = ws.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(&gitignore, format!("{STATE_DIR}/\n"))?;
    }
    let cfg = load_config(ws)?;
    let snaps = snapshots(&cfg);
    let ok = print_sources(&snaps);

    let state = ws.join(STATE_DIR);
    std::fs::create_dir_all(&state)?;
    let recorded: Vec<Value> = snaps
        .iter()
        .map(|(name, s)| match s {
            Ok(s) => serde_json::to_value(s).expect("snapshot serializes"),
            Err(e) => json!({"name": name, "error": e}),
        })
        .collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let doc = json!({"recorded_at": now, "sources": recorded});
    std::fs::write(
        state.join("sources.json"),
        serde_json::to_string_pretty(&doc)?,
    )?;

    if snaps
        .iter()
        .any(|(_, s)| s.as_ref().is_ok_and(|s| s.is_preview()))
    {
        eprintln!("경고: 커밋 안 한 변경이 있는 소스는 미리보기로만 쓸 수 있고 승인할 수 없습니다");
    }
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

async fn exec(
    ws: &Path,
    role: Role,
    schema: Option<String>,
    images: Vec<PathBuf>,
    system: &str,
    prompt: &str,
) -> Result<ExitCode> {
    let schema: Option<Value> = match schema {
        None => None,
        Some(s) => {
            let text = match s.strip_prefix('@') {
                Some(path) => std::fs::read_to_string(path)
                    .with_context(|| format!("스키마 파일 {path} 을 읽을 수 없습니다"))?,
                None => s,
            };
            Some(serde_json::from_str(&text).context("스키마가 올바른 JSON이 아닙니다")?)
        }
    };
    let d = open_dispatcher(ws, load_config(ws)?)?;
    let out = d
        .call(role, system, prompt, &images, schema.as_ref())
        .await?;
    let doc = json!({
        "run_id": d.run_id(),
        "backend": out.backend,
        "model": out.model,
        "cost_usd": out.cost_usd,
        "json": out.response.json,
        "text": out.response.text,
    });
    println!("{}", serde_json::to_string_pretty(&doc)?);
    Ok(ExitCode::SUCCESS)
}

async fn doctor(ws: &Path, no_call: bool) -> Result<ExitCode> {
    let cfg = load_config(ws)?;
    let (lines, ok) = doctor_report(ws, cfg, no_call).await?;
    for l in lines {
        println!("{l}");
    }
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

async fn doctor_report(ws: &Path, cfg: Config, no_call: bool) -> Result<(Vec<String>, bool)> {
    let (mut out, mut ok) = source_lines(&snapshots(&cfg));

    // The cheapest configured model per backend, taken from the first role that uses it.
    let model_for = |backend: &str| {
        Role::ALL.into_iter().find_map(|r| {
            let rc = cfg.role(r)?;
            if rc.backend.get_ref() == backend {
                return Some(rc.model.clone());
            }
            let fb = rc.fallback.as_ref()?;
            (fb.backend.get_ref() == backend).then(|| fb.model.clone())
        })
    };
    let names: Vec<(String, String)> = cfg
        .backends
        .iter()
        .map(|(n, b)| (n.clone(), b.kind().to_string()))
        .collect();
    let models: Vec<Option<String>> = names.iter().map(|(n, _)| model_for(n)).collect();
    let d = open_dispatcher(ws, cfg)?;

    out.push("백엔드".into());
    for ((name, kind), model) in names.iter().zip(models) {
        let backend = d.backend(name).expect("built from config");
        let probe = backend.probe().await;
        let version = probe.version.clone().unwrap_or_else(|| "-".into());
        if !probe.available {
            ok = false;
            out.push(format!("  {name:<10} {kind:<18} 실패  {}", probe.detail));
            continue;
        }
        if no_call {
            out.push(format!("  {name:<10} {kind:<18} 설치됨  {version}"));
            continue;
        }
        let Some(model) = model else {
            out.push(format!(
                "  {name:<10} {kind:<18} 설치됨  {version}  (이 백엔드를 쓰는 역할 없음, 호출 생략)"
            ));
            continue;
        };
        let req = Request {
            system: DEFAULT_SYSTEM.into(),
            prompt: "Reply with ok=true.".into(),
            images: vec![],
            schema: Some(json!({
                "type": "object",
                "properties": {"ok": {"type": "boolean"}},
                "required": ["ok"]
            })),
            model: model.clone(),
        };
        match d.call_backend("doctor", name, &req).await {
            Ok(res) if res.response.json == Some(json!({"ok": true})) => out.push(format!(
                "  {name:<10} {kind:<18} OK    {version}  {}  호출 성공{}",
                res.model,
                res.cost_usd
                    .map(|c| format!(" (${c:.4})"))
                    .unwrap_or_default()
            )),
            Ok(res) => {
                ok = false;
                out.push(format!(
                    "  {name:<10} {kind:<18} 실패  예상 밖 응답: {}",
                    res.response.text
                ));
            }
            Err(e) => {
                ok = false;
                out.push(format!("  {name:<10} {kind:<18} 실패  {model}: {e}"));
            }
        }
    }

    out.push("알려진 에이전트 CLI".into());
    for (bin, desc) in KNOWN_CLIS {
        let installed = which(bin);
        out.push(format!(
            "  {bin:<10} {desc:<30} {}",
            match &installed {
                Some(p) => format!("설치됨 {}", p.display()),
                None => "미설치".into(),
            }
        ));
    }
    Ok((out, ok))
}

async fn map(ws: &Path, openapi: Option<String>, no_openapi: bool) -> Result<ExitCode> {
    let cfg = load_config(ws)?;
    let (product_map, warnings, out) = build_map(ws, &cfg, openapi, no_openapi).await?;
    for w in warnings {
        eprintln!("경고: {w}");
    }
    for l in map_summary_lines(&product_map) {
        println!("{l}");
    }
    println!("저장: {}", out.display());
    Ok(ExitCode::SUCCESS)
}

/// Builds and saves `map/product-map.json`. Returns the map, warnings and the file path.
async fn build_map(
    ws: &Path,
    cfg: &Config,
    openapi: Option<String>,
    no_openapi: bool,
) -> Result<(mwa_productmap::map::ProductMap, Vec<String>, PathBuf)> {
    let mut warnings = Vec::new();
    let product = cfg
        .product
        .as_ref()
        .context("mwa.toml에 [product] 설정이 없습니다")?;
    let adapter = load_adapter(ws, product)?;
    let source = |name: &str| {
        let s = cfg
            .sources
            .iter()
            .find(|s| s.name == name)
            .expect("validated by config");
        snapshot_local(&s.name, &s.path).with_context(|| format!("소스 '{name}'"))
    };
    let (front, back) = (source(&product.frontend)?, source(&product.backend)?);
    for s in [&front, &back] {
        if s.is_preview() {
            warnings.push(format!(
                "소스 '{}'에 커밋 안 한 변경 {}개 — 이 지도는 미리보기용입니다",
                s.name, s.dirty_entries
            ));
        }
    }

    let origin = if no_openapi {
        None
    } else {
        openapi.or(product.openapi.clone())
    };
    let openapi = match origin {
        None => None,
        Some(o) => Some((o.clone(), read_openapi(&o).await?)),
    };

    let input = mwa_productmap::map::BuildInput {
        frontend: &front.root,
        backend: &back.root,
        frontend_sha: Some(front.sha.clone()),
        backend_sha: Some(back.sha.clone()),
        openapi,
    };
    let product_map = mwa_productmap::map::build(adapter, &input)?;
    let out = ws.join("map/product-map.json");
    std::fs::create_dir_all(out.parent().expect("has parent"))?;
    std::fs::write(&out, serde_json::to_string_pretty(&product_map)? + "\n")?;
    Ok((product_map, warnings, out))
}

/// OpenAPI over HTTP (GET only) or from a file.
async fn read_openapi(origin: &str) -> Result<String> {
    if origin.starts_with("http://") || origin.starts_with("https://") {
        let resp = reqwest::Client::new()
            .get(origin)
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .with_context(|| format!("OpenAPI를 받을 수 없습니다: {origin}"))?;
        if !resp.status().is_success() {
            anyhow::bail!("OpenAPI 요청 실패: {origin} → HTTP {}", resp.status());
        }
        Ok(resp.text().await?)
    } else {
        std::fs::read_to_string(origin)
            .with_context(|| format!("OpenAPI 파일을 읽을 수 없습니다: {origin}"))
    }
}

fn map_summary_lines(m: &mwa_productmap::map::ProductMap) -> Vec<String> {
    let mut out = Vec::new();
    let c = &m.coverage;
    let pct = |a: usize, b: usize| {
        if b == 0 {
            "-".to_string()
        } else {
            format!("{:.1}%", 100.0 * a as f64 / b as f64)
        }
    };
    out.push("제품 지도".into());
    out.push(format!(
        "  메뉴                 {}개 (화면 연결 {}개)",
        c.menus, c.menus_with_screen
    ));
    out.push(format!(
        "  화면                 {}개 (API 있는 화면 {}개)",
        c.screens, c.screens_with_api
    ));
    out.push(format!(
        "  화면 → 프록시        {}/{} ({})",
        c.client_calls_linked,
        c.client_calls,
        pct(c.client_calls_linked, c.client_calls)
    ));
    out.push(format!(
        "  프록시 → 백엔드      {}/{} ({})",
        c.proxy_resolved,
        c.proxy_handlers,
        pct(c.proxy_resolved, c.proxy_handlers)
    ));
    match &m.sources.openapi {
        Some(o) => out.push(format!(
            "  백엔드 → OpenAPI     {}/{} ({})  [{} · {}개 오퍼레이션]",
            c.upstream_with_operation,
            c.upstream_calls,
            pct(c.upstream_with_operation, c.upstream_calls),
            o.origin,
            o.operations
        )),
        None => out.push("  백엔드 → OpenAPI     생략 (OpenAPI 없음)".into()),
    }
    let server: usize = m
        .permissions
        .iter()
        .filter(|p| !p.server.is_empty())
        .count();
    out.push(format!(
        "  권한                 {}개 (서버 강제 {}개)",
        m.permissions.len(),
        server
    ));
    let g = &m.gaps;
    out.push(format!(
        "  확인 필요            URL 미해결 {} · 프록시 없는 호출 {} · 백엔드 호출 미인식 {} · OpenAPI에 없는 백엔드 호출 {} · 모르는 권한 코드 {}",
        g.unresolved_client_calls.len(),
        g.unlinked_client_calls.len(),
        g.unresolved_routes.len(),
        g.missing_backend_operations.len(),
        g.unknown_permission_codes.len()
    ));
    out
}

async fn auth(ws: &Path, action: AuthCmd) -> Result<ExitCode> {
    use mwa_capture::creds::{CredStore, key};
    use mwa_capture::login::{logout, whoami};

    let cfg = load_config(ws)?;
    let path = CredStore::default_path()?;
    let mut store = CredStore::open(&path)?;
    let profile = |name: &str| {
        cfg.profiles
            .get(name)
            .with_context(|| format!("mwa.toml에 [profiles.{name}]이 없습니다"))
    };

    match action {
        AuthCmd::Set {
            profile: name,
            stdin,
        } => {
            let p = profile(&name)?;
            let password = if stdin {
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                line.trim_end_matches(['\r', '\n']).to_string()
            } else {
                rpassword::prompt_password(format!("{} ({}) 비밀번호: ", p.user_id, p.base_url))?
            };
            if password.is_empty() {
                anyhow::bail!("비밀번호가 비어 있습니다");
            }
            store.set_password(&key(&p.user_id, &p.base_url), password);
            store.save()?;
            println!("저장했습니다: {} (권한 600)", store.path().display());
            println!("확인하려면: mwa auth check {name}  (로그인 1회 시도)");
        }
        AuthCmd::List => {
            for l in auth_list_lines(&cfg, &store) {
                println!("{l}");
            }
        }
        AuthCmd::Check { profile: name } => {
            let p = profile(&name)?;
            let spec = load_ui_spec(ws, &cfg)?;
            let cookie = login_profile(&spec.auth, &name, p).await?;
            let me = whoami(&spec.auth, &p.base_url, &cookie).await;
            let out = logout(&spec.auth, &p.base_url, &cookie).await;
            println!("로그인 성공: {} @ {}", p.user_id, p.base_url);
            match me {
                Ok(v) => {
                    let group = v["group"]
                        .as_str()
                        .or(v["group"]["name"].as_str())
                        .unwrap_or("(그룹 없음)");
                    let perms = v["permissions"].as_array().map_or(0, |a| a.len());
                    let domains = match v["domains"].as_array() {
                        None => "제한 없음".to_string(),
                        Some(d) => d
                            .iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    };
                    println!(
                        "  이름 {} / 그룹 {group} / 권한 {perms}개 / 도메인 {domains}",
                        v["name"].as_str().unwrap_or("-")
                    );
                }
                Err(e) => println!("  사용자 정보 조회 실패: {e}"),
            }
            if let Err(e) = out {
                println!("  로그아웃 실패: {e} (세션은 30분 뒤 만료)");
            }
        }
        AuthCmd::Forget { profile: name } => {
            let p = profile(&name)?;
            if store.remove(&key(&p.user_id, &p.base_url)) {
                store.save()?;
                println!("지웠습니다");
            } else {
                println!("저장된 비밀번호가 없습니다");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Logs in once with a stored profile and returns the session cookie pair.
/// Never retries; a rejection blocks the profile until `mwa auth set` (D-019).
async fn login_profile(
    auth: &mwa_capture::spec::AuthSpec,
    name: &str,
    p: &mwa_protocol::ProfileCfg,
) -> Result<String> {
    use mwa_capture::creds::{CredState, CredStore, key};
    use mwa_capture::login::{LoginOutcome, login_once};
    let mut store = CredStore::open(&CredStore::default_path()?)?;
    let k = key(&p.user_id, &p.base_url);
    let cred = store
        .get(&k)
        .with_context(|| format!("비밀번호가 없습니다. 먼저 `mwa auth set {name}`"))?;
    if cred.state != CredState::Ok {
        anyhow::bail!(
            "'{name}'은 자동 로그인이 막혀 있습니다({:?}{}). 비밀번호를 확인한 뒤 `mwa auth set {name}`으로 다시 저장하세요",
            cred.state,
            cred.note
                .as_deref()
                .map(|n| format!(": {n}"))
                .unwrap_or_default()
        );
    }
    let password = cred.password.clone();
    match login_once(auth, &p.base_url, &p.user_id, &password).await {
        LoginOutcome::Ok { cookie } => Ok(cookie),
        LoginOutcome::PasswordChangeRequired => {
            store.mark(
                &k,
                CredState::NeedsPasswordChange,
                Some("첫 로그인 비밀번호 변경 필요".into()),
            );
            store.save()?;
            anyhow::bail!(
                "첫 로그인이라 비밀번호 변경이 필요합니다. 브라우저에서 변경한 뒤 `mwa auth set {name}`"
            )
        }
        LoginOutcome::Rejected {
            status,
            title,
            info,
        } => {
            let note = format!("HTTP {status} {title} {info}").trim().to_string();
            store.mark(&k, CredState::Broken, Some(note.clone()));
            store.save()?;
            anyhow::bail!(
                "로그인 거부: {note}\n계정 잠금(하루 5회 실패)을 막으려고 이 프로파일의 자동 로그인을 중단했습니다. 재시도하지 않습니다"
            )
        }
        LoginOutcome::ServerError { status, message } => anyhow::bail!(
            "서버 오류로 로그인하지 못했습니다 ({status:?}): {message}. 비밀번호 상태는 바꾸지 않았습니다"
        ),
    }
}

/// Logs in with the capture profile, shoots every menu screen `runs` times, logs out.
async fn capture_menus(
    ws: &Path,
    cfg: &Config,
    runs: u32,
    only: &[String],
    world: bool,
    mut progress: impl FnMut(String),
) -> Result<mwa_capture::menus::Manifest> {
    use mwa_capture::menus::{Options, Target, capture};
    use mwa_capture::shoot::{Cookie, default_script};

    let cap = cfg
        .capture
        .as_ref()
        .context("mwa.toml에 [capture] 설정이 없습니다")?;
    let profile = cfg.profiles.get(&cap.profile).expect("validated by config");
    let spec = load_ui_spec(ws, cfg)?;
    let map_text = std::fs::read_to_string(ws.join("map/product-map.json"))
        .context("제품 지도가 없습니다. 먼저 mwa map (화면에서는 /map)을 실행하세요")?;
    let map: mwa_productmap::map::ProductMap = serde_json::from_str(&map_text)?;
    if let Some(unknown) = only
        .iter()
        .find(|k| !map.features.iter().any(|f| &f.key == *k))
    {
        anyhow::bail!("지도에 없는 기능 키입니다: {unknown}");
    }
    let targets: Vec<Target> = map
        .features
        .iter()
        .filter(|f| only.is_empty() || only.contains(&f.key))
        .map(|f| Target {
            key: f.key.clone(),
            label: f.label.clone(),
            route: f.href.clone(),
        })
        .collect();
    let frontend_sha = cfg.product.as_ref().and_then(|p| {
        let s = cfg.sources.iter().find(|s| s.name == p.frontend)?;
        snapshot_local(&s.name, &s.path).ok().map(|s| s.sha)
    });

    let mocks = if world {
        let m = world_mocks(ws, cfg)?;
        progress(format!("데모 데이터 응답 {}개를 씁니다", m.len()));
        m
    } else {
        Vec::new()
    };
    progress(format!("{} 로그인 1회 시도…", profile.user_id));
    let pair = login_profile(&spec.auth, &cap.profile, profile).await?;
    let cookie = Cookie::from_pair(&pair).context("세션 쿠키 형식이 올바르지 않습니다")?;
    let script = default_script();
    let job = capture_job(cap, &spec, profile, cookie, mocks)?;
    progress(format!("메뉴 {}개 × {runs}회 촬영 시작", targets.len()));
    let result = capture(
        Options {
            min_start_mb: cap.min_start_mb,
            abort_below_mb: cap.abort_below_mb,
            workspace: ws,
            node: &cap.node,
            script: &script,
            job,
            targets,
            runs,
            frontend_sha,
        },
        &mut progress,
    )
    .await;
    if let Err(e) = mwa_capture::login::logout(&spec.auth, &profile.base_url, &pair).await {
        progress(format!("로그아웃 실패: {e} (세션은 30분 뒤 만료)"));
    }
    Ok(result?)
}

/// Sidecar job settings shared by menu and detail shots (no shots yet).
fn capture_job(
    cap: &mwa_protocol::config::CaptureCfg,
    spec: &mwa_capture::spec::UiSpec,
    profile: &mwa_protocol::config::ProfileCfg,
    cookie: mwa_capture::shoot::Cookie,
    mocks: Vec<mwa_capture::shoot::Mock>,
) -> Result<mwa_capture::shoot::Job> {
    use mwa_capture::shoot::Job;
    Ok(Job {
        base_url: profile.base_url.clone(),
        cookie,
        domain: cap.domain.clone(),
        viewport: cap.viewport,
        scale: cap.scale,
        fixed_time: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64,
        allow_non_get: spec.auth.keepalive_paths.clone(),
        domain_storage_key: spec.ui.domain_storage_key.clone(),
        toast_selector: spec.ui.toast_selector.clone(),
        toast_type_attr: spec.ui.toast_type_attr.clone(),
        mask_buttons: spec.ui.mask_buttons.clone(),
        settle_ms: 800,
        timeout_ms: 30_000,
        mocks,
        shots: Vec::new(),
    })
}

/// Detail shots for the manual's "캡처 필요" blocks: plan (writer model) from the real
/// accessibility tree, shoot twice, re-plan failures up to twice. Working plans are kept and
/// replayed without the model next time.
async fn capture_detail(
    ws: &Path,
    cfg: &Config,
    features: &[String],
    world: bool,
    replan: bool,
    runs: u32,
    mut progress: impl FnMut(String),
) -> Result<bool> {
    use mwa_capture::detail::{self, Plan, Sidecar};
    use mwa_capture::shoot::{Cookie, default_script};
    use mwa_manual::shotplan::{self, Context};

    let cap = cfg
        .capture
        .as_ref()
        .context("mwa.toml에 [capture] 설정이 없습니다")?;
    let profile = cfg.profiles.get(&cap.profile).expect("validated by config");
    let spec = load_ui_spec(ws, cfg)?;
    if let Some(mb) = mwa_capture::mem::available_mb()
        && mb < cap.min_start_mb
    {
        anyhow::bail!(
            "가용 메모리 {mb}MB가 기준 {}MB보다 적어 촬영을 시작하지 않습니다",
            cap.min_start_mb
        );
    }
    let ctx = product_ctx(ws, cfg)?;
    let d = open_dispatcher(ws, cfg.clone())?;
    let mocks = if world {
        world_mocks(ws, cfg)?
    } else {
        Vec::new()
    };
    let w = mwa_manual::world::load(ws)?;
    let script = default_script();
    let sc = Sidecar {
        node: &cap.node,
        script: &script,
    };

    // Work out what needs planning before logging in.
    struct Work<'a> {
        feature: &'a mwa_productmap::map::Feature,
        reuse: Vec<Plan>,
        todo: Vec<(String, String)>,
    }
    let mut work = Vec::new();
    for key in features {
        let f = ctx
            .map
            .features
            .iter()
            .find(|f| &f.key == key)
            .with_context(|| format!("지도에 없는 기능 키입니다: {key}"))?;
        let run = mwa_manual::pipeline::load_run(ws, key)
            .with_context(|| format!("{key} 매뉴얼이 없습니다. 먼저 mwa gen {key}"))?;
        let todos = shotplan::todos(&run);
        let old = detail::load_manifest(ws, key).unwrap_or_default();
        let mut reuse = Vec::new();
        let mut todo = Vec::new();
        for (id, what) in todos {
            match old.records.iter().find(|r| r.plan.todo == id && r.ok) {
                Some(r) if !replan => reuse.push(r.plan.clone()),
                _ => todo.push((id, what)),
            }
        }
        progress(format!(
            "{key}: 캡처 필요 {}개 (저장된 계획 재생 {}개, 새로 계획 {}개)",
            reuse.len() + todo.len(),
            reuse.len(),
            todo.len()
        ));
        work.push(Work {
            feature: f,
            reuse,
            todo,
        });
    }

    progress(format!("{} 로그인 1회 시도…", profile.user_id));
    let pair = login_profile(&spec.auth, &cap.profile, profile).await?;
    let cookie = Cookie::from_pair(&pair).context("세션 쿠키 형식이 올바르지 않습니다")?;
    let job = capture_job(cap, &spec, profile, cookie, mocks)?;
    let mut all_ok = true;
    let result: Result<()> = async {
        for wk in &work {
            let f = wk.feature;
            let routes: Vec<String> = f.screens.iter().map(|s| s.route.clone()).collect();
            // Planner context: real accessibility trees of the static start pages.
            let needs_ctx = !wk.todo.is_empty() || !wk.reuse.is_empty();
            let ctx_for_plan = if needs_ctx {
                let starts: Vec<String> = routes
                    .iter()
                    .filter(|r| !r.contains('{'))
                    .cloned()
                    .collect();
                let tmp = ws.join(format!(".mwa/shots/probe-{}", job.fixed_time));
                let evs = detail::probe(sc, &job, &tmp, &starts).await?;
                let _ = std::fs::remove_dir_all(&tmp);
                let snapshots = evs
                    .into_iter()
                    .filter_map(|e| Some((e.id, e.aria?)))
                    .collect();
                let slice = mwa_manual::pipeline::build_slice(
                    &ctx.map,
                    f,
                    &gen_options(ws, &ctx, &f.key, None, None),
                )?;
                let mut labels: Vec<String> = mwa_manual::labels::extract(&slice)
                    .into_iter()
                    .map(|l| l.text)
                    .collect();
                labels.sort();
                labels.dedup();
                labels.truncate(500);
                let data: Vec<String> = w
                    .responses
                    .iter()
                    .filter(|r| r.feature == f.key && r.method == "GET")
                    .map(|r| {
                        let body: String = r.body.to_string().chars().take(1200).collect();
                        format!("{} {:?}: {body}", r.path, r.params)
                    })
                    .collect();
                Some(Context {
                    feature_label: &f.label,
                    routes: routes.clone(),
                    snapshots,
                    data: data.join("\n"),
                    labels,
                })
            } else {
                None
            };
            let mut plans = wk.reuse.clone();
            if let (false, Some(c)) = (wk.todo.is_empty(), &ctx_for_plan) {
                progress(format!(
                    "{}: 촬영 계획 작성 중 ({}개)",
                    f.key,
                    wk.todo.len()
                ));
                plans.extend(shotplan::plan(&d, c, &wk.todo).await?);
            }
            if plans.is_empty() {
                continue;
            }
            let mut records =
                detail::shoot(sc, &job, ws, &f.key, &plans, runs, &mut progress).await?;
            for round in 1..=2 {
                let failed: Vec<usize> = (0..records.len()).filter(|&i| !records[i].ok).collect();
                let (false, Some(c)) = (failed.is_empty(), &ctx_for_plan) else {
                    break;
                };
                progress(format!(
                    "{}: 실패 {}개 계획 수정 ({round}/2)",
                    f.key,
                    failed.len()
                ));
                let mut fixed = Vec::new();
                for &i in &failed {
                    let r = &records[i];
                    let err = r.error.clone().unwrap_or_default();
                    fixed.push(shotplan::repair(&d, c, &r.plan, &err, r.aria.as_deref()).await?);
                }
                let again =
                    detail::shoot(sc, &job, ws, &f.key, &fixed, runs, &mut progress).await?;
                for (i, r) in failed.into_iter().zip(again) {
                    records[i] = r;
                }
            }
            let m = detail::save_manifest(ws, &f.key, job.fixed_time / 1000, &records)?;
            // Usable pictures replace their "캡처 필요" blocks in the manual.
            let usable: Vec<(String, String)> = records
                .iter()
                .filter(|r| r.ok && r.needs_retake.is_none())
                .map(|r| {
                    (
                        r.plan.todo.clone(),
                        format!("detail/{}/{}", f.key, r.plan.todo),
                    )
                })
                .collect();
            let applied = shotplan::apply_shots(ws, &f.key, &usable)?;
            if applied > 0 {
                progress(format!("{}: 매뉴얼에 사진 {applied}장 반영", f.key));
            }
            let ok = records.iter().filter(|r| r.ok).count();
            progress(format!("{}: 촬영 {ok}/{}", f.key, records.len()));
            for r in &records {
                let mark = match (r.ok, &r.needs_retake) {
                    (true, None) => "✓",
                    (true, Some(_)) => "△",
                    _ => "✗",
                };
                let note = r
                    .needs_retake
                    .clone()
                    .or_else(|| (!r.ok).then(|| r.error.clone().unwrap_or_default()))
                    .map(|n| format!(" — {n}"))
                    .unwrap_or_default();
                progress(format!("  {mark} {}: {}{note}", r.plan.todo, r.plan.what));
            }
            progress(format!(
                "  저장: {}",
                detail::manifest_path(ws, &m.feature).display()
            ));
            all_ok &= records.iter().all(|r| r.ok && r.needs_retake.is_none());
        }
        Ok(())
    }
    .await;
    if let Err(e) = mwa_capture::login::logout(&spec.auth, &profile.base_url, &pair).await {
        progress(format!("로그아웃 실패: {e} (세션은 30분 뒤 만료)"));
    }
    result.map(|_| all_ok)
}

/// Demo responses from the world file, plus empty successes for writes whose result the
/// screens ignore (so dialogs can proceed without anything reaching the server).
fn world_mocks(ws: &Path, cfg: &Config) -> Result<Vec<mwa_capture::shoot::Mock>> {
    use mwa_capture::shoot::Mock;
    use mwa_productmap::shapes::{Shape, ShapeSource, feature_shapes};
    let w = mwa_manual::world::load(ws)?;
    anyhow::ensure!(
        !w.responses.is_empty(),
        "데모 데이터가 없습니다. 먼저 mwa world gen <기능>을 실행하세요"
    );
    let mut mocks: Vec<Mock> = w
        .responses
        .iter()
        .map(|r| Mock {
            method: r.method.clone(),
            path: r.path.clone(),
            params: r.params.clone(),
            query: r.query.clone(),
            body: r.body.clone(),
        })
        .collect();
    let ctx = product_ctx(ws, cfg)?;
    let mut project = mwa_productmap::graph::Project::new(&ctx.frontend, ctx.adapter.alias_prefix);
    for f in ctx
        .map
        .features
        .iter()
        .filter(|f| w.features.contains_key(&f.key))
    {
        for s in feature_shapes(&mut project, f, ctx.adapter.client_call_fns)? {
            let free = s.method != "GET"
                && s.source != ShapeSource::UrlOnly
                && matches!(s.shape, Shape::Any | Shape::Void);
            if free
                && !mocks
                    .iter()
                    .any(|m| m.method == s.method && m.path == s.path)
            {
                mocks.push(Mock {
                    method: s.method,
                    path: s.path,
                    params: Vec::new(),
                    query: Default::default(),
                    body: serde_json::json!({}),
                });
            }
        }
    }
    Ok(mocks)
}

/// Login and screen conventions (`[auth]`, `[ui]`) from the product adapter file.
fn load_ui_spec(ws: &Path, cfg: &Config) -> Result<mwa_capture::spec::UiSpec> {
    let product = cfg
        .product
        .as_ref()
        .context("mwa.toml에 [product] 설정이 없습니다")?;
    mwa_capture::spec::load(&ws.join(&product.adapter))
        .map_err(|e| anyhow::anyhow!("어댑터의 [auth]/[ui] 설정을 읽을 수 없습니다: {e}"))
}

/// The product adapter named in `[product] adapter` (a file relative to the workspace).
fn load_adapter(
    ws: &Path,
    product: &mwa_protocol::config::ProductCfg,
) -> Result<&'static mwa_productmap::adapters::Adapter> {
    let path = ws.join(&product.adapter);
    mwa_productmap::adapters::load(&path)
        .with_context(|| format!("제품 어댑터 파일을 읽을 수 없습니다: {}", path.display()))
}

struct ProductCtx {
    map: mwa_productmap::map::ProductMap,
    adapter: &'static mwa_productmap::adapters::Adapter,
    frontend: PathBuf,
    backend: PathBuf,
}

fn product_ctx(ws: &Path, cfg: &Config) -> Result<ProductCtx> {
    let product = cfg
        .product
        .as_ref()
        .context("mwa.toml에 [product] 설정이 없습니다")?;
    let adapter = load_adapter(ws, product)?;
    let root = |name: &str| -> Result<PathBuf> {
        let s = cfg
            .sources
            .iter()
            .find(|s| s.name == name)
            .expect("validated by config");
        Ok(snapshot_local(&s.name, &s.path)?.root)
    };
    let map_text = std::fs::read_to_string(ws.join("map/product-map.json"))
        .context("제품 지도가 없습니다. 먼저 mwa map (화면에서는 /map)을 실행하세요")?;
    Ok(ProductCtx {
        map: serde_json::from_str(&map_text)?,
        adapter,
        frontend: root(&product.frontend)?,
        backend: root(&product.backend)?,
    })
}

fn world_shapes(ws: &Path, features: &[String]) -> Result<ExitCode> {
    use mwa_productmap::shapes::{ApiShape, Shape, ShapeSource, feature_shapes};
    let cfg = load_config(ws)?;
    let ctx = product_ctx(ws, &cfg)?;
    let mut project = mwa_productmap::graph::Project::new(&ctx.frontend, ctx.adapter.alias_prefix);
    let dir = ws.join("world/shapes");
    std::fs::create_dir_all(&dir)?;
    for key in features {
        let f = ctx
            .map
            .features
            .iter()
            .find(|f| &f.key == key)
            .with_context(|| format!("지도에 없는 기능 키입니다: {key}"))?;
        let shapes = feature_shapes(&mut project, f, ctx.adapter.client_call_fns)?;
        // A write whose response the screen ignores needs no shape.
        let free_write = |s: &ApiShape| {
            s.source == ShapeSource::UrlOnly
                || s.method != "GET" && matches!(s.shape, Shape::Any | Shape::Void)
        };
        let failed = |s: &ApiShape| {
            !free_write(s) && (!s.shape.unresolved().is_empty() || s.source == ShapeSource::None)
        };
        let n_failed = shapes.iter().filter(|s| failed(s)).count();
        let n_free = shapes.iter().filter(|s| free_write(s)).count();
        println!(
            "{key}: API {}개 — 형태 있음 {}개, 형태 불필요 {}개, 미해결 {}개",
            shapes.len(),
            shapes.len() - n_failed - n_free,
            n_free,
            n_failed
        );
        for s in &shapes {
            let mark = if failed(s) {
                "✗"
            } else if free_write(s) {
                "·"
            } else {
                "✓"
            };
            let note = match s.source {
                ShapeSource::UrlOnly => " (URL만 만듦: 다운로드·스트림)",
                _ if free_write(s) => " (쓰기, 응답 무시)",
                _ => "",
            };
            println!("  {mark} {} {}{note}", s.method, s.path);
            for u in s.shape.unresolved() {
                println!("      미해결: {u}");
            }
            if failed(s) && s.source == ShapeSource::None {
                println!("      타입 없음: {}", s.client);
            }
            let free = s.shape.free_paths();
            if !free_write(s) && !free.is_empty() {
                println!("      자유 값: {}", free.join(", "));
            }
        }
        let path = dir.join(format!("{key}.json"));
        std::fs::write(&path, serde_json::to_string_pretty(&shapes)?)?;
        println!("  저장: {}", path.display());
    }
    Ok(ExitCode::SUCCESS)
}

/// Shapes, value hints and data-interpreting code of one feature, for the demo-data writer.
fn world_input(
    ws: &Path,
    ctx: &ProductCtx,
    project: &mut mwa_productmap::graph::Project,
    key: &str,
) -> Result<(mwa_manual::world::FeatureInput, String)> {
    use mwa_manual::model::Repo;
    use mwa_productmap::shapes::{feature_shapes, fingerprint, value_hints};
    let f = ctx
        .map
        .features
        .iter()
        .find(|f| f.key == key)
        .with_context(|| format!("지도에 없는 기능 키입니다: {key}"))?;
    let shapes = feature_shapes(project, f, ctx.adapter.client_call_fns)?;
    let fp = fingerprint(&shapes);
    let slice =
        mwa_manual::pipeline::build_slice(&ctx.map, f, &gen_options(ws, ctx, key, None, None))?;
    let frontend: Vec<_> = slice
        .files
        .iter()
        .filter(|s| s.repo == Repo::Frontend)
        .collect();
    // Hints only for properties the responses actually have.
    let fields: std::collections::BTreeSet<String> =
        shapes.iter().flat_map(|s| s.shape.field_names()).collect();
    let paths: Vec<PathBuf> = frontend
        .iter()
        .map(|s| ctx.frontend.join(&s.path))
        .collect();
    let mut hints = value_hints(project, &paths)?;
    hints.retain(|k, _| fields.contains(k));
    // Code that interprets data: mappers, status tables and types, not the JSX.
    let mut code = String::new();
    for s in frontend
        .iter()
        .filter(|s| s.path.contains("/_data/") || s.path.contains("/lib/api/"))
    {
        code.push_str(&format!("=== {}\n", s.path));
        for &(a, b) in &s.ranges {
            code.push_str(&s.lines[a - 1..b].join("\n"));
            code.push('\n');
        }
    }
    let input = mwa_manual::world::FeatureInput {
        key: key.to_string(),
        label: f.label.clone(),
        shapes,
        code,
        hints,
    };
    Ok((input, fp))
}

fn print_world_report(key: &str, rep: &mwa_manual::world::Report, extra: &str) {
    println!(
        "{key}: 응답 {}개, 검사 위반 {}건{}, 경고 {}건{extra}",
        rep.responses,
        rep.errors.len(),
        if rep.repaired {
            " (1회 수정 후)"
        } else {
            ""
        },
        rep.warnings.len(),
    );
    for e in &rep.errors {
        println!("  ✗ {e}");
    }
    for e in rep.warnings.iter().take(10) {
        println!("  ! {e}");
    }
}

async fn world_gen(ws: &Path, features: &[String], force: bool) -> Result<ExitCode> {
    use mwa_manual::world;
    let cfg = load_config(ws)?;
    let ctx = product_ctx(ws, &cfg)?;
    let d = open_dispatcher(ws, cfg.clone())?;
    let mut project = mwa_productmap::graph::Project::new(&ctx.frontend, ctx.adapter.alias_prefix);
    let mut w = world::load(ws)?;
    let mut ok = true;
    for key in features {
        let (input, fp) = world_input(ws, &ctx, &mut project, key)?;
        if !force && w.features.get(key) == Some(&fp) {
            println!("{key}: 응답 형태가 그대로라 건너뜁니다 (--force로 다시 만듭니다)");
            continue;
        }
        let spent = d.spent_usd();
        let rep = world::generate(&d, &mut w, &input, &fp, &|l| println!("{l}")).await?;
        world::save(ws, &w)?;
        print_world_report(
            key,
            &rep,
            &format!(", 구독 사용량 정가 환산 ${:.3}", d.spent_usd() - spent),
        );
        ok &= rep.errors.is_empty();
    }
    println!("저장: {}", ws.join(world::WORLD_FILE).display());
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn world_check(ws: &Path, features: &[String]) -> Result<ExitCode> {
    use mwa_manual::world;
    let cfg = load_config(ws)?;
    let ctx = product_ctx(ws, &cfg)?;
    let mut project = mwa_productmap::graph::Project::new(&ctx.frontend, ctx.adapter.alias_prefix);
    let w = world::load(ws)?;
    let keys: Vec<String> = if features.is_empty() {
        w.features.keys().cloned().collect()
    } else {
        features.to_vec()
    };
    let mut ok = true;
    for key in &keys {
        let (input, fp) = world_input(ws, &ctx, &mut project, key)?;
        let rep = world::validate(&input, &w);
        let stale = match w.features.get(key) {
            None => ", 아직 만들지 않음",
            Some(old) if old != &fp => ", 응답 형태가 바뀜 (mwa world gen으로 다시 만드세요)",
            Some(_) => "",
        };
        print_world_report(key, &rep, stale);
        ok &= rep.errors.is_empty() && stale.is_empty();
    }
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn gen_slice_only(ws: &Path, cfg: &Config, feature: &str) -> Result<ExitCode> {
    let ctx = product_ctx(ws, cfg)?;
    let f = ctx
        .map
        .features
        .iter()
        .find(|f| f.key == feature)
        .with_context(|| format!("지도에 없는 기능 키입니다: {feature}"))?;
    let opts = gen_options(ws, &ctx, feature, None, None);
    let slice = mwa_manual::pipeline::build_slice(&ctx.map, f, &opts)?;
    for file in &slice.files {
        let lines: usize = file.ranges.iter().map(|(a, b)| b - a + 1).sum();
        println!(
            "{:>5}줄  {}:{}",
            lines,
            mwa_manual::slice::repo_name(file.repo),
            file.path
        );
    }
    let chunks = slice.chunks(60_000);
    println!(
        "합계 {}줄, 청크 {}개 ({}자)",
        slice.total_lines(),
        chunks.len(),
        chunks.iter().map(|c| c.len()).sum::<usize>()
    );
    let labels = mwa_manual::labels::extract(&slice);
    println!("화면 라벨 {}개:", labels.len());
    for l in labels.iter().take(80) {
        println!("  [{}] {}  ({}:{})", l.origin, l.text, l.file, l.line);
    }
    Ok(ExitCode::SUCCESS)
}

fn gen_options<'a>(
    ws: &'a Path,
    ctx: &'a ProductCtx,
    feature: &'a str,
    template: Option<(String, String)>,
    compare: Option<(String, String)>,
) -> mwa_manual::pipeline::Options<'a> {
    let shots = mwa_capture::menus::load_manifest(ws)
        .map(|m| {
            m.shots
                .into_iter()
                .filter(|s| s.key == feature && s.ok && s.needs_retake.is_none())
                .map(|s| {
                    (
                        format!("menus/{}", s.key),
                        format!("{} 목록 화면 (메뉴 진입 직후)", s.label),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .chain(
            mwa_capture::detail::load_manifest(ws, feature)
                .map(|m| m.records)
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.ok && r.needs_retake.is_none())
                .map(|r| (format!("detail/{feature}/{}", r.plan.todo), r.plan.what)),
        )
        .collect();
    mwa_manual::pipeline::Options {
        workspace: ws,
        feature,
        frontend_root: &ctx.frontend,
        backend_root: &ctx.backend,
        backend_code_dir: ctx.adapter.backend_code_dir,
        alias_prefix: ctx.adapter.alias_prefix,
        stop_dirs: ctx.adapter.stop_dirs,
        template_example: template,
        compare_with: compare,
        shots,
    }
}

async fn generate_manual(
    ws: &Path,
    cfg: &Config,
    feature: &str,
    template: Option<(String, String)>,
    compare: Option<(String, String)>,
    progress: impl Fn(String) + Send + Sync,
) -> Result<mwa_manual::model::ManualRun> {
    let ctx = product_ctx(ws, cfg)?;
    let d = open_dispatcher(ws, cfg.clone())?;
    let opts = gen_options(ws, &ctx, feature, template, compare);
    Ok(mwa_manual::pipeline::generate(&d, &ctx.map, opts, &progress).await?)
}

/// Profiles and credential state; never includes passwords.
fn auth_list_lines(cfg: &Config, store: &mwa_capture::creds::CredStore) -> Vec<String> {
    use mwa_capture::creds::{CredState, key};
    let mut out = vec![format!("자격 파일: {}", store.path().display())];
    if cfg.profiles.is_empty() {
        out.push("  (mwa.toml에 [profiles.*]가 없습니다)".into());
    }
    for (name, p) in &cfg.profiles {
        let state = match store.get(&key(&p.user_id, &p.base_url)) {
            None => "비밀번호 없음".to_string(),
            Some(c) => {
                let s = match c.state {
                    CredState::Ok => "저장됨",
                    CredState::Broken => "자동 로그인 중단(거부됨)",
                    CredState::NeedsPasswordChange => "비밀번호 변경 필요",
                };
                match &c.note {
                    Some(n) => format!("{s} — {n}"),
                    None => s.to_string(),
                }
            }
        };
        out.push(format!(
            "  {name:<12} {:<10} {:<28} {:?}  {state}",
            p.user_id, p.base_url, p.purpose
        ));
    }
    out
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses() {
        let (cfg, _warnings) = Config::parse(TEMPLATE).unwrap();
        // Product sources are entered per project; the template only shows examples.
        assert!(cfg.sources.is_empty() && cfg.product.is_none());
        for role in Role::ALL {
            assert!(cfg.role(role).is_some(), "template must configure {role}");
        }
    }

    #[test]
    fn exec_rejects_bad_schema_before_calling() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join(CONFIG_FILE), TEMPLATE).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt
            .block_on(exec(
                ws.path(),
                Role::Worker,
                Some("{not json".into()),
                vec![],
                "s",
                "p",
            ))
            .unwrap_err();
        assert!(err.to_string().contains("스키마"));
        assert!(
            !ws.path().join(STATE_DIR).exists(),
            "no call or log before validation"
        );
    }
}
