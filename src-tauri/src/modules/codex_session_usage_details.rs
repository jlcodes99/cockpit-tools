#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionUsageCursor {
    timestamp: i64,
    request_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionUsageRequest {
    request_id: String,
    timestamp: i64,
    instance_id: String,
    instance_name: String,
    session_id: String,
    session_title: String,
    model: String,
    project_path: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    estimated_cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionUsagePage {
    rows: Vec<CodexSessionUsageRequest>,
    next_cursor: Option<CodexSessionUsageCursor>,
}

const USAGE_PROJECT_GROUP: &str = "COALESCE(project_key,'')";

fn usage_filters(
    query: &CodexSessionUsageQuery,
    projects_complete: bool,
) -> (String, Vec<rusqlite::types::Value>) {
    let mut conditions = String::from("WHERE 1=1");
    let mut params = Vec::new();
    for (operator, timestamp) in [(">=", query.from_timestamp), ("<=", query.to_timestamp)] {
        if let Some(timestamp) = timestamp {
            conditions.push_str(&format!(" AND timestamp {operator} ?"));
            params.push(timestamp.into());
        }
    }
    for (field, value) in [
        (
            "instance_id",
            query
                .instance_id
                .as_deref()
                .filter(|value| !value.is_empty()),
        ),
        (
            "model",
            query.model.as_deref().filter(|value| !value.is_empty()),
        ),
        (
            if query.project_key.as_deref() == Some("") {
                USAGE_PROJECT_GROUP
            } else {
                "project_key"
            },
            query.project_key.as_deref(),
        ),
        (
            "session_id",
            query
                .session_id
                .as_deref()
                .filter(|value| !value.is_empty()),
        ),
    ] {
        if let Some(value) = value {
            conditions.push_str(&format!(" AND {field} = ?"));
            params.push(value.to_owned().into());
        }
    }
    if let Some(session_id) = query
        .session_id
        .as_ref()
        .filter(|id| !id.is_empty() && projects_complete)
    {
        conditions.push_str(
            " AND file_path IN (SELECT file_path FROM session_usage_projects WHERE session_id=?)",
        );
        params.push(session_id.clone().into());
    }
    (conditions, params)
}

fn usage_detail_source() -> &'static str {
    "SELECT * FROM session_usage_events
        WHERE file_path NOT IN (SELECT file_path FROM session_rewrite_state WHERE phase>=1)
     UNION ALL SELECT * FROM session_rewrite_events
        WHERE file_path IN (SELECT file_path FROM session_rewrite_state WHERE phase>=1)"
}

pub fn query_session_usage_requests(
    query: CodexSessionUsageQuery,
    cursor: Option<CodexSessionUsageCursor>,
) -> Result<CodexSessionUsagePage, String> {
    let conn = SessionUsageStore::open_default()?.open_conn()?;
    conn.execute_batch("BEGIN")
        .map_err(|error| error.to_string())?;
    query_usage_request_page(&conn, &query, cursor.as_ref(), &current_instance_names())
}

