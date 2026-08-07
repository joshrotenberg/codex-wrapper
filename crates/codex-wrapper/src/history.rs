//! Read-side access to the CLI's on-disk session logs.
//!
//! Read-only. Mutation goes through the CLI, via
//! [`ArchiveCommand`](crate::ArchiveCommand) and
//! [`DeleteCommand`](crate::DeleteCommand).
//!
//! # On-disk layout
//!
//! ```text
//! $CODEX_HOME/sessions/<YYYY>/<MM>/<DD>/rollout-<ISO8601>-<uuid>.jsonl
//! ```
//!
//! The date partitioning is why [`SessionQuery::after`] and
//! [`SessionQuery::before`] are cheap: they filter directories, without opening
//! a file.
//!
//! # Two envelope generations
//!
//! A real machine holds sessions written by many CLI versions, and the line
//! format changed. Both were found on one machine while writing this, 205
//! files spanning both:
//!
//! **Modern**, from around 0.47 onward. Every line is an envelope:
//!
//! ```json
//! {"timestamp":"...","type":"session_meta","payload":{"id":"...","cwd":"..."}}
//! {"timestamp":"...","type":"response_item","payload":{"type":"message","role":"user"}}
//! ```
//!
//! **Legacy**, older files. No envelope at all: the first line *is* the
//! metadata, and later lines are bare records.
//!
//! ```json
//! {"id":"...","timestamp":"...","git":{},"instructions":null}
//! {"id":"...","type":"message","role":"user","content":[]}
//! ```
//!
//! A parser written against only the modern shape returns nothing at all for
//! the older half of a real history, silently. [`SessionEntry::entry_type`] is
//! `None` for a legacy line, and its `payload` is the whole line.
//!
//! Field-level drift is handled the same way: every metadata field is
//! optional, because older files carry no `cli_version`, no `cwd`, and a
//! different `instructions` key. [`SessionMeta::raw`] keeps whatever this
//! crate does not name.
//!
//! # Example
//!
//! ```no_run
//! use codex_wrapper::history::{self, SessionQuery};
//!
//! # fn example() -> codex_wrapper::Result<()> {
//! for session in history::list(&SessionQuery::new().after(2026, 8, 1))? {
//!     let log = history::read(&session.path)?;
//!     println!("{} in {:?}", session.id, log.meta.and_then(|m| m.cwd));
//! }
//! # Ok(())
//! # }
//! ```

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// A rollout file on disk, identified without opening it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFile {
    /// Full path to the `.jsonl` file.
    pub path: PathBuf,
    /// The session id, taken from the filename.
    ///
    /// This is the `thread_id` a resume takes.
    pub id: String,
    /// The date directory this file sits in, as `(year, month, day)`.
    pub date: (u16, u8, u8),
}

/// Git metadata recorded with a session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitMeta {
    /// Commit the working tree was on.
    pub commit_hash: Option<String>,
    /// Branch name.
    pub branch: Option<String>,
    /// Remote URL.
    pub repository_url: Option<String>,
}

/// The session's opening metadata.
///
/// Every field is optional. Older files carry a different and smaller set, and
/// a reader that required any one of them would fail on a real history.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct SessionMeta {
    /// Session id as recorded inside the file.
    pub id: Option<String>,
    /// ISO 8601 start time.
    pub timestamp: Option<String>,
    /// Working directory the session ran in. Absent in legacy files.
    pub cwd: Option<PathBuf>,
    /// CLI version that wrote it. Absent in legacy files.
    pub cli_version: Option<String>,
    /// What started the session, for example `codex_cli_rs`.
    pub originator: Option<String>,
    /// Entry point, for example `cli`.
    pub source: Option<String>,
    /// Git metadata, when recorded.
    pub git: Option<GitMeta>,
    /// The metadata object as written, including keys not named above.
    pub raw: serde_json::Value,
}

