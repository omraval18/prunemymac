//! Rule-driven junk scanner. Rules live in `rules/junk.toml` (embedded) and
//! can be extended by the user's own rules file.

use crate::fsutil::{self, Cancel};
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[cfg(target_os = "macos")]
const BUILTIN_RULES: &str = include_str!("../../../rules/junk.toml");
#[cfg(target_os = "linux")]
const BUILTIN_RULES: &str = include_str!("../../../rules/junk-linux.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Delete,
    Trash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Safety {
    Safe,
    Review,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub category: String,
    /// One plain-language sentence shown under the rule name.
    #[serde(default)]
    pub description: String,
    /// UI icon name (e.g. "hammer", "globe").
    #[serde(default)]
    pub icon: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    pub action: Action,
    pub safety: Safety,
    #[serde(default)]
    pub min_age_days: u64,
}

#[derive(Deserialize)]
struct RuleFile {
    #[serde(default)]
    rule: Vec<Rule>,
}

pub fn user_rules_path() -> PathBuf {
    #[cfg(target_os = "linux")]
    { return std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| fsutil::home().join(".config")).join("prunemymac/rules.toml"); }
    #[cfg(target_os = "macos")]
    fsutil::home().join("Library/Application Support/prunemymac/rules.toml")
}

/// Built-in rules, with user rules appended. A user rule whose `id` matches a
/// built-in one replaces it in place.
pub fn load_rules() -> Result<Vec<Rule>, String> {
    let mut rules = toml::from_str::<RuleFile>(BUILTIN_RULES)
        .map_err(|e| format!("built-in rules: {e}"))?
        .rule;
    if let Ok(text) = std::fs::read_to_string(user_rules_path()) {
        let user = toml::from_str::<RuleFile>(&text).map_err(|e| format!("user rules: {e}"))?;
        for r in user.rule {
            match rules.iter_mut().find(|b| b.id == r.id) {
                Some(slot) => *slot = r,
                None => rules.push(r),
            }
        }
    }
    Ok(rules)
}

#[derive(Debug, Clone)]
pub struct JunkItem {
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct JunkGroup {
    pub rule: Rule,
    pub items: Vec<JunkItem>,
}

impl JunkGroup {
    pub fn size(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }
}

fn expand_globs(patterns: &[String]) -> Vec<PathBuf> {
    patterns
        .iter()
        .flat_map(|p| glob::glob(&fsutil::expand(p)).into_iter().flatten().flatten())
        .collect()
}

fn old_enough(path: &Path, min_age_days: u64) -> bool {
    if min_age_days == 0 {
        return true;
    }
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let touched = meta.modified().ok().max(meta.accessed().ok());
    match touched.and_then(|t| SystemTime::now().duration_since(t).ok()) {
        Some(age) => age >= Duration::from_secs(min_age_days * 86_400),
        None => false,
    }
}

/// Scan every rule. `progress` is called with each rule name as it starts.
pub fn scan(rules: &[Rule], cancel: &Cancel, progress: &(dyn Fn(&str) + Sync)) -> Vec<JunkGroup> {
    // Claim paths rule by rule so overlapping globs are counted once.
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    let mut plan: Vec<(Rule, Vec<PathBuf>)> = Vec::new();
    for rule in rules {
        let excluded: HashSet<PathBuf> = expand_globs(&rule.exclude).into_iter().collect();
        let mut paths: Vec<PathBuf> = expand_globs(&rule.paths)
            .into_iter()
            .filter(|p| !excluded.contains(p))
            .filter(|p| !fsutil::is_protected(p))
            .filter(|p| !p.ancestors().any(|a| claimed.contains(a)))
            .filter(|p| !claimed.iter().any(|existing| existing.starts_with(p)))
            .collect();
        paths.sort();
        paths.dedup();
        claimed.extend(paths.iter().cloned());
        plan.push((rule.clone(), paths));
    }

    plan.into_iter()
        .filter_map(|(rule, paths)| {
            if cancel.is_cancelled() {
                return None;
            }
            progress(&rule.name);
            let mut items: Vec<JunkItem> = paths
                .par_iter()
                .filter(|p| old_enough(p, rule.min_age_days))
                .map(|p| JunkItem { size: fsutil::size_of(p, cancel), path: p.clone() })
                .filter(|i| i.size > 0)
                .collect();
            items.sort_by(|a, b| b.size.cmp(&a.size));
            (!items.is_empty()).then_some(JunkGroup { rule, items })
        })
        .collect()
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    #[test]
    fn generic_cache_rule_finds_review_only_cache_entries() {
        let rule = load_rules().unwrap().into_iter().find(|r| r.id == "other-user-caches").unwrap();
        assert_eq!(rule.safety, Safety::Review);
        let folder = fsutil::home().join(".cache").join(format!("prunemymac-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("cache"), b"cache data").unwrap();
        let mut fixture = rule;
        fixture.paths = vec![folder.to_string_lossy().into_owned()];
        let found = scan(&[fixture], &Cancel::new(), &|_| {});
        assert_eq!(found[0].items[0].path, folder);
        std::fs::remove_dir_all(folder).unwrap();
    }
}
