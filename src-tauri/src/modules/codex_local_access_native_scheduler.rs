// Soft scheduling state is independent of durable upstream ownership.
type NativeAccountRank = (u8, usize, (i64, i64, i64));

fn native_account_rank(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
) -> NativeAccountRank {
    use CodexLocalAccessRoutingStrategy as Strategy;
    let pinned = key
        .and_then(|key| {
            key.priority_account_ids
                .iter()
                .position(|id| id == &account.id)
        })
        .unwrap_or(usize::MAX);
    let rule = collection
        .custom_routing_rules
        .iter()
        .find(|rule| rule.account_id == account.id);
    let plan = resolve_plan_rank(account);
    let quota = resolve_remaining_quota(account);
    let high_plan = -i64::from(plan.unwrap_or(-1));
    let high_quota = -i64::from(quota.unwrap_or(-1));
    let order = match collection.routing_strategy {
        Strategy::QuotaHighFirst => (high_quota, high_plan, 0),
        Strategy::QuotaLowFirst => (i64::from(quota.unwrap_or(i32::MAX)), high_plan, 0),
        Strategy::PlanLowFirst => (i64::from(plan.unwrap_or(i32::MAX)), high_quota, 0),
        Strategy::ExpirySoonFirst => (
            resolve_subscription_expiry_ms(account).unwrap_or(i64::MAX),
            high_plan,
            high_quota,
        ),
        Strategy::Custom => (
            -(rule.map(|rule| i64::from(rule.priority)).unwrap_or(0)),
            0,
            0,
        ),
        Strategy::Random => (0, 0, 0),
        _ => (high_plan, high_quota, 0),
    };
    let tier = match rule {
        Some(rule) if rule.is_preferred => 0,
        Some(rule) if rule.is_backup => 2,
        _ => 1,
    };
    (tier, pinned, order)
}

fn native_routing_namespace(
    caller: &native_contract::Caller,
    model: &str,
    scope: &[String],
    target: Option<&str>,
) -> String {
    let mut scope = scope.to_vec();
    scope.sort();
    scope.dedup();
    // JSON keeps user-controlled separators unambiguous. Requested model
    // namespaces remain distinct even when they resolve to the same upstream.
    serde_json::to_string(&(caller.id.as_str(), model, scope, target)).unwrap_or_default()
}

fn native_sort_accounts_scoped(
    host: &NativeHostBridge,
    accounts: &mut [CodexAccount],
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    namespace: &str,
) {
    use CodexLocalAccessRoutingStrategy as Strategy;
    if accounts.len() < 2 {
        return;
    }
    if collection.routing_strategy == Strategy::Random {
        accounts.shuffle(&mut rand::thread_rng());
    }
    accounts.sort_by_key(|account| native_account_rank(account, collection, key));
    if collection.routing_strategy == Strategy::Random {
        return;
    }
    let cursor = host
        .routing_cursors
        .lock()
        .ok()
        .and_then(|cursors| cursors.get(namespace).cloned());
    let rank = native_account_rank(&accounts[0], collection, key);
    let end = accounts
        .iter()
        .take_while(|account| native_account_rank(account, collection, key) == rank)
        .count();
    if end < 2 {
        return;
    }
    if collection.routing_strategy == Strategy::Custom {
        let mut rng = rand::thread_rng();
        let ranks: HashMap<String, f64> = accounts[..end]
            .iter()
            .map(|account| {
                let weight = collection
                    .custom_routing_rules
                    .iter()
                    .find(|rule| rule.account_id == account.id)
                    .map_or(1, |rule| rule.weight.max(1));
                (
                    account.id.clone(),
                    -rng.gen_range(f64::MIN_POSITIVE..1.0f64).ln() / f64::from(weight),
                )
            })
            .collect();
        accounts[..end].sort_by(|left, right| ranks[&left.id].total_cmp(&ranks[&right.id]));
        return;
    }
    let start = if let Some((last, _)) = cursor {
        if let Some(index) = accounts[..end]
            .iter()
            .position(|account| account.id == last)
        {
            (index + 1) % end
        } else {
            // Cooling down a previous member must not reset this caller's turn.
            let previous = collection.account_ids.iter().position(|id| id == &last);
            accounts[..end]
                .iter()
                .position(|account| {
                    previous.is_some_and(|previous| {
                        collection
                            .account_ids
                            .iter()
                            .position(|id| id == &account.id)
                            .is_some_and(|index| index > previous)
                    })
                })
                .unwrap_or(0)
        }
    } else {
        0
    };
    accounts[..end].rotate_left(start);
}

