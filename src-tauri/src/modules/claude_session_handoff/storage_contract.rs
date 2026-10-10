//! Read-only, version-independent evidence for the ordinary local Code store.
//!
//! This is a bounded recognizer of a storage data-flow, not a JavaScript
//! interpreter, vendor-source allowlist, or proof that authenticated resume works.
//! Only the selected public app archive is opened. No vendor code is executed.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

const UNAVAILABLE: &str = "DESKTOP_CONTRACT_UNAVAILABLE";
const UNSUPPORTED: &str = "DESKTOP_CONTRACT_UNSUPPORTED";
const CHANGED: &str = "DESKTOP_CONTRACT_CHANGED";
const MAX_ARCHIVE: u64 = 192 * 1024 * 1024;
const MAX_HEADER: usize = 8 * 1024 * 1024;
const MAX_MEMBER: usize = 12 * 1024 * 1024;
const MAX_SOURCE: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 65_536;
const MAX_MODULES: usize = 512;
const MAX_NODES: usize = 8_000_000;
const MAX_DEPTH: usize = 512;
const MAX_TIME: Duration = Duration::from_secs(20);
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug)]
pub(super) struct Contract {
    /// SHA-256 of this entire archive, used only for transaction drift.
    pub fingerprint: String,
    /// Property names of the writer's linked, actually returned projection.
    pub projected_fields: BTreeSet<String>,
    archive: PathBuf,
    witness: Witness,
}

impl Contract {
    pub(super) fn belongs_to_app(&self, app: &Path) -> bool {
        self.archive == app.join("Contents/Resources/app.asar")
    }

    /// Reopen through no-follow directory anchors, then hash the entire archive.
    pub(super) fn assert_unchanged(&self) -> Result<()> {
        let (mut file, before) = open_archive(&self.archive).map_err(|_| CHANGED)?;
        if before != self.witness {
            return Err(CHANGED.into());
        }
        let hash = hash_file(&mut file).map_err(|_| CHANGED)?;
        let (_, after) = open_archive(&self.archive).map_err(|_| CHANGED)?;
        if before != after
            || archive_stat(&file.metadata().map_err(|_| CHANGED)?)? != before.file
            || hash != self.fingerprint
        {
            return Err(CHANGED.into());
        }
        Ok(())
    }

    /// Cheap per-publication witness. This does NOT prove byte equality; callers
    /// must also run assert_unchanged at the full transaction boundaries.
    pub(super) fn assert_unchanged_fast(&self) -> Result<()> {
        let (_, witness) = open_archive(&self.archive).map_err(|_| CHANGED)?;
        if witness != self.witness {
            return Err(CHANGED.into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStat {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    mode: u32,
    links: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Witness {
    directories: Vec<(u64, u64)>,
    file: FileStat,
}

#[cfg(unix)]
fn archive_stat(meta: &Metadata) -> Result<FileStat> {
    use std::os::unix::fs::MetadataExt;
    if !meta.is_file() || meta.len() > MAX_ARCHIVE {
        return Err(UNAVAILABLE.into());
    }
    Ok(FileStat {
        device: meta.dev(),
        inode: meta.ino(),
        size: meta.len(),
        modified: (meta.mtime(), meta.mtime_nsec()),
        changed: (meta.ctime(), meta.ctime_nsec()),
        mode: meta.mode(),
        links: meta.nlink(),
    })
}
#[cfg(not(unix))]
fn archive_stat(_: &Metadata) -> Result<FileStat> {
    Err(UNAVAILABLE.into())
}

#[cfg(unix)]
fn open_archive(path: &Path) -> Result<(File, Witness)> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    if !path.is_absolute() {
        return Err(UNAVAILABLE.into());
    }
    let names: Vec<_> = path
        .components()
        .filter_map(|c| match c {
            Component::RootDir => None,
            Component::Normal(n) => Some(Ok(n)),
            _ => Some(Err(UNAVAILABLE.to_owned())),
        })
        .collect::<Result<_>>()?;
    if names.is_empty() || names.len() > 128 {
        return Err(UNAVAILABLE.into());
    }
    let mut dir = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")
        .map_err(|_| UNAVAILABLE)?;
    let root = dir.metadata().map_err(|_| UNAVAILABLE)?;
    let mut directories = vec![(root.dev(), root.ino())];
    for (i, name) in names.iter().enumerate() {
        let name = CString::new(name.as_bytes()).map_err(|_| UNAVAILABLE)?;
        let last = i + 1 == names.len();
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if last { 0 } else { libc::O_DIRECTORY };
        // The parent descriptor stays open until openat has anchored its child.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(UNAVAILABLE.into());
        }
        let child = unsafe { File::from_raw_fd(fd) };
        let meta = child.metadata().map_err(|_| UNAVAILABLE)?;
        if last {
            return Ok((
                child,
                Witness {
                    directories,
                    file: archive_stat(&meta)?,
                },
            ));
        }
        if !meta.is_dir() {
            return Err(UNAVAILABLE.into());
        }
        directories.push((meta.dev(), meta.ino()));
        dir = child;
    }
    Err(UNAVAILABLE.into())
}
#[cfg(not(unix))]
fn open_archive(_: &Path) -> Result<(File, Witness)> {
    Err(UNAVAILABLE.into())
}

fn hash_file(file: &mut File) -> Result<String> {
    file.seek(SeekFrom::Start(0)).map_err(|_| UNAVAILABLE)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(|_| UNAVAILABLE)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > MAX_ARCHIVE {
            return Err(UNAVAILABLE.into());
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex(&hasher.finalize()))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

static CACHE: OnceLock<Mutex<Option<Contract>>> = OnceLock::new();

pub(super) fn inspect(app: &Path) -> Result<Contract> {
    let archive = app.join("Contents/Resources/app.asar");
    let (mut file, witness) = open_archive(&archive)?;
    // A cache hit still requires a complete anchored hash. A new hash always
    // receives structural inspection; there is no permanent hash admission gate.
    let fingerprint = hash_file(&mut file)?;
    let (_, after_hash) = open_archive(&archive)?;
    if witness != after_hash
        || archive_stat(&file.metadata().map_err(|_| UNAVAILABLE)?)? != witness.file
    {
        return Err(CHANGED.into());
    }
    if let Some(cached) = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .as_ref()
        .filter(|c| c.archive == archive && c.witness == witness && c.fingerprint == fingerprint)
    {
        return Ok(cached.clone());
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| UNAVAILABLE)?;
    let mut bytes = Vec::with_capacity(witness.file.size as usize);
    file.take(MAX_ARCHIVE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UNAVAILABLE)?;
    if bytes.len() as u64 != witness.file.size || hex(&Sha256::digest(&bytes)) != fingerprint {
        return Err(CHANGED.into());
    }
    let projected_fields = detect(&bytes)?;
    let contract = Contract {
        fingerprint,
        projected_fields,
        archive,
        witness,
    };
    contract.assert_unchanged_fast()?;
    *CACHE.get().unwrap().lock().map_err(|_| UNAVAILABLE)? = Some(contract.clone());
    Ok(contract)
}

#[derive(Clone, Copy)]
struct Entry {
    start: usize,
    size: usize,
    unpacked: bool,
}
struct Archive<'a> {
    bytes: &'a [u8],
    entries: BTreeMap<String, Entry>,
}
impl<'a> Archive<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        fn word(b: &[u8], n: usize) -> Result<usize> {
            Ok(u32::from_le_bytes(
                b.get(n..n + 4)
                    .ok_or(UNSUPPORTED)?
                    .try_into()
                    .map_err(|_| UNSUPPORTED)?,
            ) as usize)
        }
        if bytes.len() as u64 > MAX_ARCHIVE || word(bytes, 0)? != 4 {
            return Err(UNSUPPORTED.into());
        }
        let size = word(bytes, 4)?;
        let json_size = word(bytes, 12)?;
        let data = size.checked_add(8).ok_or(UNSUPPORTED)?;
        if !(8..=MAX_HEADER).contains(&size)
            || size % 4 != 0
            || word(bytes, 8)? != size - 4
            || json_size == 0
            || json_size > MAX_HEADER
            || json_size.checked_add(3).ok_or(UNSUPPORTED)? / 4 * 4 + 8 != size
            || data > bytes.len()
        {
            return Err(UNSUPPORTED.into());
        }
        let header: Value =
            serde_json::from_slice(bytes.get(16..16 + json_size).ok_or(UNSUPPORTED)?)
                .map_err(|_| UNSUPPORTED)?;
        let mut entries = BTreeMap::new();
        let mut todo = vec![("".to_owned(), &header, 0usize)];
        let mut count = 0;
        while let Some((prefix, node, depth)) = todo.pop() {
            if depth > 64 {
                return Err(UNSUPPORTED.into());
            }
            let files = node
                .get("files")
                .and_then(Value::as_object)
                .ok_or(UNSUPPORTED)?;
            for (name, value) in files {
                count += 1;
                if count > MAX_ENTRIES || !safe_segment(name) {
                    return Err(UNSUPPORTED.into());
                }
                let member = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                if member.len() > 1024 || !value.is_object() || value.get("link").is_some() {
                    return Err(UNSUPPORTED.into());
                }
                if value.get("files").is_some() {
                    todo.push((member, value, depth + 1));
                    continue;
                }
                let unpacked = match value.get("unpacked") {
                    None => false,
                    Some(Value::Bool(v)) => *v,
                    _ => return Err(UNSUPPORTED.into()),
                };
                let size: usize = value
                    .get("size")
                    .and_then(Value::as_u64)
                    .ok_or(UNSUPPORTED)?
                    .try_into()
                    .map_err(|_| UNSUPPORTED)?;
                let start = if unpacked {
                    0
                } else {
                    let offset = value
                        .get("offset")
                        .and_then(Value::as_str)
                        .ok_or(UNSUPPORTED)?;
                    if offset.is_empty() || !offset.bytes().all(|c| c.is_ascii_digit()) {
                        return Err(UNSUPPORTED.into());
                    }
                    let offset: usize = offset.parse().map_err(|_| UNSUPPORTED)?;
                    let start = data.checked_add(offset).ok_or(UNSUPPORTED)?;
                    if start.checked_add(size).ok_or(UNSUPPORTED)? > bytes.len() {
                        return Err(UNSUPPORTED.into());
                    }
                    start
                };
                entries.insert(
                    member,
                    Entry {
                        start,
                        size,
                        unpacked,
                    },
                );
            }
        }
        let mut ranges: Vec<_> = entries
            .values()
            .filter(|e| !e.unpacked && e.size > 0)
            .map(|e| (e.start, e.start + e.size))
            .collect();
        ranges.sort_unstable();
        // ASAR packers may deduplicate identical assets into one exact range.
        // Partially overlapping ranges still cannot describe independent files.
        ranges.dedup();
        if ranges.windows(2).any(|r| r[0].1 > r[1].0) {
            return Err(UNSUPPORTED.into());
        }
        Ok(Self { bytes, entries })
    }
    fn source(&self, member: &str) -> Result<&'a str> {
        let e = self.entries.get(member).ok_or(UNSUPPORTED)?;
        if e.unpacked || e.size > MAX_MEMBER {
            return Err(UNSUPPORTED.into());
        }
        std::str::from_utf8(&self.bytes[e.start..e.start + e.size]).map_err(|_| UNSUPPORTED.into())
    }
    fn resolve(&self, from: &str, spec: &str) -> Result<String> {
        if !spec.starts_with('.') {
            return Err(UNSUPPORTED.into());
        }
        let mut parts: Vec<_> = from.split('/').collect();
        parts.pop();
        for p in spec.split('/') {
            match p {
                "." => (),
                ".." => {
                    parts.pop().ok_or(UNSUPPORTED)?;
                }
                _ if safe_segment(p) => parts.push(p),
                _ => return Err(UNSUPPORTED.into()),
            }
        }
        let path = parts.join("/");
        for candidate in [
            path.clone(),
            format!("{path}.js"),
            format!("{path}.cjs"),
            format!("{path}/index.js"),
        ] {
            if self.entries.contains_key(&candidate) {
                return Ok(candidate);
            }
        }
        Err(UNSUPPORTED.into())
    }
}
fn safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.len() <= 255
        && !s.chars().any(|c| c.is_control() || c == '/' || c == '\\')
}

