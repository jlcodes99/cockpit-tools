//! Multi-dimensional dashboard telemetry aggregator, filter engine, sorter, and paginator.
//!
//! Supports `today`, `7d`, `30d`, `month`, `all`, and `custom` date ranges,
//! guaranteeing 100% mathematical reconciliation across `kpi`, `time_series`,
//! `by_model`, `by_workspace`, `by_kind`, and `filtered_conversations`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Datelike, Days, FixedOffset, Local, NaiveDate, Utc};

use super::models::format_model_display_name;
use super::pricing::estimate_exact_cost_usd;
use super::protobuf::format_utc_ms;
use super::scanner::{get_workspace_key_from_path, normalize_timestamp};
use super::types::{
    AggregatedDashboardBundle, AppliedFilters, CatalogModelOption, CatalogWorkspaceOption,
    ClientSourceKind, ConversationKind, ConversationListResponse, ConversationSortField,
    ConversationSummaryItem, DashboardQueryRequest, DashboardSummaryResponse, DatePreset,
    EstimatedTokenTotals, ExactTokenTotals, FilterOptionsCatalog, HeatmapDayPoint,
    KindUsageSummary, KpiSummary, ModelUsageSummary, ResolvedTurnGroup, ScanResult,
    ScannedConversation, SortDirection, TimeBucketPoint, TimeGranularity, WorkspaceUsageSummary,
};

const MAX_ZERO_FILL_BUCKETS: i64 = 400;
/// Proleptic Gregorian days from 0001-01-01 (`num_days_from_ce() == 1`) to 1970-01-01.
const UNIX_EPOCH_CE_DAYS: i32 = 719_163;

#[inline]
fn round2(val: f64) -> f64 {
    if !val.is_finite() {
        0.0
    } else {
        (val * 100.0).round() / 100.0
    }
}

#[inline]
fn compute_share_percent(part_exact: u64, total_exact: u64, part_est: u64, total_est: u64) -> f64 {
    if total_exact > 0 {
        round2((part_exact as f64 / total_exact as f64) * 100.0)
    } else if total_est > 0 {
        round2((part_est as f64 / total_est as f64) * 100.0)
    } else {
        0.0
    }
}

pub fn get_default_tz_offset_minutes() -> i32 {
    Local::now().offset().local_minus_utc() / 60
}

#[inline]
fn minute_slot_to_local_date(minute_slot: i64, tz_offset_minutes: i32) -> Option<NaiveDate> {
    let local_days = (minute_slot + tz_offset_minutes as i64).div_euclid(1_440);
    let ce_days = UNIX_EPOCH_CE_DAYS.checked_add(i32::try_from(local_days).ok()?)?;
    NaiveDate::from_num_days_from_ce_opt(ce_days)
}

#[inline]
fn ms_to_local_date(ms: i64, tz_offset_minutes: i32) -> Option<NaiveDate> {
    if ms <= 0 {
        return None;
    }
    minute_slot_to_local_date(ms.div_euclid(60_000), tz_offset_minutes)
}

fn parse_preset(raw: Option<&str>) -> DatePreset {
    match raw.map(str::trim).unwrap_or("30d") {
        "today" => DatePreset::Today,
        "7d" => DatePreset::Days7,
        "30d" => DatePreset::Days30,
        "month" => DatePreset::Month,
        "custom" => DatePreset::Custom,
        "all" => DatePreset::All,
        _ => DatePreset::Days30,
    }
}

fn parse_granularity(raw: Option<&str>) -> TimeGranularity {
    match raw.map(str::trim).unwrap_or("day") {
        "week" => TimeGranularity::Week,
        "month" => TimeGranularity::Month,
        _ => TimeGranularity::Day,
    }
}

fn parse_kind_filter(raw: Option<&str>) -> String {
    match raw.map(str::trim).unwrap_or("") {
        "main" => "main".to_string(),
        "swarm" => "swarm".to_string(),
        "subagent" => "subagent".to_string(),
        _ => String::new(),
    }
}

#[inline]
fn round_cost(val: f64) -> f64 {
    if !val.is_finite() || val <= 0.0 {
        0.0
    } else {
        (val * 10_000.0).round() / 10_000.0
    }
}

fn parse_sort_field(raw: Option<&str>) -> ConversationSortField {
    match raw.map(str::trim).unwrap_or("lastModifiedTime") {
        "totalTokens" => ConversationSortField::TotalTokens,
        "inputTokens" => ConversationSortField::InputTokens,
        "outputTokens" => ConversationSortField::OutputTokens,
        "combinedOutputTokens" => ConversationSortField::CombinedOutputTokens,
        "thinkingTokens" => ConversationSortField::ThinkingTokens,
        "cacheReadTokens" => ConversationSortField::CacheReadTokens,
        "estimatedCostUsd" => ConversationSortField::EstimatedCostUsd,
        "stepCount" => ConversationSortField::StepCount,
        "turnCount" => ConversationSortField::TurnCount,
        "title" => ConversationSortField::Title,
        "kind" => ConversationSortField::Kind,
        "workspace" => ConversationSortField::Workspace,
        "model" => ConversationSortField::Model,
        _ => ConversationSortField::LastModifiedTime,
    }
}

fn parse_sort_direction(raw: Option<&str>) -> SortDirection {
    match raw.map(str::trim).unwrap_or("desc") {
        "asc" => SortDirection::Asc,
        _ => SortDirection::Desc,
    }
}

fn parse_ymd_date(raw: Option<&str>) -> Option<NaiveDate> {
    let s = raw?.trim();
    if s.len() < 10 {
        return None;
    }
    NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").ok()
}

fn iso_to_local_date(iso_str: &str, tz: &FixedOffset) -> Option<NaiveDate> {
    if iso_str.trim().is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(iso_str.trim()) {
        return Some(dt.with_timezone(tz).date_naive());
    }
    let norm = normalize_timestamp(iso_str)?;
    let dt = DateTime::parse_from_rfc3339(&norm).ok()?;
    Some(dt.with_timezone(tz).date_naive())
}

