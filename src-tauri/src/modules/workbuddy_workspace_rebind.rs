//! Relocate WorkBuddy's workspace-keyed history before updating the local session index.
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// An SQLite error can release its writer lock before file cleanup finishes.
static REBIND_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn session_columns(db: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut query = db.prepare("SELECT name FROM pragma_table_info('sessions')")?;
    let columns = query.query_map([], |row| row.get(0))?;
    columns.collect()
}

fn workspace_base(path: &str) -> String {
    path.split(['/', '\\', ':', '-'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn prefix_at_byte_limit(value: &str, mut limit: usize) -> &str {
    limit = limit.min(value.len());
    while !value.is_char_boundary(limit) {
        limit -= 1;
    }
    &value[..limit]
}

fn project_key(base: &str) -> String {
    if base.len() <= 255 {
        return base.to_owned();
    }
    let mut hash = base.bytes().fold(5381u32, |hash, byte| {
        hash.wrapping_mul(33) ^ u32::from(byte)
    });
    let mut digits = Vec::new();
    loop {
        digits.push(char::from(
            b"0123456789abcdefghijklmnopqrstuvwxyz"[(hash % 36) as usize],
        ));
        hash /= 36;
        if hash == 0 {
            break;
        }
    }
    format!(
        "{}-{}",
        prefix_at_byte_limit(base, 180),
        digits.into_iter().rev().collect::<String>()
    )
}

fn workspace_path(path: &Path) -> Result<String, String> {
    let value = path
        .to_str()
        .ok_or_else(|| format!("Workspace path is not valid Unicode: {}", path.display()))?;
    #[cfg(windows)]
    let value = if let Some(tail) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{tail}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(value).to_owned()
    };
    Ok(value.replace('\\', "/"))
}

fn canonical_old_workspace(path: &Path) -> Option<PathBuf> {
    for ancestor in path.ancestors() {
        match ancestor.canonicalize() {
            Ok(canonical) => {
                // A rename removes the leaf, but existing parent aliases still
                // tell us the real path used by WorkBuddy when writing the history.
                let suffix = path.strip_prefix(ancestor).ok()?;
                return Some(canonical.join(suffix));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => break,
        }
    }
    None
}

fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.trim() == value
        && !value.contains(['/', '\\', '\0'])
        && !matches!(value, "." | "..")
        && Path::new(value)
            .file_name()
            .is_some_and(|name| name == value)
}

fn no_follow(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_symlink() => Err(io::Error::other(format!(
            "Unexpected symlink in WorkBuddy history: {}",
            path.display()
        ))),
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn io_error(action: &str, path: &Path, error: io::Error) -> String {
    format!("{action} ({}): {error}", path.display())
}

fn ensure_directory(path: &Path) -> io::Result<bool> {
    match fs::create_dir(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if no_follow(path)?.is_some_and(|metadata| metadata.is_dir()) {
                Ok(false)
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

fn alias(project: &Path, id: &str) -> io::Result<Option<String>> {
    let binding = project.join(format!("{id}.acp-session.json"));
    let Some(metadata) = no_follow(&binding)? else {
        return Ok(None);
    };
    if !metadata.is_file() {
        return Err(io::Error::other("ACP binding is not a file"));
    }
    let data: serde_json::Value = serde_json::from_reader(File::open(&binding)?)?;
    let target = data.get("acpSessionId").and_then(serde_json::Value::as_str);
    match target {
        Some(target) if component(target) => Ok(Some(target.to_owned())),
        _ => Err(io::Error::other(format!(
            "Invalid ACP binding: {}",
            binding.display()
        ))),
    }
}

fn has_transcript(project: &Path, id: &str) -> io::Result<bool> {
    let Some(directory) = no_follow(project)? else {
        return Ok(false);
    };
    if !directory.is_dir() {
        return Err(io::Error::other(format!(
            "Invalid history directory: {}",
            project.display()
        )));
    }
    if no_follow(&project.join(format!("{id}.jsonl")))?.is_some_and(|file| file.is_file()) {
        return Ok(true);
    }
    if let Some(target) = alias(project, id)? {
        return Ok(
            no_follow(&project.join(format!("{target}.jsonl")))?.is_some_and(|file| file.is_file())
        );
    }
    Ok(false)
}

fn source_project(
    projects: &Path,
    bases: &[String],
    id: &str,
    has_activity: bool,
) -> Result<Option<PathBuf>, String> {
    let find = || -> io::Result<Option<PathBuf>> {
        let keys = bases
            .iter()
            .map(|base| project_key(base))
            .collect::<Vec<_>>();
        let mut sidecar_project = None;
        for key in &keys {
            let directory = projects.join(key);
            if has_transcript(&directory, id)? {
                return Ok(Some(directory));
            }
            if !has_activity && sidecar_project.is_none() {
                for member in session_members(id) {
                    if no_follow(&directory.join(member))?.is_some() {
                        sidecar_project = Some(directory);
                        break;
                    }
                }
            }
        }

        let long = bases
            .iter()
            .filter(|base| base.len() > 255)
            .collect::<Vec<_>>();
        if long.is_empty() {
            return Ok(sidecar_project);
        }
        let mut candidates = Vec::new();
        let entries = match fs::read_dir(projects) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(sidecar_project),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(key) = name.to_str() else { continue };
            if keys.iter().any(|known| known == key) || !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(base) = long
                .iter()
                .find(|base| key.starts_with(prefix_at_byte_limit(base, 180)))
            else {
                continue;
            };
            if has_transcript(&entry.path(), id)? {
                let unhashed = key
                    .rsplit_once('-')
                    .filter(|(_, suffix)| {
                        !suffix.is_empty()
                            && suffix.len() <= 8
                            && suffix
                                .bytes()
                                .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
                    })
                    .map_or(key, |(prefix, _)| prefix);
                let compatible = unhashed.len() > prefix_at_byte_limit(base, 180).len()
                    && base.starts_with(unhashed);
                candidates.push((entry.path(), compatible));
            }
        }
        match candidates.as_slice() {
            [(path, true)] => Ok(Some(path.clone())),
            [] => Ok(sidecar_project),
            _ => Err(io::Error::other(
                "Cannot uniquely identify the older WorkBuddy history directory",
            )),
        }
    };
    find().map_err(|error| {
        io_error(
            &format!("Cannot locate WorkBuddy history for {id}"),
            projects,
            error,
        )
    })
}

fn files_match(source: &Path, destination: &Path, size: u64) -> io::Result<bool> {
    let mut left = BufReader::new(File::open(source)?);
    let mut right = BufReader::new(File::open(destination)?);
    let mut first = [0u8; 8192];
    let mut second = [0u8; 8192];
    let mut remaining = size;
    while remaining > 0 {
        let length = remaining.min(first.len() as u64) as usize;
        left.read_exact(&mut first[..length])?;
        right.read_exact(&mut second[..length])?;
        if first[..length] != second[..length] {
            return Ok(false);
        }
        remaining -= length as u64;
    }
    Ok(left.read(&mut first[..1])? == 0 && right.read(&mut second[..1])? == 0)
}

#[derive(Default)]
struct CopiedHistory {
    created: Vec<(PathBuf, bool)>,
}

impl CopiedHistory {
    fn directory(&mut self, path: &Path) -> io::Result<()> {
        if ensure_directory(path)? {
            self.created.push((path.to_owned(), true));
        }
        Ok(())
    }

    fn copy(&mut self, source: &Path, destination: &Path) -> io::Result<()> {
        if source == destination {
            return Ok(());
        }
        let Some(metadata) = no_follow(source)? else {
            return if no_follow(destination)?.is_some() {
                Err(history_conflict(destination))
            } else {
                Ok(())
            };
        };
        if metadata.is_dir() {
            if let Some(existing) = no_follow(destination)? {
                if !existing.is_dir() {
                    return Err(history_conflict(destination));
                }
                for child in fs::read_dir(destination)? {
                    let child = child?;
                    if no_follow(&source.join(child.file_name()))?.is_none() {
                        return Err(history_conflict(&child.path()));
                    }
                }
            } else {
                self.directory(destination)?;
            }
            for children in fs::read_dir(source)? {
                let child = children?;
                self.copy(&child.path(), &destination.join(child.file_name()))?;
            }
            return Ok(());
        }
        if !metadata.is_file() {
            return Err(io::Error::other(format!(
                "Invalid history file: {}",
                source.display()
            )));
        }
        if let Some(existing) = no_follow(destination)? {
            if existing.is_file()
                && existing.len() == metadata.len()
                && files_match(source, destination, metadata.len())?
            {
                return Ok(());
            }
            return Err(history_conflict(destination));
        }

        let parent = destination
            .parent()
            .ok_or_else(|| io::Error::other("Missing destination parent"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        io::copy(&mut File::open(source)?, temporary.as_file_mut())?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(destination)
            .map_err(|error| error.error)?;
        self.created.push((destination.to_owned(), false));
        Ok(())
    }

    fn rollback(mut self, cause: String) -> String {
        let mut errors = vec![cause];
        for (path, is_directory) in self.created.drain(..).rev() {
            let result = if is_directory {
                fs::remove_dir(&path)
            } else {
                fs::remove_file(&path)
            };
            if let Err(error) = result {
                errors.push(io_error(
                    "Could not roll back WorkBuddy history",
                    &path,
                    error,
                ));
            }
        }
        errors.join("\n")
    }
}

fn history_conflict(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("Different history already exists at {}", path.display()),
    )
}

fn session_members(id: &str) -> [String; 6] {
    [
        id.to_owned(),
        format!("{id}.jsonl"),
        format!("{id}.acp-session.json"),
        format!("{id}.meta.json"),
        format!("{id}.file-rollback.ndjson"),
        format!("{id}.quickask"),
    ]
}

/// Only the index of the selected instance is updated, after its files are ready.
pub(super) fn rebind_workspace(
    root: &Path,
    old_cwd: &str,
    target: &Path,
    ensure_idle: impl Fn() -> Result<(), String>,
) -> Result<usize, String> {
    if !Path::new(old_cwd).is_absolute() || !target.is_absolute() {
        return Err("Select an absolute WorkBuddy workspace path.".to_owned());
    }
    let target = target
        .canonicalize()
        .map_err(|error| io_error("Cannot open the new workspace", target, error))?;
    if !target.is_dir() {
        return Err("The new WorkBuddy workspace must be a directory.".to_owned());
    }
    let new_cwd = workspace_path(&target)?;
    if old_cwd == new_cwd {
        return Err("Select a different WorkBuddy workspace directory.".to_owned());
    }
    let _guard = REBIND_LOCK
        .lock()
        .map_err(|_| "Cannot lock WorkBuddy workspace rebinding.")?;
    let db_path = root.join("workbuddy.db");
    let projects = root.join("projects");
    if !no_follow(&db_path)
        .map_err(|error| io_error("Cannot access the WorkBuddy index", &db_path, error))?
        .is_some_and(|metadata| metadata.is_file())
    {
        return Err("The WorkBuddy instance has no session database.".to_owned());
    }
    if no_follow(&projects)
        .map_err(|error| io_error("Cannot access WorkBuddy history", &projects, error))?
        .is_some_and(|metadata| !metadata.is_dir())
    {
        return Err("The WorkBuddy history directory is invalid.".to_owned());
    }

    ensure_idle()?;
    let mut db = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|error| format!("Cannot open the WorkBuddy index: {error}"))?;
    let transaction = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("Cannot lock the WorkBuddy index: {error}"))?;
    let columns = session_columns(&transaction)
        .map_err(|error| format!("Cannot read the WorkBuddy index schema: {error}"))?;
    let local = if columns.contains("transport") {
        " AND transport = 'local'"
    } else {
        ""
    };
    let has_activity = if columns.contains("last_activity_at") {
        "last_activity_at IS NOT NULL"
    } else {
        // The older schema cannot prove that a row never had history.
        "1"
    };
    let workspace_filter = format!("cwd = ?1 AND deleted_at IS NULL{local}");
    let ids = (|| -> rusqlite::Result<Vec<(String, bool)>> {
        let mut query = transaction.prepare(&format!(
            "SELECT id, {has_activity} FROM sessions WHERE {workspace_filter} ORDER BY id"
        ))?;
        let rows = query.query_map([old_cwd], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    })()
    .map_err(|error| format!("Cannot read WorkBuddy sessions: {error}"))?;
    if ids.is_empty() {
        return Ok(0);
    }

    let mut bases = Vec::new();
    if let Some(canonical) = canonical_old_workspace(Path::new(old_cwd)) {
        bases.push(workspace_base(&workspace_path(&canonical)?));
    }
    let raw_base = workspace_base(old_cwd);
    if !bases.contains(&raw_base) {
        bases.push(raw_base);
    }
    let destination_key = project_key(&workspace_base(&new_cwd));
    if !component(&destination_key) || bases.iter().any(|base| !component(&project_key(base))) {
        return Err("WorkBuddy cannot address history for this workspace path.".to_owned());
    }
    let destination = projects.join(destination_key);
    let mut copied = CopiedHistory::default();
    let result = (|| {
        for (id, has_activity) in &ids {
            if !component(id) {
                return Err(format!("Invalid WorkBuddy session ID: {id}"));
            }
            let Some(source) = source_project(&projects, &bases, id, *has_activity)? else {
                if *has_activity {
                    return Err(format!("Missing WorkBuddy history for session {id}."));
                }
                // An empty session still must not acquire a different transcript at the target.
                for member in session_members(id) {
                    let target = destination.join(member);
                    if no_follow(&target)
                        .map_err(|error| io_error("Cannot check the new history", &target, error))?
                        .is_some()
                    {
                        return Err(history_conflict(&target).to_string());
                    }
                }
                continue;
            };
            if source == destination {
                continue;
            }
            let alias_id = alias(&source, id)
                .map_err(|error| io_error("Cannot read ACP binding", &source, error))?;
            copied.directory(&destination).map_err(|error| {
                io_error(
                    "Cannot create the new history directory",
                    &destination,
                    error,
                )
            })?;
            let members = session_members(id)
                .into_iter()
                .chain(alias_id.map(|id| format!("{id}.jsonl")));
            for member in members {
                let target = destination.join(&member);
                copied
                    .copy(&source.join(member), &target)
                    .map_err(|error| io_error("Cannot copy WorkBuddy history", &target, error))?;
            }
        }
        ensure_idle()?;
        let updated = transaction
            .execute(
                &format!("UPDATE sessions SET cwd = ?2 WHERE {workspace_filter}"),
                params![old_cwd, new_cwd],
            )
            .map_err(|error| format!("Cannot update the WorkBuddy index: {error}"))?;
        if updated != ids.len() {
            return Err("WorkBuddy session index changed during rebinding.".to_owned());
        }
        transaction
            .execute_batch("COMMIT")
            .map_err(|error| format!("Cannot commit the WorkBuddy index: {error}"))?;
        Ok(updated)
    })();
    result.map_err(|error| copied.rollback(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        old: PathBuf,
        new: PathBuf,
        old_cwd: String,
        db: Connection,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let root = base.join("configuration");
            let old = base.join("old");
            let new = base.join("renamed 工作区");
            fs::create_dir_all(root.join("projects")).unwrap();
            fs::create_dir(&new).unwrap();
            let db = Connection::open(root.join("workbuddy.db")).unwrap();
            db.execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE sessions (
                    id TEXT PRIMARY KEY, cwd TEXT, user_id TEXT, transport TEXT,
                    deleted_at INTEGER, last_activity_at INTEGER
                 )",
            )
            .unwrap();
            let old_cwd = workspace_path(&old).unwrap();
            Self {
                _temp: temp,
                root,
                old,
                new,
                old_cwd,
                db,
            }
        }

        fn insert(
            &self,
            id: &str,
            cwd: &str,
            user: &str,
            transport: &str,
            deleted: Option<i64>,
            activity: Option<i64>,
        ) {
            self.db
                .execute(
                    "INSERT INTO sessions VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, cwd, user, transport, deleted, activity],
                )
                .unwrap();
        }

        fn local(&self, id: &str) {
            self.insert(id, &self.old_cwd, "user", "local", None, Some(10));
        }

        fn project(&self, cwd: &Path) -> PathBuf {
            self.root
                .join("projects")
                .join(project_key(&workspace_base(&workspace_path(cwd).unwrap())))
        }

        fn write(&self, cwd: &Path, leaf: &str, text: &str) {
            let path = self.project(cwd).join(leaf);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn cwd(&self, id: &str) -> String {
            self.db
                .query_row("SELECT cwd FROM sessions WHERE id=?1", [id], |row| {
                    row.get(0)
                })
                .unwrap()
        }

        fn rebind(&self) -> Result<usize, String> {
            rebind_workspace(&self.root, &self.old_cwd, &self.new, || Ok(()))
        }
    }

    #[test]
    fn workbuddy_workspace_rebind_keeps_history_and_limits_updates_to_exact_local_workspace() {
        let f = Fixture::new();
        f.local("main");
        f.insert("alias", &f.old_cwd, "another-user", "local", None, Some(15));
        f.insert("cloud", &f.old_cwd, "user", "cloud", None, Some(10));
        f.insert("deleted", &f.old_cwd, "user", "local", Some(50), Some(10));
        f.insert("empty", &f.old_cwd, "user", "local", None, None);
        let nearby = format!("{}-elsewhere", f.old_cwd);
        f.insert("nearby", &nearby, "user", "local", None, Some(10));
        let members = [
            ("main.jsonl", "main original"),
            ("main/subagents/child.jsonl", "nested child"),
            ("main.quickask", ""),
            ("main.meta.json", "{}"),
            ("main.file-rollback.ndjson", "{}\n"),
            ("empty.quickask", ""),
            ("alias.acp-session.json", r#"{"acpSessionId":"other-id"}"#),
            ("alias/subagents/child.jsonl", "alias child"),
            ("other-id.jsonl", "alias original"),
        ];
        for (name, data) in members {
            f.write(&f.old, name, data);
        }
        let artifacts = f.root.join("artifact-index");
        fs::create_dir(&artifacts).unwrap();
        fs::write(artifacts.join("main.json"), "independent").unwrap();
        assert_eq!(f.rebind().unwrap(), 3);
        assert_eq!(f.rebind().unwrap(), 0);
        let new_cwd = workspace_path(&f.new).unwrap();
        for id in ["main", "alias", "empty"] {
            assert_eq!(f.cwd(id), new_cwd);
        }
        assert_eq!(f.cwd("cloud"), f.old_cwd);
        assert_eq!(f.cwd("deleted"), f.old_cwd);
        assert_eq!(f.cwd("nearby"), nearby);
        for (name, data) in members {
            assert_eq!(
                fs::read_to_string(f.project(&f.new).join(name)).unwrap(),
                data
            );
            assert_eq!(
                fs::read_to_string(f.project(&f.old).join(name)).unwrap(),
                data
            );
        }
        assert_eq!(
            fs::read_to_string(artifacts.join("main.json")).unwrap(),
            "independent"
        );
    }

    #[test]
    fn workbuddy_workspace_rebind_rolls_back_on_file_conflict_and_sql_failure_then_retries() {
        let f = Fixture::new();
        f.local("a");
        f.local("b");
        f.write(&f.old, "a.jsonl", "first");
        f.write(&f.old, "b.acp-session.json", r#"{"acpSessionId":"worker"}"#);
        f.write(&f.old, "worker.jsonl", "second");
        f.write(&f.new, "b.jsonl", "pre-existing direct history");
        let target = f.project(&f.new);
        assert!(f
            .rebind()
            .unwrap_err()
            .contains("Different history already exists"));
        assert_eq!(
            fs::read_to_string(target.join("b.jsonl")).unwrap(),
            "pre-existing direct history"
        );
        assert!(!target.join("a.jsonl").exists());
        assert_eq!(f.cwd("a"), f.old_cwd);
        fs::remove_file(target.join("b.jsonl")).unwrap();

        // An identical file left by an interrupted earlier attempt is reused.
        f.write(&f.new, "a.jsonl", "first");
        f.db.execute_batch(
            "CREATE TRIGGER fail_update BEFORE UPDATE ON sessions
             WHEN NEW.id = 'b' BEGIN SELECT RAISE(ABORT, 'fixture SQL failure'); END",
        )
        .unwrap();
        assert!(f.rebind().unwrap_err().contains("fixture SQL failure"));
        assert_eq!(f.cwd("a"), f.old_cwd);
        assert_eq!(f.cwd("b"), f.old_cwd);
        assert_eq!(fs::read_to_string(target.join("a.jsonl")).unwrap(), "first");
        assert!(!target.join("worker.jsonl").exists());

        f.db.execute_batch("DROP TRIGGER fail_update").unwrap();
        assert_eq!(f.rebind().unwrap(), 2);
        assert_eq!(
            fs::read_to_string(target.join("worker.jsonl")).unwrap(),
            "second"
        );
    }

    #[test]
    fn workbuddy_workspace_rebind_stops_if_the_selected_instance_starts_during_preparation() {
        let f = Fixture::new();
        f.local("session");
        f.write(&f.old, "session.jsonl", "original");
        let checks = std::cell::Cell::new(0);
        let result = rebind_workspace(&f.root, &f.old_cwd, &f.new, || {
            checks.set(checks.get() + 1);
            if checks.get() == 1 {
                Ok(())
            } else {
                Err("Started during rebind".to_owned())
            }
        });
        assert_eq!(result.unwrap_err(), "Started during rebind");
        assert_eq!(f.cwd("session"), f.old_cwd);
        assert!(!f.project(&f.new).exists());
        assert_eq!(f.rebind().unwrap(), 1);
    }

    #[test]
    fn workbuddy_workspace_rebind_accepts_a_unique_legacy_slot_and_rejects_an_ambiguity() {
        let f = Fixture::new();
        let old_cwd = workspace_path(
            &f.old
                .join("中".repeat(65))
                .join("older component ".repeat(5)),
        )
        .unwrap();
        f.insert("legacy", &old_cwd, "user", "local", None, Some(10));
        let base = workspace_base(&old_cwd);
        let projects = f.root.join("projects");
        let older = projects.join(format!("{}-abc", prefix_at_byte_limit(&base, 225)));
        let ambiguous = projects.join(format!("{}another-xyz", prefix_at_byte_limit(&base, 180)));
        for project in [&older, &ambiguous] {
            fs::create_dir(project).unwrap();
            fs::write(project.join("legacy.jsonl"), "fixture").unwrap();
        }
        let rebind = || rebind_workspace(&f.root, &old_cwd, &f.new, || Ok(()));
        assert!(rebind().unwrap_err().contains("Cannot uniquely identify"));
        assert_eq!(f.cwd("legacy"), old_cwd);
        fs::remove_dir_all(ambiguous).unwrap();
        assert_eq!(rebind().unwrap(), 1);
        assert_eq!(
            fs::read_to_string(f.project(&f.new).join("legacy.jsonl")).unwrap(),
            "fixture"
        );
    }

    #[test]
    fn workbuddy_workspace_rebind_reuses_history_when_paths_have_the_same_key() {
        let f = Fixture::new();
        let old = f.old.join("shared-key");
        let new = f.old.join("shared").join("key");
        fs::create_dir_all(&new).unwrap();
        let old_cwd = workspace_path(&old).unwrap();
        f.insert("same", &old_cwd, "user", "local", None, Some(10));
        f.write(&old, "same.jsonl", "fixture");
        assert_eq!(f.project(&old), f.project(&new));
        assert_eq!(
            rebind_workspace(&f.root, &old_cwd, &new, || Ok(())).unwrap(),
            1
        );
        assert_eq!(f.cwd("same"), workspace_path(&new).unwrap());
        assert_eq!(
            fs::read_to_string(f.project(&new).join("same.jsonl")).unwrap(),
            "fixture"
        );
    }

    #[test]
    fn workbuddy_workspace_rebind_reads_an_older_index_and_leaves_cached_cloud_rows() {
        let f = Fixture::new();
        f.local("legacy");
        f.insert("cached", &f.old_cwd, "user", "cloud", Some(-1), Some(10));
        f.write(&f.old, "legacy.jsonl", "kept");
        f.db.execute_batch(
            "ALTER TABLE sessions DROP COLUMN transport;
             ALTER TABLE sessions DROP COLUMN last_activity_at;",
        )
        .unwrap();
        assert_eq!(f.rebind().unwrap(), 1);
        assert_eq!(f.cwd("legacy"), workspace_path(&f.new).unwrap());
        assert_eq!(f.cwd("cached"), f.old_cwd);
        assert_eq!(
            fs::read_to_string(f.project(&f.new).join("legacy.jsonl")).unwrap(),
            "kept"
        );
    }

    #[cfg(unix)]
    #[test]
    fn workbuddy_workspace_rebind_finds_a_renamed_workspace_under_an_existing_parent_alias() {
        let f = Fixture::new();
        let real_parent = f.old.join("physical");
        let alias_parent = f.old.join("alias");
        fs::create_dir_all(&real_parent).unwrap();
        std::os::unix::fs::symlink(&real_parent, &alias_parent).unwrap();
        let old_name = alias_parent.join("renamed");
        let old_cwd = workspace_path(&old_name).unwrap();
        f.insert("session", &old_cwd, "user", "local", None, Some(1));
        f.write(&real_parent.join("renamed"), "session.jsonl", "original");
        assert!(!old_name.exists());
        assert_eq!(
            rebind_workspace(&f.root, &old_cwd, &f.new, || Ok(())).unwrap(),
            1
        );
        assert_eq!(f.cwd("session"), workspace_path(&f.new).unwrap());
        assert_eq!(
            fs::read_to_string(f.project(&f.new).join("session.jsonl")).unwrap(),
            "original"
        );
    }
}
