use crate::{Store, digest};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

pub struct ImageComparison {
    pub original: PathBuf,
    pub markup: Option<PathBuf>,
    pub previous_output: bool,
}

fn is_digest(value: &std::ffi::OsStr) -> bool {
    value
        .to_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
}

impl Store {
    // Include both versions in the key so identical marked pixels can have different originals.
    pub fn save_annotation(&self, original: &[u8], marked: &[u8]) -> Result<PathBuf> {
        let source = self.save_chat_image(original)?;
        if original == marked {
            return Ok(source);
        }
        let source_key = source
            .file_stem()
            .and_then(|name| name.to_str())
            .context("Invalid original image key")?;
        let key = digest(format!("{source_key}:{}", digest(marked)).as_bytes());
        let path = Self::write_image(self.dir.join("markup").join(format!("{key}.png")), marked)?;
        self.db.lock().unwrap().execute(
            "INSERT OR IGNORE INTO annotation_sources(attachment,original) VALUES(?1,?2)",
            params![
                path.to_str().context("Invalid annotation path")?,
                source.to_str().context("Invalid original path")?
            ],
        )?;
        Ok(path)
    }

    pub(super) fn is_chat_image(&self, path: &Path) -> bool {
        (path.parent() == Some(&self.dir.join("chat-images"))
            || path.parent() == Some(&self.dir.join("markup")))
            && path.extension().is_some_and(|e| e == "png")
            && path.file_stem().is_some_and(is_digest)
    }

    fn image_reference(&self, db: &Connection, path: &Path) -> Result<Option<ImageComparison>> {
        if !self.is_chat_image(path) {
            return Ok(None);
        }
        let source: Option<String> = if path.parent() == Some(&self.dir.join("markup")) {
            db.query_row(
                "SELECT original FROM annotation_sources WHERE attachment=?1",
                [path.to_str().context("Invalid annotation path")?],
                |r| r.get(0),
            )
            .optional()?
        } else {
            None
        };
        Ok(Some(ImageComparison {
            markup: source.as_ref().map(|_| path.to_owned()),
            original: source.map_or_else(|| path.to_owned(), PathBuf::from),
            previous_output: false,
        }))
    }

    pub(super) fn comparison_before(
        &self,
        db: &Connection,
        attempt: i64,
        event: i64,
    ) -> Result<Option<ImageComparison>> {
        let input: Option<Option<String>> = db
            .query_row(
                "SELECT i.path FROM events e LEFT JOIN event_images i ON i.event=e.id
             WHERE e.attempt=?1 AND e.id<?2 AND e.kind IN ('user/followup','steer/sent')
             ORDER BY e.id DESC LIMIT 1",
                params![attempt, event],
                |r| r.get(0),
            )
            .optional()?;
        let reference = match input {
            Some(Some(path)) => self.image_reference(db, Path::new(&path))?,
            Some(None) => None,
            None => {
                let mut q = db.prepare("SELECT path FROM task_attachments WHERE task=(SELECT task FROM attempts WHERE id=?1) ORDER BY rowid")?;
                let paths = q.query_map([attempt], |r| r.get::<_, String>(0))?;
                let mut reference = None;
                for path in paths {
                    reference = self.image_reference(db, Path::new(&path?))?;
                    if reference.is_some() {
                        break;
                    }
                }
                reference
            }
        };
        if reference.is_some() {
            return Ok(reference);
        }
        // Plain-text turns can still be compared by eye without claiming a revision relationship.
        let earlier: Option<String> = db
            .query_row(
                "SELECT i.path FROM events e JOIN event_images i ON i.event=e.id
             WHERE e.attempt=?1 AND e.id<?2 AND e.kind='workshop/image' ORDER BY e.id DESC LIMIT 1",
                params![attempt, event],
                |r| r.get(0),
            )
            .optional()
            .context("Read earlier chat image")?;
        Ok(earlier.map(|p| ImageComparison {
            original: p.into(),
            markup: None,
            previous_output: true,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Message;

    #[test]
    fn comparisons_follow_the_submitted_image_across_turns_and_restarts() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        let home = tmp.path().join("home");
        let store = Store::open(&project, &home)?;
        let first = store.save_annotation(b"original A", b"paint")?;
        let second = store.save_annotation(b"original B", b"paint")?;
        assert_ne!(first, second);
        let run =
            store.prepare_with_files("Revise", "", None, None, std::slice::from_ref(&first))?;
        let result = image::DynamicImage::new_rgba8(1, 1);
        store.post_chat_image(run.id, &result, "First")?;
        store.message_event(
            run.id,
            "steer/sent",
            &Message::compose("Use B", vec![second.clone()]),
        )?;
        store.post_chat_image(run.id, &result, "Second")?;
        store.message_event(run.id, "user/followup", &"One more variation".into())?;
        store.post_chat_image(run.id, &result, "Third")?;
        store.finish(run.id, "review", "Done")?;
        drop(store);
        let store = Store::open(&project, &home)?;
        let rows = store.messages_after(run.id, 0)?;
        let pair = |index: usize| rows[index].comparison.as_ref().unwrap();
        assert_eq!(std::fs::read(&pair(0).original)?, b"original A");
        assert_eq!(pair(0).markup.as_ref(), Some(&first));
        assert_eq!(std::fs::read(&pair(2).original)?, b"original B");
        assert_eq!(pair(2).markup.as_ref(), Some(&second));
        assert!(pair(4).markup.is_none() && pair(4).previous_output);
        assert_eq!(Some(&pair(4).original), rows[2].image.as_ref());
        let other = store.prepare("Another chat", "", None)?;
        store.post_chat_image(other.id, &result, "Independent")?;
        assert!(store.messages_after(other.id, 0)?[0].comparison.is_none());
        Ok(())
    }
}