#[derive(Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}
impl Span {
    fn of(n: Node<'_>) -> Self {
        Self {
            start: n.start_byte(),
            end: n.end_byte(),
        }
    }
    fn node(self, tree: &Tree) -> Option<Node<'_>> {
        tree.root_node()
            .descendant_for_byte_range(self.start, self.end)
    }
}
struct Module {
    source: String,
    tree: Tree,
    imports: BTreeMap<String, String>,
    exports: BTreeMap<String, String>,
    definitions: BTreeMap<String, Span>,
    classes: Vec<Span>,
    dependencies: BTreeSet<String>,
}
impl Module {
    fn text<'a>(&'a self, n: Node<'_>) -> &'a str {
        &self.source[n.byte_range()]
    }
    fn parse(source: &str, started: Instant, nodes: &mut usize) -> Result<Self> {
        if started.elapsed() > MAX_TIME {
            return Err(UNSUPPORTED.into());
        }
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .map_err(|_| UNSUPPORTED)?;
        // Bound parsing itself, not just the subsequent AST walk.
        #[allow(deprecated)]
        parser.set_timeout_micros(8_000_000);
        let tree = parser.parse(source, None).ok_or(UNSUPPORTED)?;
        if tree.root_node().has_error() {
            return Err(UNSUPPORTED.into());
        }
        let mut module = Self {
            source: source.into(),
            tree,
            imports: BTreeMap::new(),
            exports: BTreeMap::new(),
            definitions: BTreeMap::new(),
            classes: Vec::new(),
            dependencies: BTreeSet::new(),
        };
        let mut definitions = BTreeMap::new();
        let mut classes = Vec::new();
        let mut imports = BTreeMap::new();
        let mut exports = BTreeMap::new();
        let mut dependencies = BTreeSet::new();
        let mut local_ends = Vec::new();
        let mut invalid = false;
        walk_with(
            module.tree.root_node(),
            |n, depth| {
                *nodes += 1;
                if *nodes > MAX_NODES
                    || depth > MAX_DEPTH
                    || (*nodes % 4096 == 0 && started.elapsed() > MAX_TIME)
                {
                    invalid = true;
                    return false;
                }
                while local_ends.last().is_some_and(|end| *end <= n.start_byte()) {
                    local_ends.pop();
                }
                let top = local_ends.is_empty();
                if matches!(
                    n.kind(),
                    "function_declaration"
                        | "function_expression"
                        | "arrow_function"
                        | "method_definition"
                        | "class"
                        | "class_declaration"
                ) {
                    local_ends.push(n.end_byte());
                }
                if top && n.kind() == "call_expression" {
                    if let Some(spec) = require_spec(n, &module).filter(|s| s.starts_with('.')) {
                        dependencies.insert(spec);
                    }
                }
                if top && n.kind() == "variable_declarator" {
                    if let (Some(name), Some(value)) = (
                        n.child_by_field_name("name"),
                        n.child_by_field_name("value"),
                    ) {
                        if name.kind() == "identifier" {
                            let key = module.text(name).to_owned();
                            definitions.insert(key.clone(), Span::of(value));
                            if let Some(spec) = require_spec(value, &module) {
                                imports.insert(key, spec);
                            }
                        }
                    }
                }
                if top
                    && matches!(
                        n.kind(),
                        "function_declaration" | "generator_function_declaration"
                    )
                {
                    if let Some(name) = n.child_by_field_name("name") {
                        definitions.insert(module.text(name).into(), Span::of(n));
                    }
                }
                if top && matches!(n.kind(), "class" | "class_declaration") {
                    classes.push(Span::of(n));
                    if n.kind() == "class_declaration" {
                        if let Some(name) = n.child_by_field_name("name") {
                            definitions.insert(module.text(name).into(), Span::of(n));
                        }
                    }
                }
                if top && n.kind() == "assignment_expression" {
                    if let (Some(left), Some(right)) = (
                        n.child_by_field_name("left"),
                        n.child_by_field_name("right"),
                    ) {
                        if let Some(chain) = chain(left, &module) {
                            if chain.len() == 2
                                && chain[0] == "exports"
                                && right.kind() == "identifier"
                            {
                                exports.insert(chain[1].clone(), module.text(right).into());
                            }
                        }
                    }
                }
                if top && n.kind() == "call_expression" {
                    if let Some(fun) = n.child_by_field_name("function") {
                        if chain(fun, &module).as_deref()
                            == Some(&["Object".into(), "defineProperty".into()])
                        {
                            let args = arguments(n);
                            if args.len() == 3 && module.text(args[0]) == "exports" {
                                if let (Some(name), Some(get)) =
                                    (string(args[1], &module), property(args[2], "get", &module))
                                {
                                    let values = find(get, "return_statement");
                                    if values.len() == 1 {
                                        if let Some(ret) = values[0]
                                            .named_child(0)
                                            .filter(|v| v.kind() == "identifier")
                                        {
                                            exports.insert(name, module.text(ret).into());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                true
            },
            |n| {
                !matches!(
                    n.kind(),
                    "function_declaration"
                        | "function_expression"
                        | "arrow_function"
                        | "method_definition"
                        | "class"
                        | "class_declaration"
                )
            },
        );
        if invalid {
            return Err(UNSUPPORTED.into());
        }
        module.definitions = definitions;
        module.classes = classes;
        module.imports = imports;
        module.exports = exports;
        module.dependencies = dependencies;
        // Bundlers implement code imports as Promise.resolve().then(() =>
        // require(relativeChunk)). Query just those import thunks in the full
        // tree, without walking every vendor function body in Rust or admitting
        // arbitrary optional runtime requires into the module graph.
        let query=Query::new(&tree_sitter_javascript::LANGUAGE.into(),
            "((call_expression function: (identifier) @callee arguments: (arguments (string) @spec)) @call (#eq? @callee \"require\"))")
            .map_err(|_|UNSUPPORTED)?;
        let mut cursor = QueryCursor::new();
        cursor.set_match_limit(8192);
        #[allow(deprecated)]
        cursor.set_timeout_micros(2_000_000);
        let mut matches = cursor.matches(&query, module.tree.root_node(), module.source.as_bytes());
        while let Some(found) = matches.next() {
            let Some(call) = found
                .captures
                .iter()
                .find(|c| query.capture_names()[c.index as usize] == "call")
                .map(|c| c.node)
            else {
                continue;
            };
            if !require_spec(call, &module).is_some() {
                continue;
            }
            let Some(thunk) = owner(call).filter(|n| {
                n.kind() == "arrow_function" && params(*n, &module).is_some_and(|p| p.is_empty())
            }) else {
                continue;
            };
            if thunk.child_by_field_name("body").map(unwrap) != Some(call) {
                continue;
            }
            let mut parent = thunk.parent();
            while parent.is_some_and(|n| n.kind() == "parenthesized_expression") {
                parent = parent.and_then(|n| n.parent());
            }
            let Some(then) = parent
                .and_then(|n| n.parent())
                .filter(|n| n.kind() == "call_expression")
            else {
                continue;
            };
            let Some(fun) = then.child_by_field_name("function") else {
                continue;
            };
            if fun
                .child_by_field_name("property")
                .is_none_or(|n| module.text(n) != "then")
            {
                continue;
            }
            if fun
                .child_by_field_name("object")
                .is_none_or(|n| !is_call(n, &["Promise", "resolve"], &module))
            {
                continue;
            }
            if let Some(spec) = require_spec(call, &module).filter(|s| s.starts_with('.')) {
                module.dependencies.insert(spec);
            }
            if started.elapsed() > MAX_TIME {
                return Err(UNSUPPORTED.into());
            }
        }
        drop(matches);
        if cursor.did_exceed_match_limit() {
            return Err(UNSUPPORTED.into());
        }
        Ok(module)
    }
}

// Iterative cursor traversal avoids stack overflow on adversarial nesting.
fn walk<'a>(n: Node<'a>, mut f: impl FnMut(Node<'a>, usize) -> bool) {
    walk_with(n, &mut f, |_| true);
}
fn walk_with<'a>(
    mut n: Node<'a>,
    mut f: impl FnMut(Node<'a>, usize) -> bool,
    descend: impl Fn(Node<'a>) -> bool,
) {
    let mut cursor = n.walk();
    let mut depth = 0;
    loop {
        n = cursor.node();
        if !f(n, depth) {
            return;
        }
        if descend(n) && cursor.goto_first_child() {
            depth += 1;
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
            depth -= 1;
        }
    }
}
fn find<'a>(root: Node<'a>, kind: &str) -> Vec<Node<'a>> {
    let mut out = Vec::new();
    walk(root, |n, _| {
        if n.kind() == kind {
            out.push(n);
        }
        true
    });
    out
}
fn unwrap(mut n: Node<'_>) -> Node<'_> {
    while matches!(
        n.kind(),
        "parenthesized_expression" | "await_expression" | "sequence_expression"
    ) {
        let index = if n.kind() == "sequence_expression" {
            n.named_child_count().saturating_sub(1)
        } else {
            0
        };
        if let Some(child) = n.named_child(index) {
            n = child;
        } else {
            break;
        }
    }
    n
}
fn string(n: Node<'_>, module: &Module) -> Option<String> {
    let n = unwrap(n);
    if n.kind() != "string" {
        return None;
    }
    let raw = module.text(n);
    let inner = raw.get(1..raw.len().checked_sub(1)?)?;
    // Ordinary quoted paths/properties, independent of quote style. Escaped
    // spellings are deliberately unrecognized rather than guessed.
    if inner.contains('\\') {
        return None;
    }
    Some(inner.into())
}
fn chain(mut n: Node<'_>, module: &Module) -> Option<Vec<String>> {
    let mut keys = Vec::new();
    loop {
        if keys.len() > MAX_DEPTH {
            return None;
        }
        n = unwrap(n);
        match n.kind() {
            "identifier" | "this" => {
                keys.push(module.text(n).into());
                keys.reverse();
                return Some(keys);
            }
            "member_expression" | "subscript_expression" => {
                let key = n
                    .child_by_field_name("property")
                    .or_else(|| n.child_by_field_name("index"))?;
                keys.push(if n.kind() == "subscript_expression" {
                    string(key, module)?
                } else {
                    module.text(key).into()
                });
                n = n.child_by_field_name("object")?;
            }
            _ => return None,
        }
    }
}
fn arguments(n: Node<'_>) -> Vec<Node<'_>> {
    let Some(args) = n.child_by_field_name("arguments") else {
        return Vec::new();
    };
    (0..args.named_child_count())
        .filter_map(|i| args.named_child(i))
        .filter(|n| n.kind() != "comment")
        .collect()
}
fn require_spec(n: Node<'_>, module: &Module) -> Option<String> {
    let n = unwrap(n);
    if n.kind() != "call_expression" || module.text(n.child_by_field_name("function")?) != "require"
    {
        return None;
    }
    let args = arguments(n);
    if args.len() != 1 {
        return None;
    }
    string(args[0], module)
}
fn property<'a>(obj: Node<'a>, key: &str, module: &Module) -> Option<Node<'a>> {
    let obj = unwrap(obj);
    if obj.kind() != "object" {
        return None;
    }
    let mut found = None;
    for i in 0..obj.named_child_count() {
        let p = obj.named_child(i)?;
        if p.kind() == "pair" {
            let name = property_key(p.child_by_field_name("key")?, module)?;
            if name == key {
                if found.is_some() {
                    return None;
                }
                found = p.child_by_field_name("value");
            }
        }
    }
    found
}
fn property_key(n: Node<'_>, m: &Module) -> Option<String> {
    match n.kind() {
        "property_identifier" | "identifier" => Some(m.text(n).into()),
        "string" => string(n, m),
        "computed_property_name" => string(n.named_child(0)?, m),
        _ => None,
    }
}
fn params(n: Node<'_>, module: &Module) -> Option<Vec<String>> {
    let p = n
        .child_by_field_name("parameters")
        .or_else(|| n.child_by_field_name("parameter"))?;
    if p.kind() == "identifier" {
        return Some(vec![module.text(p).into()]);
    }
    // Default values do not remove a positional slot or its lexical binding.
    // Destructuring/rest forms are unrecognized, rather than partially mapped.
    (0..p.named_child_count())
        .filter_map(|i| p.named_child(i))
        .filter(|n| n.kind() != "comment")
        .map(|n| {
            let binding = if n.kind() == "assignment_pattern" {
                n.child_by_field_name("left")?
            } else {
                n
            };
            (binding.kind() == "identifier").then(|| module.text(binding).into())
        })
        .collect()
}

struct Graph {
    modules: BTreeMap<String, Module>,
    started: Instant,
}
impl Graph {
    fn load(archive: &Archive<'_>, entry: String) -> Result<Self> {
        let mut modules = BTreeMap::new();
        let mut queue = VecDeque::from([entry]);
        let mut queued = BTreeSet::new();
        let mut total = 0;
        let mut nodes = 0;
        let started = Instant::now();
        while let Some(path) = queue.pop_front() {
            if modules.contains_key(&path) {
                continue;
            }
            if modules.len() >= MAX_MODULES {
                return Err(UNSUPPORTED.into());
            }
            let source = archive.source(&path)?;
            total += source.len();
            if total > MAX_SOURCE {
                return Err(UNSUPPORTED.into());
            }
            let module = Module::parse(source, started, &mut nodes)?;
            for spec in &module.dependencies {
                // Optional native addons and data/assets are outside the JS
                // graph. Do not reject an absent platform addon before checking
                // its extension. A missing relative JavaScript module still fails.
                if Path::new(spec)
                    .extension()
                    .is_some_and(|ext| ext != "js" && ext != "cjs" && ext != "mjs")
                {
                    continue;
                }
                let next = archive.resolve(&path, spec)?;
                if next.ends_with(".js") || next.ends_with(".cjs") || next.ends_with(".mjs") {
                    if queued.insert(next.clone()) {
                        queue.push_back(next);
                    }
                }
            }
            modules.insert(path, module);
        }
        Ok(Self { modules, started })
    }
    fn resolve<'a>(
        &'a self,
        archive: &Archive<'_>,
        path: &str,
        expression: Node<'_>,
    ) -> Option<(&'a str, Node<'a>)> {
        let module = self.modules.get(path)?;
        if identifier_shadowed(expression, module) {
            return None;
        }
        let c = chain(expression, module)?;
        if c.len() == 1 {
            return Some((
                self.modules.get_key_value(path)?.0.as_str(),
                module.definitions.get(&c[0])?.node(&module.tree)?,
            ));
        }
        if c.len() != 2 {
            return None;
        }
        let target = archive.resolve(path, module.imports.get(&c[0])?).ok()?;
        let (key, other) = self.modules.get_key_value(&target)?;
        Some((
            key,
            other
                .definitions
                .get(other.exports.get(&c[1])?)?
                .node(&other.tree)?,
        ))
    }
    fn constant(
        &self,
        archive: &Archive<'_>,
        path: &str,
        expression: Node<'_>,
        depth: usize,
    ) -> Option<String> {
        if depth > 8 {
            return None;
        }
        let m = self.modules.get(path)?;
        if let Some(s) = string(expression, m) {
            return Some(s);
        }
        if expression.kind() == "identifier" {
            if let Some(value) = initializer(expression, m.text(expression), m) {
                return self.constant(archive, path, value, depth + 1);
            }
        }
        let (p, node) = self.resolve(archive, path, expression)?;
        self.constant(archive, p, node, depth + 1)
    }
}

fn identifier_shadowed(expression: Node<'_>, m: &Module) -> bool {
    let Some(c) = chain(expression, m) else {
        return false;
    };
    let name = &c[0];
    let mut parent = expression.parent();
    while let Some(p) = parent {
        if p.kind() == "program" {
            return false;
        }
        if matches!(
            p.kind(),
            "function_declaration" | "function_expression" | "arrow_function" | "method_definition"
        ) && params(p, m).is_none_or(|p| p.iter().any(|s| s == name))
        {
            return true;
        }
        if p.kind() == "statement_block" {
            let mut shadow = false;
            walk_with(
                p,
                |n, _| {
                    if n.kind() == "variable_declarator"
                        && n.child_by_field_name("name")
                            .is_some_and(|n| m.text(n) == name)
                    {
                        let mut block = n.parent();
                        while block.is_some_and(|n| n.kind() != "statement_block") {
                            block = block.and_then(|n| n.parent());
                        }
                        if block == Some(p) {
                            shadow = true;
                        }
                    }
                    if matches!(n.kind(), "class_declaration" | "function_declaration")
                        && n.child_by_field_name("name")
                            .is_some_and(|n| m.text(n) == name)
                    {
                        shadow = true;
                    }
                    true
                },
                |n| {
                    n == p
                        || !matches!(
                            n.kind(),
                            "statement_block"
                                | "class"
                                | "class_declaration"
                                | "function_declaration"
                                | "function_expression"
                                | "arrow_function"
                                | "method_definition"
                        )
                },
            );
            if shadow {
                return true;
            }
        }
        parent = p.parent();
    }
    false
}
fn possible_instances<'a>(
    class: Node<'_>,
    path: &str,
    graph: &'a Graph,
    archive: &Archive<'_>,
) -> Option<Vec<(&'a str, Node<'a>)>> {
    let query = Query::new(
        &tree_sitter_javascript::LANGUAGE.into(),
        "(new_expression constructor: (_) @constructor) @instance",
    )
    .ok()?;
    let mut out = Vec::new();
    let target = &graph.modules[path];
    let bindings: BTreeSet<_> = target
        .definitions
        .iter()
        .filter(|(_, s)| s.node(&target.tree) == Some(class))
        .map(|(n, _)| n.as_str())
        .collect();
    let exports: BTreeSet<_> = target
        .exports
        .iter()
        .filter(|(_, n)| bindings.contains(n.as_str()))
        .map(|(n, _)| n.as_str())
        .collect();
    for (p, m) in &graph.modules {
        if graph.started.elapsed() > MAX_TIME {
            return None;
        }
        if p != path
            && !m
                .imports
                .values()
                .any(|s| s.starts_with('.') && archive.resolve(p, s).ok().as_deref() == Some(path))
        {
            continue;
        }
        let mut cursor = QueryCursor::new();
        cursor.set_match_limit(8192);
        #[allow(deprecated)]
        cursor.set_timeout_micros(2_000_000);
        let mut matches = cursor.matches(&query, m.tree.root_node(), m.source.as_bytes());
        while let Some(found) = matches.next() {
            let Some(instance) = found
                .captures
                .iter()
                .find(|c| query.capture_names()[c.index as usize] == "instance")
                .map(|c| c.node)
            else {
                continue;
            };
            let fun = instance.child_by_field_name("constructor")?;
            let Some(c) = chain(fun, m) else {
                continue;
            };
            let relevant = (p == path && c.len() == 1 && bindings.contains(c[0].as_str()))
                || (c.len() == 2
                    && exports.contains(c[1].as_str())
                    && m.imports
                        .get(&c[0])
                        .is_some_and(|s| archive.resolve(p, s).ok().as_deref() == Some(path)));
            if !relevant {
                continue;
            }
            if graph
                .resolve(archive, p, fun)
                .is_some_and(|(cp, n)| cp == path && n == class)
            {
                out.push((p.as_str(), instance));
            }
            if out.len() > 8192 {
                return None;
            }
            if graph.started.elapsed() > MAX_TIME {
                return None;
            }
        }
        drop(matches);
        if cursor.did_exceed_match_limit() {
            return None;
        }
    }
    Some(out)
}

fn is_call(n: Node<'_>, expected: &[&str], m: &Module) -> bool {
    unwrap(n)
        .child_by_field_name("function")
        .and_then(|f| chain(f, m))
        .is_some_and(|c| c.iter().map(String::as_str).eq(expected.iter().copied()))
}
fn path_join(n: Node<'_>, m: &Module) -> bool {
    let Some(c) = n.child_by_field_name("function").and_then(|n| chain(n, m)) else {
        return false;
    };
    matches!(c.as_slice(), [_, last] | [_, _, last] if last == "join")
        && c.get(1).is_some_and(|s| s == "join" || s == "default")
        && m.imports
            .get(&c[0])
            .is_some_and(|s| s == "path" || s == "node:path")
}
fn this_field(n: Node<'_>, key: &str, m: &Module) -> bool {
    chain(n, m).as_deref() == Some(&["this".into(), key.into()])
}
fn method_name(n: Node<'_>, m: &Module) -> Option<String> {
    let name = n.child_by_field_name("name")?;
    string(name, m).or_else(|| Some(m.text(name).into()))
}
fn methods<'a>(class: Node<'a>, m: &Module) -> BTreeMap<String, Node<'a>> {
    let mut out = BTreeMap::new();
    if let Some(body) = class.child_by_field_name("body") {
        for i in 0..body.named_child_count() {
            if let Some(n) = body
                .named_child(i)
                .filter(|n| n.kind() == "method_definition")
            {
                if let Some(name) = method_name(n, m) {
                    out.insert(name, n);
                }
            }
        }
    }
    out
}

// Lexical initializer lookup: a same-named declaration in a sibling callback
// cannot supply evidence for this expression.
fn initializer<'a>(reference: Node<'a>, name: &str, m: &Module) -> Option<Node<'a>> {
    let mut scope = reference.parent();
    while let Some(s) = scope {
        if matches!(s.kind(), "statement_block" | "program") {
            let mut best = None;
            let mut declared = false;
            walk(s, |n, _| {
                if n.kind() == "variable_declarator" && n.start_byte() < reference.start_byte() {
                    let mut parent = n.parent();
                    while parent.is_some_and(|p| !matches!(p.kind(), "statement_block" | "program"))
                    {
                        parent = parent.and_then(|p| p.parent());
                    }
                    if parent == Some(s)
                        && n.child_by_field_name("name")
                            .is_some_and(|n| m.text(n) == name)
                    {
                        declared = true;
                        best = n.child_by_field_name("value");
                    }
                }
                true
            });
            if declared {
                return best;
            }
        }
        if matches!(
            s.kind(),
            "function_declaration" | "function_expression" | "arrow_function" | "method_definition"
        ) && params(s, m).is_none_or(|p| p.iter().any(|p| p == name))
        {
            return None;
        }
        scope = s.parent();
    }
    None
}
fn expand<'a>(n: Node<'a>, m: &Module, depth: usize) -> Node<'a> {
    let n = unwrap(n);
    if depth < 8 && n.kind() == "identifier" {
        if let Some(value) = initializer(n, m.text(n), m) {
            return expand(value, m, depth + 1);
        }
    }
    n
}

