// Read-only dialog queries are independent of the CDP/render loop. In-flight
// blocking reads keep their worker slots until they actually finish, including
// after a tab closes; a UI timeout must not launch unbounded replacement reads.
const INJECTION_DIALOG_ACTIVE_LIMIT: usize = 8;
const INJECTION_DIALOG_CACHE_LIMIT: usize = 16;
const INJECTION_DIALOG_WORKER_LIMIT: usize = 2;
const INJECTION_DIALOG_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

type InjectionDialogReader =
    std::sync::Arc<dyn Fn(&Path, &dialog_data::DialogQuery) -> Result<Value, String> + Send + Sync>;

struct InjectionDialogCacheEntry {
    query: dialog_data::DialogQuery,
    generation: u64,
    last_used: u64,
    deadline: Instant,
    result: Option<Value>,
}

struct InjectionDialogJob {
    query: dialog_data::DialogQuery,
    generation: u64,
}

struct InjectionDialogPoller {
    profile_dir: PathBuf,
    active: Vec<String>,
    cache: HashMap<String, InjectionDialogCacheEntry>,
    tasks: JoinSet<Result<Value, String>>,
    jobs: HashMap<tokio::task::Id, InjectionDialogJob>,
    sequence: u64,
    reader: InjectionDialogReader,
    last_warning: Instant,
}

impl InjectionDialogPoller {
    fn new(profile_dir: &Path) -> Self {
        Self::with_reader(
            profile_dir,
            std::sync::Arc::new(dialog_data::read_dialog_data),
        )
    }

    fn with_reader(profile_dir: &Path, reader: InjectionDialogReader) -> Self {
        Self {
            profile_dir: profile_dir.to_path_buf(),
            active: Vec::new(),
            cache: HashMap::new(),
            tasks: JoinSet::new(),
            jobs: HashMap::new(),
            sequence: 0,
            reader,
            last_warning: Instant::now() - Duration::from_secs(30),
        }
    }

    fn collect(&mut self) -> Value {
        self.collect_completed();
        self.start_pending();
        Value::Object(
            self.active
                .iter()
                .filter_map(|token| {
                    let entry = self.cache.get(token)?;
                    Some((
                        token.clone(),
                        entry
                            .result
                            .clone()
                            .unwrap_or_else(|| Self::status_response(&entry.query, "loading")),
                    ))
                })
                .collect(),
        )
    }

    fn status_response(query: &dialog_data::DialogQuery, status: &str) -> Value {
        let mut value = json!({
            "requestToken": query.request_token,
            "threadId": query.thread_id,
            "scope": query.scope,
            "period": query.period,
            "cursor": query.cursor,
            "view": query.view,
            "binding": query.binding,
        });
        value[status] = json!(true);
        value
    }

    fn request(&mut self, queries: Vec<dialog_data::DialogQuery>) {
        self.collect_completed();
        let mut accepted: Vec<dialog_data::DialogQuery> = Vec::new();
        let mut conflicts = HashSet::new();

        for query in queries {
            // Typed values are not necessarily validated: every CDP entry is
            // parsed again before it may consume an active slot or read storage.
            let Some(query) = serde_json::to_value(query)
                .ok()
                .and_then(|value| dialog_data::parse_query(&value))
            else {
                continue;
            };
            let token = &query.request_token;
            if conflicts.contains(token) {
                continue;
            }

            let changed_token = self
                .cache
                .get(token)
                .is_some_and(|entry| entry.query != query)
                || self
                    .jobs
                    .values()
                    .any(|job| job.query.request_token == *token && job.query != query)
                || accepted
                    .iter()
                    .any(|old| old.request_token == *token && *old != query);
            if changed_token {
                // Never expose an old result under a token reused for another
                // thread/scope/cursor, even if both variants occur in one batch.
                accepted.retain(|old| old.request_token != *token);
                conflicts.insert(token.clone());
                continue;
            }
            if accepted.iter().any(|old| old.request_token == *token)
                || accepted.len() >= INJECTION_DIALOG_ACTIVE_LIMIT
            {
                continue;
            }
            accepted.push(query);
        }

        self.active = accepted
            .iter()
            .map(|query| query.request_token.clone())
            .collect();
        // Losing active status invalidates an unfinished read's generation.
        // Completed pages remain in the small LRU cache, but are never sent
        // unless the exact original query is active again.
        self.cache
            .retain(|token, entry| entry.result.is_some() || self.active.contains(token));

        for query in accepted {
            self.sequence = self.sequence.wrapping_add(1);
            if let Some(entry) = self.cache.get_mut(&query.request_token) {
                entry.last_used = self.sequence;
                continue;
            }
            self.ensure_cache_slot();
            self.cache.insert(
                query.request_token.clone(),
                InjectionDialogCacheEntry {
                    query,
                    generation: self.sequence,
                    last_used: self.sequence,
                    deadline: Instant::now() + INJECTION_DIALOG_QUERY_TIMEOUT,
                    result: None,
                },
            );
        }
        self.start_pending();
    }

