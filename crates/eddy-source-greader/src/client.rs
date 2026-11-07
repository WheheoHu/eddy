use std::time::Duration;

use reqwest::{Method, StatusCode, redirect};
use tokio::sync::Mutex;

use crate::model::{RawItemIds, RawStream, RawSubscriptions};
use crate::{GReaderError, Result, StreamPage, Subscription, UpstreamItem, long_item_id, normalize_base, normalize_item_id};

#[derive(Clone)]
pub struct Credentials {
    pub base_url: String,
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("base_url", &self.base_url)
            .field("username", &self.username)
            .field("password", &"[已隐藏]")
            .finish()
    }
}

pub struct GReaderClient {
    http: reqwest::Client,
    base: String,
    username: String,
    password: String,
    auth: Mutex<Option<String>>,
}

impl GReaderClient {
    pub fn new(creds: Credentials) -> Result<Self> {
        if creds.username.trim().is_empty() || creds.password.is_empty() {
            return Err(GReaderError::Config("请填写 FreshRSS 用户名和 API 专用密码。".into()));
        }
        let http = reqwest::Client::builder()
            // Never follow redirects: credentials must not reach another endpoint.
            .redirect(redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("Eddy/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| GReaderError::Network(e.to_string()))?;
        Ok(GReaderClient {
            http,
            base: normalize_base(&creds.base_url)?,
            username: creds.username.trim().to_string(),
            password: creds.password,
            auth: Mutex::new(None),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    /// Logs in with ClientLogin and caches the auth token.
    pub async fn login(&self) -> Result<String> {
        let response = self
            .http
            .post(format!("{}/accounts/ClientLogin", self.base))
            .form(&[("Email", self.username.as_str()), ("Passwd", self.password.as_str())])
            .send()
            .await
            .map_err(network)?;
        let status = response.status();
        check_redirect(status)?;
        if !status.is_success() {
            return Err(GReaderError::Auth(status.as_u16()));
        }
        let body = response.text().await.map_err(network)?;
        let auth = body
            .lines()
            .find_map(|l| l.strip_prefix("Auth="))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && !s.contains(['\r', '\n']))
            .ok_or_else(|| GReaderError::Format("登录响应里没有 Auth 令牌，请检查 Google Reader API 地址。".into()))?;
        *self.auth.lock().await = Some(auth.clone());
        Ok(auth)
    }

    async fn auth(&self) -> Result<String> {
        if let Some(a) = self.auth.lock().await.clone() {
            return Ok(a);
        }
        self.login().await
    }

    /// Sends an authenticated request, logging in again once on 401.
    async fn request(&self, method: Method, path: &str, form: Option<&[(String, String)]>) -> Result<reqwest::Response> {
        for attempt in 0..2 {
            let auth = self.auth().await?;
            let mut req = self
                .http
                .request(method.clone(), format!("{}/reader/api/0/{path}", self.base))
                .header("Authorization", format!("GoogleLogin auth={auth}"));
            if let Some(form) = form {
                req = req.form(form);
            }
            let response = req.send().await.map_err(network)?;
            let status = response.status();
            check_redirect(status)?;
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                *self.auth.lock().await = None;
                continue;
            }
            if !status.is_success() {
                return Err(GReaderError::Http(status.as_u16()));
            }
            return Ok(response);
        }
        Err(GReaderError::Auth(401))
    }

    async fn json<T: serde::de::DeserializeOwned>(&self, method: Method, path: &str, form: Option<&[(String, String)]>) -> Result<T> {
        let response = self.request(method, path, form).await?;
        let text = response.text().await.map_err(network)?;
        serde_json::from_str(&text).map_err(|e| GReaderError::Format(e.to_string()))
    }

    pub async fn subscriptions(&self) -> Result<Vec<Subscription>> {
        let raw: RawSubscriptions = self.json(Method::GET, "subscription/list?output=json", None).await?;
        Ok(raw.subscriptions.into_iter().map(Subscription::from).collect())
    }

    /// All item IDs (hex) in `stream`, optionally excluding a state, following
    /// continuations up to `cap` IDs.
    pub async fn item_ids(&self, stream: &str, exclude: Option<&str>, cap: usize) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut continuation: Option<String> = None;
        loop {
            let query = {
                let mut q = url::form_urlencoded::Serializer::new(String::new());
                q.append_pair("output", "json").append_pair("s", stream).append_pair("n", "10000");
                if let Some(x) = exclude {
                    q.append_pair("xt", x);
                }
                if let Some(c) = &continuation {
                    q.append_pair("c", c);
                }
                q.finish()
            };
            let raw: RawItemIds = self.json(Method::GET, &format!("stream/items/ids?{query}"), None).await?;
            ids.extend(raw.item_refs.iter().filter_map(|r| normalize_item_id(&r.id)));
            let next = crate::validate_continuation(raw.continuation.as_deref(), continuation.as_deref())?;
            if next.is_none() || ids.len() >= cap || raw.item_refs.is_empty() {
                break;
            }
            continuation = next;
        }
        ids.truncate(cap);
        Ok(ids)
    }

    /// Full items for the given hex IDs, fetched in batches.
    pub async fn contents(&self, ids: &[String]) -> Result<Vec<UpstreamItem>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(100) {
            let form: Vec<(String, String)> = chunk.iter().map(|id| ("i".to_string(), long_item_id(id))).collect();
            let raw: RawStream = self.json(Method::POST, "stream/items/contents?output=json", Some(&form)).await?;
            out.extend(raw.items.into_iter().filter_map(|i| i.into_item()));
        }
        Ok(out)
    }

    /// One page of a stream, newest first.
    pub async fn stream_page(&self, path: &str, requested: Option<&str>) -> Result<StreamPage> {
        let raw: RawStream = self.json(Method::GET, path, None).await?;
        let continuation = crate::validate_continuation(raw.continuation.as_deref(), requested)?;
        Ok(StreamPage { items: raw.items.into_iter().filter_map(|i| i.into_item()).collect(), continuation })
    }

    /// Adds or removes a state tag (e.g. read, starred) on items.
    pub async fn edit_tag(&self, ids: &[String], tag: &str, add: bool) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let token = self.request(Method::GET, "token", None).await?.text().await.map_err(network)?.trim().to_string();
        for chunk in ids.chunks(200) {
            let mut form: Vec<(String, String)> = chunk.iter().map(|id| ("i".to_string(), long_item_id(id))).collect();
            form.push((if add { "a" } else { "r" }.to_string(), tag.to_string()));
            form.push(("T".to_string(), token.clone()));
            self.request(Method::POST, "edit-tag", Some(&form)).await?;
        }
        Ok(())
    }
}

fn check_redirect(status: StatusCode) -> Result<()> {
    if status.is_redirection() {
        return Err(GReaderError::Redirect(status.as_u16()));
    }
    Ok(())
}

fn network(e: reqwest::Error) -> GReaderError {
    if e.is_timeout() {
        GReaderError::Timeout
    } else {
        let mut msg = e.to_string();
        // reqwest messages include the URL; keep only the host part readable.
        if let Some(url) = e.url() {
            msg = msg.replace(url.as_str(), url.host_str().unwrap_or(""));
        }
        GReaderError::Network(msg)
    }
}
