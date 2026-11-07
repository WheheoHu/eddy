use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

use crate::FetchError;

/// Per-site fetching and extraction rules. Selectors are CSS (XPath from
/// Full-Text RSS files is translated on load).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SiteRule {
    pub domains: Vec<String>,
    pub strategies: Vec<String>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub cookies: BTreeMap<String, String>,
    pub body: Vec<String>,
    pub strip: Vec<String>,
    pub title: Option<String>,
    pub next_page: Option<String>,
    pub single_page: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct RulePack {
    pub name: String,
    pub version: u32,
    #[serde(rename = "site")]
    pub sites: Vec<SiteRule>,
}

impl RulePack {
    pub fn parse_toml(text: &str) -> Result<RulePack, FetchError> {
        let mut pack: RulePack =
            toml::from_str(text).map_err(|e| FetchError::Rules(e.to_string()))?;
        for site in &mut pack.sites {
            site.body = site.body.iter().filter_map(|s| to_css(s)).collect();
            site.strip = site.strip.iter().filter_map(|s| to_css(s)).collect();
            site.title = site.title.as_deref().and_then(to_css);
            site.next_page = site.next_page.as_deref().and_then(to_css);
            site.single_page = site.single_page.as_deref().and_then(to_css);
            site.domains.retain(|d| !d.trim().is_empty());
        }
        Ok(pack)
    }

    pub fn merge(&mut self, other: RulePack) {
        self.sites.extend(other.sites);
    }

    /// The most specific rule for `host`: exact domain beats a `.suffix`
    /// wildcard, longer suffixes beat shorter ones.
    pub fn rule_for(&self, host: &str) -> Option<&SiteRule> {
        let host = host.trim_start_matches("www.").to_lowercase();
        let mut best: Option<(usize, &SiteRule)> = None;
        for site in &self.sites {
            for domain in &site.domains {
                let d = domain.to_lowercase();
                let score = if let Some(suffix) = d.strip_prefix('.') {
                    (host.ends_with(&format!(".{suffix}"))).then_some(suffix.len())
                } else {
                    (host == d.trim_start_matches("www.")).then_some(d.len() + 1000)
                };
                if let Some(score) = score
                    && best.is_none_or(|b| score > b.0)
                {
                    best = Some((score, site));
                }
            }
        }
        best.map(|b| b.1)
    }
}

/// Parses a Full-Text RSS (ftr-site-config) file for `domain`. File names
/// starting with a dot apply to all subdomains, as in ftr.
pub fn parse_ftr(domain: &str, text: &str) -> SiteRule {
    let mut rule = SiteRule {
        domains: vec![domain.to_string()],
        ..Default::default()
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim().to_lowercase(), value.trim().to_string());
        match key.as_str() {
            "body" => rule.body.extend(to_css(&value)),
            "strip" => rule.strip.extend(to_css(&value)),
            "strip_id_or_class" => {
                let v = value.trim_matches(['"', '\'']);
                if !v.is_empty()
                    && v.chars()
                        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                {
                    rule.strip.push(format!("[id*=\"{v}\"], [class*=\"{v}\"]"));
                }
            }
            "title" => rule.title = rule.title.or_else(|| to_css(&value)),
            "next_page_link" => rule.next_page = rule.next_page.or_else(|| to_css(&value)),
            "single_page_link" => rule.single_page = rule.single_page.or_else(|| to_css(&value)),
            "http_header(user-agent)" => rule.user_agent = Some(value),
            "http_header(referer)" => rule.referer = Some(value),
            "http_header(cookie)" => {
                for pair in value.split(';') {
                    if let Some((k, v)) = pair.split_once('=') {
                        rule.cookies
                            .insert(k.trim().to_string(), v.trim().to_string());
                    }
                }
            }
            _ => {}
        }
    }
    if !rule.cookies.is_empty() || rule.user_agent.is_some() || rule.referer.is_some() {
        rule.strategies = vec!["headers".into(), "cookies".into()];
    }
    rule
}

