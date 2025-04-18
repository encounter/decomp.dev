use std::{borrow::Cow, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use minijinja::{path_loader, Environment};
use minijinja_autoreload::AutoReloader;
use time::{format_description, macros::format_description, UtcDateTime};

pub type Templates = Arc<AutoReloader>;

pub fn create(template_path: impl Into<String>) -> Templates {
    let template_path = template_path.into();
    Arc::new(AutoReloader::new(move |notifier| {
        let mut env = Environment::new();
        let template_path = template_path.as_str();
        notifier.watch_path(template_path, true);
        env.set_loader(path_loader(template_path));
        env.set_trim_blocks(true);
        env.set_lstrip_blocks(true);
        env.add_filter("date", date);
        env.add_filter("timeago", timeago);
        env.add_filter("size", size);
        Ok(env)
    }))
}

pub fn render<S>(templates: &Templates, template_name: &str, context: S) -> Result<String>
where S: serde::Serialize {
    let env = templates.acquire_env().context("Failed to get template environment")?;
    let template = env.get_template(template_name).context("Failed to get template")?;
    template.render(context).context("Failed to render template")
}

fn timeago(value: String) -> String {
    let Ok(value) = serde_json::from_str::<UtcDateTime>(&value) else {
        return format!("[invalid {}]", value);
    };
    let Ok(duration) = Duration::try_from(UtcDateTime::now() - value) else {
        return "[out of range]".to_string();
    };
    timeago::Formatter::new().convert(duration)
}

fn date(value: String, format: Option<String>) -> String {
    let Ok(value) = serde_json::from_str::<UtcDateTime>(&value) else {
        return format!("[invalid {}]", value);
    };
    let format = if let Some(format) = format.as_deref() {
        match format_description::parse_borrowed::<2>(format) {
            Ok(format) => Cow::Owned(format),
            Err(_) => return format!("[invalid format {}]", format),
        }
    } else {
        Cow::Borrowed(format_description!("[year]-[month]-[day] [hour]:[minute]:[second] [offset_hour sign:mandatory]:[offset_minute]"))
    };
    value.format(&format).unwrap_or_else(|_| "[invalid]".to_string())
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