fn native_commit_routing_cursor(host: &NativeHostBridge, namespace: &str, account: &str) {
    if let Ok(mut cursors) = host.routing_cursors.lock() {
        if !cursors.contains_key(namespace) && cursors.len() >= NATIVE_ROUTING_CURSOR_LIMIT {
            // This is disposable rotation state, not durable continuity history.
            if let Some(key) = cursors.keys().next().cloned() {
                cursors.remove(&key);
            }
        }
        let cursor = cursors.entry(namespace.to_owned()).or_default();
        cursor.0 = account.to_owned();
        cursor.1 = cursor.1.wrapping_add(1);
    }
}

#[cfg(test)]
fn native_sort_accounts(
    host: &NativeHostBridge,
    accounts: &mut [CodexAccount],
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
) {
    let namespace = key.map_or("test:primary", |key| key.id.as_str());
    native_sort_accounts_scoped(host, accounts, collection, key, namespace);
    if let Some(account) = accounts.first() {
        native_commit_routing_cursor(host, namespace, &account.id);
    }
}

fn native_soft_binding_key(caller: &str, model: &str, identity: &str) -> String {
    serde_json::to_string(&(caller, model, identity)).unwrap_or_default()
}

// A child may inherit its parent's preference, but owns only its local keys.
// Distinct session/thread aliases without an explicit parent retain their
// ordinary alias behavior; the parent relationship is authoritative here.
fn native_soft_binding_ids<'a>(
    session: Option<&'a str>,
    thread: Option<&'a str>,
    parent: Option<&'a str>,
) -> (Vec<&'a str>, Option<&'a str>) {
    let parent = parent.filter(|parent| {
        thread.is_some_and(|thread| thread != *parent)
            || session.is_some_and(|session| session != *parent)
    });
    // Codex children may retain the root session while naming an immediate
    // parent from another level. An independent child thread is its sole local
    // routing identity; never publish the shared root session as a child alias.
    if let Some(parent) = parent {
        let local = thread
            .filter(|thread| *thread != parent)
            .or_else(|| session.filter(|session| *session != parent));
        return (local.into_iter().collect(), Some(parent));
    }
    let order = [session, thread];
    let mut local = Vec::new();
    for id in order.into_iter().flatten() {
        if Some(id) != parent && !local.contains(&id) {
            local.push(id);
        }
    }
    (local, parent)
}

fn native_soft_failure_unbinds(report: &native_contract::RequestReport) -> bool {
    if report.success || report.account_id.is_empty() {
        return false;
    }
    // The full upstream body decides request scope. A generic credential-scope
    // hint must not override Go's 400/invalid-request classification.
    if report
        .diagnostics
        .as_ref()
        .is_some_and(|context| context.upstream_request_scoped)
    {
        return false;
    }
    let message = report.error_message.as_deref().unwrap_or_default();
    let code = report.error.as_deref().unwrap_or_default();
    // Actual credential scope (including HTTP 401 authentication_error),
    // invalid_grant, Cloudflare, and model failures win over request faults.
    if native_report_model_unavailable(report)
        || report.error_message.as_deref().is_some_and(|message| {
            native_api::upstream_errors::model_unavailable_message(
                report.status,
                message,
                &report.upstream_model,
            )
        })
        || report.credential_scope
        || matches!(report.status, 400 | 401)
            && (code.to_ascii_lowercase().contains("invalid_grant")
                || message.to_ascii_lowercase().contains("invalid_grant"))
        || native_api::diagnostics::is_cloudflare_challenge(message)
        || matches!(report.status, 402 | 429)
    {
        return true;
    }
    fn request_code(code: &str) -> bool {
        matches!(
            code.trim().to_ascii_lowercase().as_str(),
            "cyber_policy"
                | "context_length_exceeded"
                | "message_too_big"
                | "string_above_max_length"
                | "invalid_prompt"
                | "invalid_value"
                | "unsupported_value"
                | "invalid_request"
                | "invalid_request_error"
                | "bad_request_error"
                | "previous_response_not_found"
                | "request_scoped"
                | "connection_lifecycle"
                | "client_cancelled"
                | "client_disconnected"
                | "client_interrupted"
        )
    }
    if report.error.as_deref().is_some_and(request_code) {
        return false;
    }
    if let Some(message) = &report.error_message {
        if let Ok(value) = serde_json::from_str::<Value>(message) {
            if [
                "/error/code",
                "/error/type",
                "/code",
                "/type",
                "/response/error/code",
                "/response/error/type",
                "/body/error/code",
                "/body/error/type",
            ]
            .iter()
            .any(|path| {
                value
                    .pointer(path)
                    .and_then(Value::as_str)
                    .is_some_and(request_code)
            }) {
                return false;
            }
        }
        let message = message.to_ascii_lowercase();
        if message.contains("item with id")
            && message.contains("not found")
            && message.contains("items are not persisted when `store` is set to false")
        {
            return false;
        }
    }
    // HTTP completion retains its numbered status wrapper, not the upstream
    // error code. After the actual body has been classified above, these
    // normally request-scoped statuses may only reach here for Go's model,
    // grant, or challenge exceptions. Local errors never use this wrapper.
    if matches!(report.status, 400 | 409 | 413 | 422)
        && report.error.as_deref() == Some(format!("upstream_http_{}", report.status).as_str())
    {
        return true;
    }
    // Local cancellation, transport/preparation failures, and caller parameter
    // errors do not establish an unhealthy credential. Keep the same boundary
    // used by the host's real upstream cooldown recording.
    native_recorded_failure_cools_account(report)
}

