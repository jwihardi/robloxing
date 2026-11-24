use crate::Store;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub(super) const FORMAT: i64 = 1;
const PAGE: usize = 16 * 1024;
const HISTORY_PAGE: usize = 20;
fn folder(dir: &Path, id: i64) -> PathBuf {
    dir.join("studio").join(id.to_string())
}

pub(super) fn log(db: &Connection, dir: &Path, message: &str) -> Result<()> {
    let time: String = db.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')", [], |r| {
        r.get(0)
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("activity.log"))?;
    writeln!(file, "{time}  {message}")?;
    Ok(())
}

pub(super) fn fingerprint(input: impl Read, mut output: impl Write) -> Result<String> {
    let mut reader = BufReader::with_capacity(64 * 1024, input);
    let mut digest = Sha256::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        digest.update(chunk);
        output.write_all(chunk)?;
        let n = chunk.len();
        reader.consume(n);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn reuse(
    store: &Store,
    id: i64,
    dir: &Path,
    input_sha: &str,
    filter: &str,
) -> Result<Option<String>> {
    let candidates = {
        let db = store.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT r.job,r.report_sha,coalesce(r.reused_from,r.job)
            FROM decompile_records r JOIN studio_jobs j ON j.id=r.job
            WHERE r.input_sha=?1 AND r.filter_key=?2 AND r.format=?3 AND j.state='Ready'
            ORDER BY r.reused_from IS NOT NULL,r.job DESC",
        )?;
        stmt.query_map(params![input_sha, filter, FORMAT], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (source, expected, origin) in candidates {
        let path = folder(&store.dir, source).join("functions.c");
        let mut copy = tempfile::NamedTempFile::new_in(dir)?;
        if let Ok(file) = File::open(path)
            && fingerprint(file, &mut copy)? == expected
        {
            copy.persist(dir.join("functions.c"))?;
            let db = store.db.lock().unwrap();
            db.execute("UPDATE decompile_records SET symbols=(SELECT symbols FROM decompile_records WHERE job=?2),report_sha=?3,reused_from=?4 WHERE job=?1",
                params![id,source,expected,origin])?;
            log(
                &db,
                dir,
                &format!(
                    "Verified the archived report from run #{source}. Original analysis: run #{origin}."
                ),
            )?;
            let summary: String =
                db.query_row("SELECT detail FROM studio_jobs WHERE id=?", [origin], |r| {
                    r.get(0)
                })?;
            return Ok(Some(format!("Reused run #{origin} · {summary}")));
        }
        log(
            &store.db.lock().unwrap(),
            dir,
            &format!("Skipped run #{source}: saved report missing or checksum changed."),
        )?;
    }
    Ok(None)
}

// Older reports already contain entry-point comments. Recover their symbol locations once.
fn legacy_symbols(path: &Path) -> Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut table = String::new();
    let mut line = String::new();
    let mut offset = 0;
    let mut pending = None;
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let s = line.trim();
        if let Some(address) = s.strip_prefix("/* ").and_then(|v| v.strip_suffix(" */"))
            && address
                .rsplit(':')
                .next()
                .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            pending = Some((offset, address.to_string()));
        } else if let Some((prefix, _)) = s.split_once('(')
            && !s.starts_with(['/', '*'])
            && let Some(name) = prefix.split_whitespace().last()
            && let Some((start, address)) = pending.take()
        {
            table.push_str(&format!(
                "{start}\t{address}\t{}\n",
                name.trim_start_matches('*')
            ));
        }
        offset += n;
    }
    Ok(table)
}