    fn ensure_cache_slot(&mut self) {
        if self.cache.len() < INJECTION_DIALOG_CACHE_LIMIT {
            return;
        }
        // At most eight entries are active, so a full 16-entry cache always
        // has an inactive eviction candidate.
        if let Some(old) = self
            .cache
            .iter()
            .filter(|(token, _)| !self.active.contains(token))
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(token, _)| token.clone())
        {
            self.cache.remove(&old);
        }
    }

    fn start_pending(&mut self) {
        self.expire_pending();
        for token in &self.active {
            if self.tasks.len() >= INJECTION_DIALOG_WORKER_LIMIT {
                break;
            }
            let Some(entry) = self.cache.get(token) else {
                continue;
            };
            if entry.result.is_some()
                || self
                    .jobs
                    .values()
                    .any(|job| job.query.request_token == *token)
            {
                continue;
            }
            let query = entry.query.clone();
            let job = InjectionDialogJob {
                query: query.clone(),
                generation: entry.generation,
            };
            let profile_dir = self.profile_dir.clone();
            let reader = self.reader.clone();
            let handle = self
                .tasks
                .spawn_blocking(move || reader(&profile_dir, &query));
            self.jobs.insert(handle.id(), job);
        }
    }

    fn expire_pending(&mut self) {
        let now = Instant::now();
        let mut expired = false;
        for entry in self.cache.values_mut() {
            if entry.result.is_none() && now >= entry.deadline {
                // This deadline includes time spent waiting for a worker.
                // Settling the UI result does not cancel a blocking read or
                // release its physical worker slot. Late completions lose.
                entry.result = Some(Self::status_response(&entry.query, "error"));
                expired = true;
            }
        }
        if expired && self.last_warning.elapsed() >= Duration::from_secs(30) {
            logger::log_warn(
                "[Codex Usage Dialog] 本地统计读取超时，已返回失败并保留未结束的后台任务槽位",
            );
            self.last_warning = now;
        }
    }

    fn collect_completed(&mut self) {
        self.expire_pending();
        while let Some(completed) = self.tasks.try_join_next_with_id() {
            let (task_id, result) = match completed {
                Ok((task_id, result)) => (task_id, result),
                Err(error) => (error.id(), Err(format!("dialog worker failed: {error}"))),
            };
            let Some(job) = self.jobs.remove(&task_id) else {
                continue;
            };
            let token = &job.query.request_token;
            if !self.active.contains(token) {
                continue;
            }
            let Some(entry) = self.cache.get_mut(token) else {
                continue;
            };
            if entry.generation != job.generation
                || entry.query != job.query
                || entry.result.is_some()
            {
                continue;
            }
            entry.result = Some(match result {
                Ok(value) => value,
                Err(error) => {
                    if self.last_warning.elapsed() >= Duration::from_secs(30) {
                        logger::log_warn(&format!(
                            "[Codex Usage Dialog] 本地统计读取失败: {error}"
                        ));
                        self.last_warning = Instant::now();
                    }
                    // Keep a failure distinct from an empty successful query.
                    // Retrying requires a new token, not another CDP poll.
                    Self::status_response(&entry.query, "error")
                }
            });
        }
    }
}