fn owner(mut node: Node<'_>) -> Option<Node<'_>> {
    while let Some(p) = node.parent() {
        if matches!(
            p.kind(),
            "function_declaration" | "function_expression" | "arrow_function" | "method_definition"
        ) {
            return Some(p);
        }
        node = p;
    }
    None
}
fn binding_value<'a>(reference: Node<'a>, m: &Module) -> Option<Node<'a>> {
    let name = m.text(reference);
    if let Some(value) = initializer(reference, name, m) {
        return Some(value);
    }
    let function = owner(reference)?;
    let values: Vec<_> = find(function, "assignment_expression")
        .into_iter()
        .filter(|n| {
            n.start_byte() < reference.start_byte()
                && owner(*n) == Some(function)
                && n.child_by_field_name("left")
                    .is_some_and(|n| n.kind() == "identifier" && m.text(n) == name)
        })
        .filter_map(|n| n.child_by_field_name("right"))
        .collect();
    if values.len() == 1 {
        Some(values[0])
    } else {
        None
    }
}

fn local_json_filter(call: Node<'_>, m: &Module) -> bool {
    let Some(fun) = call.child_by_field_name("function") else {
        return false;
    };
    if fun.kind() != "member_expression"
        || fun
            .child_by_field_name("property")
            .is_none_or(|n| m.text(n) != "filter")
    {
        return false;
    }
    let args = arguments(call);
    let Some(predicate) = args.first().copied().map(unwrap) else {
        return false;
    };
    let Some(p) = params(predicate, m) else {
        return false;
    };
    let Some(input) = p.first() else {
        return false;
    };
    let body = predicate.child_by_field_name("body");
    let value = body.filter(|n| n.kind() != "statement_block").or_else(|| {
        own_returns(predicate)
            .first()
            .and_then(|n| n.named_child(0))
    });
    let Some(value) = value.map(unwrap) else {
        return false;
    };
    if value.kind() != "binary_expression"
        || value
            .child_by_field_name("operator")
            .is_none_or(|n| m.text(n) != "&&")
    {
        return false;
    }
    let (Some(left), Some(right)) = (
        value.child_by_field_name("left"),
        value.child_by_field_name("right"),
    ) else {
        return false;
    };
    let check = |n: Node<'_>, name: &str, text: &str| {
        let args = arguments(unwrap(n));
        is_call(n, &[input, name], m)
            && args.len() == 1
            && string(args[0], m).as_deref() == Some(text)
    };
    check(left, "startsWith", "local_") && check(right, "endsWith", ".json")
        || check(right, "startsWith", "local_") && check(left, "endsWith", ".json")
}
fn collection_from_directory(
    n: Node<'_>,
    storage: &str,
    class: Node<'_>,
    m: &Module,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    let n = unwrap(n);
    if n.kind() == "identifier" {
        return binding_value(n, m)
            .is_some_and(|v| collection_from_directory(v, storage, class, m, depth + 1));
    }
    let args = arguments(n);
    let Some(c) = n.child_by_field_name("function").and_then(|n| chain(n, m)) else {
        return false;
    };
    if c.last().is_some_and(|s| s == "readdir") {
        return m.imports.get(&c[0]).is_some_and(|s| {
            matches!(
                s.as_str(),
                "fs" | "node:fs" | "fs/promises" | "node:fs/promises"
            )
        }) && args
            .first()
            .is_some_and(|n| directory_binding(*n, storage, class, m));
    }
    // The installed inventory helper receives the namespace's readdir result.
    // Track that association without executing its filename discovery policy.
    args.iter()
        .any(|n| collection_from_directory(*n, storage, class, m, depth + 1))
}
fn filtered_collection(
    n: Node<'_>,
    storage: &str,
    class: Node<'_>,
    m: &Module,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    let n = unwrap(n);
    if n.kind() == "identifier" {
        return binding_value(n, m)
            .is_some_and(|v| filtered_collection(v, storage, class, m, depth + 1));
    }
    let Some(fun) = n.child_by_field_name("function") else {
        return false;
    };
    let Some(receiver) = fun.child_by_field_name("object") else {
        return false;
    };
    if local_json_filter(n, m) {
        return collection_from_directory(receiver, storage, class, m, 0);
    }
    fun.child_by_field_name("property")
        .is_some_and(|p| m.text(p) == "slice")
        && filtered_collection(receiver, storage, class, m, depth + 1)
}
fn row_from_filtered_collection(
    filename: Node<'_>,
    method: Node<'_>,
    storage: &str,
    class: Node<'_>,
    m: &Module,
) -> bool {
    if filename.kind() != "identifier" {
        return false;
    }
    let name = m.text(filename);
    let mut parent = filename.parent();
    while let Some(p) = parent {
        if p.kind() == "for_in_statement" {
            let Some(left) = p.child_by_field_name("left") else {
                return false;
            };
            let matches = m.text(left) == name
                || find(left, "variable_declarator").iter().any(|n| {
                    n.child_by_field_name("name")
                        .is_some_and(|n| m.text(n) == name)
                });
            if matches
                && p.child_by_field_name("right")
                    .is_some_and(|n| filtered_collection(n, storage, class, m, 0))
            {
                return true;
            }
        }
        if matches!(
            p.kind(),
            "arrow_function" | "function_expression" | "function_declaration"
        ) {
            break;
        }
        parent = p.parent();
    }
    let Some(reader) = owner(filename) else {
        return false;
    };
    let Some(ps) = params(reader, m) else {
        return false;
    };
    let Some(index) = ps.iter().position(|p| p == name) else {
        return false;
    };
    let Some(binding) = reader
        .parent()
        .filter(|n| n.kind() == "variable_declarator")
        .and_then(|n| n.child_by_field_name("name"))
    else {
        return false;
    };
    for call in find(method, "call_expression") {
        if call
            .child_by_field_name("function")
            .is_none_or(|n| m.text(n) != m.text(binding))
        {
            continue;
        }
        let args = arguments(call);
        let Some(input) = args.get(index) else {
            continue;
        };
        let Some(callback) = owner(call).filter(|n| n.kind() == "arrow_function") else {
            continue;
        };
        if params(callback, m).is_none_or(|p| p.first().is_none_or(|p| p != m.text(*input))) {
            continue;
        }
        let mut parent = callback.parent();
        while parent.is_some_and(|n| n.kind() == "parenthesized_expression") {
            parent = parent.and_then(|n| n.parent());
        }
        let Some(map) = parent
            .and_then(|n| n.parent())
            .filter(|n| n.kind() == "call_expression")
        else {
            continue;
        };
        let Some(fun) = map.child_by_field_name("function") else {
            continue;
        };
        if fun
            .child_by_field_name("property")
            .is_some_and(|n| m.text(n) == "map")
            && fun
                .child_by_field_name("object")
                .is_some_and(|n| filtered_collection(n, storage, class, m, 0))
        {
            return true;
        }
    }
    false
}
fn own_returns(function: Node<'_>) -> Vec<Node<'_>> {
    find(function, "return_statement")
        .into_iter()
        .filter(|ret| {
            let mut parent = ret.parent();
            while let Some(p) = parent {
                if matches!(
                    p.kind(),
                    "function_declaration"
                        | "function_expression"
                        | "arrow_function"
                        | "method_definition"
                ) {
                    return p == function;
                }
                parent = p.parent();
            }
            false
        })
        .collect()
}
fn return_leaves<'a>(function: Node<'a>, m: &Module) -> Vec<Node<'a>> {
    let mut todo: Vec<_> = own_returns(function)
        .iter()
        .filter_map(|n| n.named_child(0))
        .collect();
    let mut leaves = Vec::new();
    while let Some(n) = todo.pop() {
        let n = expand(n, m, 0);
        if n.kind() == "ternary_expression" {
            if let Some(v) = n.child_by_field_name("consequence") {
                todo.push(v);
            }
            if let Some(v) = n.child_by_field_name("alternative") {
                todo.push(v);
            }
        } else if n.kind() != "null"
            && !(n.kind() == "unary_expression" && m.text(n).starts_with("void"))
        {
            leaves.push(n);
        }
    }
    leaves
}