fn backfill(dir: &Path, db: &mut Connection) -> Result<()> {
    let mut stmt = db.prepare("SELECT id FROM studio_jobs WHERE area='Ghidra' AND state='Ready' AND id NOT IN(SELECT job FROM decompile_records)")?;
    let ids = stmt
        .query_map([], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    let tx = db.transaction()?;
    for id in ids {
        // Missing or unreadable historical artifacts must not prevent the project from opening.
        let symbols = legacy_symbols(&folder(dir, id).join("functions.c")).unwrap_or_default();
        tx.execute(
            "INSERT INTO decompile_records(job,symbols) VALUES(?1,?2)",
            params![id, symbols],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn recover(dir: &Path, db: &mut Connection) -> Result<()> {
    let mut stmt = db.prepare(
        "SELECT id FROM studio_jobs WHERE area='Ghidra' AND state IN ('Queued','Running')",
    )?;
    for id in stmt.query_map([], |r| r.get::<_, i64>(0))? {
        let job = folder(dir, id?);
        fs::create_dir_all(&job)?;
        log(
            db,
            &job,
            "Interrupted · Workshop closed before this run finished.",
        )?;
    }
    drop(stmt);
    backfill(dir, db)
}

#[derive(Serialize, Debug)]
pub struct Report {
    pub id: i64,
    pub binary: String,
    pub filter: String,
    pub created: String,
    pub state: String,
    pub summary: String,
    pub sha256: Option<String>,
    pub reused_from: Option<i64>,
    pub indexed_functions: usize,
    pub functions: Vec<Symbol>,
}
#[derive(Serialize, Debug)]
pub struct Symbol {
    pub offset: u64,
    pub address: String,
    pub name: String,
}
impl Symbol {
    fn parse(line: &str) -> Option<Self> {
        let mut fields = line.splitn(3, '\t');
        Some(Self {
            offset: fields.next()?.parse().ok()?,
            address: fields.next()?.into(),
            name: fields.next()?.into(),
        })
    }
}
#[derive(Serialize)]
pub struct Matches {
    pub reports: Vec<Report>,
    pub next_before: Option<i64>,
}
#[derive(Serialize)]
pub struct Page {
    pub id: i64,
    pub source: String,
    pub text: String,
    pub next_offset: Option<u64>,
}

pub struct Archive {
    dir: PathBuf,
    db: Connection,
}
impl Store {
    pub fn report_archive(&self) -> Result<Archive> {
        Archive::open(&self.archive_dir)
    }
}
impl Archive {
    pub fn project(&self) -> Result<String> {
        Ok(self
            .db
            .query_row("SELECT path FROM projects WHERE id=1", [], |r| r.get(0))?)
    }
    pub fn open(dir: &Path) -> Result<Self> {
        let mut db = Connection::open_with_flags(
            dir.join("workshop.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .context("Could not open this project's report history.")?;
        let indexed: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='decompile_records')",
            [],
            |r| r.get(0),
        )?;
        if !indexed {
            db.execute_batch("CREATE TEMP TABLE decompile_records(job INTEGER PRIMARY KEY,symbols TEXT DEFAULT '',input_sha TEXT DEFAULT '',reused_from INTEGER)")?;
            backfill(dir, &mut db)?;
        }
        Ok(Self {
            dir: dir.to_owned(),
            db,
        })
    }

    pub fn search(&self, query: &str, before: Option<i64>) -> Result<Matches> {
        let mut stmt=self.db.prepare("SELECT j.id,j.title,json_extract(j.request,'$.filter'),datetime(j.created,'localtime'),j.state,substr(j.detail,1,800),nullif(r.input_sha,''),r.reused_from,coalesce(r.symbols,'')
            FROM studio_jobs j LEFT JOIN decompile_records r ON r.job=j.id
            WHERE j.area='Ghidra' AND (?2 IS NULL OR j.id<?2) AND (instr(lower(j.title),lower(?1))>0
            OR instr(lower(json_extract(j.request,'$.filter')),lower(?1))>0 OR instr(lower(r.symbols),lower(?1))>0 OR instr(r.input_sha,lower(?1))>0)
            ORDER BY j.id DESC LIMIT ?3")?;
        let needle = query.to_lowercase();
        let mut reports = stmt
            .query_map(params![query, before, HISTORY_PAGE + 1], |row| {
                let symbols: String = row.get(8)?;
                let functions = symbols
                    .lines()
                    .filter(|s| s.to_lowercase().contains(&needle))
                    .take(8)
                    .filter_map(Symbol::parse)
                    .collect();
                Ok(Report {
                    id: row.get(0)?,
                    binary: row.get(1)?,
                    filter: row.get(2)?,
                    created: row.get(3)?,
                    state: row.get(4)?,
                    summary: row.get(5)?,
                    sha256: row.get(6)?,
                    reused_from: row.get(7)?,
                    indexed_functions: symbols.lines().count(),
                    functions,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = reports.len() > HISTORY_PAGE;
        reports.truncate(HISTORY_PAGE);
        let next_before = more.then(|| reports.last().unwrap().id);
        Ok(Matches {
            reports,
            next_before,
        })
    }

    pub fn read(&self, id: i64, part: &str, offset: u64) -> Result<Page> {
        let (state, detail): (String, String) = self
            .db
            .query_row(
                "SELECT state,detail FROM studio_jobs WHERE id=? AND area='Ghidra'",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .context("No such decompilation in this project.")?;
        let dir = folder(&self.dir, id);
        if part == "functions" {
            let index: String = self
                .db
                .query_row(
                    "SELECT symbols FROM decompile_records WHERE job=?",
                    [id],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_default();
            return read_page(
                std::io::Cursor::new(index.into_bytes()),
                id,
                "Function index (byte offset, address, name)",
                offset,
            );
        }
        let source = match part {
            "report" => {
                ensure!(
                    state == "Ready",
                    "Run {id} is {state}. Read its log for details."
                );
                "functions.c"
            }
            "log" if dir.join("activity.log").exists() => "activity.log",
            "log" if dir.join("tool.log").exists() => "tool.log",
            "log" => {
                return Ok(Page {
                    id,
                    source: "Run status".into(),
                    text: format!("{state}\n{detail}"),
                    next_offset: None,
                });
            }
            "tool-log" => "tool.log",
            _ => anyhow::bail!("Choose report, functions, log or tool-log."),
        };
        let file = File::open(dir.join(source))
            .with_context(|| format!("Run {id} has no saved {source}."))?;
        read_page(file, id, source, offset)
    }
}

fn read_page(mut file: impl Read + Seek, id: i64, source: &str, offset: u64) -> Result<Page> {
    ensure!(
        offset <= file.seek(SeekFrom::End(0))?,
        "Offset is beyond the end of the file."
    );
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(PAGE as u64 + 1).read_to_end(&mut bytes)?;
    let next_offset = if bytes.len() > PAGE {
        let mut end = PAGE;
        while end > 0 && bytes[end] & 0xc0 == 0x80 {
            end -= 1;
        }
        if end == 0 {
            end = PAGE;
        }
        bytes.truncate(end);
        Some(offset + end as u64)
    } else {
        None
    };
    Ok(Page {
        id,
        source: source.into(),
        text: String::from_utf8_lossy(&bytes).into_owned(),
        next_offset,
    })
}

pub fn log_tail(job: &super::Job, store: &Store) -> Result<String> {
    let dir = job.folder(store);
    let file = ["activity.log", "tool.log"]
        .iter()
        .map(|p| dir.join(p))
        .find(|p| p.is_file());
    let mut text = format!("Run #{} · {} · {}\n", job.id, job.created, job.state);
    if let super::Recipe::Decompile { binary, filter } = &job.recipe
        && !dir.join("activity.log").is_file()
    {
        text.push_str(&format!(
            "{}\nFilter: {}\n\n",
            binary.display(),
            if filter.is_empty() {
                "All functions"
            } else {
                filter
            }
        ));
    }
    if let Some(path) = file {
        let file = File::open(path)?;
        let start = file.metadata()?.len().saturating_sub(PAGE as u64);
        if start > 0 {
            text.push_str("… earlier entries in the saved log …\n");
        }
        text.push_str(&read_page(file, job.id, "log", start)?.text);
    } else {
        text.push_str(&job.detail);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_archived_bytes_and_can_reuse_an_intact_copy() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = Store::open(&tmp.path().join("project"), &tmp.path().join("data"))?;
        let binary = store.project.join("tile.o");
        fs::write(&binary, b"version one")?;
        let recipe = super::super::Recipe::Decompile {
            binary,
            filter: "Tile".into(),
        };
        let input = crate::digest(b"version one");
        let code = b"/* 1000 */\nint Tile() { return 7; }";
        let sha = crate::digest(code);
        let original = store.start_studio_job(&recipe)?;
        let first = folder(&store.dir, original);
        fs::create_dir_all(&first)?;
        fs::write(first.join("functions.c"), code)?;
        {
            let db = store.db.lock().unwrap();
            db.execute(
                "UPDATE studio_jobs SET state='Ready',detail='1 function' WHERE id=?",
                [original],
            )?;
            db.execute("INSERT INTO decompile_records(job,input_sha,filter_key,format,report_sha,symbols) VALUES(?1,?2,'tile',?3,?4,'0'||char(9)||'1000'||char(9)||'Tile')",
                params![original,input,FORMAT,sha])?;
        }
        let run = || -> Result<i64> {
            let id = store.start_studio_job(&recipe)?;
            let (_hold, cancel) = tokio::sync::oneshot::channel();
            store.run_studio_job(id, cancel)?;
            assert_eq!(fs::read(folder(&store.dir, id).join("functions.c"))?, code);
            assert!(!folder(&store.dir, id).join("tool.log").exists());
            Ok(id)
        };
        let copy = run()?;
        fs::write(first.join("functions.c"), b"edited")?;
        let next = run()?;
        let archive = store.report_archive()?;
        assert!(
            archive
                .read(next, "log", 0)?
                .text
                .contains(&format!("Verified the archived report from run #{copy}"))
        );
        assert_eq!(
            archive.search("Tile", None)?.reports[0].reused_from,
            Some(original)
        );
        let dest = tempfile::tempdir()?;
        assert!(reuse(&store, next, dest.path(), &input, "other")?.is_none());
        assert!(
            reuse(
                &store,
                next,
                dest.path(),
                &crate::digest(b"version two"),
                "tile"
            )?
            .is_none()
        );
        Ok(())
    }
}
