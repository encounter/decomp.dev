use std::path::{Path, PathBuf};

use url::Url;

pub trait UrlExt {
    fn query_param(&self, key: &str, value: Option<&str>) -> Url;
    fn with_path(&self, path: &str) -> Url;
    fn path_and_query(&self) -> &str;
}

impl UrlExt for Url {
    #[inline]
    fn query_param(&self, key: &str, value: Option<&str>) -> Url {
        let mut out = self.clone();
        let mut pairs = out.query_pairs_mut();
        pairs.clear();
        let mut updated = false;
        for (k, v) in self.query_pairs() {
            if k == key {
                if let Some(value) = value {
                    if value.is_empty() {
                        pairs.append_key_only(&k);
                    } else {
                        pairs.append_pair(&k, value);
                    }
                }
                updated = true;
            } else if v.is_empty() {
                pairs.append_key_only(&k);
            } else {
                pairs.append_pair(&k, &v);
            }
        }
        if !updated && let Some(value) = value {
            pairs.append_pair(key, value);
        }
        drop(pairs);
        if out.query() == Some("") {
            out.set_query(None);
        }
        out
    }

    #[inline]
    fn with_path(&self, path: &str) -> Url {
        let mut out = self.clone();
        out.set_path(path);
        out
    }

    #[inline]
    fn path_and_query(&self) -> &str { &self[url::Position::BeforePath..] }
}

/// Join two paths, only including the normal components.
pub fn join_normalized(base: impl AsRef<Path>, path: impl AsRef<Path>) -> PathBuf {
    let mut out = base.as_ref().to_path_buf();
    out.extend(path.as_ref().components().filter(|v| matches!(v, std::path::Component::Normal(_))));
    out
}

/// Format a size in bytes to a human-readable string.
/// Uses SI (kilo = 1000) units, formatted to two decimal places.
pub fn size(value: u64) -> String {
    let units = ["B", "kB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
    let mut value = value as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{:.2} {}", value, units[unit])
}
