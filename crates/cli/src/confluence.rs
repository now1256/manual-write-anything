//! Publishing a manual document to Confluence Cloud as a native page (D-041): storage-format
//! body (panels, status, expand as macros), pictures as attachments. Plain REST calls, no model.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use mwa_protocol::config::ConfluenceCfg;

/// Base URL, space key and page id from a page link such as
/// `https://acme.atlassian.net/wiki/spaces/DOC/pages/123456/Title`.
pub fn parse_page_url(url: &str) -> Option<(String, String, String)> {
    let url = url.trim();
    let wiki = url.find("/wiki/")?;
    let base = url[..wiki].trim_end_matches('/').to_string();
    let rest = &url[wiki + 6..];
    let parts: Vec<&str> = rest.split(['/', '?', '#']).collect();
    let space = parts
        .iter()
        .position(|p| *p == "spaces")
        .and_then(|i| parts.get(i + 1))?;
    let page = parts
        .iter()
        .position(|p| *p == "pages")
        .and_then(|i| parts.get(i + 1))?;
    (base.starts_with("https://") && page.chars().all(|c| c.is_ascii_digit()))
        .then(|| (base, space.to_string(), page.to_string()))
}

/// Credential key of the API token (same file as product passwords).
pub fn token_key(cfg: &ConfluenceCfg) -> String {
    mwa_capture::creds::key(&cfg.email, &cfg.base_url)
}

pub fn token(cfg: &ConfluenceCfg) -> Result<String> {
    use mwa_capture::creds::CredStore;
    let store = CredStore::open(&CredStore::default_path()?)?;
    store
        .get(&token_key(cfg))
        .map(|c| c.password.clone())
        .ok_or_else(|| anyhow!("컨플루언스 API 토큰이 없습니다. 게시 창에서 토큰을 저장하세요"))
}

pub fn save_token(cfg: &ConfluenceCfg, token: &str) -> Result<()> {
    use mwa_capture::creds::CredStore;
    let mut store = CredStore::open(&CredStore::default_path()?)?;
    store.set_password(&token_key(cfg), token.to_string());
    Ok(store.save()?)
}