static ATTR_EQ: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^@([\w:-]+)\s*=\s*['"]([^'"]*)['"]$"#).unwrap());
static CONTAINS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^contains\(\s*(?:concat\(\s*['"] ['"]\s*,\s*normalize-space\(\s*)?@([\w:-]+)\s*\)?\s*(?:,\s*['"] ['"]\s*\))?\s*,\s*['"]\s*([^'"]*?)\s*['"]\s*\)$"#)
        .unwrap()
});
static STARTS_WITH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^starts-with\(\s*@([\w:-]+)\s*,\s*['"]([^'"]*)['"]\s*\)$"#).unwrap()
});
static HAS_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^@([\w:-]+)$").unwrap());
static STEP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([\w*-]+)((?:\[[^\]]+\])*)$").unwrap());

/// Accepts a CSS selector as is, or translates the common XPath subset used
/// by site rules (`//tag[@attr='v']`, `contains(@class,'x')`, `//a//b`,
/// `//a/b`, unions). Returns `None` for anything else.
pub fn to_css(selector: &str) -> Option<String> {
    let s = selector.trim();
    if s.is_empty() {
        return None;
    }
    if !s.starts_with('/') && !s.starts_with('(') {
        return Some(s.to_string());
    }
    let parts: Option<Vec<String>> = s.split('|').map(|p| xpath_path(p.trim())).collect();
    parts.map(|p| p.join(", "))
}

fn xpath_path(path: &str) -> Option<String> {
    let path = path.trim_start_matches('(').trim_end_matches(')');
    if !path.starts_with('/') {
        return None;
    }
    let mut out = String::new();
    let mut rest = path;
    while !rest.is_empty() {
        let combinator = if let Some(r) = rest.strip_prefix("//") {
            rest = r;
            " "
        } else {
            let r = rest.strip_prefix('/')?;
            rest = r;
            " > "
        };
        let end = step_end(rest);
        let step = &rest[..end];
        rest = &rest[end..];
        let css = xpath_step(step)?;
        if out.is_empty() {
            out = css;
        } else {
            out.push_str(combinator);
            out.push_str(&css);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// End of the next location step: the first `/` outside brackets.
fn step_end(s: &str) -> usize {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            '/' if depth == 0 => return i,
            _ => {}
        }
    }
    s.len()
}

fn xpath_step(step: &str) -> Option<String> {
    let caps = STEP.captures(step)?;
    let tag = &caps[1];
    let mut css = if tag == "*" {
        String::new()
    } else {
        tag.to_string()
    };
    let preds = &caps[2];
    for pred in preds.split(']').filter(|p| !p.is_empty()) {
        let pred = pred.trim_start_matches('[').trim();
        for cond in pred.split(" and ") {
            let cond = cond.trim();
            if let Some(c) = ATTR_EQ.captures(cond) {
                css.push_str(&format!("[{}=\"{}\"]", &c[1], &c[2]));
            } else if let Some(c) = CONTAINS.captures(cond) {
                let value = c[2].trim();
                if c[1].eq_ignore_ascii_case("class") && !value.contains(' ') {
                    css.push_str(&format!(".{}", css_ident(value)?));
                } else {
                    css.push_str(&format!("[{}*=\"{value}\"]", &c[1]));
                }
            } else if let Some(c) = STARTS_WITH.captures(cond) {
                css.push_str(&format!("[{}^=\"{}\"]", &c[1], &c[2]));
            } else if let Some(c) = HAS_ATTR.captures(cond) {
                css.push_str(&format!("[{}]", &c[1]));
            } else if let Ok(n) = cond.parse::<u32>() {
                css.push_str(&format!(":nth-of-type({n})"));
            } else {
                return None;
            }
        }
    }
    if css.is_empty() {
        css.push('*');
    }
    Some(css)
}

fn css_ident(v: &str) -> Option<String> {
    let ok = !v.is_empty()
        && v.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    ok.then(|| v.to_string())
}
