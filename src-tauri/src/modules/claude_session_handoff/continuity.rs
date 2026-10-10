//! Collect saved account namespaces; project every active branch without
//! replacing any destination row's transcript. The global manifest only maps
//! additional row IDs to their logical conversation. Transcripts stay shared.
use super::*;

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    aliases: BTreeMap<String, String>,
}

pub(super) fn state_path(roots: &Roots) -> PathBuf {
    roots.state.join("continuity").join("catalog.json")
}

fn alias_name(logical: &str, pointer: &str, salt: usize) -> String {
    let mut bytes =
        Sha256::digest(format!("cockpit-continuity-v1:{logical}:{pointer}:{salt}").as_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut id = [0u8; 16];
    id.copy_from_slice(&bytes[..16]);
    format!("local_{}.json", Uuid::from_bytes(id))
}

fn activity(value: &Value) -> f64 {
    value["lastActivityAt"]
        .as_f64()
        .filter(|n| n.is_finite())
        .unwrap_or(0.0)
}

fn base_title(value: &Value) -> String {
    let title = value["title"].as_str().unwrap_or("Code conversation");
    // Remove only our exact generated suffix, never arbitrary user parentheses.
    if let Some((base, suffix)) = title.rsplit_once(" （保留分支 ") {
        if suffix
            .strip_suffix('）')
            .is_some_and(|id| id.len() == 8 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return base.into();
        }
    }
    title.into()
}

// These fields are local conversation/workspace state in the reviewed
// projection. Account permissions, connectors and cloud ownership are handled
// separately below; queued work and dispatch receipts are never imported.
const LOCAL_FIELDS: &[&str] = &[
    "sessionId",
    "cliSessionId",
    "cwd",
    "originCwd",
    "worktreePath",
    "worktreeName",
    "worktreeLazy",
    "worktreePinned",
    "gitAnchors",
    "gitAnchorsLookupOnly",
    "gitAnchorsFolderRealpath",
    "sourceBranch",
    "branch",
    "createdAt",
    "lastActivityAt",
    "lastFocusedAt",
    "model",
    "effort",
    "effortInherited",
    "sessionSettings",
    "agent",
    "isArchived",
    "title",
    "titleSource",
    "previousTitles",
    "writtenBranches",
    "isStarred",
    "keptDirtyWorktree",
    "keptDirtyAt",
    "keptWorktreeLeftover",
    "contextExceededCount",
    "contextRecovery",
    "completedTurns",
    "subagentsTruncatedFor",
    "postTurnSummary",
    "postTurnSummaryFor",
    "lastAssistantUuid",
    "turnWrapUp",
    "forkedFromSessionId",
    "forkedAtMessageUuid",
    "lineageDetached",
    "priorCliSessionIds",
    "rewindEdges",
    "transcriptModelStates",
    "spawnedFrom",
    "lastTurnReport",
    "indexedAt",
    "color",
    "classifierSummaryEnabled",
    "reportFindingsCard",
    "scratchPromptRecents",
    "scratchOfferFolder",
    "scratchFilesLeftIn",
    "scratchCarried",
    "promptAppendSnapshot",
    "latestUserFrameAt",
    "recap",
    "recapAt",
    "error",
    "errorCategory",
    "errorAt",
    "errorRows",
    "tccFolderKind",
    "priorErrorMark",
];
const PERMISSION_FIELDS: &[&str] = &[
    "permissionMode",
    "bypassChosenInApp",
    "autoChosenInApp",
    "chromePermissionMode",
    "chromeAllowedDomains",
    "cuAllowedApps",
    "cuGrantFlags",
    "cuFlagsGrantedAt",
    "cuSelectedDisplayId",
    "enabledMcpTools",
    "remoteMcpServersConfig",
    "withheldConnectorHosts",
    "alwaysAllowedReasons",
    "sessionPermissionUpdates",
];

fn project(value: &Value, target: Option<&Value>, name: &str, title: String) -> Value {
    let mut out = target
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    // Existing rows keep their destination-only metadata. Only a later snapshot
    // of this same active transcript can refresh local conversation fields.
    if target.is_none_or(|target| activity(value) > activity(target)) {
        for key in LOCAL_FIELDS {
            if target.is_some() && ["title", "titleSource"].contains(key) {
                continue;
            }
            out.remove(*key);
            if let Some(value) = value.get(*key) {
                out.insert((*key).into(), value.clone());
            }
        }
    }
    // Match the installed native normalizer's restart semantics. In-flight
    // running descriptors are refused by unsupported() before publication;
    // retained core counters belong to this exact active transcript snapshot.
    if let Some(recovery) = out
        .get_mut("contextRecovery")
        .and_then(Value::as_object_mut)
    {
        let attempts = recovery
            .get("attempts")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let phase = recovery.get("phase").and_then(Value::as_str).unwrap_or("");
        if phase == "compacting" || (phase == "waiting" && attempts == 2) {
            recovery.insert(
                "phase".into(),
                json!(if attempts == 2 { "spent" } else { "waiting" }),
            );
        }
    }
    out.insert("sessionId".into(), json!(name.trim_end_matches(".json")));
    if out.get("title").and_then(Value::as_str) != Some(&title) {
        out.insert("title".into(), json!(title));
        out.insert("titleSource".into(), json!("user"));
    }
    let defaults = json!({"permissionMode":"default", "bypassChosenInApp":false,
        "chromePermissionMode":"ask", "chromeAllowedDomains":[],
        "cuAllowedApps":[], "cuGrantFlags":{"clipboardRead":false,"clipboardWrite":false,"systemKeyCombos":false},
        "enabledMcpTools":{}, "remoteMcpServersConfig":[], "alwaysAllowedReasons":[], "sessionPermissionUpdates":[]});
    let same_workspace = target.is_some_and(|target| target.get("cwd") == out.get("cwd"));
    if !same_workspace {
        for key in PERMISSION_FIELDS {
            out.remove(*key);
        }
        out.extend(defaults.as_object().unwrap().clone());
    }
    // Keep destination-specific configuration only for the same workspace.
    if let Some(target) = target.filter(|_| same_workspace) {
        for key in PERMISSION_FIELDS {
            if let Some(v) = target.get(*key) {
                out.insert((*key).into(), v.clone());
            }
        }
    }
    if let Some(target) = target {
        for key in ["isArchived", "isStarred", "color"] {
            if let Some(v) = target.get(key) {
                out.insert(key.into(), v.clone());
            }
        }
        for key in ["lastActivityAt", "completedTurns", "lastFocusedAt"] {
            if target[key]
                .as_f64()
                .is_some_and(|n| n > out.get(key).and_then(Value::as_f64).unwrap_or(0.0))
            {
                out.insert(key.into(), target[key].clone());
            }
        }
    }
    if target.is_none() {
        out.insert("bridgeSessionIds".into(), json!([]));
        out.insert("remoteControlAutoEligible".into(), json!(false));
        out.insert("remoteControlUserEnabled".into(), json!(false));
    }
    if target.is_none() {
        // Source dispatch state is never imported. Existing destination work
        // belongs to that row and must survive an unrelated catalog handoff.
        for key in [
            "armedWorkAtQuit",
            "pendingFirstStart",
            "interruptedByQuitAt",
            "interruptedUnseenResume",
        ] {
            out.remove(key);
        }
    }
    if out
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|error| {
            word_match(error, "session limit") && word_match(&error.to_ascii_lowercase(), "resets")
        })
        && out.get("errorCategory").is_none()
        && out.get("tccFolderKind").is_none()
    {
        out.remove("error");
        out.remove("errorAt");
    }
    Value::Object(out)
}