/// One line of a session log.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionEntry {
    /// Envelope timestamp, when the line has an envelope.
    pub timestamp: Option<String>,
    /// Envelope type: `event_msg`, `response_item`, `turn_context`, and so on.
    ///
    /// `None` for a legacy line, which has no envelope. In that case
    /// `payload` is the whole line.
    pub entry_type: Option<String>,
    /// The payload, or the whole line for a legacy entry.
    pub payload: serde_json::Value,
}

impl SessionEntry {
    /// The payload's own `type`, which is the useful discriminator.
    ///
    /// For a modern `event_msg` this is `agent_message`, `token_count`, and
    /// so on; for a `response_item`, `message` or `reasoning`.
    #[must_use]
    pub fn payload_type(&self) -> Option<&str> {
        self.payload.get("type")?.as_str()
    }
}

/// A parsed session log.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionLog {
    /// The file this came from.
    pub path: PathBuf,
    /// Opening metadata, if the file had any.
    pub meta: Option<SessionMeta>,
    /// Every line after the metadata, in order.
    pub entries: Vec<SessionEntry>,
}

/// Which sessions to list.
#[derive(Debug, Clone, Default)]
pub struct SessionQuery {
    after: Option<(u16, u8, u8)>,
    before: Option<(u16, u8, u8)>,
    cwd: Option<PathBuf>,
}

impl SessionQuery {
    /// Every session.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// On or after this date. Filters directories, without opening files.
    #[must_use]
    pub fn after(mut self, year: u16, month: u8, day: u8) -> Self {
        self.after = Some((year, month, day));
        self
    }

    /// On or before this date. Filters directories, without opening files.
    #[must_use]
    pub fn before(mut self, year: u16, month: u8, day: u8) -> Self {
        self.before = Some((year, month, day));
        self
    }

    /// Only sessions recorded as running in this directory.
    ///
    /// Unlike the date filters this one has to open each candidate file to
    /// read its metadata, and legacy files record no `cwd` so they never
    /// match.
    #[must_use]
    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    fn matches_date(&self, date: (u16, u8, u8)) -> bool {
        self.after.is_none_or(|after| date >= after)
            && self.before.is_none_or(|before| date <= before)
    }
}

/// List sessions for the current environment, newest first.
///
/// Honors `CODEX_HOME`, defaulting to `~/.codex`.
pub fn list(query: &SessionQuery) -> Result<Vec<SessionFile>> {
    let home = crate::codex_home::resolve(&|key| std::env::var(key).ok());
    list_in(home, query)
}

/// [`list`], but against an explicit `CODEX_HOME`.
///
/// A missing `sessions` directory yields an empty list rather than an error:
/// no sessions is a normal state.
pub fn list_in(codex_home: impl AsRef<Path>, query: &SessionQuery) -> Result<Vec<SessionFile>> {
    let root = codex_home.as_ref().join("sessions");
    let mut found = Vec::new();

    for (date, day_dir) in date_dirs(&root) {
        if !query.matches_date(date) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&day_dir) else {
            continue;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let path = entry.path();
            let Some(id) = session_id_from_path(&path) else {
                continue;
            };
            if let Some(wanted) = &query.cwd
                && !session_ran_in(&path, wanted)
            {
                continue;
            }
            found.push(SessionFile { path, id, date });
        }
    }

    // Newest first. The filename carries the timestamp, so it orders within a
    // day without opening anything.
    found.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.path.cmp(&a.path)));
    Ok(found)
}