fn storage_method(
    class: Node<'_>,
    method: Node<'_>,
    path: &str,
    graph: &Graph,
    archive: &Archive<'_>,
) -> bool {
    let m = &graph.modules[path];
    let ms = methods(class, m);
    let Some(ctor) = ms.get("constructor") else {
        return false;
    };
    let leaves = return_leaves(method, m);
    if leaves.len() != 1 {
        return false;
    }
    for call in leaves.into_iter().filter(|n| path_join(*n, m)) {
        let args = arguments(call);
        if args.len() != 4
            || !this_field(args[2], "currentAccountId", m)
            || !this_field(args[3], "currentOrgId", m)
        {
            continue;
        }
        let Some(root) = chain(args[0], m).filter(|c| c.len() == 2 && c[0] == "this") else {
            continue;
        };
        let Some(base) = chain(args[1], m).filter(|c| c.len() == 2 && c[0] == "this") else {
            continue;
        };
        let assignments = find(*ctor, "assignment_expression");
        let root_ok = assignments.iter().any(|a| {
            a.child_by_field_name("left")
                .is_some_and(|n| this_field(n, &root[1], m))
                && a.child_by_field_name("right").is_some_and(|n| {
                    n.kind() == "call_expression"
                        && n.child_by_field_name("function")
                            .and_then(|f| chain(f, m))
                            .is_some_and(|c| c.last().is_some_and(|s| s == "getPath"))
                        && arguments(n).first().and_then(|n| string(*n, m)).as_deref()
                            == Some("userData")
                })
        });
        if !root_ok {
            continue;
        }
        let base_value = assignments.iter().find_map(|a| {
            if a.child_by_field_name("left")
                .is_some_and(|n| this_field(n, &base[1], m))
            {
                a.child_by_field_name("right")
            } else {
                None
            }
        });
        let Some(value) = base_value else {
            continue;
        };
        let literal = graph.constant(archive, path, value, 0);
        let Some(constructor_params) = params(*ctor, m) else {
            continue;
        };
        let index = constructor_params.iter().position(|p| p == m.text(value));
        if literal.as_deref() != Some("claude-code-sessions") && index.is_none() {
            continue;
        }
        let Some(instances) = possible_instances(class, path, graph, archive) else {
            continue;
        };
        let mut witnessed = false;
        let mut incompatible = false;
        for (other_path, instance) in instances {
            let args = arguments(instance);
            if literal.as_deref() == Some("claude-code-sessions")
                || index
                    .and_then(|i| args.get(i))
                    .and_then(|n| graph.constant(archive, other_path, *n, 0))
                    .as_deref()
                    == Some("claude-code-sessions")
            {
                witnessed = true;
            } else {
                incompatible = true;
            }
        }
        if witnessed && !incompatible {
            return true;
        }
    }
    false
}

fn calls_this(n: Node<'_>, name: &str, m: &Module) -> bool {
    is_call(n, &["this", name], m)
}
fn directory_binding(n: Node<'_>, storage: &str, class: Node<'_>, m: &Module) -> bool {
    let n = expand(n, m, 0);
    if calls_this(n, storage, m) {
        return true;
    }
    let Some(c) = n.child_by_field_name("function").and_then(|f| chain(f, m)) else {
        return false;
    };
    if c.len() != 3 || c[0] != "this" {
        return false;
    }
    let ms = methods(class, m);
    let Some(ctor) = ms.get("constructor") else {
        return false;
    };
    for assign in find(*ctor, "assignment_expression") {
        if !assign
            .child_by_field_name("left")
            .is_some_and(|n| this_field(n, &c[1], m))
        {
            continue;
        }
        let Some(new) = assign
            .child_by_field_name("right")
            .filter(|n| n.kind() == "new_expression")
        else {
            continue;
        };
        let Some(name) = new.child_by_field_name("constructor") else {
            continue;
        };
        let Some(def) = m
            .definitions
            .get(m.text(name))
            .and_then(|s| s.node(&m.tree))
        else {
            continue;
        };
        let delegate = methods(def, m);
        let Some(method) = delegate.get(&c[2]) else {
            continue;
        };
        let args = arguments(new);
        let Some(config) = args.first() else {
            continue;
        };
        for call in find(*method, "call_expression") {
            let Some(dc) = call
                .child_by_field_name("function")
                .and_then(|n| chain(n, m))
            else {
                continue;
            };
            if dc.len() != 3 || dc[0] != "this" {
                continue;
            }
            if let Some(callback) = property(*config, &dc[2], m) {
                if find(callback, "call_expression")
                    .iter()
                    .any(|n| calls_this(*n, storage, m))
                {
                    return true;
                }
            }
        }
    }
    false
}
fn row_filename(n: Node<'_>, id: &str, m: &Module) -> bool {
    let n = unwrap(n);
    if n.kind() == "template_string" {
        let substitutions = find(n, "template_substitution");
        let fragments = find(n, "string_fragment");
        return substitutions.len() == 1
            && substitutions[0]
                .named_child(0)
                .is_some_and(|n| m.text(n) == id)
            && fragments.len() == 1
            && m.text(fragments[0]) == ".json";
    }
    n.kind() == "binary_expression"
        && n.child_by_field_name("left")
            .is_some_and(|n| m.text(n) == id)
        && n.child_by_field_name("operator")
            .is_some_and(|n| m.text(n) == "+")
        && n.child_by_field_name("right")
            .and_then(|n| string(n, m))
            .as_deref()
            == Some(".json")
}
fn file_method(method: Node<'_>, storage: &str, class: Node<'_>, m: &Module) -> bool {
    if !m.text(method).contains(".json") {
        return false;
    }
    let Some(ps) = params(method, m) else {
        return false;
    };
    let Some(id) = ps.first() else {
        return false;
    };
    let leaves = return_leaves(method, m);
    !leaves.is_empty()
        && leaves.iter().all(|n| {
            let args = arguments(*n);
            path_join(*n, m)
                && args.len() == 2
                && directory_binding(args[0], storage, class, m)
                && row_filename(args[1], id, m)
        })
}

