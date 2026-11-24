//! How a product's web UI logs in and what capture must handle, from the project's
//! `adapter.toml` (`[auth]`, `[ui]`). No product's conventions are compiled in.

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct UiSpec {
    pub auth: AuthSpec,
    #[serde(default)]
    pub ui: Screen,
}

/// The product UI's own login routes, used as a browser would.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthSpec {
    pub login_path: String,
    pub me_path: String,
    pub logout_path: String,
    /// Cookie that carries the session.
    pub session_cookie: String,
    #[serde(default = "default_user_field")]
    pub user_field: String,
    #[serde(default = "default_password_field")]
    pub password_field: String,
    /// Non-GET requests the screens may send while idle (session keep-alive); allowed on a
    /// shared server because they only extend the session.
    #[serde(default)]
    pub keepalive_paths: Vec<String>,
    /// JSON pointer to a `true` flag meaning "password must be changed first".
    #[serde(default)]
    pub first_login_flag: Option<String>,
    /// JSON pointers to the rejection title and detail in an error response.
    #[serde(default)]
    pub error_title: Option<String>,
    #[serde(default)]
    pub error_info: Option<String>,
    /// JSON pointer to the user object in the `me` response (whole body if absent).
    #[serde(default)]
    pub me_data: Option<String>,
}

fn default_user_field() -> String {
    "username".into()
}

fn default_password_field() -> String {
    "password".into()
}

/// Screen conventions the capture sidecar needs.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Screen {
    /// `sessionStorage` key that selects the tenant/domain shown (`[capture] domain`).
    #[serde(default)]
    pub domain_storage_key: Option<String>,
    /// Accessible names of buttons to mask in pictures (e.g. the signed-in user's menu).
    #[serde(default)]
    pub mask_buttons: Vec<String>,
    /// CSS selector of toast notices; their text is recorded with each picture.
    #[serde(default)]
    pub toast_selector: Option<String>,
    /// Attribute on a toast that holds its type (`error`, `success`, ...).
    #[serde(default)]
    pub toast_type_attr: Option<String>,
    /// POST paths that only read (a preview, a search), checked in the product's code and let
    /// through on a shared server; every other non-GET stays blocked.
    #[serde(default)]
    pub read_posts: Vec<String>,
}

pub fn from_str(text: &str) -> Result<UiSpec, String> {
    toml::from_str(text).map_err(|e| e.to_string())
}

pub fn load(path: &Path) -> Result<UiSpec, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
pub(crate) fn test_auth() -> AuthSpec {
    from_str(
        r#"
[auth]
login_path = "/api/auth/login"
me_path = "/api/auth/me"
logout_path = "/api/auth/logout"
session_cookie = "acme_session"
user_field = "userId"
keepalive_paths = ["/api/auth/session/touch"]
first_login_flag = "/isFirstLogin"
error_title = "/message/title"
error_info = "/message/info"
me_data = "/data"
"#,
    )
    .unwrap()
    .auth
}