/// Read and parse one session log.
pub fn read(path: impl AsRef<Path>) -> Result<SessionLog> {
    let path = path.as_ref();
    let contents = std::fs::read_to_string(path).map_err(|e| Error::Io {
        message: format!("failed to read {}: {e}", path.display()),
        source: e,
        working_dir: None,
    })?;

    let mut meta = None;
    let mut entries = Vec::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // A line that will not parse is skipped rather than failing the read.
        // These files are appended to by a long-running process and a
        // truncated tail should not cost the caller the rest of the session.
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        match envelope(&value) {
            Some((timestamp, entry_type, payload)) => {
                if entry_type == "session_meta" && meta.is_none() {
                    meta = Some(parse_meta(payload));
                    continue;
                }
                entries.push(SessionEntry {
                    timestamp: timestamp.map(str::to_string),
                    entry_type: Some(entry_type.to_string()),
                    payload: payload.clone(),
                });
            }
            None => {
                // Legacy: no envelope. The first such line is the metadata,
                // recognised by carrying an id and no record type of its own.
                if meta.is_none() && value.get("id").is_some() && value.get("type").is_none() {
                    meta = Some(parse_meta(&value));
                    continue;
                }
                entries.push(SessionEntry {
                    timestamp: value
                        .get("timestamp")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    entry_type: None,
                    payload: value,
                });
            }
        }
    }

    Ok(SessionLog {
        path: path.to_path_buf(),
        meta,
        entries,
    })
}

/// `(timestamp, type, payload)` for a modern envelope, `None` for a legacy
/// line.
fn envelope(value: &serde_json::Value) -> Option<(Option<&str>, &str, &serde_json::Value)> {
    let payload = value.get("payload")?;
    let entry_type = value.get("type")?.as_str()?;
    Some((
        value.get("timestamp").and_then(serde_json::Value::as_str),
        entry_type,
        payload,
    ))
}

fn parse_meta(value: &serde_json::Value) -> SessionMeta {
    let string = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    SessionMeta {
        id: string("id").or_else(|| string("session_id")),
        timestamp: string("timestamp"),
        cwd: string("cwd").map(PathBuf::from),
        cli_version: string("cli_version"),
        originator: string("originator"),
        source: string("source"),
        git: value.get("git").and_then(|git| {
            let get = |key: &str| {
                git.get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            };
            let meta = GitMeta {
                commit_hash: get("commit_hash"),
                branch: get("branch"),
                repository_url: get("repository_url"),
            };
            (meta != GitMeta::default()).then_some(meta)
        }),
        raw: value.clone(),
    }
}

/// `(year, month, day)` directories under the sessions root.
fn date_dirs(root: &Path) -> Vec<((u16, u8, u8), PathBuf)> {
    let mut out = Vec::new();
    for year in numeric_children(root) {
        for month in numeric_children(&year.1) {
            for day in numeric_children(&month.1) {
                out.push(((year.0 as u16, month.0 as u8, day.0 as u8), day.1));
            }
        }
    }
    out
}

/// Child directories whose names are numbers, with the parsed value.
fn numeric_children(dir: &Path) -> Vec<(u32, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let value = name.parse::<u32>().ok()?;
            entry.path().is_dir().then(|| (value, entry.path()))
        })
        .collect()
}

/// The uuid tail of `rollout-<ISO8601>-<uuid>.jsonl`.
///
/// The timestamp itself contains dashes, so this takes the tail rather than
/// splitting: a uuid is five dash-separated groups.
fn session_id_from_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() < 5 {
        return None;
    }
    Some(parts[parts.len() - 5..].join("-"))
}

