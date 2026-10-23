// Per-instance, bounded background work. The CDP/render loop never awaits disk I/O.
struct InjectionUsagePoller {
    profile_dir: PathBuf,
    active: Vec<String>,
    snapshots: HashMap<String, usage::InjectionUsageSnapshot>,
    recent_accounts: HashMap<String, recent_account::RecentUsedAccount>,
    output_speeds: HashMap<String, output_speed::ThreadOutputSpeed>,
    selections: HashMap<String, output_speed::OutputSpeedSelection>,
    speed_bindings: HashMap<String, String>,
    speed_verified_at: HashMap<String, Instant>,
    pending: HashSet<String>,
    retry_after: HashMap<String, Instant>,
    next_index: usize,
    tasks: JoinSet<(
        String,
        Result<usage::InjectionUsageSnapshot, String>,
        Result<Option<recent_account::RecentUsedAccount>, String>,
        output_speed::SpeedRead,
    )>,
    last_warning: Instant,
}

impl InjectionUsagePoller {
    fn new(profile_dir: &Path) -> Self {
        Self {
            profile_dir: profile_dir.to_path_buf(),
            active: Vec::new(),
            snapshots: HashMap::new(),
            recent_accounts: HashMap::new(),
            output_speeds: HashMap::new(),
            selections: HashMap::new(),
            speed_bindings: HashMap::new(),
            speed_verified_at: HashMap::new(),
            pending: HashSet::new(),
            retry_after: HashMap::new(),
            next_index: 0,
            tasks: JoinSet::new(),
            last_warning: Instant::now() - Duration::from_secs(30),
        }
    }