#[inline]
fn conv_fallback_local_date(
    conv: &ScannedConversation,
    tz_offset_minutes: i32,
    tz: &FixedOffset,
) -> Option<NaiveDate> {
    conv.last_modified_ms
        .and_then(|ms| ms_to_local_date(ms, tz_offset_minutes))
        .or_else(|| {
            if conv.summary.last_modified_time.is_empty() {
                None
            } else {
                iso_to_local_date(&conv.summary.last_modified_time, tz)
            }
        })
        .or_else(|| {
            conv.created_at_ms
                .and_then(|ms| ms_to_local_date(ms, tz_offset_minutes))
        })
        .or_else(|| {
            conv.summary
                .created_at
                .as_deref()
                .and_then(|s| iso_to_local_date(s, tz))
        })
}

#[inline]
fn group_local_date(
    group: &ResolvedTurnGroup,
    conv: &ScannedConversation,
    tz_offset_minutes: i32,
    tz: &FixedOffset,
) -> Option<NaiveDate> {
    group
        .minute_slot
        .and_then(|slot| minute_slot_to_local_date(slot, tz_offset_minutes))
        .or_else(|| conv_fallback_local_date(conv, tz_offset_minutes, tz))
}

fn build_catalog(
    conversations: &[ScannedConversation],
    tz_offset_minutes: i32,
    tz: &FixedOffset,
) -> FilterOptionsCatalog {
    let mut model_counts: HashMap<String, (String, usize)> = HashMap::new();
    let mut ws_counts: HashMap<String, (String, String, usize)> = HashMap::new();
    let mut min_date: Option<NaiveDate> = None;
    let mut max_date: Option<NaiveDate> = None;

    let mut update_minmax = |d: NaiveDate| {
        min_date = Some(min_date.map_or(d, |cur| cur.min(d)));
        max_date = Some(max_date.map_or(d, |cur| cur.max(d)));
    };

    for conv in conversations {
        let mut seen_models: HashSet<&str> = HashSet::new();
        for (idx, m_id) in conv.summary.models.iter().enumerate() {
            if !m_id.is_empty() && seen_models.insert(m_id.as_str()) {
                if let Some(entry) = model_counts.get_mut(m_id.as_str()) {
                    entry.1 += 1;
                } else {
                    let disp = conv
                        .summary
                        .model_display_names
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| format_model_display_name(m_id));
                    model_counts.insert(m_id.clone(), (disp, 1));
                }
            }
        }
        if seen_models.is_empty()
            && !conv.summary.primary_model_id.is_empty()
            && conv.summary.primary_model_id != "unknown"
        {
            let m_id = &conv.summary.primary_model_id;
            if let Some(entry) = model_counts.get_mut(m_id.as_str()) {
                entry.1 += 1;
            } else {
                let disp = if conv.summary.primary_model_display_name.is_empty() {
                    format_model_display_name(m_id)
                } else {
                    conv.summary.primary_model_display_name.clone()
                };
                model_counts.insert(m_id.clone(), (disp, 1));
            }
        }

        let ws_key = get_workspace_key_from_path(&conv.summary.workspace_path);
        let ws_entry = ws_counts.entry(ws_key).or_insert_with(|| {
            (
                conv.summary.workspace_name.clone(),
                conv.summary.workspace_path.clone(),
                0,
            )
        });
        ws_entry.2 += 1;

        if conv.turn_groups.is_empty() {
            if let Some(d) = conv_fallback_local_date(conv, tz_offset_minutes, tz) {
                update_minmax(d);
            }
        } else {
            for g in &conv.turn_groups {
                if let Some(d) = group_local_date(g, conv, tz_offset_minutes, tz) {
                    update_minmax(d);
                }
            }
        }
    }

    let mut models: Vec<CatalogModelOption> = model_counts
        .into_iter()
        .map(
            |(model_id, (model_display_name, count))| CatalogModelOption {
                model_id,
                model_display_name,
                count,
            },
        )
        .collect();
    models.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.model_display_name.cmp(&b.model_display_name))
            .then_with(|| a.model_id.cmp(&b.model_id))
    });

    let mut workspaces: Vec<CatalogWorkspaceOption> = ws_counts
        .into_iter()
        .map(
            |(workspace_key, (workspace_name, workspace_path, count))| CatalogWorkspaceOption {
                workspace_key,
                workspace_name,
                workspace_path,
                count,
            },
        )
        .collect();
    workspaces.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.workspace_name.cmp(&b.workspace_name))
            .then_with(|| a.workspace_key.cmp(&b.workspace_key))
    });

    FilterOptionsCatalog {
        models,
        workspaces,
        min_date: min_date.map(|d| d.format("%Y-%m-%d").to_string()),
        max_date: max_date.map(|d| d.format("%Y-%m-%d").to_string()),
    }
}

fn matches_search(summary: &ConversationSummaryItem, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    let fields = [
        summary.display_title.as_str(),
        summary.title.as_str(),
        summary.conversation_id.as_str(),
        summary.preview.as_str(),
        summary.workspace_name.as_str(),
        summary.workspace_path.as_str(),
        summary.agent_name.as_str(),
        summary.parent_conversation_id.as_str(),
        summary.primary_model_id.as_str(),
        summary.primary_model_display_name.as_str(),
    ];
    if fields
        .iter()
        .any(|f| !f.is_empty() && f.to_lowercase().contains(needle_lower))
    {
        return true;
    }
    summary
        .models
        .iter()
        .chain(summary.model_display_names.iter())
        .any(|m| m.to_lowercase().contains(needle_lower))
}

fn monday_of(date: NaiveDate) -> NaiveDate {
    let diff = date.weekday().num_days_from_monday() as u64;
    date.checked_sub_days(Days::new(diff)).unwrap_or(date)
}

fn last_day_of_month(year: i32, month: u32) -> NaiveDate {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(ny, nm, 1)
        .and_then(|d| d.pred_opt())
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, month, 28).unwrap())
}

fn make_bucket_meta(
    date: NaiveDate,
    granularity: TimeGranularity,
) -> (String, String, String, String) {
    match granularity {
        TimeGranularity::Day => {
            let key = date.format("%Y-%m-%d").to_string();
            let label = format!("{:02}/{:02}", date.day(), date.month());
            (key.clone(), label, key.clone(), key)
        }
        TimeGranularity::Week => {
            let mon = monday_of(date);
            let sun = mon.checked_add_days(Days::new(6)).unwrap_or(mon);
            let key = mon.format("%Y-%m-%d").to_string();
            let label = format!(
                "{:02}/{:02} - {:02}/{:02}",
                mon.day(),
                mon.month(),
                sun.day(),
                sun.month()
            );
            (
                key,
                label,
                mon.format("%Y-%m-%d").to_string(),
                sun.format("%Y-%m-%d").to_string(),
            )
        }
        TimeGranularity::Month => {
            let first = NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date);
            let last = last_day_of_month(date.year(), date.month());
            let key = format!("{:04}-{:02}", date.year(), date.month());
            let label = format!("{:02}/{:04}", date.month(), date.year());
            (
                key,
                label,
                first.format("%Y-%m-%d").to_string(),
                last.format("%Y-%m-%d").to_string(),
            )
        }
    }
}