/// Writes `[confluence]` into `mwa.toml`, keeping comments and the rest of the file.
pub fn save_config(ws: &Path, cfg: &ConfluenceCfg) -> Result<()> {
    let path = ws.join("mwa.toml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("{} 읽기", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text.parse().context("mwa.toml 형식")?;
    let mut t = toml_edit::Table::new();
    t["base_url"] = toml_edit::value(&cfg.base_url);
    t["email"] = toml_edit::value(&cfg.email);
    t["space"] = toml_edit::value(&cfg.space);
    t["parent"] = toml_edit::value(&cfg.parent);
    doc["confluence"] = toml_edit::Item::Table(t);
    std::fs::write(&path, doc.to_string()).with_context(|| format!("{} 쓰기", path.display()))
}

struct Client {
    http: reqwest::Client,
    base: String,
    auth: String,
}

impl Client {
    fn new(cfg: &ConfluenceCfg, token: &str) -> Self {
        let auth =
            base64::engine::general_purpose::STANDARD.encode(format!("{}:{token}", cfg.email));
        Self {
            http: reqwest::Client::new(),
            base: format!("{}/wiki/rest/api", cfg.base_url.trim_end_matches('/')),
            auth: format!("Basic {auth}"),
        }
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value> {
        let res = req
            .header("Authorization", &self.auth)
            .header("Accept", "application/json")
            .send()
            .await
            .context("컨플루언스에 연결하지 못했습니다")?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| {
                    v.get("message")
                        .and_then(|m| m.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| text.chars().take(300).collect());
            bail!(
                "컨플루언스 오류 {}: {msg}{}",
                status.as_u16(),
                match status.as_u16() {
                    401 => " (이메일이나 API 토큰을 확인하세요)",
                    403 => " (이 스페이스·페이지에 쓸 권한이 없습니다)",
                    404 => " (페이지나 스페이스를 찾지 못했습니다)",
                    _ => "",
                }
            );
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let url = reqwest::Url::parse_with_params(&format!("{}{path}", self.base), query)
            .context("컨플루언스 주소 형식")?;
        self.send(self.http.get(url)).await
    }

    async fn json(&self, method: reqwest::Method, path: &str, body: &Value) -> Result<Value> {
        self.send(
            self.http
                .request(method, format!("{}{path}", self.base))
                .header("Content-Type", "application/json")
                .body(body.to_string()),
        )
        .await
    }

    /// Adds or replaces an attachment by file name.
    async fn upload(
        &self,
        page: &str,
        existing: Option<&str>,
        name: &str,
        data: Vec<u8>,
    ) -> Result<()> {
        let part = reqwest::multipart::Part::bytes(data)
            .file_name(name.to_string())
            .mime_str(if name.ends_with(".jpg") {
                "image/jpeg"
            } else {
                "image/png"
            })?;
        let form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("minorEdit", "true");
        let path = match existing {
            Some(id) => format!("/content/{page}/child/attachment/{id}/data"),
            None => format!("/content/{page}/child/attachment"),
        };
        self.send(
            self.http
                .post(format!("{}{path}", self.base))
                .header("X-Atlassian-Token", "no-check")
                .multipart(form),
        )
        .await
        .map(|_| ())
    }
}

/// What the target looks like: shown before saving the settings.
/// Names of the configured place, read when it is checked and kept in the project so the
/// publish window can show where pages go without asking Confluence each time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Target {
    pub space_name: String,
    pub parent_title: String,
    pub parent_url: String,
}

fn target_path(ws: &Path) -> std::path::PathBuf {
    ws.join(".mwa/confluence-target.json")
}

pub fn target(ws: &Path) -> Option<Target> {
    serde_json::from_str(&std::fs::read_to_string(target_path(ws)).ok()?).ok()
}

pub fn save_target(ws: &Path, t: &Target) -> Result<()> {
    let path = target_path(ws);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    Ok(std::fs::write(&path, serde_json::to_string_pretty(t)?)?)
}

pub async fn check(cfg: &ConfluenceCfg, token: &str) -> Result<Target> {
    let c = Client::new(cfg, token);
    let space = c.get(&format!("/space/{}", cfg.space), &[]).await?;
    let parent = c.get(&format!("/content/{}", cfg.parent), &[]).await?;
    Ok(Target {
        space_name: space["name"].as_str().unwrap_or(&cfg.space).to_string(),
        parent_title: parent["title"].as_str().unwrap_or(&cfg.parent).to_string(),
        parent_url: format!(
            "{}/wiki{}",
            cfg.base_url.trim_end_matches('/'),
            parent["_links"]["webui"].as_str().unwrap_or("")
        ),
    })
}

/// The page for a menu group (e.g. "소스") right under the configured parent; made when missing.
/// Page titles are unique per space, so a same-named page elsewhere gets a suffixed title.
async fn group_page(
    c: &Client,
    cfg: &ConfluenceCfg,
    group: &str,
    parent_title: &str,
) -> Result<String> {
    for title in [group.to_string(), format!("{group} ({parent_title})")] {
        let found = c
            .get(
                "/content",
                &[
                    ("spaceKey", cfg.space.as_str()),
                    ("title", title.as_str()),
                    ("expand", "ancestors"),
                ],
            )
            .await?;
        match found["results"].as_array().and_then(|r| r.first()) {
            Some(p) => {
                let under_parent = p["ancestors"]
                    .as_array()
                    .and_then(|a| a.last())
                    .is_some_and(|a| a["id"].as_str() == Some(cfg.parent.as_str()));
                if under_parent {
                    return Ok(p["id"].as_str().unwrap_or_default().to_string());
                }
            }
            None => {
                let r = c
                    .json(
                        reqwest::Method::POST,
                        "/content",
                        &json!({
                            "type": "page",
                            "title": title,
                            "space": { "key": cfg.space },
                            "ancestors": [{ "id": cfg.parent }],
                            "body": { "storage": {
                                "value": "<ac:structured-macro ac:name=\"children\" />",
                                "representation": "storage"
                            } },
                        }),
                    )
                    .await?;
                return Ok(r["id"].as_str().unwrap_or_default().to_string());
            }
        }
    }
    bail!("메뉴 그룹 페이지 '{group}'를 만들 수 없습니다: 같은 제목이 다른 곳에 이미 있습니다")
}

/// Where a feature's manual was published, kept in the project.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Published {
    pub page_id: String,
    pub url: String,
    pub title: String,
    /// Confluence page version after the last publish.
    pub version: u64,
    /// mwa document version published.
    pub doc_version: u32,
    pub at: u64,
    /// Where it sits: space › parent › (group ›) title.
    #[serde(default)]
    pub path: Vec<String>,
}

fn record_path(ws: &Path, feature: &str) -> std::path::PathBuf {
    ws.join("manual/docs").join(feature).join("confluence.json")
}

pub fn published(ws: &Path, feature: &str) -> Option<Published> {
    serde_json::from_str(&std::fs::read_to_string(record_path(ws, feature)).ok()?).ok()
}

pub struct Page<'a> {
    pub title: &'a str,
    /// Confluence storage format (XHTML).
    pub storage: &'a str,
    pub images: Vec<(String, Vec<u8>)>,
    pub doc_version: u32,
    /// Take over an existing page with the same title (asked for after a conflict).
    pub adopt: bool,
    /// Menu group page to put a new page under ("소스"); `None` = right under the parent.
    pub group: Option<&'a str>,
}

