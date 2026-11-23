//! Connects the TUI to workspace config, the dispatcher and the product map.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use mwa_core::Dispatcher;
use mwa_host::{Header, Host, OnDelta, SourceBadge, Status, Who};
use mwa_productmap::map::ProductMap;
use mwa_protocol::{Config, Role};

/// `{product}` is replaced with `[product] name` (or a neutral phrase).
const CHAT_SYSTEM: &str = "당신은 mwa(Manual Write Anything) 안의 도우미입니다. mwa는 {product}의 \
한국어 사용자 매뉴얼을 코드와 실제 화면에서 만들고 갱신하는 도구입니다. 합니다체로 간결하게 답하세요. \
아래 제품 지도 정보에 근거해 답하고, 근거가 없는 내용은 모른다고 말하세요. \
아직 구현되지 않은 기능(대화로 매뉴얼 편집, PDF·Confluence 내보내기, 자동 갱신)을 요청받으면 아직 구현 전이라고 알려 주세요. \
지금 쓸 수 있는 명령은 /map(제품 지도 생성), /shot(메뉴 화면 촬영), /gen(선택한 기능의 매뉴얼 생성), \
/doctor(점검), /auth(로그인 프로파일), /help 입니다.";

pub struct CliHost {
    ws: PathBuf,
    cfg: Config,
    dispatcher: Arc<Dispatcher>,
    /// One capture at a time: each one logs in, and parallel runs would double the load.
    capturing: std::sync::atomic::AtomicBool,
    /// One manual generation at a time (model quota and cost).
    generating: std::sync::atomic::AtomicBool,
}

impl CliHost {
    /// The file behind a picture URL the web page shows: a capture, an upload or an old manual
    /// picture.
    fn local_image(&self, url: &str) -> Option<PathBuf> {
        let path = url.split(['?', '#']).next()?;
        if let Some(rel) = path.strip_prefix("/api/shots/file/") {
            return mwa_capture::menus::shot_file(&self.ws, &percent_decode(rel)?);
        }
        if let Some(rest) = path.strip_prefix("/api/docs/") {
            let (feature, file) = rest.split_once("/assets/")?;
            return mwa_manual::document::asset(
                &self.ws,
                &percent_decode(feature)?,
                &percent_decode(file)?,
            );
        }
        let rest = path.strip_prefix("/api/references/")?;
        let (id, file) = rest.split_once("/file/")?;
        mwa_manual::reference::file(&self.ws, &percent_decode(id)?, &percent_decode(file)?)
    }

