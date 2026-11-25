use super::{App, Desk, Update, usage_meters};
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};
use workshop::{
    provider::{self, AccountLimits},
    usage::UsageWindow,
};

struct Lookup {
    id: u64,
    started: Instant,
    pending: bool,
    live: Vec<UsageWindow>,
}

#[derive(Default)]
pub(super) struct AccountUsage {
    groups: AccountLimits,
    lookups: HashMap<&'static str, Lookup>,
    next_id: u64,
    provider: &'static str,
    group: String,
}

impl AccountUsage {
    pub(super) fn select(&mut self, source: &'static str, group: &str) {
        self.provider = source;
        self.group = group.to_owned();
    }

    pub(super) fn clear(&mut self) {
        self.groups.clear();
        self.lookups.clear();
    }

    fn begin(&mut self, source: &'static str) -> Option<u64> {
        if !matches!(source, "codex" | "claude" | "antigravity")
            || self
                .lookups
                .get(source)
                .is_some_and(|r| r.pending || r.started.elapsed() < Duration::from_secs(60))
        {
            return None;
        }
        self.next_id += 1;
        self.lookups.insert(
            source,
            Lookup {
                id: self.next_id,
                started: Instant::now(),
                pending: true,
                live: Vec::new(),
            },
        );
        Some(self.next_id)
    }

    pub(super) fn expire(&mut self, source: &str) {
        if self.lookups.get(source).is_some_and(|r| !r.pending) {
            self.lookups.remove(source);
        }
    }

    pub(super) fn finish(&mut self, source: &str, id: u64, result: Option<AccountLimits>) {
        let Some(lookup) = self.lookups.get_mut(source).filter(|r| r.id == id) else {
            return;
        };
        lookup.pending = false;
        if let Some(groups) = result {
            self.groups
                .retain(|key, _| key.split(':').next() != Some(source));
            self.groups.extend(groups);
            if !lookup.live.is_empty() {
                merge(
                    self.groups.entry(source.to_owned()).or_default(),
                    &lookup.live,
                );
            }
        }
        lookup.live.clear();
    }

    pub(super) fn observe(&mut self, source: &str, windows: Vec<UsageWindow>) {
        if let Some(r) = self.lookups.get_mut(source).filter(|r| r.pending) {
            merge(&mut r.live, &windows);
        }
        let current = self.groups.entry(source.to_owned()).or_default();
        if source == "claude" {
            merge(current, &windows);
        } else {
            *current = windows;
        }
    }

    pub(super) fn display(&self, ui: &Desk) {
        ui.set_usage_loading(self.lookups.get(self.provider).is_some_and(|r| r.pending));
        ui.set_usage_meters(usage_meters(
            self.groups.get(&self.group).cloned().unwrap_or_default(),
        ));
    }
}

// Claude may send only one window, or a status event without a percentage.
fn merge(current: &mut Vec<UsageWindow>, updates: &[UsageWindow]) {
    for update in updates {
        if let Some(existing) = current.iter_mut().find(|w| w.minutes == update.minutes) {
            existing.percent_left = update.percent_left.or(existing.percent_left);
        } else {
            current.push(update.clone());
        }
    }
    current.sort_by_key(|w| w.minutes);
}

impl App {
    pub(super) fn refresh_usage(&self, ui: &Desk, project: &Path) {
        let mut usage = self.usage.borrow_mut();
        let source = usage.provider;
        let request = usage.begin(source);
        usage.display(ui);
        if let Some(id) = request {
            let project = project.to_owned();
            let updates = self.updates.clone();
            std::thread::spawn(move || {
                let limits = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .ok()
                    .and_then(|rt| rt.block_on(provider::account_limits(source, &project)).ok());
                let _ = updates.send(Update::UsageSnapshot(source, id, limits));
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_lookups_and_partial_events_preserve_current_account_readings() {
        let mut usage = AccountUsage::default();
        let source = "claude";
        let old = usage.begin(source).unwrap();
        usage.clear(); // The user changed accounts while that lookup was running.
        let current = usage.begin(source).unwrap();
        let window = |minutes, percent_left| UsageWindow {
            minutes: Some(minutes),
            percent_left,
        };
        let snapshot = || {
            AccountLimits::from([(
                source.into(),
                vec![window(300, Some(80)), window(10080, Some(40))],
            )])
        };
        usage.finish(source, old, Some(snapshot()));
        assert!(usage.groups.is_empty());
        usage.observe(source, vec![window(300, Some(71))]);
        usage.finish(source, current, Some(snapshot()));
        usage.observe(source, vec![window(300, None)]);
        assert_eq!(
            usage.groups[source]
                .iter()
                .map(|w| w.percent_left)
                .collect::<Vec<_>>(),
            [Some(71), Some(40)]
        );
        assert!(usage.begin(source).is_none());
    }
}