    fn collect(&mut self) -> Value {
        while let Some(completed) = self.tasks.try_join_next() {
            if let Ok((id, result, recent_result, speed_result)) = completed {
                self.pending.remove(&id);
                let usage_failed = result.is_err();
                let mut failure = None;
                match result {
                    Ok(snapshot) => {
                        self.ensure_cache_slot(&id);
                        self.snapshots.insert(id.clone(), snapshot);
                    }
                    Err(error) => failure = Some(error),
                }
                match recent_result {
                    Ok(Some(account)) => {
                        self.ensure_cache_slot(&id);
                        self.recent_accounts.insert(id.clone(), account);
                    }
                    Ok(None) => {
                        self.recent_accounts.remove(&id);
                    }
                    Err(error) => {
                        if failure.is_none() {
                            failure = Some(error);
                        }
                    }
                }
                let speed_failed = speed_result.result.is_err();
                if let Some(error) = self.accept_speed_result(&id, speed_result) {
                    if failure.is_none() {
                        failure = Some(error)
                    }
                }
                if usage_failed || speed_failed {
                    self.retry_after
                        .insert(id.clone(), Instant::now() + Duration::from_secs(2));
                } else {
                    self.retry_after.remove(&id);
                }
                if let Some(error) = failure {
                    if self.last_warning.elapsed() >= Duration::from_secs(30) {
                        logger::log_warn(&format!("[Codex Usage] 本地用量读取失败: {}", error));
                        self.last_warning = Instant::now();
                    }
                }
            }
        }
        let expired: Vec<_> = self
            .speed_verified_at
            .iter()
            .filter(|(_, verified)| verified.elapsed() > Duration::from_secs(15))
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.clear_speed(&id)
        }
        Value::Object(
            self.active
                .iter()
                .filter_map(|id| {
                    let mut value = match self.snapshots.get(id) {
                        Some(snapshot) => serde_json::to_value(snapshot).ok()?,
                        None if self.recent_accounts.contains_key(id)
                            || self.output_speeds.contains_key(id) =>
                        {
                            json!({
                                "threadId": id, "events": [], "currentTurnId": null,
                            })
                        }
                        None => return None,
                    };
                    value.as_object_mut()?.insert(
                        "recentAccount".into(),
                        serde_json::to_value(self.recent_accounts.get(id)).ok()?,
                    );
                    value.as_object_mut()?.insert(
                        "outputSpeed".into(),
                        serde_json::to_value(self.output_speeds.get(id)).ok()?,
                    );
                    Some((id.clone(), value))
                })
                .collect(),
        )
    }

    fn clear_speed(&mut self, id: &str) {
        self.output_speeds.remove(id);
        self.speed_bindings.remove(id);
        self.speed_verified_at.remove(id);
    }

    fn accept_speed_result(&mut self, id: &str, read: output_speed::SpeedRead) -> Option<String> {
        let expected = self.selections.get(id).cloned().unwrap_or_default();
        if read.selection != expected {
            // A completed worker for an earlier selection cannot repopulate it.
            return None;
        }
        match read.result {
            Ok(Some(speed))
                if read.binding.is_some()
                    && speed.binding == read.binding
                    && speed.selection == expected
                    && speed.thread_id == id =>
            {
                self.ensure_cache_slot(id);
                self.speed_bindings.insert(id.into(), read.binding.unwrap());
                self.speed_verified_at.insert(id.into(), Instant::now());
                self.output_speeds.insert(id.into(), speed);
                None
            }
            Ok(_) => {
                self.clear_speed(id);
                None
            }
            Err(error) => {
                let trusted = read.transient_failure
                    && read.binding.is_some()
                    && self.speed_bindings.get(id) == read.binding.as_ref()
                    && self
                        .speed_verified_at
                        .get(id)
                        .is_some_and(|verified| verified.elapsed() <= Duration::from_secs(15));
                if !trusted {
                    self.clear_speed(id)
                }
                Some(error)
            }
        }
    }

    fn ensure_cache_slot(&mut self, id: &str) {
        let keys: HashSet<_> = self
            .snapshots
            .keys()
            .chain(self.recent_accounts.keys())
            .chain(self.output_speeds.keys())
            .cloned()
            .collect();
        if keys.len() >= 32 && !keys.contains(id) {
            if let Some(old) = keys.iter().find(|key| !self.active.contains(key)) {
                self.snapshots.remove(old);
                self.recent_accounts.remove(old);
                self.output_speeds.remove(old);
                self.speed_bindings.remove(old);
                self.speed_verified_at.remove(old);
                self.selections.remove(old);
            }
        }
    }

    fn request(&mut self, threads: Vec<String>) {
        self.request_with_selection(
            threads
                .into_iter()
                .map(|id| (id, output_speed::OutputSpeedSelection::default()))
                .collect(),
        );
    }

    fn request_with_selection(
        &mut self,
        threads: Vec<(String, output_speed::OutputSpeedSelection)>,
    ) {
        self.active.clear();
        for (id, selection) in threads {
            // CDP page values are untrusted: never accept a caller-supplied path.
            if uuid::Uuid::parse_str(&id).is_err() || self.active.contains(&id) {
                continue;
            }
            let selection = selection.normalized();
            if self.selections.get(&id) != Some(&selection) {
                // Session aggregates remain valid across model and tier changes.
                // Binding verification and cache expiry still invalidate them.
                self.retry_after.remove(&id);
                self.selections.insert(id.clone(), selection);
            }
            self.active.push(id);
            if self.active.len() == 8 {
                break;
            }
        }
        self.retry_after
            .retain(|id, until| self.active.contains(id) && *until > Instant::now());
        self.selections.retain(|id, _| {
            self.active.contains(id)
                || self.pending.contains(id)
                || self.snapshots.contains_key(id)
                || self.recent_accounts.contains_key(id)
                || self.output_speeds.contains_key(id)
        });
        let count = self.active.len();
        if count == 0 {
            return;
        }
        for offset in 0..count {
            if self.tasks.len() >= 2 {
                break;
            }
            let index = (self.next_index + offset) % count;
            let id = &self.active[index];
            if self.pending.contains(id) || self.retry_after.contains_key(id) {
                continue;
            }
            self.pending.insert(id.clone());
            let id = id.clone();
            let profile_dir = self.profile_dir.clone();
            let selection = self.selections.get(&id).cloned().unwrap_or_default();
            self.tasks.spawn(async move {
                let thread_id = id.clone();
                let requested_selection = selection.clone();
                let result = tauri::async_runtime::spawn_blocking(move || {
                    (
                        // Metrics and account labels now come from completed
                        // request summaries. Do not scan rollouts or account
                        // metadata before this latency-sensitive query.
                        Ok(usage::InjectionUsageSnapshot {
                            thread_id: thread_id.clone(),
                            ..Default::default()
                        }),
                        Ok(None),
                        output_speed::read_output_speed_with_selection(
                            &profile_dir,
                            &thread_id,
                            &requested_selection,
                        ),
                    )
                })
                .await;
                match result {
                    Ok((usage, account, speed)) => (id, usage, account, speed),
                    Err(error) => {
                        let error = format!("usage worker failed: {}", error);
                        (
                            id,
                            Err(error.clone()),
                            Err(error.clone()),
                            output_speed::SpeedRead::failed(selection, error),
                        )
                    }
                }
            });
        }
        self.next_index = (self.next_index + 2) % count;
    }
}

