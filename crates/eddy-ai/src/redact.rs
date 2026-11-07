use std::sync::LazyLock;

use regex::Regex;

static BEARER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)Bearer\s+[^\s,;"<>]+"#).unwrap());
static SK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bsk-[a-z0-9_-]+").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<title\b[^>]*>(.*?)</title>").unwrap());
static RAY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[a-f0-9]{16,32}(?:-[a-z]{3})?$").unwrap());

fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20")
}

/// Provider text made safe to show: configured secrets (raw, URL-encoded and
/// JSON-escaped), bearer tokens, `sk-` keys and HTML are removed, control
/// characters collapsed, length capped.
pub fn safe_detail(value: &str, secrets: &[&str]) -> String {
    let mut text = value.to_string();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        let json = serde_json::to_string(secret).unwrap_or_default();
        let json = json.trim_matches('"').to_string();
        for encoded in [secret.to_string(), url_encode(secret), json] {
            if !encoded.is_empty() {
                text = text.replace(&encoded, "[已隐藏]");
            }
        }
    }
    let text = BEARER.replace_all(&text, "Bearer [已隐藏]");
    let text = SK.replace_all(&text, "[已隐藏]");
    let text = TAG.replace_all(&text, "");
    let text: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    text.chars().take(450).collect()
}

/// A readable diagnosis of a failed model request that never echoes request
/// headers, cookies or credentials.
pub fn model_failure(status: u16, content_type: &str, raw: &str, secrets: &[&str], headers: Option<&reqwest::header::HeaderMap>) -> String {
    let prefix = format!("模型服务返回 HTTP {status}。");
    let looks_html = content_type.contains("text/html") || {
        let t = raw.trim_start().to_lowercase();
        t.starts_with("<!doctype") || t.starts_with("<html") || t.starts_with("<head") || t.starts_with("<body")
    };
    let header = |name: &str| headers.and_then(|h| h.get(name)).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    if looks_html {
        // Only a short page title and a request ID, never the page body:
        // gateway pages can echo request headers or credentials.
        let title: String = TITLE.captures(raw).map(|c| safe_detail(&c[1], secrets)).unwrap_or_default().chars().take(160).collect();
        let ray = header("cf-ray");
        let ray_detail = if RAY.is_match(&ray) { format!(" Cloudflare Ray ID：{ray}。") } else { String::new() };
        let hint = if header("cf-mitigated") == "challenge" {
            "网关要求 Cloudflare 浏览器验证，应用里的 API 请求无法完成此验证。请联系网关管理员检查 API 路径的访问规则。"
        } else {
            "收到的是网页，未取得 JSON API 响应。请检查 API 前缀（通常为 /v1）或网关访问策略。"
        };
        let title = if title.is_empty() { String::new() } else { format!(" 网关页面：{title}。") };
        return format!("{prefix}{hint}{title}{ray_detail}");
    }
    let (mut message, mut code) = (String::new(), String::new());
    if let Ok(data) = serde_json::from_str::<serde_json::Value>(raw) {
        let m = data["error"]["message"].as_str().or(data["message"].as_str()).or(data["error"].as_str()).unwrap_or("");
        message = safe_detail(m, secrets);
        code = match &data["error"]["code"] {
            serde_json::Value::String(s) => safe_detail(s, secrets),
            serde_json::Value::Number(n) => n.to_string(),
            _ => String::new(),
        };
    }
    let hint = match status {
        401 => "API Key 未被接受，请检查是否正确、过期或已被撤销。",
        403 => "服务拒绝访问，请检查该 Key 的模型权限、IP 白名单或网关访问策略。",
        404 => "未找到 API 路径或模型，请检查完整 API Base URL 和模型名称。",
        429 => "请求被限流或配额不足，请按服务端原因处理。",
        402 => "服务要求检查余额或计费状态。",
        s if s >= 500 => "模型网关或上游服务异常，请稍后重试。",
        _ => "请求未被接受，请检查模型名称与请求参数。",
    };
    let mut out = format!("{prefix}{hint}");
    if !message.is_empty() {
        out.push_str(&format!(" 服务端原因：{message}"));
    }
    if !code.is_empty() && code != message {
        out.push_str(&format!("（{code}）"));
    }
    out
}