fn unsupported(value: &Value) -> Option<&'static str> {
    if let Some(recovery) = value.get("contextRecovery").filter(|v| nonempty(v)) {
        let Some(recovery) = recovery.as_object() else {
            return Some("UNSUPPORTED_CONTEXT_RECOVERY");
        };
        let attempts = recovery.get("attempts").and_then(Value::as_u64);
        let failures = recovery.get("failures").and_then(Value::as_u64);
        let phase = recovery.get("phase").and_then(Value::as_str);
        if recovery
            .keys()
            .any(|key| !["attempts", "failures", "phase", "errorUuid"].contains(&key.as_str()))
            || !matches!((attempts,failures),(Some(a),Some(f)) if a<=2&&f<=a)
            || !matches!(
                phase,
                Some("waiting" | "compacting" | "compacted" | "spent")
            )
            || (phase == Some("compacting") && failures == attempts)
            || recovery
                .get("errorUuid")
                .is_some_and(|value| !value.is_string())
        {
            return Some("UNSUPPORTED_CONTEXT_RECOVERY");
        }
    }
    if value
        .get("gitAnchorsFolderRealpath")
        .is_some_and(|v| nonempty(v) && !v.as_str().is_some_and(|s| Path::new(s).is_absolute()))
    {
        return Some("UNSUPPORTED_HISTORY_STATE");
    }
    // New remote/environment routing and execution receipts do not belong to
    // an ordinary-local migration. Do not silently discard meaningful state.
    if [
        "historyOnlyCliSessionId",
        "priorRemoteCliSessionIds",
        "startedFromEnvironmentId",
        "stoppedUntilPersonSends",
        "remoteControlLiveAtUpdateQuit",
        "peerMessaged",
        "simulatorToolsUsed",
        "terminalClaudeTabOrdinal",
    ]
    .iter()
    .any(|key| value.get(*key).is_some_and(nonempty))
    {
        return Some("UNSUPPORTED_HISTORY_STATE");
    }
    if [
        "sshConfig",
        "wslConfig",
        "cloudSessionId",
        "movedToCloud",
        "remoteControlSpawn",
        "stagedTranscriptPath",
        "transcriptCuts",
        "importedFrom",
    ]
    .iter()
    .any(|k| value.get(*k).is_some_and(nonempty))
    {
        return Some("UNSUPPORTED_HISTORY_STATE");
    }
    if value.get("backend").is_some_and(|v| v != "local") {
        return Some("NONLOCAL_SESSION");
    }
    None
}