/// Creates the page under the configured parent, or updates the one published before; then
/// uploads pictures. Returns the page link.
pub async fn publish(
    ws: &Path,
    feature: &str,
    cfg: &ConfluenceCfg,
    page: Page<'_>,
) -> Result<Published> {
    let c = Client::new(cfg, &token(cfg)?);
    let body =
        |storage: &str| json!({ "storage": { "value": storage, "representation": "storage" } });
    let mut known = published(ws, feature);
    if known.is_none() {
        let found = c
            .get(
                "/content",
                &[
                    ("spaceKey", cfg.space.as_str()),
                    ("title", page.title),
                    ("expand", "version"),
                ],
            )
            .await?;
        if let Some(p) = found["results"].as_array().and_then(|r| r.first()) {
            let link = format!(
                "{}/wiki{}",
                cfg.base_url,
                p["_links"]["webui"].as_str().unwrap_or("")
            );
            if !page.adopt {
                bail!("SAME_TITLE {link}");
            }
            known = Some(Published {
                page_id: p["id"].as_str().unwrap_or_default().to_string(),
                url: link,
                title: page.title.to_string(),
                ..Default::default()
            });
        }
    }
    let names = target(ws).unwrap_or_default();
    let mut path = vec![names.space_name.clone(), names.parent_title.clone()];
    let under = match (&known, page.group) {
        (None, Some(g)) => {
            path.push(g.to_string());
            group_page(&c, cfg, g, &names.parent_title).await?
        }
        _ => cfg.parent.clone(),
    };
    path.push(page.title.to_string());
    let (id, version, webui) = match known {
        Some(k) => {
            let cur = c
                .get(&format!("/content/{}", k.page_id), &[("expand", "version")])
                .await?;
            let next = cur["version"]["number"].as_u64().unwrap_or(1) + 1;
            let r = c
                .json(
                    reqwest::Method::PUT,
                    &format!("/content/{}", k.page_id),
                    &json!({
                        "id": k.page_id,
                        "type": "page",
                        "title": page.title,
                        "version": { "number": next, "message": format!("mwa 문서 버전 {}", page.doc_version) },
                        "body": body(page.storage),
                    }),
                )
                .await?;
            (
                k.page_id,
                next,
                r["_links"]["webui"].as_str().unwrap_or("").to_string(),
            )
        }
        None => {
            let r = c
                .json(
                    reqwest::Method::POST,
                    "/content",
                    &json!({
                        "type": "page",
                        "title": page.title,
                        "space": { "key": cfg.space },
                        "ancestors": [{ "id": under }],
                        "body": body(page.storage),
                    }),
                )
                .await?;
            (
                r["id"].as_str().unwrap_or_default().to_string(),
                r["version"]["number"].as_u64().unwrap_or(1),
                r["_links"]["webui"].as_str().unwrap_or("").to_string(),
            )
        }
    };
    if !page.images.is_empty() {
        let list = c
            .get(
                &format!("/content/{id}/child/attachment"),
                &[("limit", "500")],
            )
            .await?;
        let existing: std::collections::HashMap<String, String> = list["results"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| {
                        Some((
                            x["title"].as_str()?.to_string(),
                            x["id"].as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (name, data) in page.images {
            c.upload(&id, existing.get(&name).map(String::as_str), &name, data)
                .await?;
        }
    }
    let p = Published {
        page_id: id,
        url: format!("{}/wiki{webui}", cfg.base_url.trim_end_matches('/')),
        title: page.title.to_string(),
        version,
        doc_version: page.doc_version,
        path: match published(ws, feature) {
            // Moving a page is not done here: an updated page keeps its place.
            Some(old) if !old.path.is_empty() => old.path,
            _ => path,
        },
        at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    let path = record_path(ws, feature);
    std::fs::write(&path, serde_json::to_string_pretty(&p)?)
        .with_context(|| path.display().to_string())?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_links() {
        assert_eq!(
            parse_page_url("https://acme.atlassian.net/wiki/spaces/DOC/pages/123456/User+Manual"),
            Some((
                "https://acme.atlassian.net".into(),
                "DOC".into(),
                "123456".into()
            ))
        );
        assert_eq!(
            parse_page_url("https://acme.atlassian.net/wiki/spaces/~me/pages/42"),
            Some((
                "https://acme.atlassian.net".into(),
                "~me".into(),
                "42".into()
            ))
        );
        assert_eq!(
            parse_page_url("https://acme.atlassian.net/wiki/spaces/DOC/overview"),
            None
        );
        assert_eq!(parse_page_url("not a link"), None);
    }
}
