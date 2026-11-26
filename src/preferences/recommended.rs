use super::{
    Provider, Row, SKILL_SOURCE_FILE, checked, command, install_skill, plugins, skill_roots, skills,
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{fs, path::Path};

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
        key: "roblox-studio-workflow",
        title: "Roblox Studio Workflow",
        publisher: "gamedev-skills",
        description: "Edit a place or Rojo project and verify the change with the right Studio playtest.",
        skill_url: "https://github.com/gamedev-skills/awesome-gamedev-agent-skills/tree/main/skills/other-engines/roblox-studio-workflow",
        claude: None,
    },
    Pick {
        key: "roblox-luau",
        title: "Roblox Luau",
        publisher: "gamedev-skills",
        description: "Write typed Luau, organize modules and choose where client and server scripts run.",
        skill_url: "https://github.com/gamedev-skills/awesome-gamedev-agent-skills/tree/main/skills/other-engines/roblox-luau",
        claude: None,
    },
    Pick {
        key: "roblox-networking",
        title: "Roblox Networking",
        publisher: "gamedev-skills",
        description: "Build multiplayer features with RemoteEvents, server validation and two-client playtests.",
        skill_url: "https://github.com/gamedev-skills/awesome-gamedev-agent-skills/tree/main/skills/other-engines/roblox-networking",
        claude: None,
    },
    Pick {
        key: "roblox-ui",
        title: "Roblox UI",
        publisher: "gamedev-skills",
        description: "Build HUDs and menus in ScreenGui with safe areas, touch controls and gamepad focus.",
        skill_url: "https://github.com/gamedev-skills/awesome-gamedev-agent-skills/tree/main/skills/other-engines/roblox-ui",
        claude: None,
    },
    Pick {
        key: "game-ui-ux",
        title: "Game UI & UX",
        publisher: "gamedev-skills",
        description: "Design readable HUDs, inventory screens and menus across phones, controllers and wide displays.",
        skill_url: "https://github.com/gamedev-skills/awesome-gamedev-agent-skills/tree/main/skills/disciplines/game-ui-ux",
        claude: None,
    },
    Pick {
        key: "roblox-building",
        title: "Roblox Building",
        publisher: "TabooHarmony",
        description: "Model props and maps in Studio. Check scale, collisions and where players can go.",
        skill_url: "https://github.com/TabooHarmony/roblox-brain/tree/main/skills/gameplay/roblox-building",
        claude: None,
    },
    Pick {
        key: "3d-modeling",
        title: "3D Modeling",
        publisher: "omer-metin",
        description: "Plan game-ready meshes in Blender or Maya, unwrap UVs and build levels of detail for export.",
        skill_url: "https://github.com/omer-metin/skills-for-antigravity/tree/main/skills/3d-modeling",
        claude: None,
    },
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
        key: "prototype-fast",
        title: "Gameplay Prototyping",
        publisher: "gamedev-skills",
        description: "Build a playable version of one mechanic and playtest whether it’s fun before adding content.",
        skill_url: "https://github.com/gamedev-skills/awesome-gamedev-agent-skills/tree/main/skills/workflows/prototype-fast",
        claude: None,
    },
    Pick {
        key: "sound-effects",
        title: "ElevenLabs Sound Effects",
        publisher: "ElevenLabs",
        description: "Generate Foley, ambience and loops from text with your ElevenLabs API key. Uses ElevenLabs credits.",
        skill_url: "https://github.com/elevenlabs/skills/tree/main/sound-effects",
        claude: None,
    },
    Pick {
        key: "audio-design",
        title: "Game Audio Design",
        publisher: "gamedev-skills",
        description: "Vary repeated effects, balance loudness and duck music so gameplay cues stay audible.",
        skill_url: "https://github.com/gamedev-skills/awesome-gamedev-agent-skills/tree/main/skills/disciplines/audio-design",
        claude: None,
    },
    Pick {
        key: "game-feel",
        title: "Game Feel",
        publisher: "gamedev-skills",
        description: "Tune hit-stop, camera shake and animation timing so jumps, hits and pickups feel responsive.",
        skill_url: "https://github.com/gamedev-skills/awesome-gamedev-agent-skills/tree/main/skills/disciplines/game-feel",
        claude: None,
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
            let local_skill = local.iter().find(|row| row.label == pick.key);
            let plugin = installed.as_ref().ok().and_then(|rows| {
                rows.iter().find(|row| {
                    market.is_some_and(|m| m.plugin == row.id)
                        || (pick.key == "caveman" && row.id == "caveman@caveman")
                })
            });
            let (action, button) = if plugin.is_some() {
                ("", "Installed")
            } else if let Some(row) = local_skill {
                let source = fs::read_to_string(Path::new(&row.id).join(SKILL_SOURCE_FILE));
                if source.is_ok_and(|url| url == pick.skill_url) {
                    ("", "Installed")
                } else {
                    ("", "In Skills")
                }
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
            } else if button == "In Skills" {
                detail.push_str(&format!(
                    " Skills already contains a folder named {}.",
                    pick.key
                ));
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
        install_skill(pick.skill_url, &skill_roots(p)?[0]).await?;
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

    #[tokio::test]
    #[ignore = "downloads the published skill folders from GitHub"]
    async fn published_recommendations_install_under_their_catalog_keys() -> Result<()> {
        let root = tempfile::tempdir()?;
        for pick in PICKS {
            install_skill(pick.skill_url, root.path())
                .await
                .with_context(|| format!("Installing {}", pick.skill_url))?;
            let folder = root.path().join(pick.key);
            ensure!(
                folder.join("SKILL.md").is_file(),
                "Missing skill for {}",
                pick.title
            );
            assert_eq!(
                fs::read_to_string(folder.join(SKILL_SOURCE_FILE))?,
                pick.skill_url
            );
        }
        Ok(())
    }

    #[test]
    fn existing_marketplace_matches_its_publisher_for_supported_claude_source_formats() {
        let market = PICKS
            .iter()
            .find(|p| p.key == "image-to-sculpt")
            .unwrap()
            .claude
            .as_ref()
            .unwrap();
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