fn returned_object<'a>(function: Node<'a>, m: &Module) -> Option<Node<'a>> {
    let returns = own_returns(function);
    if returns.len() != 1 {
        return None;
    }
    for ret in returns {
        let Some(value) = ret.named_child(0) else {
            continue;
        };
        let value = expand(value, m, 0);
        if value.kind() != "object" {
            continue;
        }
        return Some(value);
    }
    None
}
fn object_members<'a>(
    object: Node<'a>,
    m: &Module,
    depth: usize,
) -> Option<BTreeMap<String, Node<'a>>> {
    if depth > 8 {
        return None;
    }
    let object = expand(object, m, 0);
    if object.kind() != "object" {
        return None;
    }
    let mut fields = BTreeMap::new();
    for i in 0..object.named_child_count() {
        let p = object.named_child(i)?;
        match p.kind() {
            "comment" => (),
            "pair" => {
                let key = property_key(p.child_by_field_name("key")?, m)?;
                if key.len() > 128 {
                    return None;
                }
                // JavaScript object overrides are ordered; analyze the effective
                // core identities after every spread and explicit assignment.
                fields.insert(key, p.child_by_field_name("value")?);
            }
            "spread_element" => fields.extend(object_members(p.named_child(0)?, m, depth + 1)?),
            _ => return None,
        }
        if fields.len() > 2048 {
            return None;
        }
    }
    Some(fields)
}
// Follow only the core fields through a locally resolved loader preprocessor.
// Every return must preserve both IDs; only cwd may pass through the existing
// one-argument path normalization rule. Unknown spreads/aliases fail closed.
fn binding_is_written(function: Node<'_>, name: &str, m: &Module) -> bool {
    find(function, "assignment_expression")
        .into_iter()
        .chain(find(function, "update_expression"))
        .filter(|n| owner(*n) == Some(function))
        .any(|n| {
            n.child_by_field_name("left")
                .or_else(|| n.child_by_field_name("argument"))
                .and_then(|n| chain(n, m))
                .is_some_and(|c| c.first().is_some_and(|first| first == name))
        })
}

fn original_parameter_reference(
    reference: Node<'_>,
    function: Node<'_>,
    name: &str,
    m: &Module,
) -> bool {
    let mentions = |pattern: Node<'_>| {
        let mut found = false;
        walk(pattern, |n, _| {
            if matches!(
                n.kind(),
                "identifier" | "shorthand_property_identifier_pattern"
            ) && m.text(n) == name
            {
                found = true;
            }
            true
        });
        found
    };
    let mut scope = reference.parent();
    while let Some(node) = scope {
        if node == function {
            return params(function, m).is_some_and(|p| p.iter().any(|p| p == name));
        }
        if node.kind() == "catch_clause"
            && node.child_by_field_name("parameter").is_some_and(mentions)
        {
            return false;
        }
        if node.kind() == "statement_block" {
            let mut shadowed = false;
            walk_with(
                node,
                |declaration, _| {
                    if declaration.kind() == "variable_declarator"
                        && declaration
                            .child_by_field_name("name")
                            .is_some_and(mentions)
                    {
                        shadowed = true;
                    }
                    if matches!(
                        declaration.kind(),
                        "function_declaration" | "class_declaration"
                    ) && declaration
                        .child_by_field_name("name")
                        .is_some_and(mentions)
                    {
                        shadowed = true;
                    }
                    true
                },
                |n| {
                    n == node
                        || !matches!(
                            n.kind(),
                            "statement_block"
                                | "function_declaration"
                                | "function_expression"
                                | "arrow_function"
                                | "method_definition"
                                | "class"
                                | "class_declaration"
                        )
                },
            );
            if shadowed {
                return false;
            }
        }
        scope = node.parent();
    }
    false
}

// Preprocessors must be observational for the input record. Decline all direct
// writes and passing the whole record (including lexical aliases) to another
// call. Field-only path helpers such as normalize(input.cwd) remain admissible.
fn observational_preprocessor(function: Node<'_>, input: &str, m: &Module) -> bool {
    if find(function, "assignment_expression")
        .into_iter()
        .chain(find(function, "update_expression"))
        .next()
        .is_some()
        || find(function, "unary_expression").into_iter().any(|n| {
            n.child_by_field_name("operator")
                .is_some_and(|n| m.text(n) == "delete")
        })
    {
        return false;
    }
    let mut aliases = BTreeSet::from([input.to_owned()]);
    let declarations = find(function, "variable_declarator");
    for _ in 0..8 {
        let previous = aliases.len();
        for declaration in &declarations {
            if owner(*declaration) != Some(function) {
                continue;
            }
            if let (Some(name), Some(value)) = (
                declaration.child_by_field_name("name"),
                declaration.child_by_field_name("value"),
            ) {
                if name.kind() == "identifier"
                    && unwrap(value).kind() == "identifier"
                    && aliases.contains(m.text(unwrap(value)))
                {
                    aliases.insert(m.text(name).to_owned());
                }
            }
        }
        if aliases.len() == previous {
            break;
        }
    }
    let captured = |reference: Node<'_>| {
        let name = m.text(reference);
        if !aliases.contains(name) {
            return false;
        }
        let mut scope = reference.parent();
        while let Some(node) = scope {
            if node == function {
                return true;
            }
            if matches!(
                node.kind(),
                "function_declaration"
                    | "function_expression"
                    | "arrow_function"
                    | "method_definition"
            ) && params(node, m).is_none_or(|p| p.iter().any(|p| p == name))
            {
                return false;
            }
            if node.kind() == "statement_block" && owner(node) != Some(function) {
                let mut shadowed = false;
                walk_with(
                    node,
                    |n, _| {
                        if n.kind() == "variable_declarator"
                            && n.child_by_field_name("name")
                                .is_some_and(|n| m.text(n) == name)
                        {
                            shadowed = true;
                        }
                        true
                    },
                    |n| {
                        n == node
                            || !matches!(
                                n.kind(),
                                "statement_block"
                                    | "function_declaration"
                                    | "function_expression"
                                    | "arrow_function"
                                    | "method_definition"
                            )
                    },
                );
                if shadowed {
                    return false;
                }
            }
            scope = node.parent();
        }
        false
    };
    for declaration in declarations {
        let Some(value) = declaration.child_by_field_name("value") else {
            continue;
        };
        let value = unwrap(value);
        if value.kind() == "identifier" && owner(declaration) == Some(function) {
            continue;
        }
        if find(value, "identifier")
            .into_iter()
            .chain(find(value, "shorthand_property_identifier"))
            .chain((value.kind() == "identifier").then_some(value))
            .any(|reference| {
                captured(reference)
                    && !reference.parent().is_some_and(|parent| {
                        matches!(parent.kind(), "member_expression" | "subscript_expression")
                            && parent.child_by_field_name("object") == Some(reference)
                    })
            })
        {
            return false;
        }
    }
    !find(function, "call_expression").into_iter().any(|call| {
        if is_call(call, &["eval"], m) {
            return true;
        }
        arguments(call).iter().any(|arg| {
            if unwrap(*arg).kind() == "spread_element" {
                return true;
            }
            find(*arg, "identifier")
                .into_iter()
                .chain(find(*arg, "shorthand_property_identifier"))
                .chain((arg.kind() == "identifier").then_some(*arg))
                .any(|reference| {
                    captured(reference)
                        && !reference.parent().is_some_and(|parent| {
                            matches!(parent.kind(), "member_expression" | "subscript_expression")
                                && parent.child_by_field_name("object") == Some(reference)
                        })
                })
        }) || call.child_by_field_name("function").is_some_and(|fun| {
            let mut object = unwrap(fun);
            while matches!(object.kind(), "member_expression" | "subscript_expression") {
                let Some(next) = object.child_by_field_name("object") else {
                    return true;
                };
                object = unwrap(next);
            }
            object.kind() == "identifier" && captured(object)
        })
    })
}

fn loader_value_origin(
    value: Node<'_>,
    key: &str,
    function: Node<'_>,
    input: &str,
    path: &str,
    graph: &Graph,
    archive: &Archive<'_>,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    let m = &graph.modules[path];
    let value = unwrap(value);
    if let Some(c) = chain(value, m).filter(|c| c.len() == 2 && c[1] == key) {
        let Some(record) = value.child_by_field_name("object") else {
            return false;
        };
        return loader_record_origin(
            record,
            key,
            function,
            input,
            path,
            graph,
            archive,
            depth + 1,
        ) && !binding_is_written(function, &c[0], m);
    }
    let args = arguments(value);
    key == "cwd"
        && value.kind() == "call_expression"
        && args.len() == 1
        && loader_value_origin(
            args[0],
            key,
            function,
            input,
            path,
            graph,
            archive,
            depth + 1,
        )
}

fn loader_record_origin(
    record: Node<'_>,
    key: &str,
    function: Node<'_>,
    input: &str,
    path: &str,
    graph: &Graph,
    archive: &Archive<'_>,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    let m = &graph.modules[path];
    let record = unwrap(record);
    if record.kind() == "identifier" {
        let name = m.text(record);
        if binding_is_written(function, name, m) {
            return false;
        }
        if name == input && owner(record) == Some(function) {
            return original_parameter_reference(record, function, input, m);
        }
        return initializer(record, name, m).is_some_and(|value| {
            owner(value) == Some(function)
                && loader_record_origin(
                    value,
                    key,
                    function,
                    input,
                    path,
                    graph,
                    archive,
                    depth + 1,
                )
        });
    }
    if record.kind() == "object" {
        let mut preserved = false;
        for i in 0..record.named_child_count() {
            let Some(member) = record.named_child(i) else {
                return false;
            };
            match member.kind() {
                "comment" => (),
                "pair" => {
                    let Some(name) = member
                        .child_by_field_name("key")
                        .and_then(|n| property_key(n, m))
                    else {
                        return false;
                    };
                    if name == key {
                        preserved = member.child_by_field_name("value").is_some_and(|value| {
                            loader_value_origin(
                                value,
                                key,
                                function,
                                input,
                                path,
                                graph,
                                archive,
                                depth + 1,
                            )
                        });
                    }
                }
                "spread_element" => {
                    preserved = member.named_child(0).is_some_and(|value| {
                        loader_record_origin(
                            value,
                            key,
                            function,
                            input,
                            path,
                            graph,
                            archive,
                            depth + 1,
                        )
                    });
                }
                _ => return false,
            }
        }
        return preserved;
    }
    if record.kind() == "call_expression" {
        let args = arguments(record);
        if args.len() != 1
            || !loader_record_origin(
                args[0],
                key,
                function,
                input,
                path,
                graph,
                archive,
                depth + 1,
            )
        {
            return false;
        }
        let Some((target, helper)) = record
            .child_by_field_name("function")
            .and_then(|fun| graph.resolve(archive, path, fun))
        else {
            return false;
        };
        let Some(helper_input) =
            params(helper, &graph.modules[target]).and_then(|p| p.into_iter().next())
        else {
            return false;
        };
        if !observational_preprocessor(helper, &helper_input, &graph.modules[target]) {
            return false;
        }
        let returns = own_returns(helper);
        return !returns.is_empty()
            && returns.into_iter().all(|ret| {
                ret.named_child(0).is_some_and(|value| {
                    loader_record_origin(
                        value,
                        key,
                        helper,
                        &helper_input,
                        target,
                        graph,
                        archive,
                        depth + 1,
                    )
                })
            });
    }
    false
}

