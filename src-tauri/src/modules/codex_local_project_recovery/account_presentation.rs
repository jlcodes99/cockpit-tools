use super::*;

pub(super) fn referenced_threads(sources: &[&Value]) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for source in sources {
        let pins = source[PINNED_THREADS].as_array().into_iter().flatten();
        let orders = source[THREAD_ORDERS]
            .as_object()
            .into_iter()
            .flat_map(|orders| orders.values())
            .flat_map(|order| order["threadIds"].as_array().into_iter().flatten());
        let memberships = source[ASSIGN].as_object().into_iter().flat_map(|items| items.keys()).map(String::as_str);
        for id in pins.chain(orders).filter_map(Value::as_str).chain(memberships) {
            check()?;
            if seen.insert(id) {
                result.push(id.to_owned());
                if result.len() == QUICK_MAX_THREADS {
                    return Ok(result);
                }
            }
        }
    }
    Ok(result)
}

pub(super) fn merge(
    view: &mut Value,
    sources: &[&Value],
    host: &str,
    projects: &[(&str, &str, &str)],
    threads: &[Thread],
    assignments: &BTreeMap<String, Value>,
    nonlocal: &HashSet<&str>,
) -> Result<(), String> {
    let target = sources[0];
    let current = sources[1];
    if target == current {
        return Ok(());
    }
    let mut appearances = object(&target[PROJECT_APPEARANCES]);
    let mut orders = object(&target[THREAD_ORDERS]);
    let mut copied_appearances = false;
    let mut copied_orders = false;
    for &(current_id, server_id, target_id) in projects {
        check()?;
        let appearance = current[PROJECT_APPEARANCES]
            .get(current_id)
            .or_else(|| current[PROJECT_APPEARANCES].get(server_id));
        if let Some(appearance) = appearance {
            if let (Some(color), Some(marker)) = (appearance.get("color"), appearance.get("marker")) {
                appearances.insert(target_id.to_owned(), json!({"color":color,"marker":marker}));
                copied_appearances = true;
            }
        }

        let current_order = current[THREAD_ORDERS]
            .get(current_id)
            .or_else(|| current[THREAD_ORDERS].get(server_id));
        let Some(current_ids) = current_order.and_then(|value| value["threadIds"].as_array()) else {
            continue;
        };
        let belongs_to_project = |id: &str| {
            assignments.get(id).and_then(|entry| entry["projectId"].as_str()) == Some(target_id)
        };
        let mut seen = HashSet::new();
        let mut next_ids = Vec::new();
        for id in current_ids.iter().filter_map(Value::as_str) {
            check()?;
            if belongs_to_project(id) && seen.insert(id) {
                next_ids.push(json!(id));
            }
        }
        if next_ids.is_empty() && !current_ids.is_empty() {
            continue;
        }
        if let Some(existing) = orders.get(target_id).and_then(|order| order["threadIds"].as_array()) {
            for value in existing {
                check()?;
                if value.as_str().is_none_or(|id| !belongs_to_project(id)) {
                    next_ids.push(value.clone());
                }
            }
        }
        orders.insert(target_id.to_owned(), json!({"threadIds":next_ids}));
        copied_orders = true;
    }
    if copied_appearances {
        view[PROJECT_APPEARANCES] = json!(appearances);
    }
    if copied_orders {
        view[THREAD_ORDERS] = json!(orders);
    }
    if let Some(pins) = merge_legacy_pins(target, current, sources, host, threads, assignments, nonlocal)? {
        view[PINNED_THREADS] = json!(pins);
    }
    Ok(())
}

fn merge_legacy_pins(
    target: &Value,
    current: &Value,
    sources: &[&Value],
    host: &str,
    threads: &[Thread],
    assignments: &BTreeMap<String, Value>,
    nonlocal: &HashSet<&str>,
) -> Result<Option<Vec<Value>>, String> {
    let Some(current_pins) = current[PINNED_THREADS].as_array() else {
        return Ok(None);
    };
    if target[PINNED_THREADS] == current[PINNED_THREADS] {
        return Ok(None);
    }
    let migrated = sources.iter().flat_map(|source| {
        source[MIGRATED_PINS][host].as_array().into_iter().flatten().filter_map(Value::as_str)
    }).collect::<HashSet<_>>();
    let projectless = projectless(current);
    let local = threads
        .iter()
        .filter(|thread| {
            thread.root
                && thread.section.is_none()
                && !migrated.contains(thread.id.as_str())
                && !nonlocal.contains(thread.id.as_str())
                && (assignments.contains_key(&thread.id)
                    || (thread.project.is_none()
                        && (projectless.contains(&thread.id)
                            || current[HOSTS][&thread.id].as_str() == Some("local"))
                        && thread.cwd.as_deref().is_some_and(|cwd| {
                            Path::new(cwd).is_absolute() && !cloud_root(cwd)
                        })))
        })
        .map(|thread| thread.id.as_str())
        .collect::<HashSet<_>>();
    if local.is_empty() {
        return Ok(None);
    }
    let target_pins = match target.get(PINNED_THREADS) {
        Some(value) => match value.as_array() {
            Some(pins) => pins.as_slice(),
            None => return Ok(None),
        },
        None => &[],
    };
    let mut seen = HashSet::new();
    let mut pins = Vec::new();
    for value in target_pins {
        check()?;
        if value.as_str().is_none_or(|id| !local.contains(id)) {
            pins.push(value.clone());
        }
    }
    for id in current_pins.iter().filter_map(Value::as_str) {
        check()?;
        if local.contains(id) && seen.insert(id) {
            pins.push(json!(id));
        }
    }
    Ok(Some(pins))
}