fn unsupported_persisted_fields(
    value: &Value,
    unknown_fields: &BTreeSet<String>,
) -> Option<&'static str> {
    unknown_fields
        .iter()
        .any(|key| value.get(key).is_some_and(nonempty))
        .then_some("UNSUPPORTED_PERSISTED_FIELD")
}

pub(super) fn build_plan(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    accounts: &[Identity],
) -> Result<Plan> {
    build_plan_with_fields(roots, source, target, accounts, &BTreeSet::new())
}

pub(super) fn build_plan_with_fields(
    roots: &Roots,
    source: &Identity,
    target: &Identity,
    accounts: &[Identity],
    unknown_fields: &BTreeSet<String>,
) -> Result<Plan> {
    let (source_dir, target_dir) = validate_roots(roots, source, target)?;
    let baseline_path = state_path(roots);
    let baseline = read_blob(&baseline_path)?;
    let mut manifest: Manifest = baseline
        .as_ref()
        .map(decode)
        .transpose()?
        .unwrap_or(Manifest {
            version: 1,
            aliases: BTreeMap::new(),
        });
    check(
        manifest.version == 1
            && manifest.aliases.iter().all(|(alias, original)| {
                record_name(alias)
                    && record_name(original)
                    && alias != original
                    && !manifest.aliases.contains_key(original)
            }),
        "INVALID_CATALOG",
    )?;
    let old_manifest = manifest.clone();
    let mut identities = BTreeMap::new();
    for identity in accounts.iter().chain([source, target]) {
        identities.insert(identity_key(identity)?, identity);
    }
    let mut inputs = BTreeMap::new();
    let mut registries = BTreeMap::new();
    for key in identities.keys() {
        let dir = roots.records.join(key);
        inputs.insert(dir.clone(), records(&dir)?);
        let registry = dir.join("scheduled-tasks.json");
        registries.insert(registry.clone(), read_blob(&registry)?);
    }
    let src = inputs[&source_dir].clone();
    let dst = inputs[&target_dir].clone();
    let logical = |name: &str| {
        old_manifest
            .aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.into())
    };
    let mut groups: BTreeMap<String, BTreeMap<String, Vec<(&PathBuf, &String, &Record)>>> =
        BTreeMap::new();
    let mut wanted = BTreeSet::new();
    let mut warnings = Vec::new();
    let mut nonresumable = BTreeSet::new();
    for (dir, rows) in &inputs {
        for (name, row) in rows {
            if row.value.get("cliSessionId").is_none() {
                // Cleared and not-yet-started rows are valid namespace members,
                // but have no active transcript to collect. Keep their drift
                // witnesses and any destination image instead of requiring the
                // ordinary resume contract or importing queued work.
                if nonresumable.insert(name.clone()) {
                    warnings.push(Issue {
                        session_id: name.trim_end_matches(".json").into(),
                        reason: "NON_RESUMABLE_SESSION".into(),
                        title: row.value["title"].as_str().map(str::to_owned),
                    });
                }
                continue;
            }
            validate_candidate(&row.value)?;
            let pointer = row.value["cliSessionId"]
                .as_str()
                .unwrap()
                .to_ascii_lowercase();
            wanted.insert(pointer.clone());
            wanted.extend(rewind_refs(&row.value));
            wanted.extend(prior_ids(&row.value));
            groups
                .entry(logical(name))
                .or_default()
                .entry(pointer)
                .or_default()
                .push((dir, name, row));
        }
    }
    check(!groups.is_empty(), "SOURCE_SIDEBAR_EMPTY")?;
    let fast_witness = FastWitness {
        sources: source_witness(&inputs, &target_dir)?,
        pool: pool_witness(&roots.pool)?,
        target_id: file_id(&Directory::open(&target_dir, false)?.0)?,
    };
    let index: Transcripts = transcripts_metadata(&roots.pool, &wanted)?
        .into_iter()
        .map(|(id, files)| {
            (
                id,
                files
                    .into_iter()
                    .map(|m| Transcript {
                        path: m.path,
                        hash: String::new(),
                        size: m.size,
                        id: m.id,
                        modified: m.modified,
                        changed: m.changed,
                    })
                    .collect(),
            )
        })
        .collect();
    let mut result: BTreeMap<String, Value> = dst
        .iter()
        .map(|(n, r)| (n.clone(), r.value.clone()))
        .collect();
    let all_names: BTreeSet<String> = inputs
        .values()
        .flat_map(|rows| rows.keys().cloned())
        .collect();
    let mut issues = Vec::new();
    let mut placements = BTreeMap::new();
    let mut links = Vec::new();
    let mut preserved_branches = 0;
    for (original, branches) in &groups {
        let mut best: BTreeMap<String, (&PathBuf, &String, &Record)> = BTreeMap::new();
        for (pointer, candidates) in branches {
            let chosen = *candidates
                .iter()
                .max_by(|a, b| {
                    activity(&a.2.value)
                        .total_cmp(&activity(&b.2.value))
                        .then_with(|| (a.0 == &source_dir).cmp(&(b.0 == &source_dir)))
                        .then_with(|| a.0.cmp(b.0))
                        .then_with(|| a.1.cmp(b.1))
                })
                .unwrap();
            best.insert(pointer.clone(), chosen);
        }
        let primary = best
            .iter()
            .max_by(|a, b| {
                activity(&a.1 .2.value)
                    .total_cmp(&activity(&b.1 .2.value))
                    .then_with(|| (a.1 .0 == &source_dir).cmp(&(b.1 .0 == &source_dir)))
                    .then_with(|| a.0.cmp(b.0))
            })
            .unwrap()
            .0
            .clone();
        preserved_branches += branches.len().saturating_sub(1);
        for (pointer, (input_dir, source_name, row)) in best {
            // Unknown native state cannot be classified as obsolete merely by
            // activity order. Check every snapshot of this branch, including
            // destination state retained across a workspace change.
            let reason = branches[&pointer]
                .iter()
                .find_map(|(_, _, snapshot)| {
                    unsupported_persisted_fields(&snapshot.value, unknown_fields)
                })
                .or_else(|| unsupported(&row.value))
                .or_else(|| transcript_issue(&index, &pointer))
                .or_else(|| rewind_issue(&row.value, &index));
            if let Some(reason) = reason {
                issues.push(Issue {
                    session_id: source_name.trim_end_matches(".json").into(),
                    reason: reason.into(),
                    title: row.value["title"].as_str().map(str::to_owned),
                });
                continue;
            }
            // Never replace an existing destination active pointer. The latest
            // branch keeps the plain title; all other branches are visibly named.
            let existing = dst
                .iter()
                .filter(|(n, r)| {
                    logical(n) == *original
                        && r.value["cliSessionId"]
                            .as_str()
                            .is_some_and(|id| id.eq_ignore_ascii_case(&pointer))
                })
                .min_by_key(|(n, _)| (*n != original, *n))
                .map(|(n, _)| n.clone());
            let name = if let Some(name) = existing {
                name
            } else if pointer == primary && !result.contains_key(original) {
                original.clone()
            } else {
                // Prefer an alias already assigned to this branch in another
                // account. A continued alias can point at a new transcript;
                // its old ID is not reused for the old branch at that account.
                let reusable = branches[&pointer]
                    .iter()
                    .map(|(_, n, _)| *n)
                    .filter(|n| {
                        manifest.aliases.get(*n) == Some(original) && !result.contains_key(*n)
                    })
                    .min()
                    .cloned();
                if let Some(name) = reusable {
                    name
                } else {
                    let mut selected = None;
                    for salt in 0..1024 {
                        let candidate = alias_name(original, &pointer, salt);
                        if !result.contains_key(&candidate)
                            && !all_names.contains(&candidate)
                            && !manifest.aliases.contains_key(&candidate)
                        {
                            selected = Some(candidate);
                            break;
                        }
                    }
                    selected.ok_or("CATALOG_ID_COLLISION")?
                }
            };
            // Equal or newer destination snapshots keep their local state.
            // Validate that retained state before native restart normalization
            // can turn a malformed recovery descriptor into runnable work.
            if let Some(reason) = dst
                .get(&name)
                .filter(|target| activity(&row.value) <= activity(&target.value))
                .and_then(|target| {
                    unsupported_persisted_fields(&target.value, unknown_fields)
                        .or_else(|| unsupported(&target.value))
                })
            {
                issues.push(Issue {
                    session_id: name.trim_end_matches(".json").into(),
                    reason: reason.into(),
                    title: row.value["title"].as_str().map(str::to_owned),
                });
                continue;
            }
            if let Some(target) = dst.get(&name).filter(|target| {
                activity(&row.value) > activity(&target.value)
                    && row.value.get("cwd") != target.value.get("cwd")
                    && [
                        "armedWorkAtQuit",
                        "pendingFirstStart",
                        "interruptedByQuitAt",
                        "interruptedUnseenResume",
                    ]
                    .iter()
                    .any(|key| target.value.get(*key).is_some_and(nonempty))
            }) {
                issues.push(Issue {
                    session_id: name.trim_end_matches(".json").into(),
                    reason: "TARGET_UNFINISHED_SESSION".into(),
                    title: target.value["title"].as_str().map(str::to_owned),
                });
                continue;
            }
            let rekeyed = name != *source_name;
            if rekeyed
                && ["worktreePath", "worktreeLazy"]
                    .iter()
                    .any(|k| row.value.get(*k).is_some_and(nonempty))
            {
                issues.push(Issue {
                    session_id: source_name.trim_end_matches(".json").into(),
                    reason: "WORKTREE_BRANCH_REQUIRES_NATIVE_FORK".into(),
                    title: row.value["title"].as_str().map(str::to_owned),
                });
                continue;
            }
            if name != *original {
                manifest.aliases.insert(name.clone(), original.clone());
            }
            let title = if pointer == primary {
                base_title(&row.value)
            } else {
                format!(
                    "{} （保留分支 {}）",
                    base_title(&row.value),
                    &digest(pointer.as_bytes())[..8]
                )
            };
            let mut projected = project(&row.value, dst.get(&name).map(|r| &r.value), &name, title);
            let owner_dir = if dst
                .get(&name)
                .is_some_and(|r| activity(&row.value) <= activity(&r.value))
            {
                target_dir.clone()
            } else {
                input_dir.clone()
            };
            for (field, parent) in [
                (
                    "forkedFromSessionId",
                    projected["forkedFromSessionId"].as_str(),
                ),
                (
                    "spawnedFrom",
                    projected["spawnedFrom"]["sessionId"].as_str(),
                ),
            ] {
                if let Some(parent) = parent {
                    links.push((
                        name.clone(),
                        owner_dir.clone(),
                        field.to_string(),
                        parent.to_string(),
                    ));
                }
            }
            placements.insert((original.clone(), pointer), name.clone());
            json_bytes(&projected)?;
            result.insert(name, projected);
        }
    }
    for (child, owner_dir, field, parent) in links {
        let parent_name = format!("{parent}.json");
        if let Some(parent_row) = inputs[&owner_dir]
            .get(&parent_name)
            .filter(|row| row.value["cliSessionId"].as_str().is_some())
        {
            let pointer = parent_row.value["cliSessionId"]
                .as_str()
                .unwrap()
                .to_ascii_lowercase();
            if let Some(placed) = placements.get(&(logical(&parent_name), pointer)) {
                let id = json!(placed.trim_end_matches(".json"));
                if field == "spawnedFrom" {
                    result.get_mut(&child).unwrap()["spawnedFrom"]["sessionId"] = id;
                } else {
                    result.get_mut(&child).unwrap()[&field] = id;
                }
            }
        } else {
            warnings.push(Issue {
                session_id: child.trim_end_matches(".json").into(),
                reason: "PREEXISTING_MISSING_PARENT".into(),
                title: None,
            });
        }
    }
    let expected_pointers: BTreeSet<_> = inputs
        .values()
        .flat_map(|rows| rows.values())
        .filter_map(|r| r.value["cliSessionId"].as_str())
        .map(str::to_ascii_lowercase)
        .collect();
    let actual_pointers: BTreeSet<_> = result
        .values()
        .filter_map(|r| r["cliSessionId"].as_str())
        .map(str::to_ascii_lowercase)
        .collect();
    let missing = expected_pointers.difference(&actual_pointers).count();
    let created = result.keys().filter(|n| !dst.contains_key(*n)).count();
    let updated = result
        .iter()
        .filter(|(n, value)| dst.get(*n).is_some_and(|r| &r.value != *value))
        .count();
    // Any unsupported candidate is explicit and blocks a complete transfer,
    // even if an older destination row still points at that transcript.
    let unresolved = issues.len().max(missing);
    let next_baseline = serde_json::to_value(&manifest).map_err(|_| "INVALID_CATALOG")?;
    let fingerprint = digest(&json_bytes(&json!({"kind":"continuity-v1",
        "source":identity_key(source)?,"target":identity_key(target)?,"pool":roots.pool,"state":roots.state,
        "inputs":inputs.iter().map(|(dir, rows)| (dir, record_stamps(rows))).collect::<BTreeMap<_,_>>(),
        "registry":registries.iter().map(|(p,b)| (p,b.as_ref().map(|b| &b.stamp))).collect::<BTreeMap<_,_>>(),
        "stateWitness":baseline.as_ref().map(|b| &b.stamp),"transcripts":expected_transcript_metadata(&index).iter()
            .map(|(id, files)| (id, files.iter().map(|m| (&m.path,m.size,&m.id,m.modified,m.changed)).collect::<Vec<_>>())).collect::<BTreeMap<_,_>>()
    }))?);
    let preview = Preview {
        fingerprint,
        baseline_changed: old_manifest != manifest || baseline.is_none(),
        created,
        updated,
        unchanged: dst.len() - updated,
        missing: unresolved,
        stale: 0,
        replaced_branches: 0,
        issues,
        warnings,
        quota_pauses_cleared: 0,
        preserved_branches,
    };
    result.retain(|name, value| dst.get(name).is_none_or(|r| &r.value != value));
    Ok(Plan {
        preview,
        source_dir,
        target_dir,
        source: src,
        target: dst,
        registries,
        transcripts: index,
        baseline_path,
        baseline,
        next_baseline,
        proposed: result,
        catalog_mode: true,
        inputs,
        fast_witness: Some(fast_witness),
    })
}
