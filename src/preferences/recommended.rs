use super::{Provider, Row, checked, command, install_skill, plugins, skills};
use anyhow::{Context, Result, ensure};
use serde_json::Value;

struct Pick {
    key: &'static str,
    title: &'static str,
    publisher: &'static str,
    description: &'static str,
    skill_url: &'static str,
    claude: Option<Marketplace>,
}

struct Marketplace {
    plugin: &'static str,
    name: &'static str,
    repo: &'static str,
}

impl Marketplace {
    fn matches_source(&self, entry: &Value) -> bool {
        let repo = match entry["source"].as_str() {
            Some("github") => entry["repo"].as_str(),
            Some("git") => entry["url"].as_str().and_then(|url| {
                [
                    "https://github.com/",
                    "ssh://git@github.com/",
                    "git@github.com:",
                ]
                .iter()
                .find_map(|prefix| url.strip_prefix(prefix))
            }),
            _ => None,
        };
        repo.is_some_and(|r| {
            r.trim_end_matches('/')
                .trim_end_matches(".git")
                .eq_ignore_ascii_case(self.repo)
        })
    }
}

const PICKS: &[Pick] = &[
    Pick {
        key: "image-to-sculpt",
        title: "Image to Sculpt",
        publisher: "gokayfem",
        description: "Sculpt from reference art with Blender 5.2+ and Python 3.10+. Automatic depth maps and generated views use paid fal calls.",
        skill_url: "https://github.com/gokayfem/image-to-sculpt/tree/main/skills/image-to-sculpt",
        claude: Some(Marketplace {
            plugin: "image-to-sculpt@image-to-sculpt",
            name: "image-to-sculpt",
            repo: "gokayfem/image-to-sculpt",
        }),
    },
    Pick {
        key: "frontend-design",
        title: "Frontend Design",
        publisher: "Anthropic",
        description: "Give menus, settings and other interfaces a clear layout and visual style.",
        skill_url: "https://github.com/anthropics/skills/tree/main/skills/frontend-design",
        claude: Some(Marketplace {
            plugin: "frontend-design@claude-plugins-official",
            name: "claude-plugins-official",
            repo: "anthropics/claude-plugins-official",
        }),
    },
    Pick {
        key: "caveman",
        title: "Caveman",
        publisher: "JuliusBrussee",
        description: "Ask for terse agent replies during coding sessions. Say “normal mode” to switch back.",
        skill_url: "https://github.com/JuliusBrussee/caveman/tree/main/skills/caveman",
        claude: None,
    },
];

impl Pick {
    fn marketplace(&self, p: Provider) -> Option<&Marketplace> {
        self.claude.as_ref().filter(|_| p == Provider::Claude)
    }
}

pub(super) async fn list(p: Provider) -> Result<Vec<Row>> {
    let local = skills(p)?;
    let installed = if p == Provider::Claude {
        tokio::time::timeout(std::time::Duration::from_secs(15), plugins(p, false, false))
            .await
            .context("Claude plugin list timed out")
            .and_then(|result| result)
    } else {
        Ok(Vec::new())
    };
    Ok(PICKS
        .iter()
        .map(|pick| {
            let market = pick.marketplace(p);
            let skill_installed = local.iter().any(|row| row.label == pick.key);
            let plugin = installed.as_ref().ok().and_then(|rows| {
                rows.iter().find(|row| {
                    market.is_some_and(|m| m.plugin == row.id)
                        || (pick.key == "caveman" && row.id == "caveman@caveman")
                })
            });
            let (action, button) = if skill_installed || plugin.is_some() {
                ("", "Installed")
            } else if market.is_some() && installed.is_err() {
                ("", "Unavailable")
            } else if market.is_some() {
                ("recommend", "Install plugin")
            } else {
                ("recommend", "Install skill")
            };
            let mut detail = format!("{} · {}", pick.publisher, pick.description);
            if button == "Unavailable" {
                detail.push_str(" Check the Claude CLI, then refresh.");
            }
            Row {
                id: pick.key.into(),
                label: pick.title.into(),
                detail,
                action,
                button,
            }
        })
        .collect())
}

pub(super) async fn install(p: Provider, key: &str) -> Result<String> {
    let pick = PICKS
        .iter()
        .find(|pick| pick.key == key)
        .context("Unknown recommendation")?;
    if let Some(market) = pick.marketplace(p) {
        let configured =
            checked(command(p.program()?).args(["plugin", "marketplace", "list", "--json"]))
                .await?;
        let markets: Vec<Value> = serde_json::from_slice(&configured.stdout)
            .context("Could not read Claude marketplaces")?;
        if let Some(entry) = markets.iter().find(|m| m["name"] == market.name) {
            ensure!(
                market.matches_source(entry),
                "The configured {} marketplace has a different source. Expected {}.",
                market.name,
                market.repo
            );
        } else {
            checked(command(p.program()?).args(["plugin", "marketplace", "add", market.repo]))
                .await?;
        }
        checked(command(p.program()?).args(["plugin", "install", market.plugin])).await?;
    } else {
        install_skill(p, pick.skill_url).await?;
    }
    let manager = if pick.marketplace(p).is_some() {
        "Installed plugins"
    } else {
        "Skills"
    };
    Ok(format!(
        "{} installed for {}. Manage it in {manager}.",
        pick.title,
        p.label()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn existing_marketplace_matches_its_publisher_for_supported_claude_source_formats() {
        let market = PICKS[0].claude.as_ref().unwrap();
        assert!(
            market.matches_source(&json!({"source":"github","repo":"gokayfem/image-to-sculpt"}))
        );
        assert!(market.matches_source(
            &json!({"source":"git","url":"https://github.com/gokayfem/image-to-sculpt.git"})
        ));
        assert!(market.matches_source(
            &json!({"source":"git","url":"git@github.com:gokayfem/image-to-sculpt.git"})
        ));
        assert!(
            !market.matches_source(&json!({"source":"github","repo":"another/image-to-sculpt"}))
        );
    }
}
