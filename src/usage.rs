use crate::{Attempt, Store};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct ContextUsage {
    pub used: Option<i64>,
    pub capacity: Option<i64>,
}

impl ContextUsage {
    pub(crate) fn from_report(report: &Value) -> Self {
        Self {
            used: report["last"]["totalTokens"].as_i64().filter(|n| *n >= 0),
            capacity: report["modelContextWindow"].as_i64().filter(|n| *n > 0),
        }
    }

    pub fn fraction_left(self) -> Option<f64> {
        Some((1.0 - self.used? as f64 / self.capacity? as f64).clamp(0.0, 1.0))
    }
}

#[derive(Clone, Debug)]
pub struct UsageWindow {
    pub minutes: Option<i64>,
    pub percent_left: Option<i64>,
}

pub(crate) fn account_windows(snapshot: &Value) -> Option<Vec<UsageWindow>> {
    if !snapshot.is_object() || snapshot["limitId"].as_str().is_some_and(|id| id != "codex") {
        return None;
    }
    Some(
        ["primary", "secondary"]
            .iter()
            .filter_map(|key| {
                let window = &snapshot[*key];
                Some(UsageWindow {
                    minutes: window["windowDurationMins"].as_i64().filter(|n| *n > 0),
                    percent_left: Some(100 - window["usedPercent"].as_i64()?.clamp(0, 100)),
                })
            })
            .collect(),
    )
}

pub(crate) fn claude_window(info: &Value) -> Option<UsageWindow> {
    let minutes = match info["rateLimitType"].as_str()? {
        "five_hour" => 300,
        "seven_day" => 10080,
        _ => return None,
    };
    // An "allowed" event often omits utilization, even on a paid subscription.
    let percent_left = if let Some(used) = info["utilization"].as_f64() {
        Some(100 - (used.clamp(0.0, 1.0) * 100.0).round() as i64)
    } else if info["status"] == "rejected" {
        Some(0)
    } else {
        None
    };
    Some(UsageWindow {
        minutes: Some(minutes),
        percent_left,
    })
}

impl Store {
    pub(crate) fn record_usage(&self, id: i64, total: i64, context: ContextUsage) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute("INSERT INTO usage(attempt,tokens) VALUES(?1,?2) ON CONFLICT(attempt) DO UPDATE SET tokens=excluded.tokens", params![id,total])?;
        tx.execute(
            "INSERT INTO context_usage(attempt,used,capacity) VALUES(?1,?2,?3)
             ON CONFLICT(attempt) DO UPDATE SET used=excluded.used,capacity=excluded.capacity",
            params![id, context.used, context.capacity],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn context_usage(&self, chat: &Attempt) -> Result<ContextUsage> {
        let saved = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT used,capacity FROM context_usage WHERE attempt=?1",
                [chat.id],
                |r| {
                    Ok(ContextUsage {
                        used: r.get(0)?,
                        capacity: r.get(1)?,
                    })
                },
            )
            .optional()?;
        if let Some(saved) = saved {
            return Ok(saved);
        }
        if self.usage(chat.id)?.is_none() {
            return Ok(ContextUsage::default());
        }
        // Older versions only indexed cumulative tokens. Recover the latest report from
        // the tail of the chat's log without scanning a long transcript on the UI thread.
        let mut context = ContextUsage::default();
        if let Ok(mut file) = File::open(self.folder(chat.id).join("events.jsonl")) {
            let start = file.metadata()?.len().saturating_sub(256 * 1024);
            file.seek(SeekFrom::Start(start))?;
            let mut tail = Vec::new();
            file.take(256 * 1024).read_to_end(&mut tail)?;
            for line in tail.rsplit(|b| *b == b'\n') {
                if !line
                    .windows(b"thread/tokenUsage/updated".len())
                    .any(|w| w == b"thread/tokenUsage/updated")
                {
                    continue;
                }
                if let Ok(v) = serde_json::from_slice::<Value>(line)
                    && v["method"] == "thread/tokenUsage/updated"
                    && v["params"]["threadId"].as_str() == Some(chat.session.as_str())
                {
                    context = ContextUsage::from_report(&v["params"]["tokenUsage"]);
                    break;
                }
            }
        }
        // A live notification may have arrived while we were reading the old log.
        self.db.lock().unwrap().execute(
            "INSERT OR IGNORE INTO context_usage(attempt,used,capacity) VALUES(?1,?2,?3)",
            params![chat.id, context.used, context.capacity],
        )?;
        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn context_tracks_latest_request_even_when_cumulative_usage_grows() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        let data = temp.path().join("data");
        let store = Store::open(&project, &data)?;
        let chat = store.prepare("hello", "", None)?;
        for (total, last) in [(80, 80), (105, 25)] {
            let report = json!({"total":{"totalTokens":total},"last":{"totalTokens":last},"modelContextWindow":100});
            store.record_usage(chat.id, total, ContextUsage::from_report(&report))?;
        }
        drop(store);
        let store = Store::open(&project, &data)?;
        assert_eq!(store.context_usage(&chat)?.fraction_left(), Some(0.75));
        assert_eq!(store.usage(chat.id)?, Some(105));
        assert_eq!(
            ContextUsage::from_report(&json!({"total":{"totalTokens":105}})).fraction_left(),
            None
        );
        assert_eq!(
            ContextUsage::from_report(
                &json!({"last":{"totalTokens":105},"modelContextWindow":100})
            )
            .fraction_left(),
            Some(0.0)
        );
        Ok(())
    }

    #[test]
    fn claude_can_report_a_window_without_a_remaining_percentage() {
        let mut report = json!({"rateLimitType":"five_hour","status":"allowed"});
        let window = claude_window(&report).unwrap();
        assert_eq!((window.minutes, window.percent_left), (Some(300), None));
        report["utilization"] = json!(0.32);
        assert_eq!(claude_window(&report).unwrap().percent_left, Some(68));
    }

    #[test]
    fn account_usage_keeps_windows_separate_and_ignores_other_buckets() {
        let mut snapshot = json!({"limitId":"codex","primary":{"usedPercent":100,"windowDurationMins":300},"secondary":{"usedPercent":17,"windowDurationMins":10080}});
        let windows = account_windows(&snapshot).unwrap();
        assert_eq!(
            (windows[0].minutes, windows[0].percent_left),
            (Some(300), Some(0))
        );
        assert_eq!(
            (windows[1].minutes, windows[1].percent_left),
            (Some(10080), Some(83))
        );
        snapshot["limitId"] = json!("other");
        assert!(account_windows(&snapshot).is_none());
    }
}