struct BucketAccumulator {
    point: TimeBucketPoint,
    conv_indices: HashSet<usize>,
}

impl BucketAccumulator {
    fn new(key: String, label: String, start_date: String, end_date: String) -> Self {
        Self {
            point: TimeBucketPoint {
                bucket_key: key,
                label,
                start_date,
                end_date,
                turn_count: 0,
                conversation_count: 0,
                exact: ExactTokenTotals::default(),
                estimated_total_tokens: 0,
                estimated_cost_usd: 0.0,
            },
            conv_indices: HashSet::new(),
        }
    }
}

fn seed_continuous_buckets(
    buckets: &mut BTreeMap<String, BucketAccumulator>,
    span_start: NaiveDate,
    span_end: NaiveDate,
    granularity: TimeGranularity,
) {
    if span_start > span_end {
        return;
    }
    match granularity {
        TimeGranularity::Day => {
            let count = (span_end - span_start).num_days() + 1;
            if count <= 0 || count > MAX_ZERO_FILL_BUCKETS {
                return;
            }
            let mut cur = span_start;
            while cur <= span_end {
                let (k, l, s, e) = make_bucket_meta(cur, TimeGranularity::Day);
                buckets
                    .entry(k.clone())
                    .or_insert_with(|| BucketAccumulator::new(k, l, s, e));
                let Some(next) = cur.checked_add_days(Days::new(1)) else {
                    break;
                };
                cur = next;
            }
        }
        TimeGranularity::Week => {
            let start_mon = monday_of(span_start);
            let end_mon = monday_of(span_end);
            let count = ((end_mon - start_mon).num_days() / 7) + 1;
            if count <= 0 || count > MAX_ZERO_FILL_BUCKETS {
                return;
            }
            let mut cur = start_mon;
            while cur <= end_mon {
                let (k, l, s, e) = make_bucket_meta(cur, TimeGranularity::Week);
                buckets
                    .entry(k.clone())
                    .or_insert_with(|| BucketAccumulator::new(k, l, s, e));
                let Some(next) = cur.checked_add_days(Days::new(7)) else {
                    break;
                };
                cur = next;
            }
        }
        TimeGranularity::Month => {
            let total_months = (span_end.year() as i64 - span_start.year() as i64) * 12
                + (span_end.month() as i64 - span_start.month() as i64)
                + 1;
            if total_months <= 0 || total_months > MAX_ZERO_FILL_BUCKETS {
                return;
            }
            let mut y = span_start.year();
            let mut m = span_start.month();
            while (y, m) <= (span_end.year(), span_end.month()) {
                if let Some(d) = NaiveDate::from_ymd_opt(y, m, 1) {
                    let (k, l, s, e) = make_bucket_meta(d, TimeGranularity::Month);
                    buckets
                        .entry(k.clone())
                        .or_insert_with(|| BucketAccumulator::new(k, l, s, e));
                }
                if m == 12 {
                    y += 1;
                    m = 1;
                } else {
                    m += 1;
                }
            }
        }
    }
}

fn compare_conversations(
    a: &ConversationSummaryItem,
    b: &ConversationSummaryItem,
    sort: ConversationSortField,
    order: SortDirection,
) -> Ordering {
    let primary = match sort {
        ConversationSortField::LastModifiedTime => a.last_modified_time.cmp(&b.last_modified_time),
        ConversationSortField::TotalTokens => a
            .exact
            .total_tokens
            .cmp(&b.exact.total_tokens)
            .then_with(|| a.estimated.total_tokens.cmp(&b.estimated.total_tokens)),
        ConversationSortField::InputTokens => a.exact.input_tokens.cmp(&b.exact.input_tokens),
        ConversationSortField::OutputTokens => a.exact.output_tokens.cmp(&b.exact.output_tokens),
        ConversationSortField::CombinedOutputTokens => (a.exact.output_tokens
            + a.exact.thinking_tokens)
            .cmp(&(b.exact.output_tokens + b.exact.thinking_tokens)),
        ConversationSortField::ThinkingTokens => {
            a.exact.thinking_tokens.cmp(&b.exact.thinking_tokens)
        }
        ConversationSortField::CacheReadTokens => {
            a.exact.cache_read_tokens.cmp(&b.exact.cache_read_tokens)
        }
        ConversationSortField::EstimatedCostUsd => a
            .estimated_cost_usd
            .partial_cmp(&b.estimated_cost_usd)
            .unwrap_or(Ordering::Equal),
        ConversationSortField::StepCount => a.step_count.cmp(&b.step_count),
        ConversationSortField::TurnCount => a.turn_count.cmp(&b.turn_count),
        ConversationSortField::Title => a
            .display_title
            .to_lowercase()
            .cmp(&b.display_title.to_lowercase()),
        ConversationSortField::Kind => a.kind.as_str().cmp(b.kind.as_str()),
        ConversationSortField::Workspace => a
            .workspace_name
            .to_lowercase()
            .cmp(&b.workspace_name.to_lowercase())
            .then_with(|| {
                a.workspace_path
                    .to_lowercase()
                    .cmp(&b.workspace_path.to_lowercase())
            }),
        ConversationSortField::Model => a
            .primary_model_display_name
            .to_lowercase()
            .cmp(&b.primary_model_display_name.to_lowercase())
            .then_with(|| {
                a.primary_model_id
                    .to_lowercase()
                    .cmp(&b.primary_model_id.to_lowercase())
            }),
    };

    let directed = match order {
        SortDirection::Asc => primary,
        SortDirection::Desc => primary.reverse(),
    };

    directed
        .then_with(|| b.last_modified_time.cmp(&a.last_modified_time))
        .then_with(|| a.conversation_id.cmp(&b.conversation_id))
}