fn identity_projection(
    function: Node<'_>,
    path: &str,
    graph: &Graph,
    archive: &Archive<'_>,
    writer: bool,
) -> Option<BTreeSet<String>> {
    let m = &graph.modules[path];
    let input = params(function, m)?.into_iter().next()?;
    let object = returned_object(function, m)?;
    let members = object_members(object, m, 0)?;
    for key in ["sessionId", "cliSessionId", "cwd"] {
        let value = *members.get(key)?;
        if chain(value, m).as_deref() == Some(&[input.clone(), key.into()]) {
            continue;
        }
        if !writer && loader_value_origin(value, key, function, &input, path, graph, archive, 0) {
            continue;
        }
        // Loader cwd may be normalized by a local path helper, but its only
        // argument must still be the persisted cwd. The writer is byte-direct.
        let args = arguments(unwrap(value));
        if writer
            || key != "cwd"
            || args.len() != 1
            || chain(args[0], m).as_deref() != Some(&[input.clone(), key.into()])
        {
            return None;
        }
    }
    Some(members.into_keys().collect())
}

fn call_function<'a>(call: Node<'a>, m: &Module) -> Option<Node<'a>> {
    let fun = call.child_by_field_name("function")?;
    // tree-sitter-js represents minified await(0, fs.method)(args) as a
    // nested call in this grammar. Only normalize it inside a syntactically
    // async owner, where await is the language operator, not a local function.
    if fun.kind() == "call_expression"
        && is_call(fun, &["await"], m)
        && owner(fun).is_some_and(|o| {
            (0..o.child_count()).any(|i| o.child(i).is_some_and(|n| n.kind() == "async"))
        })
    {
        let args = arguments(fun);
        if args.len() == 2 && args[0].kind() == "number" && m.text(args[0]) == "0" {
            return Some(args[1]);
        }
        if args.len() == 1 {
            return Some(unwrap(args[0]));
        }
    }
    Some(fun)
}
fn io_function(
    graph: &Graph,
    archive: &Archive<'_>,
    path: &str,
    call: Node<'_>,
    operation: &str,
) -> bool {
    let m = &graph.modules[path];
    let Some(fun) = call_function(call, m) else {
        return false;
    };
    if chain(fun, m).is_some_and(|c| {
        c.last().is_some_and(|s| s == operation)
            && m.imports.get(&c[0]).is_some_and(|s| {
                matches!(
                    s.as_str(),
                    "fs" | "node:fs" | "fs/promises" | "node:fs/promises"
                )
            })
    }) {
        return true;
    }
    let Some((target, function)) = graph.resolve(archive, path, fun) else {
        return false;
    };
    let other = &graph.modules[target];
    let Some(helper_params) = params(function, other) else {
        return false;
    };
    find(function, "call_expression").iter().any(|n| {
        call_function(*n, other)
            .and_then(|f| chain(f, other))
            .is_some_and(|c| {
                c.last().is_some_and(|s| s == operation)
                    && (operation != "writeFile" || {
                        let args = arguments(*n);
                        helper_params.len() >= 2
                            && args.len() >= 2
                            && other.text(args[0]) == helper_params[0]
                            && other.text(args[1]) == helper_params[1]
                    })
                    && (other.imports.get(&c[0]).is_some_and(|s| {
                        matches!(
                            s.as_str(),
                            "fs" | "node:fs" | "fs/promises" | "node:fs/promises"
                        )
                    }) || operation == "readFile"
                        && c.len() == 2
                        && find(function, "call_expression").iter().any(|open| {
                            open.child_by_field_name("function")
                                .and_then(|f| chain(f, other))
                                .is_some_and(|c| {
                                    c.last().is_some_and(|s| s == "open")
                                        && other
                                            .imports
                                            .get(&c[0])
                                            .is_some_and(|s| s == "node:fs" || s == "fs")
                                })
                        }))
            })
    })
}
fn loader(
    method: Node<'_>,
    storage: &str,
    class: Node<'_>,
    path: &str,
    graph: &Graph,
    archive: &Archive<'_>,
) -> bool {
    let m = &graph.modules[path];
    let calls = find(method, "call_expression");
    if !calls.iter().any(|n| is_call(*n, &["JSON", "parse"], m))
        || !calls.iter().any(|n| {
            n.child_by_field_name("function")
                .and_then(|f| chain(f, m))
                .is_some_and(|c| c.len() == 3 && c[0] == "this" && c[2] == "set")
        })
    {
        return false;
    }
    let dir_read = calls.iter().any(|n| {
        let args = arguments(*n);
        io_function(graph, archive, path, *n, "readdir")
            && args
                .first()
                .is_some_and(|n| directory_binding(*n, storage, class, m))
    });
    if !dir_read {
        return false;
    }
    // Both suffix/prefix checks must belong to the same conjunction/predicate.
    let filters = find(method, "binary_expression").iter().any(|n| {
        if !n
            .child_by_field_name("operator")
            .is_some_and(|n| m.text(n) == "&&")
        {
            return false;
        }
        let (Some(left), Some(right)) = (
            n.child_by_field_name("left"),
            n.child_by_field_name("right"),
        ) else {
            return false;
        };
        let check = |n: Node<'_>, property: &str, value: &str| -> Option<String> {
            let n = unwrap(n);
            let c = chain(n.child_by_field_name("function")?, m)?;
            let args = arguments(n);
            if c.len() != 2
                || c[1] != property
                || args.len() != 1
                || string(args[0], m).as_deref() != Some(value)
            {
                return None;
            }
            Some(c[0].clone())
        };
        check(left, "startsWith", "local_")
            .zip(check(right, "endsWith", ".json"))
            .is_some_and(|(a, b)| a == b)
            || check(right, "startsWith", "local_")
                .zip(check(left, "endsWith", ".json"))
                .is_some_and(|(a, b)| a == b)
    });
    if !filters {
        return false;
    }
    for set in calls.iter().filter(|n| {
        n.child_by_field_name("function")
            .and_then(|f| chain(f, m))
            .is_some_and(|c| c.len() == 3 && c[0] == "this" && c[2] == "set")
    }) {
        let args = arguments(*set);
        if args.len() != 2 {
            continue;
        }
        let Some(key) = chain(args[0], m).filter(|c| c.len() == 2 && c[1] == "sessionId") else {
            continue;
        };
        let Some(raw) = initializer(args[0], &key[0], m) else {
            continue;
        };
        if !is_call(unwrap(raw), &["JSON", "parse"], m) {
            continue;
        }
        let normalized = expand(args[1], m, 0);
        let nargs = arguments(normalized);
        if nargs.first().is_none_or(|n| m.text(*n) != key[0]) {
            continue;
        }
        let Some(fun) = normalized.child_by_field_name("function") else {
            continue;
        };
        let Some((target, function)) = graph.resolve(archive, path, fun) else {
            continue;
        };
        if identity_projection(function, target, graph, archive, false).is_none() {
            continue;
        }
        // Associate the parsed text with a read of a row path in this namespace.
        let pargs = arguments(unwrap(raw));
        let Some(text) = pargs.first() else {
            continue;
        };
        let text = expand(*text, m, 0);
        let read = if let Some(c) = chain(text, m).filter(|c| c.len() == 2 && c[1] == "text") {
            initializer(text, &c[0], m).map(|n| expand(n, m, 0))
        } else {
            Some(text)
        };
        let Some(read) = read else {
            continue;
        };
        if !io_function(graph, archive, path, read, "readFile") {
            continue;
        }
        let rargs = arguments(read);
        let Some(row) = rargs.first() else {
            continue;
        };
        let row = expand(*row, m, 0);
        let rargs = arguments(row);
        if path_join(row, m)
            && rargs.len() == 2
            && directory_binding(rargs[0], storage, class, m)
            && row_from_filtered_collection(rargs[1], method, storage, class, m)
        {
            return true;
        }
    }
    false
}
fn writer(
    method: Node<'_>,
    file: &str,
    path: &str,
    graph: &Graph,
    archive: &Archive<'_>,
) -> Result<Option<BTreeSet<String>>> {
    let m = &graph.modules[path];
    if !m.text(method).contains("stringify") {
        return Ok(None);
    }
    let calls = find(method, "call_expression");
    if !calls.iter().any(|n| is_call(*n, &["JSON", "stringify"], m))
        || !calls.iter().any(|n| calls_this(*n, file, m))
    {
        return Ok(None);
    }
    let Some(input) = params(method, m).and_then(|p| p.into_iter().next()) else {
        return Ok(None);
    };
    for call in calls {
        let args = arguments(call);
        if args.len() != 2 {
            continue;
        }
        let destination = expand(args[0], m, 0);
        if !calls_this(destination, file, m) {
            continue;
        }
        let ids = arguments(destination);
        if ids.len() != 1
            || chain(ids[0], m).as_deref() != Some(&[input.clone(), "sessionId".into()])
        {
            continue;
        }
        let encoded = expand(args[1], m, 0);
        if !is_call(encoded, &["JSON", "stringify"], m) {
            continue;
        }
        if !io_function(graph, archive, path, call, "writeFile") {
            continue;
        }
        let encoded_args = arguments(encoded);
        let projected = expand(*encoded_args.first().ok_or(UNSUPPORTED)?, m, 0);
        let projected_args = arguments(projected);
        if projected_args.first().is_none_or(|n| m.text(*n) != input) {
            return Err(UNSUPPORTED.into());
        }
        let fun = projected
            .child_by_field_name("function")
            .ok_or(UNSUPPORTED)?;
        let (target, function) = graph.resolve(archive, path, fun).ok_or(UNSUPPORTED)?;
        let fields =
            identity_projection(function, target, graph, archive, true).ok_or(UNSUPPORTED)?;
        return Ok(Some(fields));
    }
    Ok(None)
}
fn detect(bytes: &[u8]) -> Result<BTreeSet<String>> {
    let archive = Archive::parse(bytes)?;
    let package: Value =
        serde_json::from_str(archive.source("package.json")?).map_err(|_| UNSUPPORTED)?;
    let main = package
        .get("main")
        .and_then(Value::as_str)
        .ok_or(UNSUPPORTED)?;
    if main.starts_with('/') || main.split('/').any(|s| !safe_segment(s) && s != ".") {
        return Err(UNSUPPORTED.into());
    }
    let main = main.trim_start_matches("./").to_owned();
    let graph = Graph::load(&archive, main)?;
    let mut accepted = None;
    for (path, m) in &graph.modules {
        // Necessary-token prefilters only avoid expensive role walks. Every
        // admitted candidate still has to satisfy the structural data flow.
        if !["currentAccountId", "currentOrgId", "getPath"]
            .iter()
            .all(|s| m.source.contains(s))
        {
            continue;
        }
        for span in &m.classes {
            if graph.started.elapsed() > MAX_TIME {
                return Err(UNSUPPORTED.into());
            }
            let Some(class) = span.node(&m.tree) else {
                continue;
            };
            let ms = methods(class, m);
            for (storage, method) in &ms {
                if graph.started.elapsed() > MAX_TIME {
                    return Err(UNSUPPORTED.into());
                }
                if !["currentAccountId", "currentOrgId"]
                    .iter()
                    .all(|s| m.text(*method).contains(s))
                {
                    continue;
                }
                // Cheap role filter before following constructor instantiations.
                if !find(*method, "call_expression")
                    .iter()
                    .any(|n| path_join(*n, m) && arguments(*n).len() == 4)
                {
                    continue;
                }
                if !storage_method(class, *method, path, &graph, &archive) {
                    continue;
                }
                let files: Vec<_> = ms
                    .iter()
                    .filter(|(_, n)| file_method(**n, storage, class, m))
                    .map(|(name, _)| name)
                    .collect();
                let loaded = ms
                    .values()
                    .any(|n| loader(*n, storage, class, path, &graph, &archive));
                if files.is_empty() || !loaded {
                    continue;
                }
                for file in files {
                    for method in ms.values() {
                        if let Some(fields) = writer(*method, file, path, &graph, &archive)? {
                            if accepted
                                .as_ref()
                                .is_some_and(|previous| previous != &fields)
                            {
                                return Err(UNSUPPORTED.into());
                            }
                            accepted = Some(fields);
                        }
                    }
                }
            }
        }
    }
    accepted.ok_or_else(|| UNSUPPORTED.into())
}