fn query_usage_request_page(
    conn: &Connection,
    query: &CodexSessionUsageQuery,
    cursor: Option<&CodexSessionUsageCursor>,
    instance_names: &HashMap<String, String>,
) -> Result<CodexSessionUsagePage, String> {
    const PAGE_SIZE: usize = 50;
    let (mut conditions, mut params) = usage_filters(query, usage_projects_complete(conn));
    if let Some(cursor) = cursor {
        conditions.push_str(" AND timestamp <= ? AND (timestamp,request_id) < (?,?)");
        params.extend([
            cursor.timestamp.into(),
            cursor.timestamp.into(),
            cursor.request_id.clone().into(),
        ]);
    }
    let sql = format!(
        "SELECT request_id,timestamp,instance_id,instance_name,session_id,session_title,model,
                project_path,input_tokens,cached_input_tokens,output_tokens
         FROM (
            SELECT r.*, COALESCE(p.session_title,'') AS session_title,
                   COALESCE(p.project_path,'') AS project_path,
                   p.project_key
            FROM ({source}) r LEFT JOIN session_usage_projects p USING(file_path)
         ) {conditions}
         ORDER BY timestamp DESC,request_id DESC LIMIT {limit}",
        source = usage_detail_source(),
        limit = PAGE_SIZE + 1,
    );
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            let instance_id: String = row.get(2)?;
            let stored_name: String = row.get(3)?;
            let model: String = row.get(6)?;
            let input_tokens = row.get::<_, i64>(8)?.max(0) as u64;
            let cached_input_tokens = (row.get::<_, i64>(9)?.max(0) as u64).min(input_tokens);
            let output_tokens = row.get::<_, i64>(10)?.max(0) as u64;
            Ok(CodexSessionUsageRequest {
                request_id: row.get(0)?,
                timestamp: row.get(1)?,
                instance_name: instance_names
                    .get(&instance_id)
                    .cloned()
                    .unwrap_or(stored_name),
                instance_id,
                session_id: row.get(4)?,
                session_title: row.get(5)?,
                estimated_cost_usd:
                    crate::modules::codex_local_access::estimate_known_model_token_cost_usd(
                        &model,
                        input_tokens,
                        cached_input_tokens,
                        output_tokens,
                    ),
                model,
                project_path: row.get(7)?,
                input_tokens,
                cached_input_tokens,
                output_tokens,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut rows = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let next_cursor = if rows.len() > PAGE_SIZE {
        rows.pop();
        rows.last().map(|row| CodexSessionUsageCursor {
            timestamp: row.timestamp,
            request_id: row.request_id.clone(),
        })
    } else {
        None
    };
    Ok(CodexSessionUsagePage { rows, next_cursor })
}

fn known_usage_cost(model: &str, row: &CodexSessionUsageBreakdownRow) -> Option<f64> {
    crate::modules::codex_local_access::estimate_known_model_token_cost_usd(
        model,
        row.input_tokens,
        row.cached_input_tokens,
        row.output_tokens,
    )
}

fn price_usage_groups(
    conn: &Connection,
    where_sql: &str,
    params: &[rusqlite::types::Value],
    column: &str,
    mut groups: Vec<CodexSessionUsageBreakdownRow>,
) -> Result<Vec<CodexSessionUsageBreakdownRow>, String> {
    let sql = format!(
        "SELECT {column},model,SUM(input_tokens),SUM(cached_input_tokens),SUM(output_tokens)
         FROM session_usage_report {where_sql} GROUP BY {column},model"
    );
    let mut stmt = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let mut costs: HashMap<String, Option<f64>> = HashMap::new();
    let mut rows = stmt
        .query(rusqlite::params_from_iter(params.iter()))
        .map_err(|error| error.to_string())?;
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let read_count = |index| row.get::<_, i64>(index).map(|value| value.max(0) as u64);
        let (key, cost) = (|| -> rusqlite::Result<_> {
            let key: String = row.get(0)?;
            let model: String = row.get(1)?;
            let cost = crate::modules::codex_local_access::estimate_known_model_token_cost_usd(
                &model,
                read_count(2)?,
                read_count(3)?,
                read_count(4)?,
            );
            Ok((key, cost))
        })()
        .map_err(|error| error.to_string())?;
        let total = costs.entry(key).or_insert(Some(0.0));
        *total = total.zip(cost).map(|(left, right)| left + right);
    }
    for group in &mut groups {
        group.estimated_cost_usd = costs.remove(&group.key).flatten();
    }
    Ok(groups)
}

fn usage_options(rows: &[CodexSessionUsageBreakdownRow]) -> Vec<CodexSessionUsageInstanceOption> {
    rows.iter()
        .map(|row| CodexSessionUsageInstanceOption {
            id: row.key.clone(),
            name: row.label.clone(),
        })
        .collect()
}

fn usage_filter_options(
    conn: &Connection,
    query: &CodexSessionUsageQuery,
    source: &str,
) -> Result<
    (
        Vec<CodexSessionUsageInstanceOption>,
        Vec<CodexSessionUsageInstanceOption>,
    ),
    String,
> {
    let (conditions, params) = usage_filters(query, false);
    let scope = format!("SELECT * FROM ({source}) {conditions}");
    let models = format!("SELECT DISTINCT model,model FROM ({scope})");
    let projects = format!(
        "SELECT COALESCE(p.project_key,''),COALESCE(MAX(p.project_path),'')
         FROM ({scope}) s LEFT JOIN session_usage_projects p USING(file_path)
         GROUP BY COALESCE(p.project_key,'')"
    );
    let read = |sql: &str| -> Result<Vec<CodexSessionUsageInstanceOption>, String> {
        let mut stmt = conn.prepare(sql).map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok(CodexSessionUsageInstanceOption {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    };
    Ok((read(&models)?, read(&projects)?))
}