    pub fn open(ws: &Path) -> Result<Self> {
        let (cfg, _warnings) = crate::load_config_quiet(ws)?;
        let dispatcher = Arc::new(crate::open_dispatcher(ws, cfg.clone())?);
        Ok(Self {
            ws: ws.to_path_buf(),
            cfg,
            dispatcher,
            capturing: std::sync::atomic::AtomicBool::new(false),
            generating: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn map_overview(&self) -> Option<String> {
        let map = self.load_map()?;
        let mut out = format!("제품 지도: 기능 {}개\n", map.features.len());
        for f in &map.features {
            out.push_str(&format!(
                "- {} > {} ({})\n",
                f.group.as_deref().unwrap_or(&f.license_menu),
                f.label,
                f.href
            ));
        }
        Some(out)
    }
}

#[async_trait]
impl Host for CliHost {
    fn header(&self) -> Header {
        let ws = std::fs::canonicalize(&self.ws).unwrap_or_else(|_| self.ws.clone());
        Header {
            workspace: ws.display().to_string(),
            sources: crate::snapshots(&self.cfg)
                .into_iter()
                .filter_map(|(name, s)| {
                    let s = s.ok()?;
                    Some(SourceBadge {
                        name,
                        preview: s.is_preview(),
                        sha: s.sha,
                    })
                })
                .collect(),
        }
    }

    fn load_map(&self) -> Option<ProductMap> {
        let text = std::fs::read_to_string(self.ws.join("map/product-map.json")).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn status(&self) -> Status {
        let model = self
            .dispatcher
            .role_model(Role::Chat)
            .map(|(b, m)| format!("{b}·{m}"))
            .unwrap_or_else(|| "chat 역할 없음".into());
        Status {
            model,
            spent_usd: self.dispatcher.spent_usd(),
            budget_usd: self.dispatcher.budget_usd(),
            quota: self.dispatcher.last_quota(),
        }
    }

    async fn chat(
        &self,
        transcript: Vec<(Who, String)>,
        context: Option<String>,
        on_delta: OnDelta,
    ) -> Result<(), String> {
        let product = self
            .cfg
            .product
            .as_ref()
            .and_then(|p| p.name.clone())
            .unwrap_or_else(|| "연결된 제품".into());
        let mut system = CHAT_SYSTEM.replace("{product}", &product);
        if let Some(o) = self.map_overview() {
            system.push_str("\n\n");
            system.push_str(&o);
        }
        if let Some(c) = context {
            system.push_str("\n\n");
            system.push_str(&c);
        }
        let mut prompt = String::from("[대화]\n");
        for (who, text) in &transcript {
            let label = if *who == Who::User {
                "사용자"
            } else {
                "도우미"
            };
            prompt.push_str(&format!("{label}: {text}\n"));
        }
        prompt.push_str("\n위 대화의 마지막 사용자 메시지에 답하세요.");
        self.dispatcher
            .call_streaming(Role::Chat, &system, &prompt, &*on_delta)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn build_map(&self) -> Result<(ProductMap, Vec<String>), String> {
        let (map, warnings, path) = crate::build_map(&self.ws, &self.cfg, None, false)
            .await
            .map_err(|e| format!("{e:#}"))?;
        let mut lines: Vec<String> = warnings.into_iter().map(|w| format!("경고: {w}")).collect();
        lines.extend(crate::map_summary_lines(&map));
        lines.push(format!("저장: {}", path.display()));
        Ok((map, lines))
    }

    async fn check(&self, kind: &str, value: &str, extra: &str) -> mwa_host::Check {
        crate::setup::check(&self.ws, kind, value, extra).await
    }

    fn references(&self) -> Result<serde_json::Value, String> {
        use mwa_manual::reference;
        let owned = crate::map_features(&self.ws);
        let features: Vec<reference::FeatureRef> = owned
            .iter()
            .map(|(k, l, g)| (k.as_str(), l.as_str(), g.as_deref()))
            .collect();
        let mut out = Vec::new();
        for mut d in reference::list(&self.ws) {
            // Match documents imported before the product map existed, unless a person chose.
            if d.feature.is_none()
                && !d.ignored
                && !d.chosen
                && let Some(k) = reference::match_feature(&d.title, d.tag.as_deref(), &features)
            {
                d.feature = Some(k);
                let _ = reference::save(&self.ws, &d);
            }
            out.push(serde_json::json!({
                "id": d.id,
                "title": d.title,
                "tag": d.tag,
                "feature": d.feature,
                "ignored": d.ignored,
                "pages": d.pages,
                "images": d.images.len(),
                "described": d.images.iter().filter(|i| i.scene.is_some()).count(),
                "source": d.source,
                "imported_at": d.imported_at,
            }));
        }
        Ok(serde_json::json!({
            "items": out,
            "features": owned.iter().map(|(k, l, g)| serde_json::json!({"key": k, "label": l, "group": g})).collect::<Vec<_>>(),
        }))
    }

    fn reference(&self, id: &str) -> Option<serde_json::Value> {
        serde_json::to_value(mwa_manual::reference::load(&self.ws, id)?).ok()
    }

    async fn import_references(&self, paths: Vec<String>) -> Result<Vec<String>, String> {
        let ws = self.ws.clone();
        tokio::task::spawn_blocking(move || {
            use mwa_manual::reference;
            let owned = crate::map_features(&ws);
            let features: Vec<reference::FeatureRef> = owned
                .iter()
                .map(|(k, l, g)| (k.as_str(), l.as_str(), g.as_deref()))
                .collect();
            // A folder imports every PDF in it.
            let mut files: Vec<std::path::PathBuf> = Vec::new();
            for p in &paths {
                let p = std::path::PathBuf::from(p);
                if p.is_dir() {
                    let mut pdfs: Vec<std::path::PathBuf> = std::fs::read_dir(&p)
                        .map(|rd| {
                            rd.filter_map(Result::ok)
                                .map(|e| e.path())
                                .filter(|f| {
                                    f.extension().is_some_and(|x| {
                                        ["pdf", "md", "markdown"]
                                            .iter()
                                            .any(|e| x.eq_ignore_ascii_case(e))
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    pdfs.sort();
                    files.extend(pdfs);
                } else {
                    files.push(p);
                }
            }
            files
                .iter()
                .map(|p| match reference::import(&ws, p, &features) {
                    Ok(d) => format!(
                        "{}: {}쪽, 캡처 {}장 → {}",
                        d.title,
                        d.pages,
                        d.images.len(),
                        d.feature.as_deref().unwrap_or("기능 미지정")
                    ),
                    Err(e) => format!("{}: 실패 — {e}", p.display()),
                })
                .collect()
        })
        .await
        .map_err(|e| e.to_string())
    }

    fn set_reference(
        &self,
        id: &str,
        feature: Option<String>,
        ignored: bool,
    ) -> Result<(), String> {
        let mut d = mwa_manual::reference::load(&self.ws, id)
            .ok_or_else(|| format!("기존 매뉴얼이 없습니다: {id}"))?;
        d.feature = feature.filter(|f| !f.is_empty());
        d.ignored = ignored;
        mwa_manual::reference::save(&self.ws, &d).map_err(|e| e.to_string())
    }

    fn reference_file(&self, id: &str, rel: &str) -> Option<std::path::PathBuf> {
        mwa_manual::reference::file(&self.ws, id, rel)
    }

    fn reference_page(&self, id: &str, page: u32) -> Result<std::path::PathBuf, String> {
        mwa_manual::reference::page(&self.ws, id, page).map_err(|e| e.to_string())
    }

    async fn describe_reference(&self, id: &str, progress: OnDelta) -> Result<Vec<String>, String> {
        let spent = self.dispatcher.spent_usd();
        let doc =
            mwa_manual::reference::describe(&self.dispatcher, &self.ws, id, &|l| progress(&l))
                .await
                .map_err(|e| format!("{e:#}"))?;
        let capture = doc
            .images
            .iter()
            .filter(|i| i.scene.as_ref().is_some_and(|s| s.capture))
            .count();
        Ok(vec![format!(
            "{}: 그림 {}장 설명 · 찍을 장면 {}장 · 사용량 정가 환산 ${:.3}",
            doc.title,
            doc.images.len(),
            capture,
            self.dispatcher.spent_usd() - spent
        )])
    }

    fn document(&self, feature: &str) -> Option<serde_json::Value> {
        let doc = mwa_manual::document::migrate(&self.ws, feature)
            .ok()
            .flatten()?;
        serde_json::to_value(doc).ok()
    }

    fn save_document(
        &self,
        feature: &str,
        html: &str,
        changes: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let changes: Vec<mwa_manual::provenance::Change> =
            serde_json::from_value(changes).map_err(|e| format!("변경 기록 형식: {e}"))?;
        let doc = mwa_manual::document::save(&self.ws, feature, html, &changes)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(doc).unwrap_or_default())
    }

    fn save_document_asset(
        &self,
        feature: &str,
        name: &str,
        data: &[u8],
    ) -> Result<String, String> {
        mwa_manual::document::save_asset(&self.ws, feature, name, data).map_err(|e| e.to_string())
    }

    fn document_asset(&self, feature: &str, file: &str) -> Option<PathBuf> {
        mwa_manual::document::asset(&self.ws, feature, file)
    }

    async fn import_document(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        let spent = self.dispatcher.spent_usd();
        let doc =
            mwa_manual::document::import(&self.dispatcher, &self.ws, feature, &|l| progress(&l))
                .await
                .map_err(|e| format!("{e:#}"))?;
        Ok(vec![format!(
            "{}: 기존 매뉴얼을 문서로 옮겼습니다 · 사용량 정가 환산 ${:.3}",
            doc.title,
            self.dispatcher.spent_usd() - spent
        )])
    }

    fn document_versions(&self, feature: &str) -> serde_json::Value {
        let v: Vec<serde_json::Value> = mwa_manual::document::versions(&self.ws, feature)
            .into_iter()
            .map(|(version, updated_at)| serde_json::json!({ "version": version, "updated_at": updated_at }))
            .collect();
        serde_json::Value::Array(v)
    }

    fn document_version(&self, feature: &str, n: u32) -> Option<serde_json::Value> {
        serde_json::to_value(mwa_manual::document::version(&self.ws, feature, n)?).ok()
    }

    fn proposals(&self, feature: &str) -> serde_json::Value {
        serde_json::to_value(mwa_manual::document::load_proposals(&self.ws, feature))
            .unwrap_or_default()
    }

    fn resolve_proposal(&self, feature: &str, id: &str) -> Result<serde_json::Value, String> {
        let p = mwa_manual::document::resolve_proposal(&self.ws, feature, id)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(p).unwrap_or_default())
    }

    async fn propose_document(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        let run = mwa_manual::pipeline::load_run(&self.ws, feature).ok_or_else(|| {
            "코드에서 뽑은 사실이 없습니다. 먼저 생성본을 만드세요 (/gen)".to_string()
        })?;
        progress(&format!(
            "코드 사실 {}개와 문서를 비교하는 중…",
            run.facts.len()
        ));
        let spent = self.dispatcher.spent_usd();
        let p = mwa_manual::document::propose(&self.dispatcher, &self.ws, feature, &run.facts)
            .await
            .map_err(|e| format!("{e:#}"))?;
        Ok(vec![format!(
            "갱신 제안 {}개 (매뉴얼 탭에서 수락·거절) · 사용량 정가 환산 ${:.3}",
            p.items.len(),
            self.dispatcher.spent_usd() - spent
        )])
    }

    fn set_scene_upload(
        &self,
        feature: &str,
        image: &str,
        upload: Option<(&str, &[u8])>,
    ) -> Result<serde_json::Value, String> {
        let file = match upload {
            Some((name, data)) => Some(
                mwa_manual::document::save_asset(&self.ws, feature, name, data)
                    .map_err(|e| e.to_string())?,
            ),
            None => None,
        };
        let shots = mwa_manual::reference::set_scene_upload(&self.ws, feature, image, file)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(shots).unwrap_or_default())
    }

    fn set_scene_choice(
        &self,
        feature: &str,
        image: &str,
        choice: Option<String>,
    ) -> Result<serde_json::Value, String> {
        let shots = mwa_manual::reference::set_scene_choice(&self.ws, feature, image, choice)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(shots).unwrap_or_default())
    }

    fn set_scene_instruction(
        &self,
        feature: &str,
        image: &str,
        instruction: Option<String>,
    ) -> Result<serde_json::Value, String> {
        let shots =
            mwa_manual::reference::set_scene_instruction(&self.ws, feature, image, instruction)
                .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(shots).unwrap_or_default())
    }

    fn link_reference(&self, feature: &str, id: &str) -> Result<(), String> {
        // An empty id unpairs the feature.
        if id.is_empty() {
            return mwa_manual::reference::unlink(&self.ws, feature).map_err(|e| e.to_string());
        }
        mwa_manual::reference::link(&self.ws, feature, id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn upload_reference(
        &self,
        feature: &str,
        name: &str,
        data: Vec<u8>,
    ) -> Result<serde_json::Value, String> {
        let (ws, feature, name) = (self.ws.clone(), feature.to_string(), name.to_string());
        tokio::task::spawn_blocking(move || {
            let owned = crate::map_features(&ws);
            let features: Vec<mwa_manual::reference::FeatureRef> = owned
                .iter()
                .map(|(k, l, g)| (k.as_str(), l.as_str(), g.as_deref()))
                .collect();
            mwa_manual::reference::import_upload(&ws, &feature, &name, &data, &features)
                .map(|d| serde_json::to_value(d).unwrap_or_default())
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    fn restore_document(&self, feature: &str, n: u32) -> Result<serde_json::Value, String> {
        let doc = mwa_manual::document::restore(&self.ws, feature, n).map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(doc).unwrap_or_default())
    }

    async fn make_document(&self, feature: &str, progress: OnDelta) -> Result<Vec<String>, String> {
        // D-043: the content comes from the code; an old manual is the model for its form.
        // "<feature>?fresh": start over without the paired old manual (D-047).
        let (feature, fresh) = feature
            .strip_suffix("?fresh")
            .map_or((feature, false), |f| (f, true));
        let run = mwa_manual::pipeline::load_run(&self.ws, feature).ok_or_else(|| {
            "코드를 아직 읽지 않았습니다. ③에서 \"코드 읽기\"를 먼저 하세요".to_string()
        })?;
        let spent = self.dispatcher.spent_usd();
        let progress: Arc<dyn Fn(&str) + Send + Sync> = Arc::from(progress);
        let relay = |p: &Arc<dyn Fn(&str) + Send + Sync>| -> OnDelta {
            let p = p.clone();
            Box::new(move |t: &str| p(t))
        };
        let mut lines = Vec::new();
        let paired = mwa_manual::reference::for_feature(&self.ws, feature);
        let doc = if let Some(rdoc) = paired.filter(|_| !fresh) {
            // The old manual as it was is kept as a version: rewrites start from it and the new
            // manual is compared with it. Imported again only for another old manual.
            let imported = mwa_manual::document::load(&self.ws, feature)
                .and_then(|d| {
                    let b = d.base?;
                    if b == d.version {
                        Some(d)
                    } else {
                        mwa_manual::document::version(&self.ws, feature, b)
                    }
                })
                .is_some_and(|b| b.reference.as_deref() == Some(rdoc.id.as_str()));
            if !imported {
                lines.extend(self.import_document(feature, relay(&progress)).await?);
            }
            mwa_manual::rewrite::from_code(&self.dispatcher, &self.ws, feature, &run.facts, &|l| {
                progress(&l)
            })
            .await
            .map_err(|e| format!("{e:#}"))?
        } else {
            // The settings' sample manual is learned once, as the format of the very first manuals.
            if let Some(sample) = crate::first_template(&self.ws, &self.cfg) {
                progress("견본 매뉴얼에서 양식을 배우는 중");
                let ex = mwa_manual::reference::load_example(&self.ws, &sample)
                    .map_err(|e| format!("견본 매뉴얼을 읽지 못했습니다: {e}"))?;
                let t =
                    mwa_manual::template::learn(&self.dispatcher, &ex.text, &ex.name, &ex.pages)
                        .await
                        .map_err(|e| format!("{e:#}"))?;
                let path = self.ws.join(mwa_manual::pipeline::TEMPLATE_FILE);
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                std::fs::write(&path, serde_json::to_string_pretty(&t).unwrap_or_default())
                    .map_err(|e| e.to_string())?;
            }
            progress("본보기 매뉴얼을 따라 새로 쓰는 중");
            mwa_manual::rewrite::new_feature(
                &self.dispatcher,
                &self.ws,
                feature,
                &run.title,
                &run.facts,
                &|l| progress(&l),
            )
            .await
            .map_err(|e| format!("{e:#}"))?
        };
        let log = mwa_manual::provenance::load(&self.ws, feature)
            .log
            .last()
            .and_then(|l| l.items.first().map(|i| i.summary.clone()))
            .unwrap_or_default();
        lines.push(format!(
            "{}: 버전 {} · {log} · 사용량 정가 환산 ${:.3}",
            doc.title,
            doc.version,
            self.dispatcher.spent_usd() - spent
        ));
        // A manual written without an old one has picture slots: shoot them right away.
        let slots = mwa_manual::document::picture_slots(&doc.html)
            .iter()
            .filter(|s| s.shot.is_none())
            .count();
        if slots > 0 && self.cfg.capture.is_some() {
            use std::sync::atomic::Ordering;
            if self.capturing.swap(true, Ordering::SeqCst) {
                lines.push(format!(
                    "그림 자리 {slots}개: 다른 촬영 중이라 찍지 않았습니다. ④에서 \"그림 자리 찍기\"를 누르세요"
                ));
            } else {
                progress(&format!("그림 자리 {slots}개 촬영"));
                let shot =
                    crate::capture_doc(&self.ws, &self.cfg, feature, None, |l| progress(&l)).await;
                self.capturing.store(false, Ordering::SeqCst);
                match shot {
                    Ok(l) => {
                        lines.extend(l);
                        // What the pictures show and the facts did not say goes into the text.
                        let spent = self.dispatcher.spent_usd();
                        match mwa_manual::rewrite::from_pictures(
                            &self.dispatcher,
                            &self.ws,
                            feature,
                            &run.facts,
                            &|l| progress(&l),
                        )
                        .await
                        {
                            Ok(Some(d)) => lines.push(format!(
                                "찍은 그림을 보고 글을 보완했습니다 · 문서 버전 {} · 사용량 정가 환산 ${:.3}",
                                d.version,
                                self.dispatcher.spent_usd() - spent
                            )),
                            Ok(None) => {}
                            Err(e) => lines.push(format!("그림을 보고 보완하지 못했습니다: {e:#}")),
                        }
                    }
                    Err(e) => lines.push(format!(
                        "그림 자리를 찍지 못했습니다: {e:#} (④에서 \"그림 자리 찍기\"로 다시 할 수 있습니다)"
                    )),
                }
            }
        }
        Ok(lines)
    }

    async fn assist(
        &self,
        feature: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        #[derive(serde::Deserialize)]
        struct Body {
            request: String,
            html: String,
            #[serde(default)]
            focus: Vec<String>,
            #[serde(default)]
            history: Vec<mwa_manual::document::Turn>,
        }
        let b: Body = serde_json::from_value(body).map_err(|e| format!("요청 형식: {e}"))?;
        let facts = mwa_manual::pipeline::load_run(&self.ws, feature)
            .map(|r| r.facts)
            .unwrap_or_default();
        let a = mwa_manual::document::assist(
            &self.dispatcher,
            &self.ws,
            &b.request,
            &b.html,
            &b.focus,
            &b.history,
            &facts,
        )
        .await
        .map_err(|e| format!("{e:#}"))?;
        Ok(serde_json::to_value(a).unwrap_or_default())
    }

    fn grounding(&self, feature: &str) -> serde_json::Value {
        serde_json::to_value(mwa_manual::grounding::load(&self.ws, feature)).unwrap_or_default()
    }

    async fn ground_document(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        let run = mwa_manual::pipeline::load_run(&self.ws, feature).ok_or_else(|| {
            "코드 사실이 없습니다. ③에서 \"코드 다시 읽기\"를 먼저 하세요".to_string()
        })?;
        let spent = self.dispatcher.spent_usd();
        let g =
            mwa_manual::grounding::ground(&self.dispatcher, &self.ws, feature, &run.facts, &|l| {
                progress(&l)
            })
            .await
            .map_err(|e| format!("{e:#}"))?;
        Ok(vec![format!(
            "{} · 사용량 정가 환산 ${:.3}",
            mwa_manual::grounding::summarize(&g),
            self.dispatcher.spent_usd() - spent
        )])
    }

    fn provenance(&self, feature: &str) -> serde_json::Value {
        serde_json::to_value(mwa_manual::provenance::load(&self.ws, feature)).unwrap_or_default()
    }

    fn manuals_status(&self) -> serde_json::Value {
        use mwa_manual::{document, grounding, pipeline, reference, versions};
        let ws = &self.ws;
        let publish = crate::load_config(ws).is_ok_and(|c| c.extensions.confluence);
        let now: Vec<(String, String)> = self
            .header()
            .sources
            .iter()
            .map(|s| (s.name.clone(), s.sha.clone()))
            .collect();
        let map = self.load_map();
        let features: Vec<serde_json::Value> = map
            .as_ref()
            .map(|m| m.features.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|f| {
                let rdoc = reference::for_feature(ws, &f.key);
                let shots = reference::load_scene_shots(ws, &f.key);
                let doc = document::load(ws, &f.key);
                let g = grounding::load(ws, &f.key);
                let run = pipeline::load_run(ws, &f.key);
                let count = |v: &str| g.blocks.values().filter(|x| x.verdict == v).count();
                // Code read for the draft vs. the code checked out now.
                let read_at = versions::list(ws, &f.key).last().map(|m| m.sources.clone());
                let code_changed = read_at.map(|then| {
                    now.iter().any(|(name, sha)| {
                        !then.iter().any(|t| t.name == *name && (t.sha.starts_with(sha.as_str()) || sha.starts_with(t.sha.as_str())))
                    })
                });
                let scene = |st: &str| shots.as_ref().map_or(0, |s| s.scenes.iter().filter(|x| x.status == st).count());
                serde_json::json!({
                    "key": f.key,
                    "label": f.label,
                    "group": f.group.clone().unwrap_or_else(|| f.section.clone()),
                    "reference": rdoc.as_ref().map(|r| serde_json::json!({
                        "title": r.title,
                        "pictures": r.images.len(),
                        "described": r.images.iter().filter(|i| i.scene.is_some()).count(),
                    })),
                    "scenes": shots.as_ref().map(|s| serde_json::json!({
                        "total": s.scenes.len(),
                        "reproduced": scene("reproduced"),
                        "different": scene("different"),
                        "failed": scene("failed"),
                        "uploaded": s.scenes.iter().filter(|x| x.uploaded.is_some()).count(),
                    })),
                    "doc": doc.as_ref().map(|d| serde_json::json!({
                        "version": d.version,
                        "updated_at": d.updated_at,
                        "origin": d.origin,
                    })),
                    "grounding": (g.at > 0).then(|| serde_json::json!({
                        "at": g.at,
                        "version": g.version,
                        "supported": count("supported"),
                        "partial": count("partial"),
                        "contradicted": count("contradicted"),
                        "unsupported": count("unsupported"),
                        "stale": doc.as_ref().is_some_and(|d| d.version != g.version),
                    })),
                    "code": run.as_ref().map(|r| serde_json::json!({
                        "read_at": r.generated_at,
                        "facts": r.facts.len(),
                        "changed": code_changed,
                    })),
                    "published": if publish { serde_json::to_value(crate::confluence::published(ws, &f.key)).unwrap_or_default() } else { serde_json::Value::Null },
                })
            })
            .collect();
        serde_json::json!({ "features": features, "publish": publish })
    }

    fn confluence_status(&self, feature: &str) -> serde_json::Value {
        let cfg = crate::load_config(&self.ws).ok().and_then(|c| c.confluence);
        serde_json::json!({
            "enabled": crate::load_config(&self.ws).is_ok_and(|c| c.extensions.confluence),
            "configured": cfg.is_some(),
            "base_url": cfg.as_ref().map(|c| c.base_url.clone()),
            "email": cfg.as_ref().map(|c| c.email.clone()),
            "space": cfg.as_ref().map(|c| c.space.clone()),
            "parent": cfg.as_ref().map(|c| c.parent.clone()),
            "has_token": cfg.as_ref().is_some_and(|c| crate::confluence::token(c).is_ok()),
            "published": crate::confluence::published(&self.ws, feature),
            "target": crate::confluence::target(&self.ws),
        })
    }

    async fn confluence_setup(&self, body: serde_json::Value) -> Result<String, String> {
        #[derive(serde::Deserialize)]
        struct Body {
            page_url: String,
            email: String,
            #[serde(default)]
            token: String,
        }
        let b: Body = serde_json::from_value(body).map_err(|e| format!("요청 형식: {e}"))?;
        let (base_url, space, parent) = crate::confluence::parse_page_url(&b.page_url).ok_or(
            "상위 페이지 주소 형식이 아닙니다. 예: https://회사.atlassian.net/wiki/spaces/DOC/pages/123456/제목",
        )?;
        let cfg = mwa_protocol::config::ConfluenceCfg {
            base_url,
            email: b.email.trim().to_string(),
            space,
            parent,
        };
        if cfg.email.is_empty() {
            return Err("이메일을 넣으세요".into());
        }
        // A blank token keeps the stored one.
        let token = if b.token.trim().is_empty() {
            crate::confluence::token(&cfg).map_err(|e| e.to_string())?
        } else {
            b.token.trim().to_string()
        };
        let found = crate::confluence::check(&cfg, &token)
            .await
            .map_err(|e| format!("{e:#}"))?;
        crate::confluence::save_token(&cfg, &token).map_err(|e| format!("{e:#}"))?;
        crate::confluence::save_config(&self.ws, &cfg).map_err(|e| format!("{e:#}"))?;
        crate::confluence::save_target(&self.ws, &found).map_err(|e| format!("{e:#}"))?;
        Ok(format!(
            "스페이스 '{}' › 상위 페이지 '{}' 확인됨",
            found.space_name, found.parent_title
        ))
    }

    async fn confluence_publish(
        &self,
        feature: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        #[derive(serde::Deserialize)]
        struct Img {
            name: String,
            url: String,
        }
        #[derive(serde::Deserialize)]
        struct Body {
            title: String,
            storage: String,
            #[serde(default)]
            images: Vec<Img>,
            #[serde(default)]
            adopt: bool,
            #[serde(default)]
            group: Option<String>,
        }
        let b: Body = serde_json::from_value(body).map_err(|e| format!("요청 형식: {e}"))?;
        let cfg = crate::load_config(&self.ws)
            .ok()
            .and_then(|c| c.confluence)
            .ok_or("컨플루언스 게시 설정이 없습니다")?;
        let mut images = Vec::new();
        for i in b.images {
            let path = self
                .local_image(&i.url)
                .ok_or_else(|| format!("그림을 찾지 못했습니다: {}", i.url))?;
            images.push((i.name, std::fs::read(&path).map_err(|e| e.to_string())?));
        }
        let doc_version = mwa_manual::document::load(&self.ws, feature).map_or(0, |d| d.version);
        let p = crate::confluence::publish(
            &self.ws,
            feature,
            &cfg,
            crate::confluence::Page {
                title: &b.title,
                storage: &b.storage,
                images,
                doc_version,
                adopt: b.adopt,
                group: b.group.as_deref().filter(|g| !g.trim().is_empty()),
            },
        )
        .await
        .map_err(|e| format!("{e:#}"))?;
        Ok(serde_json::to_value(p).unwrap_or_default())
    }

    fn reference_shots(&self, feature: &str) -> Option<serde_json::Value> {
        serde_json::to_value(mwa_manual::reference::load_scene_shots(&self.ws, feature)?).ok()
    }

    async fn capture_reference(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        use std::sync::atomic::Ordering;
        if self.capturing.swap(true, Ordering::SeqCst) {
            return Err("이미 촬영 중입니다".into());
        }
        // "<feature>[|<image>][?demo][&quick]": one scene only, demo data instead of the
        // server's, saved plans replayed without a model.
        let (target, flags) = feature.split_once('?').unwrap_or((feature, ""));
        let world = flags.split('&').any(|f| f == "demo");
        let quick = flags.split('&').any(|f| f == "quick");
        let (feature, only) = target
            .split_once('|')
            .map_or((target, None), |(f, i)| (f, Some(i)));
        let result =
            crate::capture_reference(&self.ws, &self.cfg, feature, world, only, quick, |l| {
                progress(&l)
            })
            .await;
        self.capturing.store(false, Ordering::SeqCst);
        result
            .map(|s| vec![mwa_manual::reference::summarize(&s)])
            .map_err(|e| format!("{e:#}"))
    }

    fn automation(&self) -> serde_json::Value {
        crate::automation::view(&self.ws)
    }

    fn save_automation(&self, rules: serde_json::Value) -> Result<serde_json::Value, String> {
        let rules = serde_json::from_value(rules).map_err(|e| format!("규칙 형식: {e}"))?;
        crate::automation::save(&self.ws, rules).map_err(|e| format!("{e:#}"))?;
        Ok(crate::automation::view(&self.ws))
    }

    async fn run_automation(&self, id: &str, progress: OnDelta) -> Result<Vec<String>, String> {
        crate::automation::run(self, &self.ws, id, Arc::from(progress))
            .await
            .map_err(|e| format!("{e:#}"))
    }

    async fn automation_tick(&self) {
        crate::automation::tick(self, &self.ws).await;
    }

    async fn make_adapter(&self, progress: OnDelta) -> Result<Vec<String>, String> {
        crate::adapter_gen::make(&self.ws, |l| progress(&l))
            .await
            .map_err(|e| format!("{e:#}"))
    }

    async fn capture_document(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        use std::sync::atomic::Ordering;
        if self.capturing.swap(true, Ordering::SeqCst) {
            return Err("이미 촬영 중입니다".into());
        }
        let (feature, only) = feature
            .split_once('|')
            .map_or((feature, None), |(f, b)| (f, Some(b)));
        let result = crate::capture_doc(&self.ws, &self.cfg, feature, only, |l| progress(&l)).await;
        self.capturing.store(false, Ordering::SeqCst);
        result.map_err(|e| format!("{e:#}"))
    }

    fn manual_versions(&self, feature: &str) -> serde_json::Value {
        let items: Vec<serde_json::Value> = mwa_manual::versions::history(&self.ws, feature)
            .into_iter()
            .map(|(meta, diff)| serde_json::json!({ "meta": meta, "diff": diff }))
            .collect();
        serde_json::json!({ "versions": items })
    }

    fn manual_version(&self, feature: &str, n: u32) -> Option<serde_json::Value> {
        serde_json::to_value(mwa_manual::versions::load(&self.ws, feature, n)?).ok()
    }

    fn browse(&self, path: Option<&str>, ext: Option<&str>) -> Result<serde_json::Value, String> {
        crate::setup::browse(path, ext)
    }

    fn setup_steps(&self) -> Vec<mwa_host::SetupStep> {
        crate::setup::steps(&self.ws, &self.cfg)
    }

    fn settings(&self) -> Result<serde_json::Value, String> {
        let s = crate::settings::read(&self.ws)?;
        serde_json::to_value(s).map_err(|e| e.to_string())
    }

    fn save_settings(&self, settings: serde_json::Value) -> Result<(), String> {
        if self.busy() {
            return Err("촬영이나 매뉴얼 생성이 끝난 뒤에 저장하세요".into());
        }
        let s: crate::settings::Settings =
            serde_json::from_value(settings).map_err(|e| format!("설정 형식 오류: {e}"))?;
        crate::settings::save(&self.ws, &s)
    }

    fn set_password(&self, profile: &str, password: &str) -> Result<(), String> {
        use mwa_capture::creds::{CredStore, key};
        if password.is_empty() {
            return Err("비밀번호가 비어 있습니다".into());
        }
        // A login profile, or an SSH code source by its name.
        let k = match self.cfg.profiles.get(profile) {
            Some(p) => key(&p.user_id, &p.base_url),
            None => {
                let target = self
                    .cfg
                    .sources
                    .iter()
                    .find(|s| s.name == profile)
                    .and_then(|s| s.ssh.clone())
                    .ok_or_else(|| {
                        format!(
                            "프로파일이나 SSH 소스 '{profile}'이 없습니다. 먼저 설정을 저장하세요"
                        )
                    })?;
                crate::ssh_cred_key(&mwa_source::remote::who(&target))
            }
        };
        let mut store = CredStore::default_path()
            .and_then(|path| CredStore::open(&path))
            .map_err(|e| e.to_string())?;
        store.set_password(&k, password.to_string());
        store.save().map_err(|e| e.to_string())
    }

    async fn probe_source(&self, source: serde_json::Value) -> Result<serde_json::Value, String> {
        use mwa_source::remote;
        let text = |k: &str| source[k].as_str().unwrap_or("").trim().to_string();
        let action = text("action");
        let data = crate::projects::data_dir();
        // mwa's own key: shown so a person can put it on the servers.
        if action == "key" {
            return remote::ensure_key(&data)
                .map(|k| serde_json::json!({ "public_key": k }))
                .map_err(|e| e.to_string());
        }
        let (target, docker, path) = (text("ssh"), text("docker"), text("remote_path"));
        if target.is_empty() {
            return Err("접속할 서버(사용자@주소)를 적으세요".into());
        }
        let jump: Vec<String> = text("jump")
            .split(',')
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty())
            .collect();
        let at = text("path");
        // Passwords typed just now (not saved yet) go first; saved ones fill the rest.
        let typed: Vec<(String, String)> = source["passwords"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        let hop = p["hop"].as_str()?.trim();
                        let pw = p["password"].as_str()?;
                        (!hop.is_empty() && !pw.is_empty())
                            .then(|| (mwa_source::remote::who(hop), pw.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        tokio::task::spawn_blocking(move || {
            let mut pw = typed.clone();
            for (w, p) in crate::ssh_passwords(&target, &jump) {
                if !pw.iter().any(|(x, _)| *x == w) {
                    pw.push((w, p));
                }
            }
            let kh = remote::known_hosts(&data);
            let r = remote::Remote {
                target: &target,
                jump: &jump,
                docker: (!docker.is_empty()).then_some(docker.as_str()),
                path: if path.is_empty() { "." } else { &path },
                passwords: &pw,
                known_hosts: &kh,
            };
            let v = match action.as_str() {
                "check" => remote::check(&r).map(|who| {
                    // It worked: the passwords typed for it were right; keep them.
                    if !typed.is_empty()
                        && let Ok(mut store) = mwa_capture::creds::CredStore::default_path()
                            .and_then(|p| mwa_capture::creds::CredStore::open(&p))
                    {
                        for (w, p) in &typed {
                            store.set_password(&crate::ssh_cred_key(w), p.clone());
                        }
                        let _ = store.save();
                    }
                    serde_json::json!({ "who": who, "saved": typed.len() })
                }),
                "ls" => remote::list_dir(&r, if at.is_empty() { "/" } else { &at }).map(|e| {
                    serde_json::json!({ "entries": e.into_iter().map(|(n, d)| serde_json::json!({ "name": n, "dir": d })).collect::<Vec<_>>() })
                }),
                "guess" => remote::guess(&r).map(|f| {
                    serde_json::json!({ "found": f.into_iter().map(|(p, k)| serde_json::json!({ "path": p, "kind": k })).collect::<Vec<_>>() })
                }),
                _ => remote::probe(&r).map(|p| serde_json::to_value(p).unwrap_or_default()),
            };
            v.map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    async fn install_browser(&self, progress: OnDelta) -> Result<Vec<String>, String> {
        let cfg = self.cfg.clone();
        tokio::task::spawn_blocking(move || crate::install_browser(&cfg, |l| progress(&l)))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("{e:#}"))
    }

    async fn sync_sources(&self, progress: OnDelta) -> Result<Vec<String>, String> {
        let cfg = self.cfg.clone();
        tokio::task::spawn_blocking(move || crate::sync_sources(&cfg, |l| progress(&l)))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("{e:#}"))
    }

    async fn doctor(&self) -> Vec<String> {
        match crate::doctor_report(&self.ws, self.cfg.clone(), false).await {
            Ok((lines, _)) => lines,
            Err(e) => vec![format!("점검 실패: {e:#}")],
        }
    }

    fn chat_models(&self) -> Vec<String> {
        let Some((backend, current)) = self.dispatcher.role_model(Role::Chat) else {
            return Vec::new();
        };
        match self.cfg.backends.get(&backend) {
            // Aliases accepted by `claude --model` (docs/cli-flags.md).
            Some(mwa_protocol::BackendCfg::ClaudeCode { .. }) => {
                ["haiku", "sonnet", "opus", "fable"]
                    .map(String::from)
                    .to_vec()
            }
            _ => vec![current],
        }
    }

    fn set_chat_model(&self, model: &str) -> Result<(), String> {
        if !self.chat_models().iter().any(|m| m == model) {
            return Err(format!("쓸 수 없는 모델입니다: {model}"));
        }
        self.dispatcher
            .set_role_model(Role::Chat, model)
            .map_err(|e| e.to_string())
    }

    async fn capture_menus(&self, progress: OnDelta) -> Result<Vec<String>, String> {
        use std::sync::atomic::Ordering;
        if self.capturing.swap(true, Ordering::SeqCst) {
            return Err("이미 촬영 중입니다".into());
        }
        let result =
            crate::capture_menus(&self.ws, &self.cfg, 2, &[], false, |line| progress(&line)).await;
        self.capturing.store(false, Ordering::SeqCst);
        result.map(|m| m.summary()).map_err(|e| format!("{e:#}"))
    }

    async fn generate_manual(
        &self,
        feature: &str,
        progress: OnDelta,
    ) -> Result<Vec<String>, String> {
        use std::sync::atomic::Ordering;
        if self.generating.swap(true, Ordering::SeqCst) {
            return Err("이미 매뉴얼을 만들고 있습니다".into());
        }
        let result = crate::generate_manual(&self.ws, &self.cfg, feature, |l| progress(&l)).await;
        self.generating.store(false, Ordering::SeqCst);
        result
            .map(|run| mwa_manual::pipeline::summary(&run))
            .map_err(|e| format!("{e:#}"))
    }

    fn manual_run(&self, feature: &str) -> Option<serde_json::Value> {
        serde_json::to_value(mwa_manual::pipeline::load_run(&self.ws, feature)?).ok()
    }

    fn shots_manifest(&self) -> Option<serde_json::Value> {
        serde_json::to_value(mwa_capture::menus::load_manifest(&self.ws)?).ok()
    }

    fn shot_file(&self, rel: &str) -> Option<PathBuf> {
        mwa_capture::menus::shot_file(&self.ws, rel)
    }

    fn busy(&self) -> bool {
        self.capturing.load(std::sync::atomic::Ordering::SeqCst)
            || self.generating.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn auth_lines(&self) -> Vec<String> {
        match mwa_capture::creds::CredStore::default_path()
            .and_then(|p| mwa_capture::creds::CredStore::open(&p))
        {
            Ok(store) => crate::auth_list_lines(&self.cfg, &store),
            Err(e) => vec![format!("자격 파일 오류: {e}")],
        }
    }
}

/// `%XX` escapes in a URL path segment (UTF-8).
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