pub fn aggregate_dashboard(
    scan_result: &ScanResult,
    query: Option<&DashboardQueryRequest>,
    now: DateTime<Utc>,
) -> AggregatedDashboardBundle {
    let tz_offset_minutes = query
        .and_then(|q| q.tz_offset_minutes)
        .unwrap_or_else(get_default_tz_offset_minutes)
        .clamp(-840, 840);
    let tz = FixedOffset::east_opt(tz_offset_minutes * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());

    let preset = parse_preset(query.and_then(|q| q.preset.as_deref()));
    let granularity = parse_granularity(query.and_then(|q| q.granularity.as_deref()));
    let kind_filter = parse_kind_filter(query.and_then(|q| q.kind.as_deref()));
    let model_filter = query
        .and_then(|q| q.model.as_deref())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let workspace_filter = query
        .and_then(|q| q.workspace.as_deref())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let search_filter = query
        .and_then(|q| q.search.as_deref())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let search_lower = search_filter.to_lowercase();

    let today_local = now.with_timezone(&tz).date_naive();
    let (from_date, to_date) = match preset {
        DatePreset::Today => (Some(today_local), Some(today_local)),
        DatePreset::Days7 => (
            today_local.checked_sub_days(Days::new(6)),
            Some(today_local),
        ),
        DatePreset::Days30 => (
            today_local.checked_sub_days(Days::new(29)),
            Some(today_local),
        ),
        DatePreset::Month => (
            NaiveDate::from_ymd_opt(today_local.year(), today_local.month(), 1),
            Some(today_local),
        ),
        DatePreset::All => (None, None),
        DatePreset::Custom => {
            let mut f = parse_ymd_date(query.and_then(|q| q.from.as_deref()));
            let mut t = parse_ymd_date(query.and_then(|q| q.to.as_deref()));
            if let (Some(fd), Some(td)) = (f, t) {
                if fd > td {
                    f = Some(td);
                    t = Some(fd);
                }
            }
            (f, t)
        }
    };

    let catalog = build_catalog(&scan_result.conversations, tz_offset_minutes, &tz);

    let has_date_or_model_filter =
        from_date.is_some() || to_date.is_some() || !model_filter.is_empty();

    let mut filtered_conversations: Vec<ConversationSummaryItem> =
        Vec::with_capacity(scan_result.conversations.len());
    let mut kpi_exact = ExactTokenTotals::default();
    let mut kpi_est_in = 0u64;
    let mut kpi_est_out = 0u64;
    let mut kpi_est_total = 0u64;
    let mut exact_turns = 0usize;
    let mut estimated_turns = 0usize;
    let mut total_steps = 0usize;
    let mut main_conversations = 0usize;
    let mut swarm_conversations = 0usize;
    let mut subagent_conversations = 0usize;
    let mut desktop_conversations = 0usize;
    let mut cli_conversations = 0usize;
    let mut kpi_cost_usd = 0.0f64;
    let mut kpi_savings_usd = 0.0f64;
    let mut active_day_set: HashSet<NaiveDate> = HashSet::new();
    let mut min_turn_date: Option<NaiveDate> = None;
    let mut max_turn_date: Option<NaiveDate> = None;

    struct DailyHeatmapAcc {
        total_tokens: u64,
        input_tokens: u64,
        output_tokens: u64,
        thinking_tokens: u64,
        cache_tokens: u64,
        turn_count: usize,
        conv_indices: HashSet<usize>,
        cost_usd: f64,
    }
    let mut daily_heatmap_map: BTreeMap<NaiveDate, DailyHeatmapAcc> = BTreeMap::new();
    let heatmap_min_date = today_local.checked_sub_days(Days::new(364));

    struct ModelAgg {
        display_name: String,
        api_provider: String,
        conv_indices: HashSet<usize>,
        turn_count: usize,
        exact_turn_count: usize,
        estimated_turn_count: usize,
        exact: ExactTokenTotals,
        estimated_total_tokens: u64,
    }
    let mut model_aggs: HashMap<String, ModelAgg> = HashMap::with_capacity(32);
    let mut buckets: BTreeMap<String, BucketAccumulator> = BTreeMap::new();
    let mut date_bucket_cache: HashMap<NaiveDate, String> = HashMap::with_capacity(128);
    let mut matching_groups: Vec<(&ResolvedTurnGroup, Option<NaiveDate>)> = Vec::with_capacity(64);

    for (conv_idx, conv) in scan_result.conversations.iter().enumerate() {
        if !kind_filter.is_empty() && conv.summary.kind.as_str() != kind_filter {
            continue;
        }
        if !workspace_filter.is_empty() {
            let conv_ws_key = get_workspace_key_from_path(&conv.summary.workspace_path);
            if conv_ws_key != workspace_filter {
                continue;
            }
        }
        if !matches_search(&conv.summary, &search_lower) {
            continue;
        }

        if conv.turn_groups.is_empty() {
            if !model_filter.is_empty()
                && conv.summary.primary_model_id != model_filter
                && !conv.summary.models.iter().any(|m| m == &model_filter)
            {
                continue;
            }
            if from_date.is_some() || to_date.is_some() {
                let Some(conv_d) = conv_fallback_local_date(conv, tz_offset_minutes, &tz) else {
                    continue;
                };
                if from_date.is_some_and(|f| conv_d < f) || to_date.is_some_and(|t| conv_d > t) {
                    continue;
                }
            }
            match conv.summary.kind {
                ConversationKind::Main => main_conversations += 1,
                ConversationKind::Swarm => swarm_conversations += 1,
                ConversationKind::Subagent => subagent_conversations += 1,
            }
            match conv.summary.client_source {
                ClientSourceKind::Desktop => desktop_conversations += 1,
                ClientSourceKind::Cli => cli_conversations += 1,
            }
            total_steps = total_steps.saturating_add(conv.summary.step_count);
            filtered_conversations.push(conv.summary.clone());
            continue;
        }

        // Accumulate into 365-day calendar heatmap (respects kind / workspace / search / model filters, independent of preset range)
        for g in &conv.turn_groups {
            if !model_filter.is_empty() && g.model_id != model_filter {
                continue;
            }
            let Some(d) = group_local_date(g, conv, tz_offset_minutes, &tz) else {
                continue;
            };
            if heatmap_min_date.is_some_and(|min_d| d < min_d) || d > today_local {
                continue;
            }
            let (g_cost, _) = if g.exact_turn_count > 0 {
                estimate_exact_cost_usd(&g.model_id, &g.exact)
            } else {
                (0.0, 0.0)
            };
            let acc = daily_heatmap_map.entry(d).or_insert_with(|| DailyHeatmapAcc {
                total_tokens: 0,
                input_tokens: 0,
                output_tokens: 0,
                thinking_tokens: 0,
                cache_tokens: 0,
                turn_count: 0,
                conv_indices: HashSet::new(),
                cost_usd: 0.0,
            });
            acc.turn_count += g.exact_turn_count + g.estimated_turn_count;
            acc.conv_indices.insert(conv_idx);
            if g.exact_turn_count > 0 {
                acc.total_tokens = acc.total_tokens.saturating_add(g.exact.total_tokens);
                acc.input_tokens = acc.input_tokens.saturating_add(g.exact.input_tokens);
                acc.output_tokens = acc.output_tokens.saturating_add(g.exact.output_tokens);
                acc.thinking_tokens = acc.thinking_tokens.saturating_add(g.exact.thinking_tokens);
                acc.cache_tokens = acc
                    .cache_tokens
                    .saturating_add(g.exact.cache_read_tokens + g.exact.cache_write_tokens);
                acc.cost_usd += g_cost;
            } else {
                acc.total_tokens = acc.total_tokens.saturating_add(g.estimated.total_tokens);
            }
        }

        matching_groups.clear();
        for g in &conv.turn_groups {
            if !model_filter.is_empty() && g.model_id != model_filter {
                continue;
            }
            let d_opt = group_local_date(g, conv, tz_offset_minutes, &tz);
            if from_date.is_some() || to_date.is_some() {
                let Some(d) = d_opt else {
                    continue;
                };
                if from_date.is_some_and(|f| d < f) || to_date.is_some_and(|to| d > to) {
                    continue;
                }
            }
            matching_groups.push((g, d_opt));
        }

        if matching_groups.is_empty() {
            continue;
        }

        let mut item = conv.summary.clone();
        if has_date_or_model_filter && matching_groups.len() != conv.turn_groups.len() {
            let mut c_exact = ExactTokenTotals::default();
            let mut c_est_in = 0u64;
            let mut c_est_out = 0u64;
            let mut c_est_tot = 0u64;
            let mut c_exact_turns = 0usize;
            let mut c_est_turns = 0usize;
            let mut c_cost_usd = 0.0f64;
            let mut c_savings_usd = 0.0f64;
            let mut per_model_tok: HashMap<String, (String, u64, usize)> = HashMap::new();
            let mut latest_ms: Option<i64> = None;

            for &(g, _) in &matching_groups {
                if let Some(ms) = g.max_ts_ms {
                    latest_ms = Some(latest_ms.map_or(ms, |p| p.max(ms)));
                }
                let g_turns = g.exact_turn_count + g.estimated_turn_count;
                let pm = per_model_tok
                    .entry(g.model_id.clone())
                    .or_insert_with(|| (g.model_display_name.clone(), 0, 0));
                pm.2 += g_turns;
                if g.exact_turn_count > 0 {
                    c_exact_turns += g.exact_turn_count;
                    c_exact.add_assign(&g.exact);
                    pm.1 = pm.1.saturating_add(g.exact.total_tokens);
                    let (gc, gs) = estimate_exact_cost_usd(&g.model_id, &g.exact);
                    c_cost_usd += gc;
                    c_savings_usd += gs;
                }
                if g.estimated_turn_count > 0 {
                    c_est_turns += g.estimated_turn_count;
                    c_est_in = c_est_in.saturating_add(g.estimated.input_tokens);
                    c_est_out = c_est_out.saturating_add(g.estimated.output_tokens);
                    c_est_tot = c_est_tot.saturating_add(g.estimated.total_tokens);
                }
            }

            let mut sorted_m: Vec<(String, String, u64, usize)> = per_model_tok
                .into_iter()
                .map(|(id, (disp, tok, tc))| (id, disp, tok, tc))
                .collect();
            sorted_m.sort_by(|a, b| {
                b.2.cmp(&a.2)
                    .then_with(|| b.3.cmp(&a.3))
                    .then_with(|| a.0.cmp(&b.0))
            });

            let c_total_turns = c_exact_turns + c_est_turns;
            item.turn_count = c_total_turns;
            item.step_count = c_total_turns;
            item.exact_turn_count = c_exact_turns;
            item.estimated_turn_count = c_est_turns;
            item.exact = c_exact;
            item.estimated_cost_usd = round_cost(c_cost_usd);
            item.cache_savings_usd = round_cost(c_savings_usd);
            item.estimated = EstimatedTokenTotals {
                input_tokens: c_est_in,
                output_tokens: c_est_out,
                total_tokens: c_est_tot,
                estimated_turn_count: c_est_turns,
            };
            if let Some(first) = sorted_m.first() {
                item.primary_model_id = first.0.clone();
                item.primary_model_display_name = first.1.clone();
                item.models = sorted_m.iter().map(|x| x.0.clone()).collect();
                item.model_display_names = sorted_m.iter().map(|x| x.1.clone()).collect();
            }
            if let Some(ts) = latest_ms.and_then(format_utc_ms) {
                item.last_modified_time = ts;
            }
        }

        match item.kind {
            ConversationKind::Main => main_conversations += 1,
            ConversationKind::Swarm => swarm_conversations += 1,
            ConversationKind::Subagent => subagent_conversations += 1,
        }
        match item.client_source {
            ClientSourceKind::Desktop => desktop_conversations += 1,
            ClientSourceKind::Cli => cli_conversations += 1,
        }
        total_steps = total_steps.saturating_add(item.step_count);

        for &(g, d_opt) in &matching_groups {
            let g_turns = g.exact_turn_count + g.estimated_turn_count;
            let (g_cost_usd, g_savings_usd) = if g.exact_turn_count > 0 {
                estimate_exact_cost_usd(&g.model_id, &g.exact)
            } else {
                (0.0, 0.0)
            };
            kpi_cost_usd += g_cost_usd;
            kpi_savings_usd += g_savings_usd;
            let m_agg = if let Some(existing) = model_aggs.get_mut(g.model_id.as_str()) {
                existing
            } else {
                model_aggs
                    .entry(g.model_id.clone())
                    .or_insert_with(|| ModelAgg {
                        display_name: g.model_display_name.clone(),
                        api_provider: g.api_provider.clone(),
                        conv_indices: HashSet::new(),
                        turn_count: 0,
                        exact_turn_count: 0,
                        estimated_turn_count: 0,
                        exact: ExactTokenTotals::default(),
                        estimated_total_tokens: 0,
                    })
            };
            m_agg.conv_indices.insert(conv_idx);
            m_agg.turn_count += g_turns;

            if g.exact_turn_count > 0 {
                exact_turns += g.exact_turn_count;
                kpi_exact.add_assign(&g.exact);
                m_agg.exact_turn_count += g.exact_turn_count;
                m_agg.exact.add_assign(&g.exact);
            }
            if g.estimated_turn_count > 0 {
                estimated_turns += g.estimated_turn_count;
                kpi_est_in = kpi_est_in.saturating_add(g.estimated.input_tokens);
                kpi_est_out = kpi_est_out.saturating_add(g.estimated.output_tokens);
                kpi_est_total = kpi_est_total.saturating_add(g.estimated.total_tokens);
                m_agg.estimated_turn_count += g.estimated_turn_count;
                m_agg.estimated_total_tokens = m_agg
                    .estimated_total_tokens
                    .saturating_add(g.estimated.total_tokens);
            }

            if let Some(d) = d_opt {
                active_day_set.insert(d);
                min_turn_date = Some(min_turn_date.map_or(d, |cur| cur.min(d)));
                max_turn_date = Some(max_turn_date.map_or(d, |cur| cur.max(d)));

                let bucket_key = if let Some(k) = date_bucket_cache.get(&d) {
                    k.as_str()
                } else {
                    let (b_key, b_label, b_start, b_end) = make_bucket_meta(d, granularity);
                    buckets.entry(b_key.clone()).or_insert_with(|| {
                        BucketAccumulator::new(b_key.clone(), b_label, b_start, b_end)
                    });
                    date_bucket_cache.entry(d).or_insert(b_key).as_str()
                };

                if let Some(bucket) = buckets.get_mut(bucket_key) {
                    bucket.point.turn_count += g_turns;
                    bucket.conv_indices.insert(conv_idx);
                    if g.exact_turn_count > 0 {
                        bucket.point.exact.add_assign(&g.exact);
                        bucket.point.estimated_cost_usd =
                            round_cost(bucket.point.estimated_cost_usd + g_cost_usd);
                    }
                    if g.estimated_turn_count > 0 {
                        bucket.point.estimated_total_tokens = bucket
                            .point
                            .estimated_total_tokens
                            .saturating_add(g.estimated.total_tokens);
                    }
                }
            }
        }

        filtered_conversations.push(item);
    }

    let span_bounds: Option<(NaiveDate, NaiveDate)> = match (from_date, to_date) {
        (Some(f), Some(t))
            if !filtered_conversations.is_empty() || preset != DatePreset::Custom =>
        {
            Some((f, t))
        }
        (Some(f), None) => max_turn_date.or(Some(today_local)).map(|t| (f, f.max(t))),
        (None, Some(t)) => min_turn_date.map(|f| (f.min(t), t)),
        (None, None) => min_turn_date.zip(max_turn_date),
        _ => None,
    };
    if let Some((s_start, s_end)) = span_bounds {
        if !filtered_conversations.is_empty()
            || matches!(
                preset,
                DatePreset::Today | DatePreset::Days7 | DatePreset::Days30 | DatePreset::Month
            )
        {
            let mut seeded = BTreeMap::new();
            seed_continuous_buckets(&mut seeded, s_start, s_end, granularity);
            for (k, v) in buckets {
                seeded.insert(k, v);
            }
            buckets = seeded;
        }
    }

    let time_series: Vec<TimeBucketPoint> = buckets
        .into_values()
        .map(|mut b| {
            b.point.conversation_count = b.conv_indices.len();
            b.point
        })
        .collect();

    let mut by_model: Vec<ModelUsageSummary> = model_aggs
        .into_iter()
        .map(|(model_id, m)| {
            let (cost_usd, savings_usd) = estimate_exact_cost_usd(&model_id, &m.exact);
            ModelUsageSummary {
                model_id,
                model_display_name: m.display_name,
                api_provider: m.api_provider,
                conversation_count: m.conv_indices.len(),
                turn_count: m.turn_count,
                exact_turn_count: m.exact_turn_count,
                estimated_turn_count: m.estimated_turn_count,
                share_percent: compute_share_percent(
                    m.exact.total_tokens,
                    kpi_exact.total_tokens,
                    m.estimated_total_tokens,
                    kpi_est_total,
                ),
                exact: m.exact,
                estimated_total_tokens: m.estimated_total_tokens,
                estimated_cost_usd: cost_usd,
                cache_savings_usd: savings_usd,
            }
        })
        .collect();
    by_model.sort_by(|a, b| {
        b.exact
            .total_tokens
            .cmp(&a.exact.total_tokens)
            .then_with(|| b.estimated_total_tokens.cmp(&a.estimated_total_tokens))
            .then_with(|| b.turn_count.cmp(&a.turn_count))
            .then_with(|| a.model_id.cmp(&b.model_id))
    });

    let mut ws_aggs: HashMap<String, WorkspaceUsageSummary> = HashMap::new();
    let mut kind_aggs: HashMap<ConversationKind, KindUsageSummary> = HashMap::new();

    if !filtered_conversations.is_empty() {
        for k in [
            ConversationKind::Main,
            ConversationKind::Swarm,
            ConversationKind::Subagent,
        ] {
            if kind_filter.is_empty() || k.as_str() == kind_filter {
                kind_aggs.insert(
                    k,
                    KindUsageSummary {
                        kind: k,
                        conversation_count: 0,
                        turn_count: 0,
                        exact: ExactTokenTotals::default(),
                        estimated_total_tokens: 0,
                        share_percent: 0.0,
                        estimated_cost_usd: 0.0,
                    },
                );
            }
        }
    }

    for item in &filtered_conversations {
        let ws_key = get_workspace_key_from_path(&item.workspace_path);
        let ws_entry = ws_aggs
            .entry(ws_key.clone())
            .or_insert_with(|| WorkspaceUsageSummary {
                workspace_key: ws_key,
                workspace_path: item.workspace_path.clone(),
                workspace_name: item.workspace_name.clone(),
                conversation_count: 0,
                turn_count: 0,
                exact: ExactTokenTotals::default(),
                estimated_total_tokens: 0,
                share_percent: 0.0,
                estimated_cost_usd: 0.0,
                last_active_time: None,
            });
        ws_entry.conversation_count += 1;
        ws_entry.turn_count += item.turn_count;
        ws_entry.exact.add_assign(&item.exact);
        ws_entry.estimated_cost_usd =
            round_cost(ws_entry.estimated_cost_usd + item.estimated_cost_usd);
        ws_entry.estimated_total_tokens = ws_entry
            .estimated_total_tokens
            .saturating_add(item.estimated.total_tokens);
        if !item.last_modified_time.is_empty()
            && ws_entry
                .last_active_time
                .as_ref()
                .is_none_or(|prev| &item.last_modified_time > prev)
        {
            ws_entry.last_active_time = Some(item.last_modified_time.clone());
        }

        let k_entry = kind_aggs
            .entry(item.kind)
            .or_insert_with(|| KindUsageSummary {
                kind: item.kind,
                conversation_count: 0,
                turn_count: 0,
                exact: ExactTokenTotals::default(),
                estimated_total_tokens: 0,
                share_percent: 0.0,
                estimated_cost_usd: 0.0,
            });
        k_entry.conversation_count += 1;
        k_entry.turn_count += item.turn_count;
        k_entry.exact.add_assign(&item.exact);
        k_entry.estimated_cost_usd =
            round_cost(k_entry.estimated_cost_usd + item.estimated_cost_usd);
        k_entry.estimated_total_tokens = k_entry
            .estimated_total_tokens
            .saturating_add(item.estimated.total_tokens);
    }

    let mut by_workspace: Vec<WorkspaceUsageSummary> = ws_aggs
        .into_values()
        .map(|mut w| {
            w.share_percent = compute_share_percent(
                w.exact.total_tokens,
                kpi_exact.total_tokens,
                w.estimated_total_tokens,
                kpi_est_total,
            );
            w
        })
        .collect();
    by_workspace.sort_by(|a, b| {
        b.exact
            .total_tokens
            .cmp(&a.exact.total_tokens)
            .then_with(|| b.estimated_total_tokens.cmp(&a.estimated_total_tokens))
            .then_with(|| b.conversation_count.cmp(&a.conversation_count))
            .then_with(|| a.workspace_name.cmp(&b.workspace_name))
    });

    let kind_rank = |k: ConversationKind| -> u8 {
        match k {
            ConversationKind::Main => 0,
            ConversationKind::Swarm => 1,
            ConversationKind::Subagent => 2,
        }
    };
    let mut by_kind: Vec<KindUsageSummary> = kind_aggs
        .into_values()
        .map(|mut k| {
            k.share_percent = compute_share_percent(
                k.exact.total_tokens,
                kpi_exact.total_tokens,
                k.estimated_total_tokens,
                kpi_est_total,
            );
            k
        })
        .collect();
    by_kind.sort_by(|a, b| {
        b.exact
            .total_tokens
            .cmp(&a.exact.total_tokens)
            .then_with(|| b.conversation_count.cmp(&a.conversation_count))
            .then_with(|| kind_rank(a.kind).cmp(&kind_rank(b.kind)))
    });

    let total_turns = exact_turns + estimated_turns;
    let exact_coverage_percent = if total_turns == 0 || estimated_turns == 0 {
        100.0
    } else if exact_turns == 0 {
        0.0
    } else {
        round2((exact_turns as f64 / total_turns as f64) * 100.0).clamp(0.01, 99.99)
    };

    let active_days = active_day_set.len();
    let total_conversations = filtered_conversations.len();
    let avg_exact_tokens_per_active_day = if active_days > 0 {
        (kpi_exact.total_tokens as f64 / active_days as f64).round() as u64
    } else {
        0
    };
    let avg_exact_tokens_per_conversation = if total_conversations > 0 {
        (kpi_exact.total_tokens as f64 / total_conversations as f64).round() as u64
    } else {
        0
    };

    let sort = parse_sort_field(query.and_then(|q| q.sort.as_deref()));
    let order = parse_sort_direction(query.and_then(|q| q.order.as_deref()));
    filtered_conversations.sort_by(|a, b| compare_conversations(a, b, sort, order));

    let filters = AppliedFilters {
        preset,
        from: from_date.map(|d| d.format("%Y-%m-%d").to_string()),
        to: to_date.map(|d| d.format("%Y-%m-%d").to_string()),
        granularity,
        model: model_filter,
        workspace: workspace_filter,
        kind: kind_filter,
        search: search_filter,
        tz_offset_minutes,
    };

    let max_daily_tok = daily_heatmap_map
        .values()
        .map(|a| a.total_tokens)
        .max()
        .unwrap_or(0);
    let heatmap: Vec<HeatmapDayPoint> = daily_heatmap_map
        .into_iter()
        .map(|(d, acc)| {
            let level = if acc.total_tokens == 0 || max_daily_tok == 0 {
                0
            } else {
                let ratio = (acc.total_tokens as f64) / (max_daily_tok as f64);
                if ratio >= 0.75 {
                    4
                } else if ratio >= 0.45 {
                    3
                } else if ratio >= 0.20 {
                    2
                } else {
                    1
                }
            };
            HeatmapDayPoint {
                date: d.format("%Y-%m-%d").to_string(),
                total_tokens: acc.total_tokens,
                input_tokens: acc.input_tokens,
                output_tokens: acc.output_tokens,
                thinking_tokens: acc.thinking_tokens,
                cache_tokens: acc.cache_tokens,
                turn_count: acc.turn_count,
                conversation_count: acc.conv_indices.len(),
                estimated_cost_usd: round_cost(acc.cost_usd),
                level,
            }
        })
        .collect();

    let kpi = KpiSummary {
        exact: kpi_exact,
        estimated: EstimatedTokenTotals {
            input_tokens: kpi_est_in,
            output_tokens: kpi_est_out,
            total_tokens: kpi_est_total,
            estimated_turn_count: estimated_turns,
        },
        total_conversations,
        main_conversations,
        swarm_conversations,
        subagent_conversations,
        desktop_conversations,
        cli_conversations,
        total_steps,
        total_turns,
        exact_turns,
        estimated_turns,
        exact_coverage_percent,
        active_days,
        avg_exact_tokens_per_active_day,
        avg_exact_tokens_per_conversation,
        estimated_cost_usd: round_cost(kpi_cost_usd),
        cache_savings_usd: round_cost(kpi_savings_usd),
    };

    AggregatedDashboardBundle {
        summary: DashboardSummaryResponse {
            scan: scan_result.scan.clone(),
            filters,
            catalog,
            kpi,
            time_series,
            heatmap,
            by_model,
            by_workspace,
            by_kind,
        },
        filtered_conversations,
    }
}