#[cfg(test)]
mod usage_poll_tests {
    use super::*;

    fn empty_speed() -> output_speed::SpeedRead {
        output_speed::SpeedRead {
            binding: None,
            selection: output_speed::OutputSpeedSelection::default(),
            result: Ok(None),
            transient_failure: false,
        }
    }

    #[tokio::test]
    async fn usage_poll_does_not_read_rollouts_or_account_metadata_before_metrics() {
        let production = include_str!("codex_app_injection_usage_poll.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(!production.contains("usage::read_usage_snapshot("));
        assert!(!production.contains("recent_account::read_recent_account("));
        let directory = tempfile::tempdir().unwrap();
        let id = "019e92aa-0000-7000-8000-000000000001";
        let sessions = directory.path().join("sessions");
        std::fs::create_dir(&sessions).unwrap();
        let rollout =
            std::fs::File::create(sessions.join(format!("rollout-2026-01-01-{id}.jsonl"))).unwrap();
        rollout.set_len(64 * 1024 * 1024).unwrap();
        std::fs::write(
            directory.path().join("state_5.sqlite"),
            "invalid unused rollout index",
        )
        .unwrap();
        let mut poll = InjectionUsagePoller::new(directory.path());
        poll.request(vec![id.into()]);
        let (thread, usage, account, metrics) = poll.tasks.join_next().await.unwrap().unwrap();
        assert_eq!(thread, id);
        let usage = usage.unwrap();
        assert_eq!(usage.thread_id, id);
        assert!(usage.events.is_empty());
        assert!(usage.current_turn_id.is_none());
        assert!(account.unwrap().is_none());
        assert!(metrics.result.unwrap().is_none());
    }

    fn recent_account_fixture() -> recent_account::RecentUsedAccount {
        recent_account::RecentUsedAccount {
            account_id: "actual".into(),
            display_name: "actual@example.test".into(),
            email: Some("actual@example.test".into()),
            plan: Some("Team".into()),
            last_used_at_ms: 1000,
        }
    }

    fn measured_speed(
        id: &str,
        selection: output_speed::OutputSpeedSelection,
    ) -> output_speed::ThreadOutputSpeed {
        output_speed::ThreadOutputSpeed {
            thread_id: id.into(),
            binding: Some("binding-a".into()),
            tokens_per_second: 30.0,
            samples: 2,
            observed_at_ms: 1000,
            latest_tokens_per_second: Some(66.0),
            latest_observed_at_ms: Some(1000),
            latest_input_tokens: Some(1000),
            latest_cached_tokens: Some(900),
            latest_cache_hit_rate: Some(0.9),
            avg_cache_hit_rate: Some(0.8),
            total_input_tokens: Some(1000),
            total_cached_tokens: Some(800),
            request_id: Some("measured-request".into()),
            model: selection.model.clone(),
            requested_service_tier: selection.service_tier.clone(),
            response_service_tier: Some("default".into()),
            selection,
        }
    }

    fn speed_read(
        id: &str,
        binding: &str,
        selection: &output_speed::OutputSpeedSelection,
    ) -> output_speed::SpeedRead {
        output_speed::SpeedRead {
            binding: Some(binding.into()),
            selection: selection.clone(),
            transient_failure: false,
            result: Ok(Some(measured_speed(id, selection.clone()))),
        }
    }

    #[test]
    fn usage_poll_temporary_sqlite_failure_keeps_same_measurement_with_bounded_freshness() {
        let directory = tempfile::tempdir().unwrap();
        let mut poll = InjectionUsagePoller::new(directory.path());
        let id = "019e92aa-0000-7000-8000-000000000001";
        let selection = output_speed::OutputSpeedSelection {
            model: Some("gpt-6.1-sol".into()),
            service_tier: Some("priority".into()),
        };
        poll.active = vec![id.into()];
        poll.selections.insert(id.into(), selection.clone());
        poll.accept_speed_result(id, speed_read(id, "binding-a", &selection));
        let verified_at = poll.speed_verified_at[id];
        let temporary = || output_speed::SpeedRead {
            binding: Some("binding-a".into()),
            selection: selection.clone(),
            result: Err("database is locked".into()),
            transient_failure: true,
        };
        assert!(poll.accept_speed_result(id, temporary()).is_some());
        assert_eq!(poll.speed_verified_at[id], verified_at); // failure never extends its TTL
        let value = poll.collect();
        assert_eq!(value[id]["outputSpeed"]["latestTokensPerSecond"], 66.0);
        assert_eq!(value[id]["outputSpeed"]["latestCacheHitRate"], 0.9);
        poll.speed_verified_at
            .insert(id.into(), Instant::now() - Duration::from_secs(16));
        assert!(poll.accept_speed_result(id, temporary()).is_some());
        assert!(!poll.output_speeds.contains_key(id));
        assert!(!poll.speed_bindings.contains_key(id));
    }

    #[test]
    fn usage_poll_clears_measurement_when_binding_or_selection_is_no_longer_proven() {
        let directory = tempfile::tempdir().unwrap();
        let mut poll = InjectionUsagePoller::new(directory.path());
        let id = "019e92aa-0000-7000-8000-000000000001";
        let selection = output_speed::OutputSpeedSelection::default();
        for (binding, transient) in [
            (Some("other-key"), true),
            (None, true),
            (Some("binding-a"), false),
        ] {
            poll.accept_speed_result(id, speed_read(id, "binding-a", &selection));
            poll.accept_speed_result(
                id,
                output_speed::SpeedRead {
                    binding: binding.map(str::to_string),
                    selection: selection.clone(),
                    result: Err("untrusted metadata".into()),
                    transient_failure: transient,
                },
            );
            assert!(!poll.output_speeds.contains_key(id));
        }
        poll.accept_speed_result(id, speed_read(id, "binding-a", &selection));
        poll.selections.insert(
            id.into(),
            output_speed::OutputSpeedSelection {
                model: Some("other-model".into()),
                service_tier: Some("priority".into()),
            },
        );
        poll.clear_speed(id);
        poll.accept_speed_result(id, speed_read(id, "binding-a", &selection));
        assert!(!poll.output_speeds.contains_key(id)); // late old worker cannot restore it
    }

    #[tokio::test]
    async fn usage_poll_selection_changes_keep_session_means_and_reject_late_workers() {
        let directory = tempfile::tempdir().unwrap();
        let mut poll = InjectionUsagePoller::new(directory.path());
        let id = "019e92aa-0000-7000-8000-000000000001";
        let normal = output_speed::OutputSpeedSelection {
            model: Some("gpt-6.1-sol".into()),
            service_tier: Some("default".into()),
        };
        let fast = output_speed::OutputSpeedSelection {
            service_tier: Some("priority".into()),
            ..normal.clone()
        };
        poll.selections.insert(id.into(), normal.clone());
        poll.accept_speed_result(id, speed_read(id, "binding-a", &normal));
        poll.request_with_selection(vec![(id.into(), fast.clone())]);
        assert_eq!(poll.output_speeds[id].tokens_per_second, 30.0);
        poll.accept_speed_result(id, speed_read(id, "binding-a", &normal));
        assert_eq!(poll.output_speeds[id].selection, normal);
        poll.accept_speed_result(id, speed_read(id, "binding-a", &fast));
        assert_eq!(poll.output_speeds[id].avg_cache_hit_rate, Some(0.8));
        let other = output_speed::OutputSpeedSelection {
            model: Some("grok-4".into()),
            ..fast
        };
        poll.request_with_selection(vec![(id.into(), other)]);
        assert_eq!(poll.output_speeds[id].total_cached_tokens, Some(800));
        while poll.tasks.join_next().await.is_some() {}
    }

    #[tokio::test]
    async fn usage_poll_recent_errors_keep_account_without_delaying_usage() {
        let dir = tempfile::tempdir().unwrap();
        let id = "019e92aa-0000-7000-8000-000000000001".to_string();
        let mut poll = InjectionUsagePoller::new(dir.path());
        poll.active = vec![id.clone()];
        poll.recent_accounts
            .insert(id.clone(), recent_account_fixture());
        let task_id = id.clone();
        poll.tasks.spawn(async move {
            (
                task_id,
                Ok(usage::InjectionUsageSnapshot::default()),
                Err("database busy".into()),
                empty_speed(),
            )
        });
        tokio::time::sleep(Duration::from_millis(1)).await;
        let state = poll.collect();
        assert_eq!(state[&id]["recentAccount"]["plan"], "Team");
        assert!(!poll.retry_after.contains_key(&id));
        let task_id = id.clone();
        poll.tasks.spawn(async move {
            (
                task_id,
                Ok(usage::InjectionUsageSnapshot::default()),
                Ok(None),
                empty_speed(),
            )
        });
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(poll.collect()[&id]["recentAccount"].is_null());
        assert!(!poll.recent_accounts.contains_key(&id));
    }

    #[test]
    fn usage_poll_account_only_cache_has_a_bounded_union_with_rollout_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut poll = InjectionUsagePoller::new(dir.path());
        for n in 0..50 {
            let id = format!("019e92aa-0000-7000-8000-{n:012}");
            poll.active = vec![id.clone()];
            poll.ensure_cache_slot(&id);
            poll.recent_accounts.insert(id, recent_account_fixture());
        }
        assert_eq!(poll.recent_accounts.len(), 32);
        assert!(poll.snapshots.is_empty());
        assert_eq!(poll.collect().as_object().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn usage_poll_deduplicates_and_keeps_empty_snapshot_until_worker_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let id = "019e92aa-0000-7000-8000-000000000001".to_string();
        let mut poll = InjectionUsagePoller::new(dir.path());
        poll.request(vec![id.clone(), id.clone(), "../auth.json".into()]);
        assert_eq!(poll.active, vec![id.clone()]);
        assert_eq!(poll.pending.len(), 1);
        assert_eq!(poll.tasks.len(), 1);
        poll.request(vec![id.clone()]);
        assert_eq!(poll.tasks.len(), 1);
        let empty = poll.collect();
        assert!(empty.is_object());
        // The result is deliberately not awaited by collect or request.
        let (finished, snapshot, recent_account, speed) =
            poll.tasks.join_next().await.unwrap().unwrap();
        assert_eq!(finished, id);
        assert!(snapshot.unwrap().events.is_empty());
        assert!(recent_account.unwrap().is_none());
        assert!(speed.result.unwrap().is_none());
    }

    #[tokio::test]
    async fn usage_poll_limits_parallel_reads_and_does_not_expose_other_chats() {
        let dir = tempfile::tempdir().unwrap();
        let mut poll = InjectionUsagePoller::new(dir.path());
        poll.request(
            (1..=20)
                .map(|value| format!("019e92aa-0000-7000-8000-{value:012}"))
                .collect(),
        );
        assert_eq!(poll.tasks.len(), 2);
        assert_eq!(poll.active.len(), 8);
        poll.snapshots.insert(
            "another-chat".into(),
            usage::InjectionUsageSnapshot::default(),
        );
        assert!(poll.collect().get("another-chat").is_none());
        poll.request(Vec::new());
        assert_eq!(poll.collect(), json!({}));
    }
}
