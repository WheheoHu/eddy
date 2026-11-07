use std::path::PathBuf;
use std::sync::Arc;

use eddy_fetch::{Fetcher, RulePack, WebRenderer, parse_ftr};

use crate::engine::{Engine, Inner};
use crate::{EngineError, Result};

/// Summary of the private rule packs loaded on this device.
#[derive(Clone, Debug, PartialEq)]
pub struct RulePackSummary {
    pub packs: Vec<(String, u32)>,
    pub ftr_sites: u32,
}

impl Inner {
    pub(crate) fn rules_dir(&self) -> PathBuf {
        self.data_dir.join("rulepacks")
    }

    /// Loads every `*.toml` pack and `ftr/*.txt` site file from the private
    /// rule directory. Broken files are skipped and logged.
    pub(crate) fn load_rules(&self) -> (RulePack, RulePackSummary) {
        let mut pack = RulePack::default();
        let mut summary = RulePackSummary {
            packs: Vec::new(),
            ftr_sites: 0,
        };
        let dir = self.rules_dir();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut paths: Vec<_> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .collect();
            paths.sort();
            for path in paths {
                match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| RulePack::parse_toml(&t).map_err(|e| e.to_string()))
                {
                    Ok(p) => {
                        let name = if p.name.is_empty() {
                            file_stem(&path)
                        } else {
                            p.name.clone()
                        };
                        summary.packs.push((name, p.sites.len() as u32));
                        pack.merge(p);
                    }
                    Err(e) => self.platform.log(
                        crate::LogLevel::Warn,
                        &format!("skipping rule pack {}: {e}", file_stem(&path)),
                    ),
                }
            }
        }
        if let Ok(entries) = std::fs::read_dir(dir.join("ftr")) {
            for path in entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "txt"))
            {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    pack.sites.push(parse_ftr(&file_stem(&path), &text));
                    summary.ftr_sites += 1;
                }
            }
        }
        (pack, summary)
    }

    pub(crate) fn rebuild_fetcher(&self) -> Result<()> {
        let (pack, _) = self.load_rules();
        let renderer = self.web_renderer.read().clone();
        let fetcher =
            Fetcher::new(pack, renderer).map_err(|e| EngineError::Network(e.to_string()))?;
        *self.fetcher.write() = Arc::new(fetcher);
        Ok(())
    }
}

fn file_stem(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn safe_name(name: &str) -> Result<String> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.len() <= 120
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.starts_with("..");
    ok.then(|| name.to_string()).ok_or_else(|| {
        EngineError::Invalid("规则文件名只能包含字母、数字、点、短横线和下划线。".into())
    })
}

impl Engine {
    /// Validates and stores a private TOML rule pack, then reloads the
    /// fetcher. The pack stays in the local data directory only.
    pub fn import_rule_pack(&self, name: &str, contents: &str) -> Result<RulePackSummary> {
        RulePack::parse_toml(contents).map_err(|e| EngineError::Invalid(e.to_string()))?;
        let name = safe_name(name.trim_end_matches(".toml"))?;
        let dir = self.inner.rules_dir();
        std::fs::create_dir_all(&dir).map_err(|e| EngineError::Invalid(e.to_string()))?;
        std::fs::write(dir.join(format!("{name}.toml")), contents)
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
        self.inner.rebuild_fetcher()?;
        Ok(self.rule_packs())
    }

    /// Stores a Full-Text RSS site file for `domain` (`.example.com` for all
    /// subdomains).
    pub fn import_ftr_rule(&self, domain: &str, contents: &str) -> Result<RulePackSummary> {
        let domain = safe_name(domain.trim_end_matches(".txt"))?;
        let dir = self.inner.rules_dir().join("ftr");
        std::fs::create_dir_all(&dir).map_err(|e| EngineError::Invalid(e.to_string()))?;
        std::fs::write(dir.join(format!("{domain}.txt")), contents)
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
        self.inner.rebuild_fetcher()?;
        Ok(self.rule_packs())
    }

    pub fn remove_rule_pack(&self, name: &str) -> Result<RulePackSummary> {
        let name = safe_name(name)?;
        let _ = std::fs::remove_file(self.inner.rules_dir().join(format!("{name}.toml")));
        self.inner.rebuild_fetcher()?;
        Ok(self.rule_packs())
    }

    pub fn rule_packs(&self) -> RulePackSummary {
        self.inner.load_rules().1
    }

    /// Installs the platform WebView used by the `webview` strategy.
    pub fn set_web_renderer(&self, renderer: Option<Arc<dyn WebRenderer>>) -> Result<()> {
        *self.inner.web_renderer.write() = renderer;
        self.inner.rebuild_fetcher()
    }
}
