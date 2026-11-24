//! One-shot login through the product UI's own login routes (adapter `[auth]`), as a browser
//! would. Never retries: products may lock an account after a few failures.

use std::fmt;
use std::time::Duration;

use serde_json::Value;

use crate::spec::AuthSpec;

pub enum LoginOutcome {
    /// `<session_cookie>=...` cookie pair.
    Ok {
        cookie: String,
    },
    PasswordChangeRequired,
    /// 4xx from the server: wrong password, locked account, ...
    Rejected {
        status: u16,
        title: String,
        info: String,
    },
    /// 5xx or malformed response; the credential itself may be fine.
    ServerError {
        status: Option<u16>,
        message: String,
    },
}

impl fmt::Debug for LoginOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok { .. } => f.write_str("Ok { cookie: <redacted> }"),
            Self::PasswordChangeRequired => f.write_str("PasswordChangeRequired"),
            Self::Rejected {
                status,
                title,
                info,
            } => write!(f, "Rejected({status}, {title}, {info})"),
            Self::ServerError { status, message } => {
                write!(f, "ServerError({status:?}, {message})")
            }
        }
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("reqwest client")
}

fn url(base: &str, path: &str) -> String {
    format!("{}{path}", base.trim_end_matches('/'))
}

/// String at a JSON pointer, or "".
fn text_at(body: &Value, pointer: Option<&str>) -> String {
    pointer
        .and_then(|p| body.pointer(p))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub async fn login_once(
    auth: &AuthSpec,
    base_url: &str,
    user_id: &str,
    password: &str,
) -> LoginOutcome {
    let mut form = serde_json::Map::new();
    form.insert(auth.user_field.clone(), user_id.into());
    form.insert(auth.password_field.clone(), password.into());
    let resp = client()
        .post(url(base_url, &auth.login_path))
        .json(&form)
        .send()
        .await;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            return LoginOutcome::ServerError {
                status: None,
                message: format!("연결 실패: {e}"),
            };
        }
    };
    let status = resp.status().as_u16();
    let cookie = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .find(|pair| pair.starts_with(&format!("{}=", auth.session_cookie)))
        .map(String::from);
    let body: Value = resp.json().await.unwrap_or(Value::Null);

    let first_login = auth
        .first_login_flag
        .as_deref()
        .and_then(|p| body.pointer(p))
        .is_some_and(|v| v == true);
    match status {
        200 if first_login => LoginOutcome::PasswordChangeRequired,
        200 => match cookie {
            Some(cookie) => LoginOutcome::Ok { cookie },
            None => LoginOutcome::ServerError {
                status: Some(200),
                message: "성공 응답에 세션 쿠키가 없습니다".into(),
            },
        },
        400..=499 => LoginOutcome::Rejected {
            status,
            title: text_at(&body, auth.error_title.as_deref()),
            info: text_at(&body, auth.error_info.as_deref()),
        },
        _ => LoginOutcome::ServerError {
            status: Some(status),
            message: text_at(&body, auth.error_info.as_deref()),
        },
    }
}

/// `GET <me_path>` with the session (read only).
pub async fn whoami(auth: &AuthSpec, base_url: &str, cookie: &str) -> Result<Value, String> {
    let resp = client()
        .get(url(base_url, &auth.me_path))
        .header("cookie", cookie)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    if status.is_success() {
        let data = auth
            .me_data
            .as_deref()
            .and_then(|p| body.pointer(p))
            .cloned();
        Ok(data.unwrap_or(body))
    } else {
        Err(format!("HTTP {status}"))
    }
}

/// `POST <logout_path>` so checks do not leave sessions behind.
/// Extends the session the way the product's screen does after a person's input (the
/// adapter's `[auth] keepalive_paths`). Capture runs for longer than an idle session lives and
/// a headless browser types nothing, so the screen itself never extends it.
pub async fn touch(base_url: &str, path: &str, cookie: &str) -> Result<(), String> {
    let resp = client()
        .post(url(base_url, path))
        .header("cookie", cookie)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

pub async fn logout(auth: &AuthSpec, base_url: &str, cookie: &str) -> Result<(), String> {
    let resp = client()
        .post(url(base_url, &auth.logout_path))
        .header("cookie", cookie)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// Serves `n` requests with canned responses; records request heads and bodies.
    fn server(
        responses: Vec<(&'static str, &'static str, &'static str)>,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            for (status, headers, body) in responses {
                let (mut sock, _) = listener.accept().unwrap();
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).unwrap();
                seen2
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let resp = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n{headers}\r\n{body}",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).unwrap();
            }
        });
        (base, seen)
    }

    #[tokio::test]
    async fn success_returns_session_cookie_once() {
        let (base, seen) = server(vec![(
            "200 OK",
            "set-cookie: other=1; Path=/\r\nset-cookie: acme_session=abc; HttpOnly; Path=/\r\n",
            r#"{"ok":true}"#,
        )]);
        match login_once(&crate::spec::test_auth(), &base, "u", "p").await {
            LoginOutcome::Ok { cookie } => assert_eq!(cookie, "acme_session=abc"),
            other => panic!("{other:?}"),
        }
        let req = seen.lock().unwrap()[0].clone();
        assert!(req.starts_with("POST /api/auth/login"));
        assert!(req.contains(r#""userId":"u""#));
        assert_eq!(seen.lock().unwrap().len(), 1, "exactly one attempt");
    }

    #[tokio::test]
    async fn rejection_and_first_login_and_server_error() {
        let (base, _) = server(vec![(
            "401 Unauthorized",
            "",
            r#"{"message":{"title":"로그인 실패","info":"비밀번호가 올바르지 않습니다. (1/5)"}}"#,
        )]);
        match login_once(&crate::spec::test_auth(), &base, "u", "p").await {
            LoginOutcome::Rejected { status, info, .. } => {
                assert_eq!(status, 401);
                assert!(info.contains("1/5"));
            }
            other => panic!("{other:?}"),
        }
        let (base, _) = server(vec![("200 OK", "", r#"{"isFirstLogin":true,"token":"t"}"#)]);
        assert!(matches!(
            login_once(&crate::spec::test_auth(), &base, "u", "p").await,
            LoginOutcome::PasswordChangeRequired
        ));
        let (base, _) = server(vec![("500 Internal Server Error", "", "{}")]);
        assert!(matches!(
            login_once(&crate::spec::test_auth(), &base, "u", "p").await,
            LoginOutcome::ServerError {
                status: Some(500),
                ..
            }
        ));
        assert!(matches!(
            login_once(&crate::spec::test_auth(), "http://127.0.0.1:9", "u", "p").await,
            LoginOutcome::ServerError { status: None, .. }
        ));
    }

    #[test]
    fn debug_redacts_cookie() {
        let o = LoginOutcome::Ok {
            cookie: "acme_session=zzz".into(),
        };
        assert!(!format!("{o:?}").contains("zzz"));
    }
}