#[cfg(test)]
pub(super) fn synthetic_archive_for_test(extra_projection: &str) -> Vec<u8> {
    tests::fixture(
        tests::MAIN,
        &tests::PROJECTION.replace(
            "title:state.title",
            &format!("title:state.title{extra_projection}"),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn asar(files: &[(&str, &str)]) -> Vec<u8> {
        let mut root = serde_json::json!({"files":{}});
        let mut data = Vec::new();
        for (path, source) in files {
            let parts: Vec<_> = path.split('/').collect();
            let mut node = &mut root;
            for part in &parts[..parts.len() - 1] {
                if node["files"].get(*part).is_none() {
                    node["files"][*part] = serde_json::json!({"files":{}});
                }
                node = &mut node["files"][*part];
            }
            node["files"][parts[parts.len() - 1]] =
                serde_json::json!({"offset":data.len().to_string(),"size":source.len()});
            data.extend_from_slice(source.as_bytes());
        }
        let json = serde_json::to_vec(&root).unwrap();
        let size = 8 + json.len().div_ceil(4) * 4;
        let mut out = Vec::new();
        for word in [4, size, size - 4, json.len()] {
            out.extend_from_slice(&(word as u32).to_le_bytes());
        }
        out.extend(json);
        out.resize(8 + size, 0);
        out.extend(data);
        out
    }
    pub(super) const MAIN: &str = r#"
const p=require('node:path'), f=require('node:fs'), electron=require('electron'), x=require('./projection.js');
class Manager {
 constructor(){this.userDataPath=electron.app.getPath('userData');this.baseDir='claude-code-sessions';}
 storage(){return p.join(this.userDataPath,this.baseDir,this.currentAccountId,this.currentOrgId);}
 rowPath(id){const dir=this.storage();return p.join(dir,`${id}.json`);}
 async load(){const dir=this.storage();const names=await f.promises.readdir(dir);
  const rows=names.filter(name=>name.startsWith('local_')&&name.endsWith('.json'));
  for(const name of rows){const text=await f.promises.readFile(p.join(dir,name));const raw=JSON.parse(text);
   const active=x.load(raw);this.sessions.set(raw.sessionId,active);}}
 async save(row){const dest=this.rowPath(row.sessionId);const projected=x.persist(row);
  const encoded=JSON.stringify(projected);await f.promises.writeFile(dest,encoded);}
}
new Manager();
"#;
    pub(super) const PROJECTION: &str = r#"
function persist(state){return {sessionId:state.sessionId,cliSessionId:state.cliSessionId,cwd:state.cwd,title:state.title};}
function load(raw){return {sessionId:raw.sessionId,cliSessionId:raw.cliSessionId,cwd:raw.cwd};}
exports.persist=persist;exports.load=load;
"#;
    pub(super) fn fixture(main: &str, projection: &str) -> Vec<u8> {
        asar(&[
            ("package.json", r#"{"main":"entry.js"}"#),
            ("entry.js", main),
            ("projection.js", projection),
        ])
    }
    #[test]
    fn linked_storage_contract_without_package_version() {
        let fields = detect(&fixture(MAIN, PROJECTION)).unwrap();
        assert_eq!(fields.len(), 4);
        assert!(fields.contains("cwd"));
    }
    fn preprocessing_projection(helper: &str) -> String {
        PROJECTION.replace(
            "function load(raw){return {sessionId:raw.sessionId,cliSessionId:raw.cliSessionId,cwd:raw.cwd};}",
            &format!("{helper}\nfunction load(raw){{const normalized=preprocess(raw);return {{sessionId:normalized.sessionId,cliSessionId:normalized.cliSessionId,cwd:normalize(normalized.cwd)}};}}"),
        )
    }
    #[test]
    fn loader_preprocessing_preserves_ids_on_every_return() {
        let projection = preprocessing_projection(
            "function preprocess(raw){if(!raw.cwd)return raw;const path=value=>value;return {...raw,cwd:path(raw.cwd)};}",
        );
        assert_eq!(detect(&fixture(MAIN, &projection)).unwrap().len(), 4);
    }
    #[test]
    fn loader_preprocessing_rejects_changed_ids_unknown_spreads_and_mutation() {
        for helper in [
            "function preprocess(raw){if(raw.flag)return {...raw,sessionId:'changed'};return raw;}",
            "function preprocess(raw){return {...raw,cliSessionId:raw.sessionId};}",
            "function preprocess(raw){return {...raw,...unknown};}",
            "function preprocess(raw){raw.sessionId='changed';return raw;}",
            "function preprocess(raw){return unknown(raw);}",
            "function preprocess(raw){const alias=raw;alias.sessionId='changed';return {...raw};}",
            "function preprocess(raw){raw[key]='changed';return raw;}",
            "function preprocess(raw){Object.assign(raw,{sessionId:'changed'});return raw;}",
            "function preprocess(raw){delete raw.sessionId;return raw;}",
            "function preprocess(raw){{const {raw}=other;return raw;}}",
            "function preprocess(raw){{let raw;return raw;}}",
            "function preprocess(raw){try{return raw;}catch(raw){return raw;}}",
            "function preprocess(raw){(()=>{raw.sessionId='changed';})();return raw;}",
            "function preprocess(raw){{function raw(){}return raw;}}",
            "function preprocess(raw){{class raw{}return raw;}}",
            "function preprocess(raw){(()=>Object.assign(raw,{sessionId:'changed'}))();return raw;}",
            "function preprocess(raw){const alias=raw;(()=>Object.assign(alias,{sessionId:'changed'}))();return raw;}",
            "function preprocess(raw){mutate({record:raw});return raw;}",
            "function preprocess(raw){(()=>{const alias=raw;Object.assign(alias,{sessionId:'changed'});})();return raw;}",
            "function preprocess(raw){const container=[raw];Object.assign(container[0],{sessionId:'changed'});return raw;}",
            "function preprocess(raw){eval('raw.sessionId=1');return raw;}",
            "function preprocess(raw){Object.assign(({raw}).raw,{sessionId:'changed'});return raw;}",
            "function preprocess(raw){const box={raw};Object.assign(box.raw,{sessionId:'changed'});return raw;}",
        ] {
            assert_eq!(
                detect(&fixture(MAIN, &preprocessing_projection(helper))).unwrap_err(),
                UNSUPPORTED
            );
        }
    }
    #[test]
    fn loader_preprocessing_rejects_shadowed_input_and_reassigned_alias() {
        for load in [
            "function load(raw){{const raw={};const normalized=preprocess(raw);return {sessionId:normalized.sessionId,cliSessionId:normalized.cliSessionId,cwd:normalized.cwd};}}",
            "function load(raw){let normalized=preprocess(raw);normalized=unknown;return {sessionId:normalized.sessionId,cliSessionId:normalized.cliSessionId,cwd:normalized.cwd};}",
        ] {
            let projection = PROJECTION.replace(
                "function load(raw){return {sessionId:raw.sessionId,cliSessionId:raw.cliSessionId,cwd:raw.cwd};}",
                &format!("function preprocess(raw){{return raw;}}{load}"),
            );
            assert_eq!(detect(&fixture(MAIN, &projection)).unwrap_err(), UNSUPPORTED);
        }
    }
    #[test]
    fn writer_does_not_inherit_loader_preprocessing_permission() {
        let projection = PROJECTION.replace(
            "function persist(state){return {sessionId:state.sessionId,cliSessionId:state.cliSessionId,cwd:state.cwd,title:state.title};}",
            "function preprocess(raw){return raw;}function persist(state){const normalized=preprocess(state);return {sessionId:normalized.sessionId,cliSessionId:normalized.cliSessionId,cwd:normalized.cwd};}",
        );
        assert_eq!(
            detect(&fixture(MAIN, &projection)).unwrap_err(),
            UNSUPPORTED
        );
    }
    #[test]
    fn formatting_chunks_identifiers_and_version_are_not_gates() {
        let main = MAIN
            .replace("Manager", "Renamed")
            .replace("storage()", "s()")
            .replace("this.storage()", "this.s()")
            .replace("rowPath", "r")
            .replace("projection.js", "changed.chunk.cjs")
            .replace("raw", "input")
            .replace("raw.sessionId", "input['sessionId']")
            .replace("row.sessionId", "row['sessionId']");
        let projection = PROJECTION
            .replace("state", "z")
            .replace("raw", "q")
            .replace("sessionId:z.sessionId", "'sessionId':z['sessionId']");
        let bytes = asar(&[
            (
                "package.json",
                r#"{"main":"entry.js","version":"future-arbitrary"}"#,
            ),
            ("entry.js", &main),
            ("changed.chunk.cjs", &projection),
        ]);
        assert!(detect(&bytes).is_ok());
    }
    #[test]
    fn changed_storage_or_identity_is_unsupported() {
        for main in [
            MAIN.replace("claude-code-sessions", "other-store"),
            MAIN.replace("this.currentOrgId", "this.otherOrg"),
            MAIN.replace("raw.sessionId,active", "raw.cliSessionId,active"),
            MAIN.replace("`${id}.json`", "`${id}.db`"),
            MAIN.replace("row.sessionId", "row.cliSessionId"),
            MAIN.replace("'.json'", "'.jsonl'"),
        ] {
            assert_eq!(
                detect(&fixture(&main, PROJECTION)).unwrap_err(),
                UNSUPPORTED
            );
        }
        for projection in [
            PROJECTION.replace("sessionId:state.sessionId", "sessionId:state.cliSessionId"),
            PROJECTION.replace(
                "cliSessionId:raw.cliSessionId",
                "cliSessionId:raw.sessionId",
            ),
            PROJECTION.replace("cwd:state.cwd", "cwd:'/different'"),
        ] {
            assert_eq!(
                detect(&fixture(MAIN, &projection)).unwrap_err(),
                UNSUPPORTED
            );
        }
    }
    #[test]
    fn unreachable_projection_comments_and_split_classes_do_not_pass() {
        let fake = MAIN.replace("const active=x.load(raw)", "const active=raw");
        assert!(detect(&fixture(&fake, PROJECTION)).is_err());
        let comments = format!(
            "// {}\nconst explanation={:?};",
            MAIN.replace('\n', " "),
            MAIN
        );
        assert!(detect(&fixture(&comments, PROJECTION)).is_err());
        let bytes = asar(&[
            ("package.json", r#"{"main":"entry.js"}"#),
            ("entry.js", "// require('./unused.js')"),
            ("unused.js", MAIN),
            ("projection.js", PROJECTION),
        ]);
        assert!(detect(&bytes).is_err());
        let split = MAIN.replace("async save(row)", "} class Other { async save(row)");
        assert!(detect(&fixture(&split, PROJECTION)).is_err());
    }
    #[test]
    fn malformed_frames_and_paths_are_rejected() {
        for bytes in [
            Vec::new(),
            vec![0; 16],
            asar(&[("../outside", "bad")]),
            asar(&[("package.json", r#"{"main":"../escape.js"}"#)]),
        ] {
            assert!(detect(&bytes).is_err());
        }
        let mut bytes = fixture(MAIN, PROJECTION);
        bytes[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert!(detect(&bytes).is_err());
    }
    #[test]
    fn archive_bound_applies_before_allocation() {
        let bytes = [
            4u32.to_le_bytes(),
            u32::MAX.to_le_bytes(),
            0u32.to_le_bytes(),
            0u32.to_le_bytes(),
        ]
        .concat();
        assert!(Archive::parse(&bytes).is_err());
    }

    #[test]
    fn unused_compatible_joins_do_not_prove_returned_paths() {
        for main in [
            MAIN.replace("storage(){return p.join(this.userDataPath,this.baseDir,this.currentAccountId,this.currentOrgId);}",
                "storage(){p.join(this.userDataPath,this.baseDir,this.currentAccountId,this.currentOrgId);return p.join(this.userDataPath,'other',this.currentAccountId,this.currentOrgId);}"),
            MAIN.replace("return p.join(dir,`${id}.json`);", "p.join(dir,`${id}.json`);return p.join('/elsewhere',`${id}.json`);"),
        ] { assert_eq!(detect(&fixture(&main,PROJECTION)).unwrap_err(),UNSUPPORTED); }
    }
    #[test]
    fn optional_native_and_data_modules_do_not_hide_missing_javascript() {
        let main=format!("try{{require('./crypto/build/Release/sshcrypto.node')}}catch{{}};require('./data.json');{MAIN}");
        assert!(detect(&fixture(&main, PROJECTION)).is_ok());
        let missing = format!("require('./required.js');{MAIN}");
        assert_eq!(
            detect(&fixture(&missing, PROJECTION)).unwrap_err(),
            UNSUPPORTED
        );
    }
    #[test]
    fn returned_spread_includes_outer_fields_and_effective_overrides() {
        let projection=PROJECTION.replace("return {sessionId:state.sessionId,cliSessionId:state.cliSessionId,cwd:state.cwd,title:state.title};",
            "const normalized={sessionId:state.sessionId,cliSessionId:state.cliSessionId,cwd:state.cwd,title:state.title};return {...normalized,futureNativeState:state.futureNativeState};");
        let fields = detect(&fixture(MAIN, &projection)).unwrap();
        assert!(fields.contains("futureNativeState"));
        assert_eq!(fields.len(), 5);
        let wrong = projection.replace(
            "futureNativeState:state.futureNativeState",
            "futureNativeState:state.futureNativeState,cliSessionId:state.sessionId",
        );
        assert_eq!(detect(&fixture(MAIN, &wrong)).unwrap_err(), UNSUPPORTED);
        let dynamic = projection.replace("...normalized", "...state");
        assert!(detect(&fixture(MAIN, &dynamic)).is_err());
    }
    #[test]
    fn computed_static_keys_are_names_but_dynamic_keys_are_unsupported() {
        let projection = PROJECTION.replace(
            "title:state.title",
            "title:state.title,['futureNativeState']:state.futureNativeState",
        );
        let fields = detect(&fixture(MAIN, &projection)).unwrap();
        assert!(fields.contains("futureNativeState"));
        assert!(!fields.contains("['futureNativeState']"));
        let dynamic = projection.replace("['futureNativeState']", "[state.fieldName]");
        assert_eq!(detect(&fixture(MAIN, &dynamic)).unwrap_err(), UNSUPPORTED);
    }

    struct SyntheticApp {
        root: PathBuf,
        app: PathBuf,
    }
    impl SyntheticApp {
        fn new() -> Self {
            let temp = std::env::temp_dir().canonicalize().unwrap();
            let root = temp.join(format!(
                "cockpit-contract-fixture-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let app = root.join("Fixture.app");
            std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
            Self { root, app }
        }
        fn archive(&self) -> PathBuf {
            self.app.join("Contents/Resources/app.asar")
        }
        fn write(&self, main: &str, projection: &str) {
            let bytes = asar(&[
                (
                    "package.json",
                    r#"{"main":"entry.js","version":"same-diagnostic"}"#,
                ),
                ("entry.js", main),
                ("projection.js", projection),
            ]);
            std::fs::write(self.archive(), bytes).unwrap();
        }
    }
    impl Drop for SyntheticApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn archive_changes_invalidate_old_transaction_but_new_compatible_archive_is_admitted() {
        let app = SyntheticApp::new();
        app.write(MAIN, PROJECTION);
        assert!(!app.app.join("Contents/Info.plist").exists());
        let old = inspect(&app.app).unwrap();
        old.assert_unchanged().unwrap();
        old.assert_unchanged_fast().unwrap();
        let upgraded = PROJECTION.replace(
            "title:state.title",
            "title:state.title,futureDisplay:state.futureDisplay",
        );
        app.write(MAIN, &upgraded);
        assert_eq!(old.assert_unchanged_fast().unwrap_err(), CHANGED);
        assert_eq!(old.assert_unchanged().unwrap_err(), CHANGED);
        let new = inspect(&app.app).unwrap();
        assert_ne!(old.fingerprint, new.fingerprint);
        assert!(new.projected_fields.contains("futureDisplay"));
        new.assert_unchanged().unwrap();
        app.write(
            &MAIN.replace("claude-code-sessions", "other-store"),
            PROJECTION,
        );
        assert_eq!(inspect(&app.app).unwrap_err(), UNSUPPORTED);
    }
    #[test]
    fn equal_archive_bytes_on_another_inode_invalidate_in_flight_witness() {
        let app = SyntheticApp::new();
        app.write(MAIN, PROJECTION);
        let old = inspect(&app.app).unwrap();
        let replacement = app.archive().with_extension("replacement");
        std::fs::write(&replacement, std::fs::read(app.archive()).unwrap()).unwrap();
        std::fs::rename(replacement, app.archive()).unwrap();
        assert_eq!(old.assert_unchanged_fast().unwrap_err(), CHANGED);
        assert_eq!(old.assert_unchanged().unwrap_err(), CHANGED);
        let fresh = inspect(&app.app).unwrap();
        assert_eq!(old.fingerprint, fresh.fingerprint);
        fresh.assert_unchanged().unwrap();
    }
    #[test]
    fn constructor_factory_literal_is_proved_but_shadowed_parameter_is_not_a_global_constant() {
        let parameter = MAIN
            .replace("constructor(){", "constructor(base){")
            .replace("this.baseDir='claude-code-sessions'", "this.baseDir=base");
        let factory = parameter.replace(
            "new Manager();",
            "function create(){return new Manager('claude-code-sessions')}create();",
        );
        assert!(detect(&fixture(&factory, PROJECTION)).is_ok());
        let wrong=parameter.replace("new Manager();","const base='claude-code-sessions';function create(base){return new Manager(base)}create('other-store');");
        assert_eq!(
            detect(&fixture(&wrong, PROJECTION)).unwrap_err(),
            UNSUPPORTED
        );
    }
    #[test]
    fn decoy_filter_cannot_prove_unfiltered_rows_and_import_thunks_remain_reachable() {
        let main = MAIN.replace("for(const name of rows)", "for(const name of names)");
        assert_eq!(
            detect(&fixture(&main, PROJECTION)).unwrap_err(),
            UNSUPPORTED
        );
        let bytes=asar(&[("package.json",r#"{"main":"bootstrap.js"}"#),("bootstrap.js","function start(){return Promise.resolve().then((()=>require('./entry.js')))}start();"),("entry.js",MAIN),("projection.js",PROJECTION)]);
        assert!(detect(&bytes).is_ok());
    }

    #[test]
    fn minified_async_writer_getter_preserves_path_data_and_options() {
        let main = format!(
            "const io=require('./io.js');{}",
            MAIN.replace(
                "await f.promises.writeFile(dest,encoded)",
                "await io.store(dest,encoded)"
            )
        );
        let io = r#"const fs=require('node:fs/promises');async function privateWrite(filename,bytes,options={}){await(0,fs.writeFile)(filename,bytes,{...options,mode:384})}Object.defineProperty(exports,'store',{get:function(){return privateWrite}});"#;
        let make = |io: &str| {
            asar(&[
                ("package.json", r#"{"main":"entry.js"}"#),
                ("entry.js", &main),
                ("projection.js", PROJECTION),
                ("io.js", io),
            ])
        };
        assert!(detect(&make(io)).is_ok());
        let nonasync = io.replace("async function privateWrite", "function privateWrite");
        assert_eq!(detect(&make(&nonasync)).unwrap_err(), UNSUPPORTED);
        let wrong_path = io.replace("(filename,bytes,{", "('/other',bytes,{");
        assert_eq!(detect(&make(&wrong_path)).unwrap_err(), UNSUPPORTED);
    }
    #[test]
    fn multiple_writers_with_the_same_projection_are_consistent_but_conflicting_writers_fail() {
        let copy="async saveAgain(row){const dest=this.rowPath(row.sessionId);const encoded=JSON.stringify(x.persist(row));await f.promises.writeFile(dest,encoded);}";
        let main = MAIN.replace(
            "\n}\nnew Manager();",
            &format!("\n{copy}\n}}\nnew Manager();"),
        );
        assert!(detect(&fixture(&main, PROJECTION)).is_ok());
        let wrong = main.replace("x.persist(row)", "x.bad(row)");
        let projection=format!("{PROJECTION}\nfunction bad(state){{return {{sessionId:state.sessionId,cliSessionId:state.sessionId,cwd:state.cwd}}}}exports.bad=bad;");
        assert_eq!(
            detect(&fixture(&wrong, &projection)).unwrap_err(),
            UNSUPPORTED
        );
    }

    #[test]
    fn default_parameters_preserve_projection_argument_positions() {
        let main = MAIN.replace("x.persist(row)", "x.persist(row,otherRow)");
        let projection =
            PROJECTION.replace("function persist(state)", "function persist(meta={},state)");
        assert_eq!(
            detect(&fixture(&main, &projection)).unwrap_err(),
            UNSUPPORTED
        );
        let valid = PROJECTION.replace(
            "function persist(state)",
            "function persist(state={},meta={})",
        );
        assert!(detect(&fixture(MAIN, &valid)).is_ok());
        for parameter in ["{state}", "...state"] {
            let unsupported = PROJECTION.replace(
                "function persist(state)",
                &format!("function persist({parameter})"),
            );
            assert_eq!(
                detect(&fixture(MAIN, &unsupported)).unwrap_err(),
                UNSUPPORTED
            );
        }
    }
    #[test]
    fn default_bindings_shadow_global_constants_and_spread_sources() {
        let parameter = MAIN
            .replace("constructor(){", "constructor(base){")
            .replace("this.baseDir='claude-code-sessions'", "this.baseDir=base");
        let main=parameter.replace("new Manager();","const base='claude-code-sessions';function create(base={}){return new Manager(base)}create('other-store');");
        assert_eq!(
            detect(&fixture(&main, PROJECTION)).unwrap_err(),
            UNSUPPORTED
        );
        let projection=PROJECTION.replace(
            "function persist(state){return {sessionId:state.sessionId,cliSessionId:state.cliSessionId,cwd:state.cwd,title:state.title};}",
            "const state={};const fields={sessionId:state.sessionId,cliSessionId:state.cliSessionId,cwd:state.cwd};function persist(state,fields={}){return {...fields};}");
        assert_eq!(
            detect(&fixture(MAIN, &projection)).unwrap_err(),
            UNSUPPORTED
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Explicit read-only inspection of the public installed Claude ASAR"]
    fn read_only_installed_contract() {
        let started = Instant::now();
        let contract = inspect(Path::new("/Applications/Claude.app")).unwrap();
        let cold_ms = started.elapsed().as_millis();
        let cached_at = Instant::now();
        let cached = inspect(Path::new("/Applications/Claude.app")).unwrap();
        let cached_ms = cached_at.elapsed().as_millis();
        assert_eq!(cached.fingerprint, contract.fingerprint);
        assert_eq!(cached.projected_fields, contract.projected_fields);
        contract.assert_unchanged().unwrap();
        contract.assert_unchanged_fast().unwrap();
        println!(
            "archive_fingerprint={} projected_fields={} cold_inspect_ms={} cached_inspect_ms={} elapsed_ms={}",
            contract.fingerprint,
            contract.projected_fields.len(),
            cold_ms,cached_ms,
            started.elapsed().as_millis()
        );
    }
}