#[cfg(test)]
mod dialog_poll_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    };

    fn query(index: usize) -> dialog_data::DialogQuery {
        dialog_data::parse_query(&json!({
            "requestToken": format!("dialog-{index}"),
            "threadId": "019e92aa-0000-7000-8000-000000000001",
            "scope": "chat",
            "period": "today",
            "cursor": null,
            "view": "overview",
        }))
        .expect("valid dialog query")
    }

    fn response(query: &dialog_data::DialogQuery) -> Value {
        json!({
            "requestToken": query.request_token,
            "threadId": query.thread_id,
            "scope": query.scope,
            "period": query.period,
            "cursor": query.cursor,
            "view": query.view,
            "requests": { "rows": [], "nextCursor": null },
        })
    }

    async fn ready(poll: &mut InjectionDialogPoller, tokens: &[String]) -> Value {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let state = poll.collect();
                if tokens
                    .iter()
                    .all(|token| state.get(token).is_some() && state[token]["loading"] != true)
                {
                    return state;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("dialog worker should complete")
    }

    async fn wait_calls(calls: &AtomicUsize, expected: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while calls.load(Ordering::SeqCst) < expected {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("dialog worker should start");
    }

    struct Gate {
        released: Mutex<bool>,
        condvar: Condvar,
        calls: AtomicUsize,
        running: AtomicUsize,
        peak: AtomicUsize,
    }

    impl Gate {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                released: Mutex::new(false),
                condvar: Condvar::new(),
                calls: AtomicUsize::new(0),
                running: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
            })
        }

        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.condvar.notify_all();
        }

        fn reader(self: &Arc<Self>) -> InjectionDialogReader {
            let gate = self.clone();
            Arc::new(move |_, query| {
                let running = gate.running.fetch_add(1, Ordering::SeqCst) + 1;
                gate.peak.fetch_max(running, Ordering::SeqCst);
                let call = gate.calls.fetch_add(1, Ordering::SeqCst) + 1;
                let released = gate.released.lock().unwrap();
                let (released, _) = gate
                    .condvar
                    .wait_timeout_while(released, Duration::from_secs(3), |released| !*released)
                    .unwrap();
                gate.running.fetch_sub(1, Ordering::SeqCst);
                if !*released {
                    return Err("test gate timed out".into());
                }
                let mut value = response(query);
                value["call"] = json!(call);
                Ok(value)
            })
        }
    }

    #[tokio::test]
    async fn dialog_poll_bounds_workers_deduplicates_and_pumps_queued_reads() {
        let dir = tempfile::tempdir().unwrap();
        let gate = Gate::new();
        let mut poll = InjectionDialogPoller::with_reader(dir.path(), gate.reader());
        let mut invalid = query(0);
        invalid.request_token.clear();
        let mut queries = vec![invalid, query(1), query(1)];
        queries.extend((2..=10).map(query));
        poll.request(queries.clone());
        wait_calls(&gate.calls, 2).await;
        for _ in 0..5 {
            poll.request(queries.clone());
            let state = poll.collect();
            assert_eq!(state.as_object().unwrap().len(), 8);
            assert_eq!(state["dialog-1"]["loading"], true);
            assert_eq!(state["dialog-1"]["threadId"], query(1).thread_id.unwrap());
            assert_eq!(state["dialog-1"]["scope"], "chat");
            assert_eq!(state["dialog-1"]["period"], "today");
            assert_eq!(state["dialog-1"]["view"], "overview");
            assert!(state["dialog-1"]["cursor"].is_null());
            assert_eq!(poll.tasks.len(), 2);
        }
        assert_eq!(gate.calls.load(Ordering::SeqCst), 2);
        assert_eq!(poll.active.len(), 8);
        let tokens = poll.active.clone();
        gate.release();
        let state = ready(&mut poll, &tokens).await;
        assert_eq!(state.as_object().unwrap().len(), 8);
        assert_eq!(gate.calls.load(Ordering::SeqCst), 8);
        assert_eq!(gate.peak.load(Ordering::SeqCst), 2);
        for _ in 0..5 {
            poll.request(queries.clone());
            poll.collect();
        }
        assert_eq!(gate.calls.load(Ordering::SeqCst), 8);
        assert!(poll.tasks.is_empty());
    }

    #[tokio::test]
    async fn dialog_poll_hides_inactive_results_and_rejects_stale_generations() {
        let dir = tempfile::tempdir().unwrap();
        let gate = Gate::new();
        let mut poll = InjectionDialogPoller::with_reader(dir.path(), gate.reader());
        let first = query(1);
        poll.request(vec![first.clone()]);
        wait_calls(&gate.calls, 1).await;
        let old_generation = poll.cache[&first.request_token].generation;
        poll.request(vec![]);
        assert!(poll.collect().as_object().unwrap().is_empty());
        assert!(poll.cache.is_empty());
        // Reopening with the same token does not reuse the invalidated read.
        poll.request(vec![first.clone()]);
        assert_ne!(poll.cache[&first.request_token].generation, old_generation);
        assert_eq!(poll.tasks.len(), 1);
        gate.release();
        let state = ready(&mut poll, std::slice::from_ref(&first.request_token)).await;
        assert_eq!(state[&first.request_token]["call"], 2);
        assert_eq!(gate.calls.load(Ordering::SeqCst), 2);
        poll.request(vec![]);
        assert!(poll.collect().as_object().unwrap().is_empty());
        assert_eq!(poll.cache.len(), 1);
    }

    #[tokio::test]
    async fn dialog_poll_old_thread_completion_never_enters_new_active_payload() {
        let dir = tempfile::tempdir().unwrap();
        let gate = Gate::new();
        let mut poll = InjectionDialogPoller::with_reader(dir.path(), gate.reader());
        let first = query(1);
        let mut second = query(2);
        second.thread_id = Some("019e92aa-0000-7000-8000-000000000002".into());
        poll.request(vec![first.clone()]);
        wait_calls(&gate.calls, 1).await;
        poll.request(vec![second.clone()]);
        gate.release();
        let state = ready(&mut poll, std::slice::from_ref(&second.request_token)).await;
        assert!(state.get(&first.request_token).is_none());
        assert_eq!(state.as_object().unwrap().len(), 1);
        assert_eq!(
            state[&second.request_token]["threadId"],
            second.thread_id.as_deref().unwrap()
        );
        assert!(!poll.cache.contains_key(&first.request_token));
    }

    #[tokio::test]
    async fn dialog_poll_rejects_token_reuse_for_another_query_and_batch_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = calls.clone();
        let mut poll = InjectionDialogPoller::with_reader(
            dir.path(),
            Arc::new(move |_, query| {
                worker_calls.fetch_add(1, Ordering::SeqCst);
                Ok(response(query))
            }),
        );
        let original = query(1);
        poll.request(vec![original.clone()]);
        ready(&mut poll, std::slice::from_ref(&original.request_token)).await;
        let mut changed = original.clone();
        changed.thread_id = Some("019e92aa-0000-7000-8000-000000000002".into());
        poll.request(vec![changed]);
        assert!(poll.collect().as_object().unwrap().is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Returning to the original exact query may reuse its completed page.
        poll.request(vec![original.clone()]);
        assert_eq!(
            poll.collect()[&original.request_token]["threadId"],
            original.thread_id.as_deref().unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let fresh = query(2);
        let mut duplicate = fresh.clone();
        duplicate.thread_id = Some("019e92aa-0000-7000-8000-000000000003".into());
        poll.request(vec![fresh, duplicate, query(3)]);
        let state = ready(&mut poll, &[query(3).request_token]).await;
        assert!(state.get("dialog-2").is_none());
        assert_eq!(state.as_object().unwrap().len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn dialog_poll_rejects_token_reuse_while_inactive_worker_is_running() {
        let dir = tempfile::tempdir().unwrap();
        let gate = Gate::new();
        let mut poll = InjectionDialogPoller::with_reader(dir.path(), gate.reader());
        let original = query(1);
        poll.request(vec![original.clone()]);
        wait_calls(&gate.calls, 1).await;
        poll.request(vec![]);
        let mut changed = original;
        changed.thread_id = Some("019e92aa-0000-7000-8000-000000000002".into());
        poll.request(vec![changed]);
        assert!(poll.collect().as_object().unwrap().is_empty());
        assert_eq!(poll.tasks.len(), 1);
        gate.release();
        tokio::time::timeout(Duration::from_secs(3), async {
            while !poll.tasks.is_empty() {
                assert!(poll.collect().as_object().unwrap().is_empty());
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert!(poll.jobs.is_empty());
        assert!(poll.cache.is_empty());
    }

    #[tokio::test]
    async fn dialog_poll_errors_and_panics_are_cached_failures_not_empty_pages() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = calls.clone();
        let mut poll = InjectionDialogPoller::with_reader(
            dir.path(),
            Arc::new(move |_, query| {
                worker_calls.fetch_add(1, Ordering::SeqCst);
                match query.request_token.as_str() {
                    "dialog-1" => Err("database busy".into()),
                    "dialog-2" => panic!("test worker panic"),
                    _ => Ok(response(query)),
                }
            }),
        );
        let queries = vec![query(1), query(2)];
        let tokens: Vec<_> = queries.iter().map(|q| q.request_token.clone()).collect();
        poll.request(queries.clone());
        let state = ready(&mut poll, &tokens).await;
        for token in &tokens {
            assert_eq!(state[token]["error"], true);
            assert_eq!(state[token]["requestToken"], *token);
            assert_eq!(state[token]["threadId"], query(1).thread_id.unwrap());
            assert_eq!(state[token]["scope"], "chat");
            assert_eq!(state[token]["period"], "today");
            assert_eq!(state[token]["view"], "overview");
            assert!(state[token]["cursor"].is_null());
            assert!(state[token].get("requests").is_none());
        }
        assert!(poll.jobs.is_empty());
        poll.request(queries);
        assert!(poll.tasks.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // A new refresh token gets a new read after previous worker failures.
        poll.request(vec![query(3)]);
        let state = ready(&mut poll, &[query(3).request_token]).await;
        assert!(state["dialog-3"].get("requests").is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn dialog_poll_completed_cache_is_bounded_and_evicts_old_inactive_pages() {
        let dir = tempfile::tempdir().unwrap();
        let mut poll = InjectionDialogPoller::with_reader(
            dir.path(),
            Arc::new(|_, query| Ok(response(query))),
        );
        for index in 1..=32 {
            let query = query(index);
            poll.request(vec![query.clone()]);
            let state = ready(&mut poll, &[query.request_token]).await;
            assert_eq!(state.as_object().unwrap().len(), 1);
            assert!(poll.cache.len() <= 16);
        }
        assert_eq!(poll.cache.len(), 16);
        assert!(!poll.cache.contains_key("dialog-1"));
        assert!(poll.cache.contains_key("dialog-32"));
        let queries: Vec<_> = (33..=40).map(query).collect();
        let tokens: Vec<_> = queries.iter().map(|q| q.request_token.clone()).collect();
        poll.request(queries);
        let state = ready(&mut poll, &tokens).await;
        assert_eq!(poll.cache.len(), 16);
        assert_eq!(state.as_object().unwrap().len(), 8);
        assert!(tokens.iter().all(|token| poll.cache.contains_key(token)));
    }

    #[tokio::test]
    async fn dialog_poll_normalizes_identity_and_skips_invalid_queries_before_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = calls.clone();
        let mut poll = InjectionDialogPoller::with_reader(
            dir.path(),
            Arc::new(move |_, query| {
                worker_calls.fetch_add(1, Ordering::SeqCst);
                Ok(response(query))
            }),
        );
        let canonical = query(1);
        let mut upper = canonical.clone();
        upper.thread_id = upper.thread_id.map(|thread| thread.to_ascii_uppercase());
        let mut invalid_thread = query(2);
        invalid_thread.thread_id = Some("../../another-profile".into());
        let mut invalid_cursor = query(3);
        invalid_cursor.view = "logs".into();
        invalid_cursor.cursor = Some("arbitrary-sql-cursor".into());
        let mut invalid_scope = query(4);
        invalid_scope.scope = "unknown-scope".into();
        poll.request(vec![
            invalid_thread,
            invalid_cursor,
            invalid_scope,
            upper,
            canonical.clone(),
        ]);
        let state = ready(&mut poll, std::slice::from_ref(&canonical.request_token)).await;
        assert_eq!(state.as_object().unwrap().len(), 1);
        assert_eq!(
            state[&canonical.request_token]["threadId"],
            canonical.thread_id.unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(poll.cache.len(), 1);
    }

    #[tokio::test]
    async fn dialog_poll_timeout_settles_loading_without_releasing_blocked_worker_slots() {
        let dir = tempfile::tempdir().unwrap();
        let gate = Gate::new();
        let mut poll = InjectionDialogPoller::with_reader(dir.path(), gate.reader());
        let queries = vec![query(1), query(2)];
        let before = Instant::now();
        poll.request(queries.clone());
        let deadline = poll.cache["dialog-1"].deadline;
        assert!(deadline >= before + INJECTION_DIALOG_QUERY_TIMEOUT);
        wait_calls(&gate.calls, 2).await;
        poll.request(queries.clone());
        assert_eq!(poll.cache["dialog-1"].deadline, deadline);

        // Shorten the deadline to exercise the real expiry path without adding
        // a five-second sleep to every regression run.
        for entry in poll.cache.values_mut() {
            entry.deadline = Instant::now() + Duration::from_millis(20);
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        let timed_out = poll.collect();
        for query in &queries {
            let value = &timed_out[&query.request_token];
            assert_eq!(value["error"], true);
            assert_eq!(value["requestToken"], query.request_token);
            assert_eq!(value["threadId"], query.thread_id.as_deref().unwrap());
            assert_eq!(value["scope"], query.scope);
            assert_eq!(value["period"], query.period);
            assert_eq!(value["view"], query.view);
            assert!(value["cursor"].is_null());
            assert!(value.get("loading").is_none());
        }
        for _ in 0..5 {
            poll.request(queries.clone());
            assert_eq!(poll.collect(), timed_out);
            assert_eq!(poll.tasks.len(), 2);
        }
        assert_eq!(gate.calls.load(Ordering::SeqCst), 2);

        gate.release();
        tokio::time::timeout(Duration::from_secs(3), async {
            while !poll.tasks.is_empty() {
                assert_eq!(poll.collect(), timed_out);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        // Both successful reads arrived after their UI deadline. Neither may
        // replace the already returned error, even for the still-active query.
        assert_eq!(poll.collect(), timed_out);
        assert!(poll.jobs.is_empty());
        poll.request(vec![query(3)]);
        let retry = ready(&mut poll, &[query(3).request_token]).await;
        assert!(retry["dialog-3"].get("requests").is_some());
        assert_eq!(gate.calls.load(Ordering::SeqCst), 3);
        assert_eq!(gate.peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn dialog_poll_queued_timeouts_and_refreshes_cannot_create_extra_workers() {
        let dir = tempfile::tempdir().unwrap();
        let gate = Gate::new();
        let mut poll = InjectionDialogPoller::with_reader(dir.path(), gate.reader());
        poll.request(vec![query(1), query(2), query(3)]);
        wait_calls(&gate.calls, 2).await;

        // The third query has no worker yet: queue time still counts against
        // its UI deadline, and each subsequent refresh remains equally bounded.
        for index in 3..=24 {
            let query = query(index);
            poll.request(vec![query.clone()]);
            poll.cache.get_mut(&query.request_token).unwrap().deadline =
                Instant::now() - Duration::from_millis(1);
            let state = poll.collect();
            assert_eq!(state[&query.request_token]["error"], true);
            assert_eq!(poll.tasks.len(), 2);
            assert_eq!(poll.jobs.len(), 2);
            assert!(poll.cache.len() <= INJECTION_DIALOG_CACHE_LIMIT);
            assert_eq!(gate.calls.load(Ordering::SeqCst), 2);
        }
        // A fresh token can wait for the original physical reads to finish,
        // then start within its own deadline. No timed-out queued token runs.
        let retry = query(25);
        poll.request(vec![retry.clone()]);
        assert_eq!(poll.collect()[&retry.request_token]["loading"], true);
        assert_eq!(poll.tasks.len(), 2);
        gate.release();
        let state = ready(&mut poll, &[retry.request_token]).await;
        assert!(state["dialog-25"].get("requests").is_some());
        assert_eq!(gate.calls.load(Ordering::SeqCst), 3);
        assert_eq!(gate.peak.load(Ordering::SeqCst), 2);
        assert!(poll.jobs.is_empty());
    }
}