pub fn query_conversation_list(
    filtered_conversations: &[ConversationSummaryItem],
    query: Option<&DashboardQueryRequest>,
) -> ConversationListResponse {
    let sort = parse_sort_field(query.and_then(|q| q.sort.as_deref()));
    let order = parse_sort_direction(query.and_then(|q| q.order.as_deref()));
    let page_size = query.and_then(|q| q.page_size).unwrap_or(20).clamp(1, 200);

    let mut sorted = filtered_conversations.to_vec();
    sorted.sort_by(|a, b| compare_conversations(a, b, sort, order));

    let total_items = sorted.len();
    let total_pages = if total_items == 0 {
        1
    } else {
        total_items.div_ceil(page_size)
    };
    let page = query
        .and_then(|q| q.page)
        .unwrap_or(1)
        .max(1)
        .min(total_pages);

    let start = (page - 1) * page_size;
    let end = (start + page_size).min(total_items);
    let items = if start < total_items {
        sorted[start..end].to_vec()
    } else {
        Vec::new()
    };

    ConversationListResponse {
        items,
        total_items,
        page,
        page_size,
        total_pages,
        sort,
        order,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::antigravity_token_usage::scanner::parse_timestamp_ms;
    use crate::modules::antigravity_token_usage::types::{
        ConversationModelSlice, DataSourceKind, ScanMetadata, TurnEstimatedTotals,
    };
    use std::path::PathBuf;

    #[test]
    fn supports_month_and_7d_presets_accurately() {
        let exact = ExactTokenTotals {
            input_tokens: 100,
            output_tokens: 50,
            thinking_tokens: 25,
            cache_read_tokens: 200,
            cache_write_tokens: 0,
            total_tokens: 375,
        };
        let ts_ms = parse_timestamp_ms("2026-11-02T10:00:00.000Z").unwrap();
        let conv = ScannedConversation {
            summary: ConversationSummaryItem {
                conversation_id: "c1".to_string(),
                title: "Test".to_string(),
                display_title: "Test".to_string(),
                preview: String::new(),
                kind: ConversationKind::Main,
                client_source: ClientSourceKind::Desktop,
                has_swarm_children: false,
                agent_name: String::new(),
                parent_conversation_id: String::new(),
                parent_title: String::new(),
                nesting_depth: 0,
                workspace_uri: String::new(),
                workspace_path: "E:\\proj".to_string(),
                workspace_name: "proj".to_string(),
                project_id: String::new(),
                status: String::new(),
                created_at: Some("2026-11-02T10:00:00.000Z".to_string()),
                last_modified_time: "2026-11-02T10:00:00.000Z".to_string(),
                step_count: 1,
                turn_count: 1,
                exact_turn_count: 1,
                estimated_turn_count: 0,
                primary_model_id: "gemini-4-carbon-high".to_string(),
                primary_model_display_name: "Gemini 4 Carbon (High)".to_string(),
                models: vec!["gemini-4-carbon-high".to_string()],
                model_display_names: vec!["Gemini 4 Carbon (High)".to_string()],
                data_source: DataSourceKind::SqliteDb,
                exact,
                estimated: EstimatedTokenTotals::default(),
                estimated_cost_usd: 0.0,
                cache_savings_usd: 0.0,
            },
            last_modified_ms: Some(ts_ms),
            created_at_ms: Some(ts_ms),
            model_hint: None,
            model_breakdown: vec![ConversationModelSlice {
                model_id: "gemini-4-carbon-high".to_string(),
                model_display_name: "Gemini 4 Carbon (High)".to_string(),
                api_provider: "Google Gemini".to_string(),
                turn_count: 1,
                exact,
                estimated_total_tokens: 0,
                estimated_cost_usd: 0.0,
                cache_savings_usd: 0.0,
            }],
            turn_groups: vec![ResolvedTurnGroup {
                minute_slot: Some(ts_ms.div_euclid(60_000)),
                max_ts_ms: Some(ts_ms),
                model_id: "gemini-4-carbon-high".to_string(),
                model_display_name: "Gemini 4 Carbon (High)".to_string(),
                api_provider: "Google Gemini".to_string(),
                exact_turn_count: 1,
                estimated_turn_count: 0,
                exact,
                estimated: TurnEstimatedTotals::default(),
            }],
            children: Vec::new(),
        };

        let mut index_by_id = HashMap::new();
        index_by_id.insert("c1".to_string(), 0);
        let scan = ScanResult {
            scan: ScanMetadata {
                data_dir: "/tmp".to_string(),
                data_dir_exists: true,
                scanned_at: "2026-11-15T12:00:00.000Z".to_string(),
                scan_duration_ms: 1,
                total_db_files: 1,
                cache_hits: 0,
                cache_misses: 1,
                rpc_cache_sessions_found: 0,
                brain_dirs_found: 0,
                cli_sessions_found: 0,
                warnings: Vec::new(),
            },
            data_dir: PathBuf::from("/tmp"),
            cli_data_dir: None,
            dynamic_enum_map: HashMap::new(),
            conversations: vec![conv],
            index_by_id,
        };

        let now = DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let month_bundle = aggregate_dashboard(
            &scan,
            Some(&DashboardQueryRequest {
                preset: Some("month".to_string()),
                tz_offset_minutes: Some(0),
                ..Default::default()
            }),
            now,
        );
        assert_eq!(month_bundle.summary.kpi.exact.total_tokens, 375);
        assert_eq!(month_bundle.summary.time_series.len(), 15);

        let seven_d_bundle = aggregate_dashboard(
            &scan,
            Some(&DashboardQueryRequest {
                preset: Some("7d".to_string()),
                tz_offset_minutes: Some(0),
                ..Default::default()
            }),
            now,
        );
        assert_eq!(seven_d_bundle.summary.kpi.exact.total_tokens, 0);
        assert_eq!(seven_d_bundle.summary.time_series.len(), 7);
    }
}