fn native_prune_soft_bindings(
    host: &NativeHostBridge,
    bindings: &mut HashMap<String, (String, i64)>,
    epoch: u64,
    enabled: bool,
    ttl: i64,
) {
    let Ok(mut order) = host.binding_lru.lock() else {
        bindings.clear();
        return;
    };
    if host.binding_cache_epoch.swap(epoch, Ordering::AcqRel) != epoch || !enabled {
        bindings.clear();
        order.clear();
        return;
    }
    let now = now_ms();
    while let Some((at, key)) = order.keys().next().cloned() {
        if now.saturating_sub(at) < ttl {
            break;
        }
        order.remove(&(at, key.clone()));
        if bindings
            .get(&key)
            .is_some_and(|(_, recorded)| *recorded == at)
        {
            bindings.remove(&key);
        }
    }
}

fn native_publish_soft_bindings(
    host: &NativeHostBridge,
    bindings: &mut HashMap<String, (String, i64)>,
    keys: impl IntoIterator<Item = String>,
    account: &str,
    at: i64,
) {
    let Ok(mut order) = host.binding_lru.lock() else {
        return;
    };
    for key in keys {
        if let Some((_, previous)) = bindings.get(&key) {
            order.remove(&(*previous, key.clone()));
        }
        bindings.insert(key.clone(), (account.to_owned(), at));
        order.insert((at, key), ());
    }
    while bindings.len() > NATIVE_SOFT_BINDING_LIMIT {
        let Some((at, key)) = order.keys().next().cloned() else {
            break;
        };
        order.remove(&(at, key.clone()));
        if bindings
            .get(&key)
            .is_some_and(|(_, recorded)| *recorded == at)
        {
            bindings.remove(&key);
        }
    }
}

impl NativeHostBridge {
    async fn record_soft_binding_result(&self, report: &native_contract::RequestReport) {
        if !self.session_affinity_enabled.load(Ordering::Acquire) {
            return;
        }
        let Some(context) = &report.diagnostics else {
            return;
        };
        if !report.success && !native_soft_failure_unbinds(report) {
            return;
        }
        let mut bindings = self.bindings.lock().await;
        if !self.session_affinity_enabled.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut order) = self.binding_lru.lock() else {
            return;
        };
        let (session, thread, parent) = if let Some(identity) = &context.routing_identity {
            (
                identity.session_id.as_deref(),
                identity.thread_id.as_deref(),
                identity.parent_thread_id.as_deref(),
            )
        } else {
            (
                context.client_session_id.as_deref(),
                context.client_thread_id.as_deref(),
                None,
            )
        };
        let (local, _) = native_soft_binding_ids(session, thread, parent);
        for id in local {
            let key = native_soft_binding_key(&report.caller.id, &report.requested_model, id);
            if let Some((account, at)) = bindings.get_mut(&key) {
                if account != &report.account_id {
                    continue;
                }
                order.remove(&(*at, key.clone()));
                if report.success {
                    *at = now_ms();
                    order.insert((*at, key), ());
                } else {
                    bindings.remove(&key);
                }
            }
        }
    }
}

#[cfg(test)]
include!("codex_local_access_native_scheduler_tests.rs");
#[cfg(test)]
#[path = "native_soft_affinity_regression_tests.rs"]
mod native_soft_affinity_regression_tests;
