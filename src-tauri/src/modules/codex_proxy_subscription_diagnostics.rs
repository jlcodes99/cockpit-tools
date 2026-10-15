//! Structural diagnostics contain display names and fixed error codes only.
use super::{ParsedCatalog, ParsedNode};
use crate::modules::codex_proxy_catalog_binding as binding;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Serialize)]
pub struct GroupIssue {
    pub name: String,
    pub error: String,
}

pub(super) fn derived_group_error(error: &str) -> bool {
    matches!(
        error,
        "SUBSCRIPTION_GROUP_UNAVAILABLE"
            | "SUBSCRIPTION_GROUP_CYCLE"
            | "SUBSCRIPTION_GROUP_MEMBER_MISSING"
            | "SUBSCRIPTION_GROUP_MEMBER_UNSUPPORTED"
            | "PROXY_TLS_INSECURE"
    )
}

fn own_group_error(error: &str) -> &str {
    match error {
        "SUBSCRIPTION_GROUP_OPTIONS"
        | "SUBSCRIPTION_GROUP_STRATEGY"
        | "SUBSCRIPTION_PROVIDER_UNSUPPORTED"
        | "SUBSCRIPTION_INVALID" => error,
        _ => "SUBSCRIPTION_GROUP_MEMBER_UNSUPPORTED",
    }
}

/// Report the actual rejected leaves, missing references and cycles without
/// flattening or removing candidates from automatic groups. A manual selector
/// may remain usable while showing issues in its other, unselected branches.
pub(crate) fn group_issues(catalog: &ParsedCatalog) -> Vec<Vec<GroupIssue>> {
    let nodes: HashMap<_, _> = catalog.nodes.iter().map(|n| (n.name.as_str(), n)).collect();
    let groups: HashMap<_, _> = catalog
        .groups
        .iter()
        .enumerate()
        .map(|(i, g)| (g.name.as_str(), i))
        .collect();
    catalog
        .groups
        .iter()
        .map(|root| {
            let mut issues = BTreeSet::new();
            let mut visited = HashSet::new();
            let mut active = HashSet::new();
            let mut pending = vec![(root.name.as_str(), false)];
            while let Some((name, leaving)) = pending.pop() {
                if leaving {
                    active.remove(name);
                    continue;
                }
                if let Some(node) = nodes.get(name) {
                    if let Some(error) = &node.error {
                        issues.insert((name.to_owned(), super::safe_node_error(error)));
                    } else if node.outbound.is_none() {
                        issues.insert((name.to_owned(), "PROXY_UNSUPPORTED_OPTION".into()));
                    }
                    continue;
                }
                if binding::is_builtin_name(name) {
                    if !binding::is_blocking_builtin(name) {
                        issues.insert((name.to_owned(), "PROXY_UNSUPPORTED_OPTION".into()));
                    }
                    continue;
                }
                let Some(index) = groups.get(name) else {
                    issues.insert((name.to_owned(), "SUBSCRIPTION_GROUP_MEMBER_MISSING".into()));
                    continue;
                };
                if active.contains(name) {
                    issues.insert((name.to_owned(), "SUBSCRIPTION_GROUP_CYCLE".into()));
                    continue;
                }
                if !visited.insert(name) {
                    continue;
                }
                let group = &catalog.groups[*index];
                if let Some(error) = group.error.as_deref().filter(|e| !derived_group_error(e)) {
                    issues.insert((name.to_owned(), own_group_error(error).into()));
                }
                if group.members.is_empty() {
                    issues.insert((name.to_owned(), "SUBSCRIPTION_GROUP_MEMBER_MISSING".into()));
                }
                active.insert(name);
                pending.push((name, true));
                pending.extend(
                    group
                        .members
                        .iter()
                        .rev()
                        .map(|member| (member.as_str(), false)),
                );
            }
            issues
                .into_iter()
                .map(|(name, error)| GroupIssue { name, error })
                .collect()
        })
        .collect()
}

/// Strict structural traversal for explicitly authorized group operations.
/// Detect cycles before the visited set, so shared descendants are deduplicated
/// but a back edge is never mistaken for an already processed valid branch.
pub(crate) fn reachable_nodes<'a>(
    catalog: &'a ParsedCatalog,
    group_id: &str,
) -> Result<Vec<&'a ParsedNode>, String> {
    let root = catalog
        .groups
        .iter()
        .find(|group| group.id == group_id)
        .ok_or("CATALOG_NOT_FOUND")?;
    let nodes: HashMap<_, _> = catalog
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    let groups: HashMap<_, _> = catalog
        .groups
        .iter()
        .map(|group| (group.name.as_str(), group))
        .collect();
    let mut found = Vec::new();
    let mut visited = HashSet::new();
    let mut active = HashSet::new();
    let mut pending = vec![(root.name.as_str(), false)];
    while let Some((name, leaving)) = pending.pop() {
        if leaving {
            active.remove(name);
            continue;
        }
        if active.contains(name) {
            return Err("SUBSCRIPTION_GROUP_CYCLE".into());
        }
        if !visited.insert(name) || binding::is_builtin_name(name) {
            continue;
        }
        if let Some(node) = nodes.get(name) {
            found.push(*node);
            continue;
        }
        let group = groups
            .get(name)
            .ok_or("SUBSCRIPTION_GROUP_MEMBER_MISSING")?;
        active.insert(name);
        pending.push((name, true));
        pending.extend(
            group
                .members
                .iter()
                .rev()
                .map(|member| (member.as_str(), false)),
        );
    }
    Ok(found)
}