/// Whether a session's metadata records it as having run in `wanted`.
fn session_ran_in(path: &Path, wanted: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    let Some(first) = contents.lines().find(|line| !line.trim().is_empty()) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(first) else {
        return false;
    };
    let meta = value.get("payload").unwrap_or(&value);
    meta.get("cwd").and_then(serde_json::Value::as_str) == wanted.to_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "codex-wrapper-history-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn write_session(home: &Path, date: (u16, u8, u8), name: &str, lines: &[&str]) -> PathBuf {
        let dir = home
            .join("sessions")
            .join(format!("{:04}", date.0))
            .join(format!("{:02}", date.1))
            .join(format!("{:02}", date.2));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
        path
    }

    /// Transcribed from a real 2026 rollout: envelope of timestamp, type and
    /// payload on every line.
    const MODERN: &[&str] = &[
        r#"{"timestamp":"2026-08-06T10:11:24Z","type":"session_meta","payload":{"session_id":"019fd80e-eb27-70e3-ad2e-ed939930901a","id":"019fd80e-eb27-70e3-ad2e-ed939930901a","timestamp":"2026-08-06T10:11:24Z","cwd":"/repo","originator":"codex_cli_rs","cli_version":"0.145.0","source":"cli","git":{"commit_hash":"abc123","branch":"main","repository_url":"git@example.com:o/r.git"}}}"#,
        r#"{"timestamp":"2026-08-06T10:11:25Z","type":"turn_context","payload":{"model":"gpt-5.6-sol","cwd":"/repo"}}"#,
        r#"{"timestamp":"2026-08-06T10:11:30Z","type":"event_msg","payload":{"type":"agent_message","message":"hello"}}"#,
        r#"{"timestamp":"2026-08-06T10:11:31Z","type":"response_item","payload":{"type":"message","role":"assistant"}}"#,
    ];

    /// Transcribed from a real 2025-09 rollout: no envelope at all.
    const LEGACY: &[&str] = &[
        r#"{"id":"7b332612-1b8e-424b-bbb4-a239a64377fb","timestamp":"2025-09-01T19:51:34Z","instructions":null,"git":{"commit_hash":"old123"}}"#,
        r#"{"record_type":"state"}"#,
        r#"{"id":"item_0","type":"message","role":"user","content":[]}"#,
    ];

    #[test]
    fn a_missing_sessions_directory_is_empty_not_an_error() {
        let home = temp_home("empty");
        assert_eq!(list_in(&home, &SessionQuery::new()).unwrap(), vec![]);
    }

    #[test]
    fn lists_sessions_newest_first() {
        let home = temp_home("order");
        write_session(
            &home,
            (2026, 8, 1),
            "rollout-2026-08-01T10-00-00-aaaaaaaa-1111-2222-3333-444444444444.jsonl",
            MODERN,
        );
        write_session(
            &home,
            (2026, 8, 6),
            "rollout-2026-08-06T10-11-24-bbbbbbbb-1111-2222-3333-444444444444.jsonl",
            MODERN,
        );
        write_session(
            &home,
            (2025, 9, 1),
            "rollout-2025-09-01T19-51-34-cccccccc-1111-2222-3333-444444444444.jsonl",
            LEGACY,
        );

        let found = list_in(&home, &SessionQuery::new()).unwrap();
        let dates: Vec<_> = found.iter().map(|s| s.date).collect();
        assert_eq!(dates, vec![(2026, 8, 6), (2026, 8, 1), (2025, 9, 1)]);
        assert_eq!(found[0].id, "bbbbbbbb-1111-2222-3333-444444444444");
    }

    #[test]
    fn date_filters_narrow_the_listing() {
        let home = temp_home("dates");
        write_session(
            &home,
            (2026, 8, 1),
            "rollout-2026-08-01T10-00-00-aaaaaaaa-1111-2222-3333-444444444444.jsonl",
            MODERN,
        );
        write_session(
            &home,
            (2026, 8, 6),
            "rollout-2026-08-06T10-11-24-bbbbbbbb-1111-2222-3333-444444444444.jsonl",
            MODERN,
        );

        let after = list_in(&home, &SessionQuery::new().after(2026, 8, 5)).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].date, (2026, 8, 6));

        let before = list_in(&home, &SessionQuery::new().before(2026, 8, 5)).unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].date, (2026, 8, 1));

        let between = list_in(
            &home,
            &SessionQuery::new().after(2026, 8, 1).before(2026, 8, 6),
        )
        .unwrap();
        assert_eq!(between.len(), 2);
    }

    #[test]
    fn cwd_filter_matches_the_recorded_directory() {
        let home = temp_home("cwd");
        write_session(
            &home,
            (2026, 8, 6),
            "rollout-2026-08-06T10-11-24-bbbbbbbb-1111-2222-3333-444444444444.jsonl",
            MODERN,
        );

        assert_eq!(
            list_in(&home, &SessionQuery::new().cwd("/repo"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            list_in(&home, &SessionQuery::new().cwd("/elsewhere"))
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn reads_a_modern_session() {
        let home = temp_home("modern");
        let path = write_session(
            &home,
            (2026, 8, 6),
            "rollout-2026-08-06T10-11-24-bbbbbbbb-1111-2222-3333-444444444444.jsonl",
            MODERN,
        );

        let log = read(&path).unwrap();
        let meta = log.meta.unwrap();
        assert_eq!(meta.cwd, Some(PathBuf::from("/repo")));
        assert_eq!(meta.cli_version.as_deref(), Some("0.145.0"));
        assert_eq!(meta.git.unwrap().branch.as_deref(), Some("main"));

        // The metadata line is not also an entry.
        assert_eq!(log.entries.len(), 3);
        assert_eq!(log.entries[0].entry_type.as_deref(), Some("turn_context"));
        assert_eq!(log.entries[1].payload_type(), Some("agent_message"));
    }

    /// The whole point of the two-generation handling: a parser written for
    /// the modern envelope returns nothing here, silently.
    #[test]
    fn reads_a_legacy_session_without_an_envelope() {
        let home = temp_home("legacy");
        let path = write_session(
            &home,
            (2025, 9, 1),
            "rollout-2025-09-01T19-51-34-cccccccc-1111-2222-3333-444444444444.jsonl",
            LEGACY,
        );

        let log = read(&path).unwrap();
        let meta = log.meta.expect("the first line is the metadata");
        assert_eq!(
            meta.id.as_deref(),
            Some("7b332612-1b8e-424b-bbb4-a239a64377fb")
        );
        assert_eq!(meta.cli_version, None, "legacy files record no version");
        assert_eq!(meta.cwd, None, "legacy files record no cwd");
        assert_eq!(meta.git.unwrap().commit_hash.as_deref(), Some("old123"));

        assert_eq!(log.entries.len(), 2);
        // No envelope, so the whole line is the payload.
        assert_eq!(log.entries[0].entry_type, None);
        assert_eq!(log.entries[1].payload_type(), Some("message"));
    }

    /// These files are appended to by a running process, so a truncated last
    /// line must not cost the caller the rest of the session.
    #[test]
    fn a_truncated_tail_does_not_lose_the_rest() {
        let home = temp_home("truncated");
        let mut lines = MODERN.to_vec();
        lines.push(r#"{"timestamp":"2026-08-06T10:11:32Z","type":"event_ms"#);
        let path = write_session(
            &home,
            (2026, 8, 6),
            "rollout-2026-08-06T10-11-24-dddddddd-1111-2222-3333-444444444444.jsonl",
            &lines,
        );

        let log = read(&path).unwrap();
        assert!(log.meta.is_some());
        assert_eq!(log.entries.len(), 3, "the good lines survive");
    }

    /// The timestamp in the filename also contains dashes, so the id has to be
    /// taken from the tail rather than by splitting on the first one.
    #[test]
    fn session_id_comes_from_the_uuid_tail() {
        assert_eq!(
            session_id_from_path(Path::new(
                "/x/rollout-2026-08-06T10-11-24-019fd80e-eb27-70e3-ad2e-ed939930901a.jsonl"
            )),
            Some("019fd80e-eb27-70e3-ad2e-ed939930901a".to_string())
        );
        assert_eq!(session_id_from_path(Path::new("/x/notes.txt")), None);
    }

    #[test]
    fn unknown_entry_types_are_kept_rather_than_dropped() {
        let home = temp_home("unknown");
        let path = write_session(
            &home,
            (2026, 8, 6),
            "rollout-2026-08-06T10-11-24-eeeeeeee-1111-2222-3333-444444444444.jsonl",
            &[
                MODERN[0],
                r#"{"timestamp":"2026-08-06T10:11:40Z","type":"something_new","payload":{"type":"unheard_of"}}"#,
            ],
        );

        let log = read(&path).unwrap();
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].entry_type.as_deref(), Some("something_new"));
        assert_eq!(log.entries[0].payload_type(), Some("unheard_of"));
    }
}
