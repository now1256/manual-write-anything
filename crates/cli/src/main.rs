use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mwa_core::{Dispatcher, RunLog};
use mwa_protocol::{Config, Request, Role};
use mwa_source::{Snapshot, snapshot_local};
use serde_json::{Value, json};

mod adapter_gen;
mod automation;
mod confluence;
mod projects;
mod settings;
mod setup;
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
        /// 열 주소. 기본은 이 서버의 모든 주소 7000번: 사내망에서 http://<서버 IP>:7000 으로 접속
        #[arg(long, default_value = "0.0.0.0:7000")]
        addr: std::net::SocketAddr,
        /// 개발용: 내장 화면 대신 이 폴더(web/dist)의 화면을 매번 읽습니다. 화면만 바뀌면 새로고침으로 반영됩니다
        #[arg(long, value_name = "DIR")]
        web_dir: Option<PathBuf>,
    },
    /// 작업 공간을 만들고 소스의 현재 커밋을 기록합니다
    Init,
    /// 코드를 읽어 제품 어댑터(adapter.toml)를 만들고 제품 지도로 확인합니다 (시작하기 "코드 읽고 제품 파악")
    Adapter,
    /// 촬영용 브라우저(Playwright + Chromium)를 데이터 폴더에 설치합니다 (화면의 "브라우저 설치"와 같음)
    Browser,
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
        /// menus: 메뉴 첫 화면 전체 / detail: 매뉴얼의 "캡처 필요" 화면 / ref: 기존 매뉴얼의 장면 다시 찍기 / doc: 쓴 매뉴얼의 그림 자리 찍기 (detail·ref·doc은 --only로 기능 지정)
        #[arg(value_parser = ["menus", "detail", "ref", "doc"])]
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
        /// ref: 저장된 촬영 계획으로 한 번씩만 빠르게 찍습니다 (모델 호출·대조 없음)
        #[arg(long)]
        quick: bool,
        /// ref: 이 그림(img/...)의 장면만 다시 찍고 나머지 결과는 둡니다. doc: 이 그림 자리(블록 id)만 다시 찍습니다
        #[arg(long)]
        scene: Option<String>,
    },
    /// 기능의 코드를 읽습니다: 사실을 뽑고 인용 줄을 검사합니다 (③ 코드 읽기)
    Gen {
        /// 기능 키 (예: user-account). 지도에서 확인
        feature: String,
        /// LLM 없이 코드 조각과 화면 라벨만 보여줍니다
        #[arg(long)]
        slice_only: bool,
    },
    /// 매뉴얼 양식을 관리합니다
    Template {
        #[command(subcommand)]
        action: TemplateCmd,
    },
    /// 편집하는 매뉴얼 문서를 다룹니다
    Doc {
        #[command(subcommand)]
        action: DocCmd,
    },
    /// 기존 매뉴얼(PDF·MD)을 가져옵니다
    Ref {
        #[command(subcommand)]
        action: RefCmd,
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
enum TemplateCmd {
    /// 견본 매뉴얼(PDF·Markdown)에서 양식만 배워 template/schema.json에 저장합니다 (writer 1회 호출)
    Learn { example: PathBuf },
}

#[derive(Subcommand)]
enum DocCmd {
    /// 기능의 기존 매뉴얼 글을 그대로 문서로 옮깁니다 (예비 방식, PDF는 쪽마다 worker 1번)
    Import { feature: String },
    /// 문서를 코드 사실과 비교해 갱신 제안을 만듭니다 (writer 1번, 코드 읽기 결과 사용)
    Update { feature: String },
    /// 코드로 매뉴얼을 새로 씁니다. 기존 매뉴얼은 양식·구조의 본보기 (③ 매뉴얼 만들기와 같음).
    /// 기존 매뉴얼이 없으면(또는 --fresh) 다른 매뉴얼을 본보기로 쓰고 그림 자리를 바로 찍습니다
    Write {
        feature: String,
        /// 짝지은 기존 매뉴얼을 쓰지 않고 처음부터 새로 씁니다
        #[arg(long)]
        fresh: bool,
    },
}

#[derive(Subcommand)]
enum RefCmd {
    /// 기존 매뉴얼(PDF·MD, 폴더면 안의 파일 모두)을 가져와 그림·제목을 뽑고 기능과 짝짓습니다 (reference/)
    Import {
        #[arg(required = true)]
        pdfs: Vec<PathBuf>,
    },
    /// 가져온 기존 매뉴얼과 짝지은 기능을 봅니다
    List,
    /// 기존 매뉴얼 캡처마다 장면 설명을 만듭니다 (worker 모델, 이미지)
    Describe { id: String },
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
        Cmd::Adapter => {
            for l in adapter_gen::make(cli_workspace, |l| println!("{l}")).await? {
                println!("{l}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Browser => {
            ensure_project(cli_workspace)?;
            let cfg = load_config(cli_workspace)?;
            for l in install_browser(&cfg, |l| println!("{l}"))? {
                println!("{l}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Serve { addr, web_dir } => serve(cli_workspace, addr, web_dir).await,
        Cmd::Doctor { no_call } => doctor(cli_workspace, no_call).await,
        Cmd::Map {
            openapi,
            no_openapi,
        } => map(cli_workspace, openapi, no_openapi).await,
        Cmd::Auth { action } => auth(cli_workspace, action).await,
        Cmd::Ref { action } => reference_cmd(cli_workspace, action),
        Cmd::Doc {
            action: DocCmd::Import { feature },
        } => {
            let cfg = load_config(cli_workspace)?;
            let d = open_dispatcher(cli_workspace, cfg)?;
            let doc =
                mwa_manual::document::import(&d, cli_workspace, &feature, &|l| println!("{l}"))
                    .await?;
            println!(
                "{}: 문서로 옮김 ({}자) · 사용량 정가 환산 ${:.3}",
                doc.title,
                doc.html.chars().count(),
                d.spent_usd()
            );
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Doc {
            action: DocCmd::Write { feature, fresh },
        } => {
            use mwa_host::Host;
            let host = tui_host::CliHost::open(cli_workspace)?;
            let target = if fresh {
                format!("{feature}?fresh")
            } else {
                feature
            };
            for l in host
                .make_document(&target, Box::new(|l: &str| println!("{l}")))
                .await
                .map_err(|e| anyhow::anyhow!(e))?
            {
                println!("{l}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Doc {
            action: DocCmd::Update { feature },
        } => {
            let cfg = load_config(cli_workspace)?;
            let run = mwa_manual::pipeline::load_run(cli_workspace, &feature)
                .context("코드에서 뽑은 사실이 없습니다. 먼저 mwa gen")?;
            let d = open_dispatcher(cli_workspace, cfg)?;
            let p = mwa_manual::document::propose(&d, cli_workspace, &feature, &run.facts).await?;
            for x in &p.items {
                println!("[{} {}] {}\n  {}", x.action, x.block, x.reason, x.html);
            }
            println!(
                "갱신 제안 {}개 · 사용량 정가 환산 ${:.3}",
                p.items.len(),
                d.spent_usd()
            );
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Template {
            action: TemplateCmd::Learn { example },
        } => {
            let cfg = load_config(cli_workspace)?;
            let ex = mwa_manual::reference::load_example(cli_workspace, &example)?;
            println!(
                "견본: {} (본문 {}자, 페이지 이미지 {}장)",
                ex.name,
                ex.text.chars().count(),
                ex.pages.len()
            );
            let d = open_dispatcher(cli_workspace, cfg)?;
            let t = mwa_manual::template::learn(&d, &ex.text, &ex.name, &ex.pages).await?;
            let path = cli_workspace.join(mwa_manual::pipeline::TEMPLATE_FILE);
            std::fs::create_dir_all(path.parent().expect("has parent"))?;
            std::fs::write(&path, serde_json::to_string_pretty(&t)?)?;
            println!("절 {}개:", t.sections.len());
            for s in &t.sections {
                println!("  - {} ({})", s.title, s.typical_blocks.join(", "));
            }
            println!("문체 규칙 {}개:", t.style_rules.len());
            for r in &t.style_rules {
                println!("  · {r}");
            }
            println!(
                "저장: {}  (사용량 정가 환산 ${:.3})",
                path.display(),
                d.spent_usd()
            );
            Ok(ExitCode::SUCCESS)
        }
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
            slice_only,
        } => {
            let cfg = load_config(cli_workspace)?;
            if slice_only {
                return gen_slice_only(cli_workspace, &cfg, &feature);
            }
            let run = generate_manual(cli_workspace, &cfg, &feature, |l| println!("{l}")).await?;
            for l in mwa_manual::pipeline::summary(&run) {
                println!("{l}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Shot {
            what,
            runs,
            only,
            world,
            replan,
            scene,
            quick,
        } => {
            let cfg = load_config(cli_workspace)?;
            if what == "doc" {
                anyhow::ensure!(only.len() == 1, "doc은 --only <기능 키> 하나가 필요합니다");
                let lines = capture_doc(cli_workspace, &cfg, &only[0], scene.as_deref(), |l| {
                    println!("{l}")
                })
                .await?;
                for l in lines {
                    println!("{l}");
                }
                return Ok(ExitCode::SUCCESS);
            }
            if what == "ref" {
                anyhow::ensure!(only.len() == 1, "ref는 --only <기능 키> 하나가 필요합니다");
                let shots = capture_reference(
                    cli_workspace,
                    &cfg,
                    &only[0],
                    world,
                    scene.as_deref(),
                    quick,
                    |l| println!("{l}"),
                )
                .await?;
                for s in &shots.scenes {
                    let mark = match s.status.as_str() {
                        "reproduced" => "✓",
                        "different" => "△",
                        _ => "✗",
                    };
                    let why = match s.status.as_str() {
                        "failed" => format!(
                            " — {}: {}",
                            mwa_manual::reference::reason_label(
                                s.reason_kind.as_deref().unwrap_or("error")
                            ),
                            s.reason.as_deref().unwrap_or("")
                        ),
                        "different" => format!(" — {}", s.differences.join(" / ")),
                        _ => String::new(),
                    };
                    println!("  {mark} p.{} {}{why}", s.page, s.what);
                }
                println!("{}", mwa_manual::reference::summarize(&shots));
                return Ok(ExitCode::SUCCESS);
            }
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

/// A new folder becomes a project, as in the desktop app.
fn ensure_project(ws: &Path) -> Result<()> {
    if !ws.join(CONFIG_FILE).exists() {
        std::fs::create_dir_all(ws)?;
        std::fs::write(ws.join(CONFIG_FILE), TEMPLATE)?;
        std::fs::write(ws.join(".gitignore"), format!("{STATE_DIR}/\n"))?;
        println!("새 프로젝트를 만들었습니다: {}", ws.display());
    }
    Ok(())
}

async fn serve(
    ws: &Path,
    addr: std::net::SocketAddr,
    web_dir: Option<PathBuf>,
) -> Result<ExitCode> {
    ensure_project(ws)?;
    let host = std::sync::Arc::new(tui_host::CliHost::open(ws)?);
    let registry = projects::Registry::new(&projects::data_dir());
    let ws_abs = std::fs::canonicalize(ws).unwrap_or_else(|_| ws.to_path_buf());
    if let Err(e) = registry.touch(&ws_abs, None) {
        eprintln!("경고: 프로젝트 목록에 등록하지 못했습니다: {e}");
    }
    let hub = mwa_server::Hub::new(host, Some(std::sync::Arc::new(registry)));
    let listener = mwa_server::bind(addr).await?;
    let port = listener.local_addr()?.port();
    let _ = automation::SERVE_PORT.set(port);
    if addr.ip().is_unspecified() {
        // The address others type: the one this machine uses on its network.
        let ip = std::net::UdpSocket::bind("0.0.0.0:0")
            .and_then(|u| {
                u.connect("10.254.254.254:1")?;
                u.local_addr()
            })
            .map(|a| a.ip().to_string())
            .unwrap_or_else(|_| "<서버 IP>".into());
        println!("mwa serve: http://{ip}:{port}  (이 서버: http://localhost:{port})");
        println!("같은 사내망에서 위 주소로 접속합니다. 접속 비밀번호는 없습니다.");
    } else {
        println!("mwa serve: http://{addr}");
    }
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

/// The configured sample manual, used only until the project has learned a format.
fn first_template(ws: &Path, cfg: &Config) -> Option<PathBuf> {
    if ws.join(mwa_manual::pipeline::TEMPLATE_FILE).is_file() {
        return None;
    }
    cfg.manual.template.as_deref().map(|p| {
        let p = PathBuf::from(p);
        if p.is_relative() { ws.join(p) } else { p }
    })
}

/// Node, Playwright, Chromium, memory and the adapter file: what screenshots need.
fn capture_env_lines(ws: &Path, cfg: &Config) -> (Vec<String>, bool) {
    let mut out = vec!["촬영 환경".to_string()];
    let mut ok = true;
    let node = cfg
        .capture
        .as_ref()
        .map_or("node".to_string(), |c| c.node.clone());
    let script = mwa_capture::shoot::default_script();
    let dir = script.parent().unwrap_or(Path::new(".")).to_path_buf();
    let run = |args: &[&str]| {
        std::process::Command::new(&node)
            .args(args)
            .current_dir(&dir)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    match run(&["--version"]) {
        None => {
            ok = false;
            out.push(format!(
                "  node           실패  '{node}'를 실행할 수 없습니다 (Node.js 20 이상 설치)"
            ));
        }
        Some(v) => {
            let major: u32 = v
                .trim_start_matches('v')
                .split('.')
                .next()
                .and_then(|m| m.parse().ok())
                .unwrap_or(0);
            if major < 20 {
                ok = false;
                out.push(format!("  node           실패  {v} (20 이상 필요)"));
            } else {
                out.push(format!("  node           OK    {v}"));
            }
            let probe =
                "import('playwright').then(({chromium}) => console.log(chromium.executablePath()))";
            match run(&["--input-type=module", "-e", probe]) {
                None => {
                    ok = false;
                    out.push(format!(
                        "  playwright     실패  {}에서 찾을 수 없습니다 (npm ci)",
                        dir.display()
                    ));
                }
                Some(exe) if Path::new(&exe).exists() => {
                    out.push("  chromium       OK    설치됨".into())
                }
                Some(_) => {
                    ok = false;
                    out.push(
                        "  chromium       실패  촬영용 브라우저가 없습니다. /browser 로 설치하세요"
                            .into(),
                    );
                }
            }
        }
    }
    match which("pdftohtml") {
        Some(_) => out.push("  PDF 도구       OK    poppler (기존 매뉴얼·견본 PDF 읽기)".into()),
        None => out.push(
            "  PDF 도구       없음  PDF 견본을 쓰려면 poppler 설치 (apt install poppler-utils / brew install poppler)"
                .into(),
        ),
    }
    if let Some(mb) = mwa_capture::mem::available_mb() {
        let need = cfg.capture.as_ref().map_or(2500, |c| c.min_start_mb);
        let mark = if mb >= need { "OK  " } else { "부족" };
        out.push(format!(
            "  메모리         {mark}  가용 {mb}MB (촬영 시작 기준 {need}MB)"
        ));
    }
    if let Some(p) = &cfg.product {
        let path = ws.join(&p.adapter);
        if path.is_file() {
            out.push(format!("  어댑터         OK    {}", p.adapter));
        } else {
            ok = false;
            out.push(format!(
                "  어댑터         실패  {} 이 없습니다 (docs/adapter.md)",
                path.display()
            ));
        }
    }
    (out, ok)
}

/// Downloads Playwright's Chromium next to the capture runner (one time, ~150MB).
fn install_browser(cfg: &Config, progress: impl Fn(String)) -> Result<Vec<String>> {
    let script = mwa_capture::shoot::default_script();
    let dir = script.parent().unwrap_or(Path::new("."));
    let npx = cfg
        .capture
        .as_ref()
        .map(|c| Path::new(&c.node).with_file_name("npx"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("npx"));
    // An installed mwa unpacks its capture script into the data folder: its Playwright package
    // comes first.
    if !dir.join("node_modules/playwright").is_dir() {
        progress(format!(
            "촬영기 준비 중 (Playwright 받기)… ({})",
            dir.display()
        ));
        let npm = npx.with_file_name("npm");
        let npm = if npm.is_file() {
            npm
        } else {
            PathBuf::from("npm")
        };
        let out = std::process::Command::new(npm)
            .args(["install", "--omit=dev", "--no-audit", "--no-fund"])
            .current_dir(dir)
            .output()
            .context("npm을 실행할 수 없습니다 (Node.js 설치 필요)")?;
        anyhow::ensure!(
            out.status.success(),
            "Playwright를 받지 못했습니다: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("")
        );
    }
    progress(format!(
        "촬영용 브라우저(Chromium) 설치 중… ({})",
        dir.display()
    ));
    let out = std::process::Command::new(npx)
        .args(["playwright", "install", "chromium"])
        .current_dir(dir)
        .output()
        .context("npx를 실행할 수 없습니다 (Node.js 설치 필요)")?;
    let tail = |b: &[u8]| {
        String::from_utf8_lossy(b)
            .lines()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(String::from)
            .collect::<Vec<_>>()
    };
    anyhow::ensure!(
        out.status.success(),
        "설치 실패: {}",
        tail(&out.stderr).join(" / ")
    );
    Ok(vec!["촬영용 브라우저를 설치했습니다".into()])
}

/// (key, label, group) of the current map's features, if a map exists.
fn map_features(ws: &Path) -> Vec<(String, String, Option<String>)> {
    std::fs::read_to_string(ws.join("map/product-map.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<mwa_productmap::map::ProductMap>(&t).ok())
        .map(|m| {
            m.features
                .into_iter()
                .map(|f| (f.key, f.label, f.group))
                .collect()
        })
        .unwrap_or_default()
}

fn reference_cmd(ws: &Path, action: RefCmd) -> Result<ExitCode> {
    use mwa_manual::reference;
    let owned = map_features(ws);
    let features: Vec<reference::FeatureRef> = owned
        .iter()
        .map(|(k, l, g)| (k.as_str(), l.as_str(), g.as_deref()))
        .collect();
    match action {
        RefCmd::Import { pdfs } => {
            if features.is_empty() {
                println!(
                    "제품 지도가 없어 기능 짝짓기는 건너뜁니다 (mwa map 뒤에 mwa ref list로 다시 짝짓습니다)"
                );
            }
            for pdf in pdfs {
                let d = reference::import(ws, &pdf, &features)?;
                println!(
                    "{}{}: {}쪽, 캡처 {}장 → {}",
                    d.title,
                    d.tag
                        .as_deref()
                        .map(|t| format!(" ({t})"))
                        .unwrap_or_default(),
                    d.pages,
                    d.images.len(),
                    d.feature.as_deref().unwrap_or("기능 미지정")
                );
            }
        }
        RefCmd::Describe { id } => {
            let cfg = load_config(ws)?;
            let d = open_dispatcher(ws, cfg)?;
            let rt = tokio::runtime::Handle::current();
            let doc = tokio::task::block_in_place(|| {
                rt.block_on(reference::describe(&d, ws, &id, &|l| println!("{l}")))
            })?;
            let capture = doc
                .images
                .iter()
                .filter(|i| i.scene.as_ref().is_some_and(|s| s.capture))
                .count();
            println!(
                "{}: 그림 {}장 설명 · 찍을 장면 {}장 · 사용량 정가 환산 ${:.3}",
                doc.title,
                doc.images.len(),
                capture,
                d.spent_usd()
            );
        }
        RefCmd::List => {
            for mut d in reference::list(ws) {
                if d.feature.is_none()
                    && let Some(k) = reference::match_feature(&d.title, d.tag.as_deref(), &features)
                {
                    d.feature = Some(k);
                    reference::save(ws, &d)?;
                }
                println!(
                    "{:<24} {:>3}쪽 캡처 {:>3}장  {}",
                    d.id,
                    d.pages,
                    d.images.len(),
                    d.feature.as_deref().unwrap_or("기능 미지정")
                );
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Clones or updates every `[[source]]` that has `git`; one result line per source.
fn sync_sources(cfg: &Config, progress: impl Fn(String)) -> Result<Vec<String>> {
    let git: Vec<_> = cfg.sources.iter().filter(|s| s.git.is_some()).collect();
    let ssh: Vec<_> = cfg.sources.iter().filter(|s| s.ssh.is_some()).collect();
    anyhow::ensure!(
        !git.is_empty() || !ssh.is_empty(),
        "Git이나 SSH 서버로 지정한 소스가 없습니다 (설정 → 코드 소스)"
    );
    let mut out = Vec::new();
    for s in ssh {
        let target = s.ssh.as_deref().expect("filtered");
        progress(format!("{}: {target} 에서 받는 중…", s.name));
        let pw = ssh_passwords(target, &s.jump);
        let kh = mwa_source::remote::known_hosts(&crate::projects::data_dir());
        let r = mwa_source::remote::Remote {
            target,
            jump: &s.jump,
            docker: s.docker.as_deref(),
            path: s.remote_path.as_deref().unwrap_or_default(),
            passwords: &pw,
            known_hosts: &kh,
        };
        out.push(match mwa_source::remote::sync(&r, &s.path) {
            Ok(msg) => format!("{}: {msg}", s.name),
            Err(e) => format!("{}: 실패 — {e}", s.name),
        });
    }
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

/// Saved SSH passwords for every hop on the way (only where no key works), as `(user@host,
/// password)`.
pub(crate) fn ssh_passwords(target: &str, jump: &[String]) -> Vec<(String, String)> {
    use mwa_capture::creds::CredStore;
    let Ok(store) = CredStore::default_path().and_then(|p| CredStore::open(&p)) else {
        return Vec::new();
    };
    jump.iter()
        .map(String::as_str)
        .chain([target])
        .filter_map(|hop| {
            let who = mwa_source::remote::who(hop);
            let pw = store.get(&ssh_cred_key(&who))?.password.clone();
            (!pw.is_empty()).then_some((who, pw))
        })
        .collect()
}

/// Where an SSH hop's password is kept in the user's credentials file.
pub(crate) fn ssh_cred_key(who: &str) -> String {
    let (user, host) = who.rsplit_once('@').unwrap_or(("", who));
    mwa_capture::creds::key(user, &format!("ssh://{host}"))
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
    let (capture_lines, capture_ok) = capture_env_lines(ws, &cfg);
    out.extend(capture_lines);
    ok &= capture_ok;

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
    use mwa_manual::shotplan;

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
            let needs_ctx = !wk.todo.is_empty() || !wk.reuse.is_empty();
            let ctx_for_plan = if needs_ctx {
                Some(planner_context(ws, &ctx, &w, f, sc, &job).await?)
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
            let mut records = detail::shoot(
                sc,
                &job,
                ws,
                &format!("detail/{}", f.key),
                &plans,
                runs,
                &mut progress,
            )
            .await?;
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
                let again = detail::shoot(
                    sc,
                    &job,
                    ws,
                    &format!("detail/{}", f.key),
                    &fixed,
                    runs,
                    &mut progress,
                )
                .await?;
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

/// Reproduces the old manual's scenes for one feature (D-031 ④) and judges every picture
/// against the old one. A scene counts as reproduced only when the judge says so; anything
/// the planner declares impossible, any failed step and any wrong screen is reported as such.
async fn capture_reference(
    ws: &Path,
    cfg: &Config,
    feature: &str,
    world: bool,
    only: Option<&str>,
    quick: bool,
    progress: impl Fn(String) + Send + Sync,
) -> Result<mwa_manual::reference::SceneShots> {
    use mwa_capture::detail::Sidecar;
    use mwa_capture::shoot::{Cookie, default_script};
    use mwa_manual::reference::{self, SceneShot, SceneShots};
    use mwa_manual::shotplan::{self, SceneTodo};

    let doc = reference::for_feature(ws, feature).with_context(|| {
        format!("{feature}에 짝지은 기존 매뉴얼이 없습니다 (설정 → 기존 매뉴얼)")
    })?;
    anyhow::ensure!(
        doc.images.iter().all(|i| i.scene.is_some()),
        "장면 설명을 먼저 만드세요 (기존 매뉴얼 탭 → 장면 설명 만들기)"
    );
    let ref_dir = reference::doc_dir(ws, &doc.id);
    let targets: Vec<(usize, &reference::RefImage)> = doc
        .images
        .iter()
        .enumerate()
        .filter(|(_, i)| i.shootable())
        .filter(|(_, i)| only.is_none_or(|o| o == i.file))
        .collect();
    anyhow::ensure!(
        !targets.is_empty(),
        "{}",
        match only {
            Some(o) => format!("찍을 장면이 아닌 그림입니다: {o}"),
            None => "기존 매뉴얼에 찍을 장면이 없습니다".into(),
        }
    );
    let id_of = |n: usize, img: &reference::RefImage| format!("p{}-{}", img.page, n + 1);

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
    let f = ctx
        .map
        .features
        .iter()
        .find(|f| f.key == feature)
        .with_context(|| format!("지도에 없는 기능 키입니다: {feature}"))?;
    let d = open_dispatcher(ws, cfg.clone())?;

    // Demo data shaped by the scenes (made once per feature).
    let mut w = mwa_manual::world::load(ws)?;
    if world && !w.features.contains_key(feature) {
        progress(format!(
            "{feature}: 데모 데이터 작성 중 (기존 매뉴얼 장면 {}개 반영)",
            targets.len()
        ));
        let mut project =
            mwa_productmap::graph::Project::new(&ctx.frontend, ctx.adapter.alias_prefix);
        let (input, fp) = world_input(ws, &ctx, &mut project, feature)?;
        let rep = mwa_manual::world::generate(&d, &mut w, &input, &fp, &|l| progress(l)).await?;
        mwa_manual::world::save(ws, &w)?;
        progress(format!(
            "{feature}: 데모 응답 {}개, 검사 위반 {}건",
            rep.responses,
            rep.errors.len()
        ));
    }
    let mocks = if world {
        world_mocks(ws, cfg)?
    } else {
        Vec::new()
    };
    let script = default_script();
    let sc = Sidecar {
        node: &cap.node,
        script: &script,
    };

    progress(format!("{} 로그인 1회 시도…", profile.user_id));
    let pair = login_profile(&spec.auth, &cap.profile, profile).await?;
    let cookie = Cookie::from_pair(&pair).context("세션 쿠키 형식이 올바르지 않습니다")?;
    let job = capture_job(cap, &spec, profile, cookie, mocks)?;
    let dir = format!("reference/{feature}");
    let result: Result<SceneShots> = async {
        // Quick: replay the plans that worked before, once each, without a model (no planning,
        // no judging). For checking crops and sizes fast; a normal run judges again.
        if quick {
            let saved = reference::load_scene_shots(ws, feature)
                .context("저장된 촬영 계획이 없습니다. 먼저 일반 다시 찍기를 하세요")?;
            let mut plans = Vec::new();
            for (n, img) in &targets {
                if let Some(mut p) = saved
                    .scenes
                    .iter()
                    .find(|s| s.image == img.file)
                    .and_then(|s| s.plan.clone())
                {
                    p.todo = id_of(*n, img);
                    if img.height > 0 {
                        p.ratio = Some(f64::from(img.width) / f64::from(img.height));
                    }
                    plans.push(p);
                }
            }
            anyhow::ensure!(
                !plans.is_empty(),
                "저장된 촬영 계획이 있는 장면이 없습니다. 먼저 일반 다시 찍기를 하세요"
            );
            progress(format!(
                "{feature}: 저장된 계획으로 {}장 빠르게 찍는 중 (대조 생략)",
                plans.len()
            ));
            let records =
                mwa_capture::detail::shoot(sc, &job, ws, &dir, &plans, 1, &progress).await?;
            let mut scenes = Vec::new();
            for (n, img) in &targets {
                let id = id_of(*n, img);
                let Some(r) = records.iter().find(|r| r.plan.todo == id) else {
                    continue;
                };
                let Some(mut shot) = saved.scenes.iter().find(|s| s.image == img.file).cloned()
                else {
                    continue;
                };
                shot.attempts.push(reference::attempt_of(r));
                if r.ok {
                    shot.file = r.file.clone();
                    if shot.status == "failed" {
                        shot.status = "different".into();
                    }
                    shot.reason_kind = None;
                    shot.reason = None;
                    shot.differences =
                        vec!["빠른 다시 찍기: 대조를 생략했습니다 (일반 다시 찍기로 판정)".into()];
                } else {
                    // A replay that fails keeps the earlier result: usually the data changed (a
                    // row the plan clicks is gone) and a normal run plans again.
                    shot.differences.insert(
                        0,
                        format!(
                            "빠른 다시 찍기 실패 — 이전 그림을 그대로 둡니다. 일반 다시 찍기로 계획을 새로 짜세요: {}",
                            r.error.clone().unwrap_or_default()
                        ),
                    );
                }
                scenes.push(shot);
            }
            let shots = reference::merge_scene_shots(
                ws,
                SceneShots {
                    feature: feature.to_string(),
                    doc: doc.id.clone(),
                    taken_at: job.fixed_time / 1000,
                    scenes,
                },
            );
            reference::save_scene_shots(ws, &shots)?;
            progress(format!(
                "{feature}: 빠른 다시 찍기 {}장 중 {}장 성공 · 모델 호출 없음",
                records.len(),
                records.iter().filter(|r| r.ok).count()
            ));
            return Ok(shots);
        }
        let pctx = planner_context(ws, &ctx, &w, f, sc, &job).await?;
        // A person's instruction for a scene goes with it to the planner (D-053).
        let saved = reference::load_scene_shots(ws, feature);
        let instruction = |file: &str| {
            saved
                .as_ref()?
                .scenes
                .iter()
                .find(|s| s.image == file)?
                .instruction
                .clone()
        };
        let todos: Vec<SceneTodo> = targets
            .iter()
            .map(|(n, img)| {
                let s = img.scene.as_ref().expect("filtered");
                let what = format!("{} — {}", s.screen, s.state);
                SceneTodo {
                    id: id_of(*n, img),
                    what: match instruction(&img.file) {
                        Some(i) => format!("{what} [사람의 지시: {i}]"),
                        None => what,
                    },
                    how: s.how.clone(),
                    image: Some(ref_dir.join(&img.file)),
                }
            })
            .collect();
        // Same proportions as the old picture.
        let ratio_of = |id: &str| {
            targets
                .iter()
                .find(|(n, img)| id_of(*n, img) == id)
                .filter(|(_, img)| img.height > 0)
                .map(|(_, img)| f64::from(img.width) / f64::from(img.height))
        };
        let ScenesShot {
            records,
            infeasible,
            mut tries,
        } = shoot_scenes(
            &d, &pctx, sc, &job, ws, &dir, &todos, &ratio_of, feature, &progress,
        )
        .await?;

        let mut scenes = Vec::new();
        for (n, img) in &targets {
            let id = id_of(*n, img);
            let s = img.scene.as_ref().expect("filtered");
            let what = format!("{} — {}", s.screen, s.state);
            let mut shot = SceneShot {
                image: img.file.clone(),
                page: img.page,
                heading: img.heading.clone(),
                what: what.clone(),
                status: "failed".into(),
                reason_kind: None,
                reason: None,
                differences: Vec::new(),
                changes: Vec::new(),
                file: None,
                data: None,
                plan: None,
                attempts: tries.remove(&id).unwrap_or_default(),
                blocked: Vec::new(),
                mocked: Vec::new(),
                uploaded: None,
                choice: None,
                instruction: None,
            };
            if let Some(why) = infeasible.get(&id) {
                shot.reason_kind = Some(why.kind.clone());
                shot.reason = Some(why.reason.clone());
                scenes.push(shot);
                continue;
            }
            let Some(r) = records.iter().find(|r| r.plan.todo == id) else {
                shot.reason_kind = Some("error".into());
                shot.reason = Some("계획이 만들어지지 않았습니다".into());
                scenes.push(shot);
                continue;
            };
            shot.plan = Some(r.plan.clone());
            shot.data = r.data.clone();
            shot.mocked = r.mocked.clone();
            shot.blocked = r
                .blocked
                .iter()
                .map(|b| format!("{} {}", b.method, b.url))
                .collect();
            if !r.ok {
                let err = r.error.clone().unwrap_or_default();
                shot.reason_kind = Some(
                    if err.contains("일치 0개") || err.contains("Timeout") {
                        "not_found"
                    } else {
                        "error"
                    }
                    .into(),
                );
                shot.reason = Some(err);
                scenes.push(shot);
                continue;
            }
            if !r.blocked.is_empty() {
                shot.reason_kind = Some("needs_write".into());
                shot.reason = Some(format!(
                    "데모 응답이 없는 쓰기 요청을 막았습니다: {}",
                    r.blocked
                        .iter()
                        .map(|b| format!("{} {}", b.method, b.url))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            shot.file = r.file.clone();
            if r.region_missed {
                shot.differences.push(format!(
                    "기존 그림과 같은 영역({})을 찾지 못해 잘라내지 않고 찍었습니다",
                    r.plan.region.join(", ")
                ));
            }
            let new_path = ws.join(r.file.as_deref().unwrap_or_default());
            progress(format!("{feature}: 대조 {id}"));
            match shotplan::judge(&d, &ref_dir.join(&img.file), &new_path, &what).await {
                Ok(v) => {
                    shot.differences.extend(v.differences);
                    shot.changes = v.changes;
                    shot.status = match v.verdict.as_str() {
                        "same" if shot.reason_kind.is_none() => "reproduced",
                        "same" | "different" => "different",
                        _ => {
                            shot.reason_kind
                                .get_or_insert_with(|| "wrong_screen".into());
                            "failed"
                        }
                    }
                    .into();
                }
                Err(e) => {
                    shot.reason_kind = Some("error".into());
                    shot.reason = Some(format!("대조 실패: {e}"));
                }
            }
            scenes.push(shot);
        }
        let shots = reference::merge_scene_shots(
            ws,
            SceneShots {
                feature: feature.to_string(),
                doc: doc.id.clone(),
                taken_at: job.fixed_time / 1000,
                scenes,
            },
        );
        reference::save_scene_shots(ws, &shots)?;
        progress(format!(
            "{feature}: {} · 사용량 정가 환산 ${:.3}",
            reference::summarize(&shots),
            d.spent_usd()
        ));
        Ok(shots)
    }
    .await;
    if let Err(e) = mwa_capture::login::logout(&spec.auth, &profile.base_url, &pair).await {
        progress(format!("로그아웃 실패: {e} (세션은 30분 뒤 만료)"));
    }
    result
}

/// Shoots the picture slots of a written manual (D-046): every slot without a picture, or the
/// ones named by `only` (block ids, comma-separated). The pictures (or why a slot could not be shot) go into the document.
async fn capture_doc(
    ws: &Path,
    cfg: &Config,
    feature: &str,
    only: Option<&str>,
    progress: impl Fn(String) + Send + Sync,
) -> Result<Vec<String>> {
    use mwa_capture::detail::Sidecar;
    use mwa_capture::shoot::{Cookie, default_script};
    use mwa_manual::{document, reference, shotplan::SceneTodo};

    let doc = document::load(ws, feature)
        .with_context(|| format!("{feature}의 매뉴얼 문서가 없습니다 (③에서 먼저 쓰세요)"))?;
    let slots: Vec<document::Slot> = document::picture_slots(&doc.html)
        .into_iter()
        .filter(|s| only.map_or(s.shot.is_none(), |o| o.split(',').any(|b| b == s.bid)))
        .collect();
    anyhow::ensure!(!slots.is_empty(), "찍을 그림 자리가 없습니다");

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
    let f = ctx
        .map
        .features
        .iter()
        .find(|f| f.key == feature)
        .with_context(|| format!("지도에 없는 기능 키입니다: {feature}"))?;
    let d = open_dispatcher(ws, cfg.clone())?;
    // Demo data is used when the feature has some (made from ② for the old manual's scenes).
    let w = mwa_manual::world::load(ws)?;
    let mocks = if w.features.contains_key(feature) {
        world_mocks(ws, cfg)?
    } else {
        Vec::new()
    };
    let script = default_script();
    let sc = Sidecar {
        node: &cap.node,
        script: &script,
    };

    progress(format!("{} 로그인 1회 시도…", profile.user_id));
    let pair = login_profile(&spec.auth, &cap.profile, profile).await?;
    let cookie = Cookie::from_pair(&pair).context("세션 쿠키 형식이 올바르지 않습니다")?;
    let job = capture_job(cap, &spec, profile, cookie, mocks)?;
    let dir = format!("doc/{feature}");
    let result: Result<Vec<String>> = async {
        let pctx = planner_context(ws, &ctx, &w, f, sc, &job).await?;
        let todos: Vec<SceneTodo> = slots
            .iter()
            .map(|s| SceneTodo {
                id: s.bid.clone(),
                what: s.want.clone(),
                how: String::new(),
                image: None,
            })
            .collect();
        let shot = shoot_scenes(
            &d,
            &pctx,
            sc,
            &job,
            ws,
            &dir,
            &todos,
            &|_| None,
            feature,
            &progress,
        )
        .await?;
        let results: Vec<(String, std::result::Result<String, String>)> = slots
            .iter()
            .map(|s| {
                let r = if let Some(why) = shot.infeasible.get(&s.bid) {
                    Err(format!(
                        "{}: {}",
                        reference::reason_label(&why.kind),
                        why.reason
                    ))
                } else {
                    match shot.records.iter().find(|r| r.plan.todo == s.bid) {
                        Some(r) if r.ok => {
                            if let Some(file) = &r.file {
                                // A drawer is often taller than what it shows.
                                let _ = mwa_capture::diff::trim_bottom(&ws.join(file), 32);
                            }
                            r.file.clone().ok_or_else(|| "그림 파일이 없습니다".into())
                        }
                        Some(r) => Err(r.error.clone().unwrap_or_else(|| "찍지 못했습니다".into())),
                        None => Err("촬영 계획이 만들어지지 않았습니다".into()),
                    }
                };
                (s.bid.clone(), r)
            })
            .collect();
        let doc = document::fill_slots(ws, feature, &results)?;
        let ok = results.iter().filter(|(_, r)| r.is_ok()).count();
        let mut lines = vec![format!(
            "{}: 그림 자리 {}개 중 {ok}개 찍음 · 문서 버전 {} · 사용량 정가 환산 ${:.3}",
            doc.title,
            results.len(),
            doc.version,
            d.spent_usd()
        )];
        for (s, (_, r)) in slots.iter().zip(&results) {
            if let Err(why) = r {
                lines.push(format!("✗ {} — {why}", s.want));
            }
        }
        Ok(lines)
    }
    .await;
    if let Err(e) = mwa_capture::login::logout(&spec.auth, &profile.base_url, &pair).await {
        progress(format!("로그아웃 실패: {e} (세션은 30분 뒤 만료)"));
    }
    result
}

/// Scenes planned and shot: the records, scenes that cannot be reached here (with why), and
/// every try per scene.
struct ScenesShot {
    records: Vec<mwa_capture::detail::DetailRecord>,
    infeasible: std::collections::HashMap<String, mwa_manual::shotplan::Infeasible>,
    tries: std::collections::HashMap<String, Vec<mwa_manual::reference::Attempt>>,
}

/// Plans the scenes (8 per model call), shoots them, and fixes the plans of failed ones twice.
#[allow(clippy::too_many_arguments)]
async fn shoot_scenes(
    d: &Dispatcher,
    pctx: &mwa_manual::shotplan::Context<'_>,
    sc: mwa_capture::detail::Sidecar<'_>,
    job: &mwa_capture::shoot::Job,
    ws: &Path,
    dir: &str,
    todos: &[mwa_manual::shotplan::SceneTodo],
    ratio_of: &(dyn Fn(&str) -> Option<f64> + Sync),
    feature: &str,
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<ScenesShot> {
    use mwa_capture::detail;
    use mwa_manual::{reference, shotplan};
    let mut planned = Vec::new();
    for (b, chunk) in todos.chunks(8).enumerate() {
        progress(format!(
            "{feature}: 장면 촬영 계획 {}/{}",
            b + 1,
            todos.len().div_ceil(8)
        ));
        planned.extend(shotplan::plan_scenes(d, pctx, chunk).await?);
    }
    let mut infeasible = std::collections::HashMap::new();
    let mut plans = Vec::new();
    for (mut p, why) in planned {
        p.ratio = ratio_of(&p.todo);
        match why {
            Some(w) => {
                infeasible.insert(p.todo.clone(), w);
            }
            None => plans.push(p),
        }
    }
    // Every try per scene, shown to the user as "어디서 안 됐나".
    let mut tries: std::collections::HashMap<String, Vec<reference::Attempt>> =
        std::collections::HashMap::new();
    let mut records = if plans.is_empty() {
        Vec::new()
    } else {
        detail::shoot(sc, job, ws, dir, &plans, 2, progress).await?
    };
    for round in 1..=2 {
        let failed: Vec<usize> = (0..records.len()).filter(|&i| !records[i].ok).collect();
        if failed.is_empty() {
            break;
        }
        progress(format!(
            "{feature}: 실패 {}개 계획 수정 ({round}/2)",
            failed.len()
        ));
        let mut fixed = Vec::new();
        for &i in &failed {
            let r = &records[i];
            let err = r.error.clone().unwrap_or_default();
            fixed.push(shotplan::repair(d, pctx, &r.plan, &err, r.aria.as_deref()).await?);
        }
        let again = detail::shoot(sc, job, ws, dir, &fixed, 2, progress).await?;
        for (i, r) in failed.into_iter().zip(again) {
            tries
                .entry(records[i].plan.todo.clone())
                .or_default()
                .push(reference::attempt_of(&records[i]));
            records[i] = r;
        }
    }
    for r in &records {
        tries
            .entry(r.plan.todo.clone())
            .or_default()
            .push(reference::attempt_of(r));
    }

    Ok(ScenesShot {
        records,
        infeasible,
        tries,
    })
}

/// What the shot planner sees: real accessibility trees of the feature's static pages, the
/// demo data's ids, the code's labels and which writes are answered by demo data.
async fn planner_context<'a>(
    ws: &Path,
    ctx: &ProductCtx,
    w: &mwa_manual::world::World,
    f: &'a mwa_productmap::map::Feature,
    sc: mwa_capture::detail::Sidecar<'_>,
    job: &mwa_capture::shoot::Job,
) -> Result<mwa_manual::shotplan::Context<'a>> {
    let routes: Vec<String> = f.screens.iter().map(|s| s.route.clone()).collect();
    let starts: Vec<String> = routes
        .iter()
        .filter(|r| !r.contains('{'))
        .cloned()
        .collect();
    let tmp = ws.join(format!(".mwa/shots/probe-{}", job.fixed_time));
    let evs = mwa_capture::detail::probe(sc, job, &tmp, &starts).await?;
    let _ = std::fs::remove_dir_all(&tmp);
    let evs_data: Vec<mwa_capture::shoot::ApiSample> =
        evs.iter().flat_map(|e| e.api_data.clone()).collect();
    let snapshots = evs
        .into_iter()
        .filter_map(|e| Some((e.id, e.aria?)))
        .collect();
    let slice =
        mwa_manual::pipeline::build_slice(&ctx.map, f, &gen_options(ws, ctx, &f.key, None, None))?;
    let mut labels: Vec<String> = mwa_manual::labels::extract(&slice)
        .into_iter()
        .map(|l| l.text)
        .collect();
    labels.sort();
    labels.dedup();
    labels.truncate(500);
    // With demo data the planner sees the demo records; otherwise what the real server
    // returned while probing (never mix the two: names would not exist on screen).
    let data: Vec<String> = if job.mocks.is_empty() {
        let mut seen = std::collections::BTreeSet::new();
        evs_data
            .into_iter()
            .filter(|a| seen.insert(a.path.clone()))
            .map(|a| format!("(실제 서버) GET {}: {}", a.path, a.body))
            .collect()
    } else {
        w.responses
            .iter()
            .filter(|r| r.feature == f.key && r.method == "GET")
            .map(|r| {
                let body: String = r.body.to_string().chars().take(1200).collect();
                format!("(데모 데이터) {} {:?}: {body}", r.path, r.params)
            })
            .collect()
    };
    let mocked_writes = job
        .mocks
        .iter()
        .filter(|m| m.method != "GET")
        .map(|m| format!("{} {}", m.method, m.path))
        .collect();
    Ok(mwa_manual::shotplan::Context {
        feature_label: &f.label,
        routes,
        snapshots,
        data: data.join("\n"),
        labels,
        mocked_writes,
    })
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
    // Scenes of the old manual (if described) shape the demo data.
    let scenes = mwa_manual::reference::for_feature(ws, key)
        .map(|d| {
            d.images
                .iter()
                .filter_map(|i| i.scene.as_ref().filter(|s| s.capture))
                .map(|s| format!("- {} — {}", s.screen, s.state))
                .collect()
        })
        .unwrap_or_default();
    let input = mwa_manual::world::FeatureInput {
        key: key.to_string(),
        label: f.label.clone(),
        shapes,
        code,
        hints,
        scenes,
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
    template: Option<mwa_manual::reference::Example>,
    compare: Option<mwa_manual::reference::Example>,
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
        compare_with: compare.map(|c| (c.name, c.text)),
        shots,
    }
}

/// ③ "코드 읽기": the feature's code facts, kept with the commits they were read from (so a
/// later change of the code is noticed).
async fn generate_manual(
    ws: &Path,
    cfg: &Config,
    feature: &str,
    progress: impl Fn(String) + Send + Sync,
) -> Result<mwa_manual::model::ManualRun> {
    let ctx = product_ctx(ws, cfg)?;
    let d = open_dispatcher(ws, cfg.clone())?;
    let opts = gen_options(ws, &ctx, feature, None, None);
    let run = mwa_manual::pipeline::read_code(&d, &ctx.map, opts, &progress).await?;
    let sources = snapshots(cfg)
        .into_iter()
        .filter_map(|(name, s)| {
            let s = s.ok()?;
            Some(mwa_manual::versions::SourceAt {
                name,
                preview: s.is_preview(),
                sha: s.sha,
                branch: s.branch,
            })
        })
        .collect();
    let template = std::fs::read_to_string(ws.join(mwa_manual::pipeline::TEMPLATE_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["source"].as_str().map(String::from))
        .unwrap_or_default();
    mwa_manual::versions::save(ws, &run, sources, &template)?;
    Ok(run)
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
    mwa_backends::find_command(bin)
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
