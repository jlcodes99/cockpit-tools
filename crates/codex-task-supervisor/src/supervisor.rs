use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_EVIDENCE_MESSAGE_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexEventSource {
    AppServer,
    ExecJson,
    RolloutJsonl,
    Proxy,
}

impl CodexEventSource {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "app_server" | "app-server" | "appserver" => Ok(Self::AppServer),
            "exec_json" | "exec-json" | "exec" => Ok(Self::ExecJson),
            "rollout_jsonl" | "rollout-jsonl" | "rollout" | "jsonl" => Ok(Self::RolloutJsonl),
            "proxy" | "gateway" => Ok(Self::Proxy),
            other => Err(format!("unsupported Codex event source: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexEvidenceKind {
    Activity,
    TurnStarted,
    QuotaWarning,
    TurnCompleted,
    TurnFailed,
    TurnInterrupted,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexEvidenceConfidence {
    Informational,
    Suspected,
    Confirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexFailureClass {
    QuotaExhausted,
    Authentication,
    Network,
    ContextWindow,
    ModelCapacity,
    UserInterrupted,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTaskEvidence {
    pub id: String,
    pub observed_at: i64,
    pub source: CodexEventSource,
    pub kind: CodexEvidenceKind,
    pub confidence: CodexEvidenceConfidence,
    pub terminal: bool,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub raw_event_type: Option<String>,
    pub error_code: Option<String>,
    pub failure_class: Option<CodexFailureClass>,
    pub message: Option<String>,
}

impl CodexTaskEvidence {
    fn new(source: CodexEventSource) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            observed_at: chrono::Utc::now().timestamp_millis(),
            source,
            kind: CodexEvidenceKind::Unknown,
            confidence: CodexEvidenceConfidence::Informational,
            terminal: false,
            thread_id: None,
            turn_id: None,
            raw_event_type: None,
            error_code: None,
            failure_class: None,
            message: None,
        }
    }

    pub fn confirms_quota_exhaustion(&self) -> bool {
        self.terminal
            && self.confidence == CodexEvidenceConfidence::Confirmed
            && self.failure_class == Some(CodexFailureClass::QuotaExhausted)
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        let id = id.into();
        if !id.trim().is_empty() {
            self.id = id;
        }
        self
    }

    pub fn activity(thread_id: Option<String>, event_type: impl Into<String>) -> Self {
        let mut evidence = Self::new(CodexEventSource::ExecJson);
        evidence.kind = CodexEvidenceKind::Activity;
        evidence.thread_id = thread_id;
        evidence.raw_event_type = Some(event_type.into());
        evidence
    }

    pub fn confirmed_exec_exit(previous: Option<&Self>, exit_code: Option<i32>) -> Self {
        let mut evidence = previous
            .cloned()
            .unwrap_or_else(|| Self::new(CodexEventSource::ExecJson));
        evidence.id = uuid::Uuid::new_v4().to_string();
        evidence.observed_at = chrono::Utc::now().timestamp_millis();
        evidence.source = CodexEventSource::ExecJson;
        evidence.kind = CodexEvidenceKind::TurnFailed;
        evidence.confidence = CodexEvidenceConfidence::Confirmed;
        evidence.terminal = true;
        evidence.raw_event_type = Some("process.exited".to_string());
        evidence
            .failure_class
            .get_or_insert(CodexFailureClass::Other);
        if evidence.message.is_none() {
            evidence.message = Some(match exit_code {
                Some(code) => format!("Codex exec exited with code {code}"),
                None => "Codex exec exited without a status code".to_string(),
            });
        }
        evidence
    }

    pub fn confirmed_exec_completion(thread_id: Option<String>) -> Self {
        let mut evidence = Self::new(CodexEventSource::ExecJson);
        evidence.kind = CodexEvidenceKind::TurnCompleted;
        evidence.confidence = CodexEvidenceConfidence::Confirmed;
        evidence.terminal = true;
        evidence.thread_id = thread_id;
        evidence.raw_event_type = Some("process.completed".to_string());
        evidence
    }

    pub fn confirmed_user_interruption(thread_id: Option<String>) -> Self {
        let mut evidence = Self::new(CodexEventSource::ExecJson);
        evidence.kind = CodexEvidenceKind::TurnInterrupted;
        evidence.confidence = CodexEvidenceConfidence::Confirmed;
        evidence.terminal = true;
        evidence.thread_id = thread_id;
        evidence.raw_event_type = Some("process.cancelled".to_string());
        evidence.failure_class = Some(CodexFailureClass::UserInterrupted);
        evidence.message = Some("managed Codex task was cancelled by the user".to_string());
        evidence
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedCodexTaskStatus {
    Queued,
    Preparing,
    Running,
    Draining,
    Switching,
    Resuming,
    Completed,
    Failed,
    Cancelled,
    NeedsAttention,
}

impl ManagedCodexTaskStatus {
    pub fn is_final(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn is_terminal(self) -> bool {
        self.is_final() || self == Self::NeedsAttention
    }

    pub fn is_runtime_active(self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Running | Self::Draining | Self::Switching | Self::Resuming
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManagedCodexAccountScope {
    CockpitPool,
    Selected {
        #[serde(rename = "accountIds")]
        account_ids: Vec<String>,
    },
}

impl ManagedCodexAccountScope {
    pub fn selected_account_ids(&self) -> Option<&[String]> {
        match self {
            Self::CockpitPool => None,
            Self::Selected { account_ids } => Some(account_ids),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedCodexTaskConfig {
    pub objective: String,
    pub cwd: String,
    pub account_scope: ManagedCodexAccountScope,
    pub initial_account_id: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub max_switches: Option<u32>,
}

impl ManagedCodexTaskConfig {
    pub fn normalized(mut self) -> Result<Self, String> {
        self.objective = self.objective.trim().to_string();
        if self.objective.is_empty() {
            return Err("managed Codex task objective cannot be empty".to_string());
        }
        self.cwd = self.cwd.trim().to_string();
        if self.cwd.is_empty() {
            return Err("managed Codex task cwd cannot be empty".to_string());
        }
        self.initial_account_id = normalize_optional_string(self.initial_account_id);
        self.model = normalize_optional_string(self.model);
        self.reasoning_effort = normalize_optional_string(self.reasoning_effort);

        if let ManagedCodexAccountScope::Selected { account_ids } = &mut self.account_scope {
            let mut seen = HashSet::new();
            *account_ids = account_ids
                .drain(..)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .filter(|value| seen.insert(value.clone()))
                .collect();
            if account_ids.is_empty() {
                return Err("selected managed Codex task scope cannot be empty".to_string());
            }
            if let Some(initial_account_id) = self.initial_account_id.as_ref() {
                if !account_ids.iter().any(|value| value == initial_account_id) {
                    return Err(
                        "initial Codex account is outside the selected task scope".to_string()
                    );
                }
            }
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedCodexTask {
    pub id: String,
    pub config: ManagedCodexTaskConfig,
    pub status: ManagedCodexTaskStatus,
    #[serde(default)]
    pub queue_position: Option<u32>,
    pub active_account_id: Option<String>,
    pub pending_account_id: Option<String>,
    pub attempted_account_ids: Vec<String>,
    pub thread_id: Option<String>,
    pub active_turn_id: Option<String>,
    pub switch_count: u32,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub updated_at: i64,
    pub last_activity_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_failure_class: Option<CodexFailureClass>,
    pub needs_attention_reason: Option<String>,
    #[serde(default)]
    pub run_generation: u64,
    #[serde(default)]
    pub process_id: Option<u32>,
    #[serde(default)]
    pub process_started_at: Option<i64>,
    #[serde(default)]
    pub executable_path: Option<String>,
    #[serde(default)]
    pub last_event_seq: u64,
    #[serde(default)]
    pub recovery_attempts: u32,
    #[serde(default)]
    processed_evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodexSupervisorAction {
    WaitForTerminal {
        reason: String,
    },
    RequestAccountSelection {
        failed_account_id: Option<String>,
        excluded_account_ids: Vec<String>,
    },
    ResumeThread {
        thread_id: String,
        prompt: String,
    },
    MarkCompleted,
    Stop {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSupervisorDecision {
    pub task: ManagedCodexTask,
    pub evidence: Option<CodexTaskEvidence>,
    pub actions: Vec<CodexSupervisorAction>,
}

impl ManagedCodexTask {
    pub fn create(config: ManagedCodexTaskConfig) -> Result<Self, String> {
        let config = config.normalized()?;
        let now = chrono::Utc::now().timestamp_millis();
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            config,
            status: ManagedCodexTaskStatus::Queued,
            queue_position: None,
            active_account_id: None,
            pending_account_id: None,
            attempted_account_ids: Vec::new(),
            thread_id: None,
            active_turn_id: None,
            switch_count: 0,
            created_at: now,
            started_at: None,
            updated_at: now,
            last_activity_at: None,
            completed_at: None,
            last_error: None,
            last_failure_class: None,
            needs_attention_reason: None,
            run_generation: 0,
            process_id: None,
            process_started_at: None,
            executable_path: None,
            last_event_seq: 0,
            recovery_attempts: 0,
            processed_evidence_ids: Vec::new(),
        })
    }

    pub fn objective(&self) -> &str {
        &self.config.objective
    }

    pub fn cwd(&self) -> &str {
        &self.config.cwd
    }

    pub fn mark_preparing(&mut self, account_id: &str) -> Result<(), String> {
        if !matches!(
            self.status,
            ManagedCodexTaskStatus::Queued | ManagedCodexTaskStatus::NeedsAttention
        ) {
            return Err(format!(
                "cannot prepare a task while it is {:?}",
                self.status
            ));
        }
        let account_id = account_id.trim();
        if account_id.is_empty() {
            return Err("managed Codex task account id cannot be empty".to_string());
        }
        self.active_account_id = Some(account_id.to_string());
        if !self
            .attempted_account_ids
            .iter()
            .any(|value| value == account_id)
        {
            self.attempted_account_ids.push(account_id.to_string());
        }
        self.status = ManagedCodexTaskStatus::Preparing;
        self.queue_position = None;
        self.needs_attention_reason = None;
        self.started_at
            .get_or_insert_with(|| chrono::Utc::now().timestamp_millis());
        self.updated_at = chrono::Utc::now().timestamp_millis();
        Ok(())
    }

    pub fn mark_process_started(&mut self, process_id: u32, executable_path: String) {
        let now = chrono::Utc::now().timestamp_millis();
        self.run_generation = self.run_generation.saturating_add(1);
        self.process_id = Some(process_id);
        self.process_started_at = Some(now);
        self.executable_path = normalize_optional_string(Some(executable_path));
        self.started_at.get_or_insert(now);
        self.updated_at = now;
    }

    pub fn clear_process(&mut self) {
        self.process_id = None;
        self.process_started_at = None;
        self.executable_path = None;
        self.updated_at = chrono::Utc::now().timestamp_millis();
    }

    pub fn apply_evidence(&mut self, evidence: &CodexTaskEvidence) -> Vec<CodexSupervisorAction> {
        if self
            .processed_evidence_ids
            .iter()
            .any(|value| value == &evidence.id)
        {
            return Vec::new();
        }
        self.processed_evidence_ids.push(evidence.id.clone());
        if self.processed_evidence_ids.len() > 256 {
            let remove_count = self.processed_evidence_ids.len() - 256;
            self.processed_evidence_ids.drain(0..remove_count);
        }
        if self.status.is_terminal() {
            return Vec::new();
        }

        self.updated_at = evidence.observed_at;
        self.last_activity_at = Some(evidence.observed_at);
        if let Some(thread_id) = evidence.thread_id.as_ref() {
            if self.status == ManagedCodexTaskStatus::Resuming
                && self
                    .thread_id
                    .as_ref()
                    .is_some_and(|existing| existing != thread_id)
            {
                return self.mark_needs_attention(format!(
                    "Codex resume returned a different thread id: expected {}, received {}",
                    self.thread_id.as_deref().unwrap_or("<none>"),
                    thread_id
                ));
            }
            self.thread_id = Some(thread_id.clone());
        }
        if let Some(turn_id) = evidence.turn_id.as_ref() {
            self.active_turn_id = Some(turn_id.clone());
        }
        if matches!(
            evidence.kind,
            CodexEvidenceKind::QuotaWarning
                | CodexEvidenceKind::TurnFailed
                | CodexEvidenceKind::TurnInterrupted
        ) {
            if let Some(message) = evidence.message.as_ref() {
                self.last_error = Some(message.clone());
            }
        }
        if let Some(failure_class) = evidence.failure_class {
            self.last_failure_class = Some(failure_class);
        }

        match evidence.kind {
            CodexEvidenceKind::TurnStarted => {
                self.status = ManagedCodexTaskStatus::Running;
                Vec::new()
            }
            CodexEvidenceKind::Activity => {
                if matches!(
                    self.status,
                    ManagedCodexTaskStatus::Preparing | ManagedCodexTaskStatus::Resuming
                ) {
                    self.status = ManagedCodexTaskStatus::Running;
                }
                Vec::new()
            }
            CodexEvidenceKind::QuotaWarning => {
                self.status = ManagedCodexTaskStatus::Draining;
                vec![CodexSupervisorAction::WaitForTerminal {
                    reason: "quota evidence is not terminal; wait for Codex to finish the turn"
                        .to_string(),
                }]
            }
            CodexEvidenceKind::TurnCompleted if evidence.terminal => {
                self.status = ManagedCodexTaskStatus::Completed;
                self.active_turn_id = None;
                self.completed_at = Some(evidence.observed_at);
                self.last_error = None;
                self.needs_attention_reason = None;
                vec![CodexSupervisorAction::MarkCompleted]
            }
            CodexEvidenceKind::TurnFailed if evidence.confirms_quota_exhaustion() => {
                self.plan_quota_failover()
            }
            CodexEvidenceKind::TurnFailed if !evidence.terminal => {
                if evidence.failure_class == Some(CodexFailureClass::QuotaExhausted) {
                    self.status = ManagedCodexTaskStatus::Draining;
                }
                vec![CodexSupervisorAction::WaitForTerminal {
                    reason: "runtime error is not terminal; wait for the authoritative turn result"
                        .to_string(),
                }]
            }
            CodexEvidenceKind::TurnFailed => {
                if evidence.failure_class == Some(CodexFailureClass::QuotaExhausted) {
                    self.status = ManagedCodexTaskStatus::Draining;
                    return vec![CodexSupervisorAction::WaitForTerminal {
                        reason: "quota failure came from a non-authoritative source".to_string(),
                    }];
                }
                self.status = ManagedCodexTaskStatus::Failed;
                self.active_turn_id = None;
                self.completed_at = Some(evidence.observed_at);
                vec![CodexSupervisorAction::Stop {
                    reason: evidence
                        .message
                        .clone()
                        .unwrap_or_else(|| "Codex turn failed for a non-quota reason".to_string()),
                }]
            }
            CodexEvidenceKind::TurnInterrupted if evidence.terminal => {
                self.status = if evidence.failure_class == Some(CodexFailureClass::UserInterrupted)
                {
                    ManagedCodexTaskStatus::Cancelled
                } else {
                    ManagedCodexTaskStatus::Failed
                };
                self.active_turn_id = None;
                self.completed_at = Some(evidence.observed_at);
                vec![CodexSupervisorAction::Stop {
                    reason: evidence
                        .message
                        .clone()
                        .unwrap_or_else(|| "Codex turn was interrupted".to_string()),
                }]
            }
            _ => Vec::new(),
        }
    }

    fn plan_quota_failover(&mut self) -> Vec<CodexSupervisorAction> {
        self.active_turn_id = None;
        if self
            .config
            .max_switches
            .is_some_and(|max_switches| self.switch_count >= max_switches)
        {
            return self.mark_needs_attention(
                "confirmed usage-limit failure, but the task switch budget is exhausted",
            );
        }
        self.status = ManagedCodexTaskStatus::Switching;
        self.pending_account_id = None;
        self.needs_attention_reason = None;
        vec![CodexSupervisorAction::RequestAccountSelection {
            failed_account_id: self.active_account_id.clone(),
            excluded_account_ids: self.attempted_account_ids.clone(),
        }]
    }

    pub fn mark_account_selected(&mut self, account_id: &str) -> Result<(), String> {
        if self.status != ManagedCodexTaskStatus::Switching {
            return Err(format!(
                "cannot select an account while task is {:?}",
                self.status
            ));
        }
        let account_id = account_id.trim();
        if account_id.is_empty() {
            return Err("selected account id cannot be empty".to_string());
        }
        if self
            .attempted_account_ids
            .iter()
            .any(|value| value == account_id)
        {
            return Err("selected account was already attempted by this task".to_string());
        }
        self.pending_account_id = Some(account_id.to_string());
        self.updated_at = chrono::Utc::now().timestamp_millis();
        Ok(())
    }

    pub fn reject_pending_account(
        &mut self,
        account_id: &str,
        reason: impl Into<String>,
    ) -> Result<(), String> {
        if self.status != ManagedCodexTaskStatus::Switching {
            return Err(format!(
                "cannot reject an account while task is {:?}",
                self.status
            ));
        }
        let account_id = account_id.trim();
        if self.pending_account_id.as_deref() != Some(account_id) {
            return Err(format!(
                "account rejection mismatch: expected {}, received {}",
                self.pending_account_id.as_deref().unwrap_or("<none>"),
                account_id
            ));
        }
        if !self
            .attempted_account_ids
            .iter()
            .any(|value| value == account_id)
        {
            self.attempted_account_ids.push(account_id.to_string());
        }
        self.pending_account_id = None;
        self.last_error = Some(truncate_chars(&reason.into(), MAX_EVIDENCE_MESSAGE_CHARS));
        self.updated_at = chrono::Utc::now().timestamp_millis();
        Ok(())
    }

    pub fn mark_account_switched(
        &mut self,
        account_id: &str,
    ) -> Result<Vec<CodexSupervisorAction>, String> {
        if self.status != ManagedCodexTaskStatus::Switching {
            return Err(format!(
                "cannot bind a new account while task is {:?}",
                self.status
            ));
        }
        let account_id = account_id.trim();
        if self.pending_account_id.as_deref() != Some(account_id) {
            return Err(format!(
                "account switch mismatch: expected {}, received {}",
                self.pending_account_id.as_deref().unwrap_or("<none>"),
                account_id
            ));
        }
        let thread_id = self
            .thread_id
            .clone()
            .ok_or_else(|| "cannot resume a managed task without a Codex thread id".to_string())?;

        self.active_account_id = Some(account_id.to_string());
        self.pending_account_id = None;
        self.attempted_account_ids.push(account_id.to_string());
        self.switch_count = self.switch_count.saturating_add(1);
        self.status = ManagedCodexTaskStatus::Resuming;
        self.updated_at = chrono::Utc::now().timestamp_millis();
        Ok(vec![CodexSupervisorAction::ResumeThread {
            thread_id,
            prompt: self.continuation_prompt(),
        }])
    }

    pub fn prepare_manual_resume_same_account(
        &mut self,
    ) -> Result<Vec<CodexSupervisorAction>, String> {
        if self.status != ManagedCodexTaskStatus::NeedsAttention {
            return Err("manual resume is only available for tasks needing attention".to_string());
        }
        if self.active_account_id.is_none() {
            return Err("managed Codex task has no active account to resume".to_string());
        }
        let thread_id = self
            .thread_id
            .clone()
            .ok_or_else(|| "managed Codex task has no thread id to resume".to_string())?;
        self.status = ManagedCodexTaskStatus::Resuming;
        self.needs_attention_reason = None;
        self.updated_at = chrono::Utc::now().timestamp_millis();
        Ok(vec![CodexSupervisorAction::ResumeThread {
            thread_id,
            prompt: self.continuation_prompt(),
        }])
    }

    pub fn prepare_manual_resume_next_account(
        &mut self,
    ) -> Result<Vec<CodexSupervisorAction>, String> {
        if self.status != ManagedCodexTaskStatus::NeedsAttention {
            return Err(
                "manual account selection is only available for tasks needing attention"
                    .to_string(),
            );
        }
        self.status = ManagedCodexTaskStatus::Switching;
        self.pending_account_id = None;
        self.needs_attention_reason = None;
        self.updated_at = chrono::Utc::now().timestamp_millis();
        Ok(vec![CodexSupervisorAction::RequestAccountSelection {
            failed_account_id: self.active_account_id.clone(),
            excluded_account_ids: self.attempted_account_ids.clone(),
        }])
    }

    pub fn requeue_before_first_launch(&mut self) -> Result<(), String> {
        if self.status != ManagedCodexTaskStatus::NeedsAttention {
            return Err("only a task needing attention can be requeued".to_string());
        }
        if self.thread_id.is_some() {
            return Err(
                "a task with a thread id must use resume instead of fresh requeue".to_string(),
            );
        }
        self.status = ManagedCodexTaskStatus::Queued;
        self.needs_attention_reason = None;
        self.last_error = None;
        self.pending_account_id = None;
        self.updated_at = chrono::Utc::now().timestamp_millis();
        Ok(())
    }

    pub fn plan_single_recovery_resume_current(
        &mut self,
    ) -> Result<Vec<CodexSupervisorAction>, String> {
        if self.recovery_attempts >= 1 {
            return Ok(self.mark_needs_attention(
                "automatic crash recovery was already attempted once for this task",
            ));
        }
        if self.active_account_id.is_none() {
            return Err("managed Codex task has no current account for crash recovery".to_string());
        }
        let thread_id = self
            .thread_id
            .clone()
            .ok_or_else(|| "managed Codex task has no thread id for crash recovery".to_string())?;
        self.recovery_attempts = self.recovery_attempts.saturating_add(1);
        self.status = ManagedCodexTaskStatus::Resuming;
        self.needs_attention_reason = None;
        self.updated_at = chrono::Utc::now().timestamp_millis();
        Ok(vec![CodexSupervisorAction::ResumeThread {
            thread_id,
            prompt: self.continuation_prompt(),
        }])
    }

    pub fn register_single_recovery_failover(&mut self) -> Result<(), String> {
        if self.recovery_attempts >= 1 {
            self.mark_needs_attention(
                "automatic crash recovery was already attempted once for this task",
            );
            return Err("automatic crash recovery limit reached".to_string());
        }
        if self.status != ManagedCodexTaskStatus::Switching {
            return Err(format!(
                "cannot register crash recovery failover while task is {:?}",
                self.status
            ));
        }
        self.recovery_attempts = self.recovery_attempts.saturating_add(1);
        self.updated_at = chrono::Utc::now().timestamp_millis();
        Ok(())
    }

    pub fn mark_resume_started(&mut self, turn_id: Option<String>) -> Result<(), String> {
        if self.status != ManagedCodexTaskStatus::Resuming {
            return Err(format!(
                "cannot mark resume started while task is {:?}",
                self.status
            ));
        }
        self.active_turn_id = normalize_optional_string(turn_id);
        self.status = ManagedCodexTaskStatus::Running;
        self.updated_at = chrono::Utc::now().timestamp_millis();
        Ok(())
    }

    pub fn mark_cancelled(&mut self, reason: impl Into<String>) {
        let now = chrono::Utc::now().timestamp_millis();
        self.status = ManagedCodexTaskStatus::Cancelled;
        self.active_turn_id = None;
        self.pending_account_id = None;
        self.last_failure_class = Some(CodexFailureClass::UserInterrupted);
        self.last_error = Some(reason.into());
        self.completed_at = Some(now);
        self.updated_at = now;
    }

    pub fn mark_needs_attention(
        &mut self,
        reason: impl Into<String>,
    ) -> Vec<CodexSupervisorAction> {
        let reason = reason.into();
        self.status = ManagedCodexTaskStatus::NeedsAttention;
        self.pending_account_id = None;
        self.active_turn_id = None;
        self.needs_attention_reason = Some(reason.clone());
        self.last_error = Some(reason.clone());
        self.updated_at = chrono::Utc::now().timestamp_millis();
        vec![CodexSupervisorAction::Stop { reason }]
    }

    pub fn continuation_prompt(&self) -> String {
        format!(
            "Continue the original task from the exact point where the previous turn stopped. \
Inspect the current working tree first, do not repeat completed steps, and finish this objective:\n{}",
            self.config.objective
        )
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn classify_codex_event(source: CodexEventSource, payload: &Value) -> CodexTaskEvidence {
    let mut evidence = CodexTaskEvidence::new(source);
    evidence.thread_id = first_string(
        payload,
        &[
            "/params/threadId",
            "/params/thread_id",
            "/params/turn/threadId",
            "/params/turn/thread_id",
            "/thread_id",
            "/threadId",
            "/payload/thread_id",
            "/payload/threadId",
            "/payload/id",
        ],
    );
    evidence.turn_id = first_string(
        payload,
        &[
            "/params/turn/id",
            "/params/turnId",
            "/params/turn_id",
            "/turn_id",
            "/turnId",
            "/payload/turn_id",
            "/payload/turnId",
        ],
    );

    let raw_type = raw_event_type(source, payload);
    evidence.raw_event_type = raw_type.clone();
    let event_type = raw_type
        .as_deref()
        .map(normalize_event_type)
        .unwrap_or_default();
    let error_root = find_error_root(payload);
    evidence.error_code = error_root.and_then(extract_error_code);
    evidence.message = error_root
        .and_then(extract_error_message)
        .or_else(|| {
            first_string(
                payload,
                &["/message", "/params/message", "/payload/message"],
            )
        })
        .map(|value| truncate_chars(&value, MAX_EVIDENCE_MESSAGE_CHARS));
    let http_status = extract_http_status(payload, error_root);
    evidence.failure_class = classify_failure(
        evidence.error_code.as_deref(),
        evidence.message.as_deref(),
        http_status,
    );

    match source {
        CodexEventSource::AppServer => {
            classify_app_server_event(&mut evidence, payload, &event_type)
        }
        CodexEventSource::ExecJson => classify_exec_event(&mut evidence, payload, &event_type),
        CodexEventSource::RolloutJsonl => {
            classify_rollout_event(&mut evidence, payload, &event_type)
        }
        CodexEventSource::Proxy => classify_proxy_event(&mut evidence, payload, &event_type),
    }
    evidence
}

pub fn classify_codex_event_text(source: &str, payload: &str) -> Result<CodexTaskEvidence, String> {
    let source = CodexEventSource::parse(source)?;
    let payload = serde_json::from_str::<Value>(payload.trim())
        .map_err(|error| format!("invalid Codex event JSON: {error}"))?;
    Ok(classify_codex_event(source, &payload))
}

fn classify_app_server_event(evidence: &mut CodexTaskEvidence, payload: &Value, event_type: &str) {
    match event_type {
        "turn/started" => {
            evidence.kind = CodexEvidenceKind::TurnStarted;
            evidence.confidence = CodexEvidenceConfidence::Confirmed;
        }
        "turn/completed" => {
            let status = first_string(
                payload,
                &["/params/turn/status", "/params/status", "/turn/status"],
            )
            .unwrap_or_else(|| "completed".to_string())
            .to_ascii_lowercase();
            evidence.terminal = true;
            evidence.confidence = CodexEvidenceConfidence::Confirmed;
            match status.as_str() {
                "completed" => {
                    evidence.kind = CodexEvidenceKind::TurnCompleted;
                    evidence.failure_class = None;
                }
                "interrupted" | "cancelled" | "canceled" => {
                    evidence.kind = CodexEvidenceKind::TurnInterrupted;
                    evidence
                        .failure_class
                        .get_or_insert(CodexFailureClass::UserInterrupted);
                }
                "failed" => evidence.kind = CodexEvidenceKind::TurnFailed,
                _ => {
                    evidence.kind = CodexEvidenceKind::TurnFailed;
                    evidence
                        .failure_class
                        .get_or_insert(CodexFailureClass::Other);
                }
            }
        }
        "error" => classify_nonterminal_error(evidence),
        "account/ratelimits/updated" => {
            if rate_limit_payload_is_exhausted(payload) {
                evidence.kind = CodexEvidenceKind::QuotaWarning;
                evidence.confidence = CodexEvidenceConfidence::Suspected;
                evidence.failure_class = Some(CodexFailureClass::QuotaExhausted);
            } else {
                evidence.kind = CodexEvidenceKind::Activity;
            }
        }
        _ => evidence.kind = CodexEvidenceKind::Activity,
    }
}

fn classify_exec_event(evidence: &mut CodexTaskEvidence, _payload: &Value, event_type: &str) {
    match event_type {
        "turn.started" | "turn/started" => {
            evidence.kind = CodexEvidenceKind::TurnStarted;
            evidence.confidence = CodexEvidenceConfidence::Confirmed;
        }
        "turn.completed" | "turn/completed" => {
            evidence.kind = CodexEvidenceKind::TurnCompleted;
            evidence.confidence = CodexEvidenceConfidence::Confirmed;
            evidence.terminal = true;
            evidence.failure_class = None;
        }
        "turn.failed" | "turn/failed" => {
            evidence.kind = CodexEvidenceKind::TurnFailed;
            evidence.confidence = CodexEvidenceConfidence::Confirmed;
            evidence.terminal = true;
            evidence
                .failure_class
                .get_or_insert(CodexFailureClass::Other);
        }
        "error" => classify_nonterminal_error(evidence),
        "thread.started" | "thread/started" | "item.started" | "item.completed" => {
            evidence.kind = CodexEvidenceKind::Activity
        }
        _ => evidence.kind = CodexEvidenceKind::Unknown,
    }
}

fn classify_rollout_event(evidence: &mut CodexTaskEvidence, payload: &Value, event_type: &str) {
    let nested_type = first_string(payload, &["/payload/type"])
        .map(|value| normalize_event_type(&value))
        .unwrap_or_default();
    if evidence.failure_class == Some(CodexFailureClass::QuotaExhausted) {
        evidence.kind = CodexEvidenceKind::QuotaWarning;
        evidence.confidence = CodexEvidenceConfidence::Suspected;
        evidence.terminal = false;
        return;
    }
    if matches!(nested_type.as_str(), "turn_started" | "task_started") {
        evidence.kind = CodexEvidenceKind::TurnStarted;
        evidence.confidence = CodexEvidenceConfidence::Informational;
    } else if matches!(nested_type.as_str(), "turn_aborted" | "task_aborted") {
        evidence.kind = CodexEvidenceKind::TurnInterrupted;
        evidence.confidence = CodexEvidenceConfidence::Suspected;
        evidence.terminal = false;
    } else if matches!(nested_type.as_str(), "turn_complete" | "task_complete") {
        evidence.kind = CodexEvidenceKind::TurnCompleted;
        evidence.confidence = CodexEvidenceConfidence::Informational;
        evidence.terminal = false;
    } else if matches!(event_type, "session_meta" | "response_item" | "event_msg") {
        evidence.kind = CodexEvidenceKind::Activity;
    } else {
        evidence.kind = CodexEvidenceKind::Unknown;
    }
}

fn classify_proxy_event(evidence: &mut CodexTaskEvidence, payload: &Value, event_type: &str) {
    let http_status = extract_http_status(payload, find_error_root(payload));
    if evidence.failure_class == Some(CodexFailureClass::QuotaExhausted) || http_status == Some(429)
    {
        evidence.kind = CodexEvidenceKind::QuotaWarning;
        evidence.confidence = CodexEvidenceConfidence::Suspected;
        evidence.terminal = false;
    } else if matches!(event_type, "error" | "response.failed") {
        evidence.kind = CodexEvidenceKind::TurnFailed;
        evidence.confidence = CodexEvidenceConfidence::Suspected;
        evidence.terminal = false;
        evidence
            .failure_class
            .get_or_insert(CodexFailureClass::Other);
    } else {
        evidence.kind = CodexEvidenceKind::Activity;
    }
}

fn classify_nonterminal_error(evidence: &mut CodexTaskEvidence) {
    if evidence.failure_class == Some(CodexFailureClass::QuotaExhausted) {
        evidence.kind = CodexEvidenceKind::QuotaWarning;
        evidence.confidence = CodexEvidenceConfidence::Suspected;
    } else {
        evidence.kind = CodexEvidenceKind::TurnFailed;
        evidence.confidence = CodexEvidenceConfidence::Suspected;
    }
    evidence.terminal = false;
}

fn raw_event_type(source: CodexEventSource, payload: &Value) -> Option<String> {
    let paths: &[&str] = match source {
        CodexEventSource::AppServer => &["/method", "/type"],
        CodexEventSource::ExecJson => &["/type", "/event"],
        CodexEventSource::RolloutJsonl => &["/type"],
        CodexEventSource::Proxy => &["/type", "/event", "/body/type"],
    };
    first_string(payload, paths)
}

fn normalize_event_type(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn first_string(value: &Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| {
        value
            .pointer(path)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn find_error_root(value: &Value) -> Option<&Value> {
    [
        "/params/turn/error",
        "/params/error",
        "/turn/error",
        "/payload/error",
        "/body/error",
        "/error",
    ]
    .iter()
    .find_map(|path| value.pointer(path))
}

fn extract_error_code(error: &Value) -> Option<String> {
    let mut candidates = Vec::new();
    collect_error_codes(error, &mut candidates, 0);
    candidates
        .into_iter()
        .find(|value| !value.trim().is_empty())
}

fn collect_error_codes(value: &Value, output: &mut Vec<String>, depth: usize) {
    if depth > 6 || output.len() >= 24 {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let normalized_key = normalize_token(key);
                if normalized_key == "codexerrorinfo" {
                    match child {
                        Value::String(value) => output.push(value.clone()),
                        Value::Object(variants) => {
                            output.extend(variants.keys().cloned());
                            collect_error_codes(child, output, depth + 1);
                        }
                        _ => collect_error_codes(child, output, depth + 1),
                    }
                } else if matches!(
                    normalized_key.as_str(),
                    "code" | "type" | "errortype" | "errorcode"
                ) {
                    if let Some(value) = child.as_str() {
                        output.push(value.to_string());
                    }
                } else if normalized_key == "error" || depth == 0 {
                    collect_error_codes(child, output, depth + 1);
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_error_codes(child, output, depth + 1);
            }
        }
        Value::String(value) if depth == 0 => output.push(value.clone()),
        _ => {}
    }
}

fn extract_error_message(error: &Value) -> Option<String> {
    match error {
        Value::String(value) => Some(value.trim().to_string()).filter(|value| !value.is_empty()),
        Value::Object(map) => {
            for key in ["message", "detail", "reason"] {
                if let Some(value) = map
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Some(value.to_string());
                }
            }
            map.get("error").and_then(extract_error_message)
        }
        _ => None,
    }
}

fn extract_http_status(payload: &Value, error: Option<&Value>) -> Option<u16> {
    let direct = [
        "/status",
        "/statusCode",
        "/status_code",
        "/params/error/codexErrorInfo/httpStatusCode",
        "/params/turn/error/codexErrorInfo/httpStatusCode",
        "/error/status",
        "/body/status",
    ]
    .iter()
    .find_map(|path| value_as_u16(payload.pointer(path)?));
    direct.or_else(|| error.and_then(find_http_status_recursive))
}

fn find_http_status_recursive(value: &Value) -> Option<u16> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if matches!(
                    normalize_token(key).as_str(),
                    "httpstatuscode" | "statuscode" | "status"
                ) {
                    if let Some(status) = value_as_u16(child) {
                        return Some(status);
                    }
                }
                if let Some(status) = find_http_status_recursive(child) {
                    return Some(status);
                }
            }
            None
        }
        Value::Array(values) => values.iter().find_map(find_http_status_recursive),
        _ => None,
    }
}

fn value_as_u16(value: &Value) -> Option<u16> {
    value
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .or_else(|| value.as_str()?.trim().parse::<u16>().ok())
}

fn classify_failure(
    code: Option<&str>,
    message: Option<&str>,
    http_status: Option<u16>,
) -> Option<CodexFailureClass> {
    let code = code.map(normalize_token).unwrap_or_default();
    let message = message.unwrap_or_default().to_ascii_lowercase();
    if matches!(
        code.as_str(),
        "usagelimitexceeded"
            | "usagelimitreached"
            | "ratelimitexceeded"
            | "insufficientquota"
            | "quotaexceeded"
    ) || message.contains("usage limit")
        || message.contains("quota exceeded")
        || message.contains("insufficient quota")
        || message.contains("额度已用完")
        || message.contains("额度耗尽")
    {
        return Some(CodexFailureClass::QuotaExhausted);
    }
    if matches!(
        code.as_str(),
        "unauthorized" | "invalidapikey" | "invalidtoken"
    ) || http_status == Some(401)
        || message.contains("authentication")
    {
        return Some(CodexFailureClass::Authentication);
    }
    if code == "contextwindowexceeded" || message.contains("context window") {
        return Some(CodexFailureClass::ContextWindow);
    }
    if matches!(
        code.as_str(),
        "httpconnectionfailed" | "responsestreamconnectionfailed" | "responsestreamdisconnected"
    ) || message.contains("connection failed")
        || message.contains("network error")
    {
        return Some(CodexFailureClass::Network);
    }
    if message.contains("model capacity") || message.contains("overloaded") {
        return Some(CodexFailureClass::ModelCapacity);
    }
    if matches!(code.as_str(), "cancelled" | "canceled" | "interrupted")
        || message.contains("cancelled by user")
    {
        return Some(CodexFailureClass::UserInterrupted);
    }
    if !code.is_empty() || !message.trim().is_empty() || http_status.is_some() {
        return Some(CodexFailureClass::Other);
    }
    None
}

fn normalize_token(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn rate_limit_payload_is_exhausted(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, child)| {
            let key = normalize_token(key);
            (matches!(key.as_str(), "limitreached" | "exhausted") && child == &Value::Bool(true))
                || (matches!(key.as_str(), "remainingpercent" | "remaining")
                    && child.as_f64().is_some_and(|value| value <= 0.0))
                || rate_limit_payload_is_exhausted(child)
        }),
        Value::Array(values) => values.iter().any(rate_limit_payload_is_exhausted),
        _ => false,
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value.chars().take(max_chars).collect::<String>();
    output.push('…');
    output
}

#[cfg(test)]
mod codex_task_supervisor_tests {
    use serde_json::json;

    use super::*;

    fn test_task() -> ManagedCodexTask {
        let mut task = ManagedCodexTask::create(ManagedCodexTaskConfig {
            objective: "finish the repository change".to_string(),
            cwd: "C:/workspace".to_string(),
            account_scope: ManagedCodexAccountScope::Selected {
                account_ids: vec!["account-a".to_string(), "account-b".to_string()],
            },
            initial_account_id: Some("account-a".to_string()),
            model: None,
            reasoning_effort: None,
            max_switches: None,
        })
        .expect("create task");
        task.mark_preparing("account-a").expect("prepare task");
        task
    }

    #[test]
    fn app_server_terminal_usage_limit_is_confirmed() {
        let evidence = classify_codex_event(
            CodexEventSource::AppServer,
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-1",
                    "turn": {
                        "id": "turn-1",
                        "status": "failed",
                        "error": {
                            "message": "You have reached your usage limit",
                            "codexErrorInfo": "UsageLimitExceeded"
                        }
                    }
                }
            }),
        );

        assert_eq!(evidence.kind, CodexEvidenceKind::TurnFailed);
        assert_eq!(evidence.confidence, CodexEvidenceConfidence::Confirmed);
        assert_eq!(
            evidence.failure_class,
            Some(CodexFailureClass::QuotaExhausted)
        );
        assert!(evidence.terminal);
        assert!(evidence.confirms_quota_exhaustion());
        assert_eq!(evidence.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(evidence.turn_id.as_deref(), Some("turn-1"));
    }

    #[test]
    fn app_server_error_event_waits_for_terminal_notification() {
        let evidence = classify_codex_event(
            CodexEventSource::AppServer,
            &json!({
                "method": "error",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "error": {
                        "message": "usage limit reached",
                        "codexErrorInfo": { "UsageLimitExceeded": { "httpStatusCode": 429 } }
                    }
                }
            }),
        );

        assert_eq!(evidence.kind, CodexEvidenceKind::QuotaWarning);
        assert_eq!(evidence.confidence, CodexEvidenceConfidence::Suspected);
        assert!(!evidence.terminal);
    }

    #[test]
    fn retryable_non_quota_error_does_not_stop_task() {
        let evidence = classify_codex_event(
            CodexEventSource::AppServer,
            &json!({
                "method": "error",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "willRetry": true,
                    "error": {
                        "message": "response stream disconnected",
                        "codexErrorInfo": "ResponseStreamDisconnected"
                    }
                }
            }),
        );
        let mut task = test_task();
        let actions = task.apply_evidence(&evidence);

        assert!(!task.status.is_terminal());
        assert!(matches!(
            actions.as_slice(),
            [CodexSupervisorAction::WaitForTerminal { .. }]
        ));
    }

    #[test]
    fn proxy_429_is_suspected_not_terminal() {
        let evidence = classify_codex_event(
            CodexEventSource::Proxy,
            &json!({
                "status": 429,
                "error": { "type": "usage_limit_reached", "message": "quota exceeded" }
            }),
        );

        assert_eq!(evidence.kind, CodexEvidenceKind::QuotaWarning);
        assert_eq!(evidence.confidence, CodexEvidenceConfidence::Suspected);
        assert!(!evidence.terminal);
    }

    #[test]
    fn exec_terminal_non_quota_failure_does_not_rotate() {
        let evidence = classify_codex_event(
            CodexEventSource::ExecJson,
            &json!({
                "type": "turn.failed",
                "error": {
                    "message": "response stream disconnected",
                    "codexErrorInfo": "ResponseStreamDisconnected"
                }
            }),
        );
        let mut task = test_task();
        let actions = task.apply_evidence(&evidence);

        assert_eq!(task.status, ManagedCodexTaskStatus::Failed);
        assert!(matches!(
            actions.as_slice(),
            [CodexSupervisorAction::Stop { .. }]
        ));
        assert_eq!(task.switch_count, 0);
    }

    #[test]
    fn suspected_quota_moves_to_draining_without_switching() {
        let evidence = classify_codex_event(
            CodexEventSource::Proxy,
            &json!({
                "status": 429,
                "error": { "type": "usage_limit_reached" }
            }),
        );
        let mut task = test_task();
        let actions = task.apply_evidence(&evidence);

        assert_eq!(task.status, ManagedCodexTaskStatus::Draining);
        assert!(matches!(
            actions.as_slice(),
            [CodexSupervisorAction::WaitForTerminal { .. }]
        ));
        assert!(task.pending_account_id.is_none());
    }

    #[test]
    fn confirmed_quota_failure_switches_then_resumes_same_thread() {
        let evidence = classify_codex_event(
            CodexEventSource::AppServer,
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-1",
                    "turn": {
                        "id": "turn-1",
                        "status": "failed",
                        "error": {
                            "message": "usage limit reached",
                            "codexErrorInfo": "UsageLimitExceeded"
                        }
                    }
                }
            }),
        );
        let mut task = test_task();
        let actions = task.apply_evidence(&evidence);

        assert_eq!(task.status, ManagedCodexTaskStatus::Switching);
        assert!(task.pending_account_id.is_none());
        assert!(matches!(
            actions.as_slice(),
            [CodexSupervisorAction::RequestAccountSelection { failed_account_id, excluded_account_ids }]
                if failed_account_id.as_deref() == Some("account-a")
                    && excluded_account_ids == &["account-a".to_string()]
        ));

        task.mark_account_selected("account-b")
            .expect("select account");
        let actions = task
            .mark_account_switched("account-b")
            .expect("mark account switched");
        assert_eq!(task.status, ManagedCodexTaskStatus::Resuming);
        assert_eq!(task.switch_count, 1);
        assert!(matches!(
            actions.as_slice(),
            [CodexSupervisorAction::ResumeThread { thread_id, prompt }]
                if thread_id == "thread-1" && prompt.contains("finish the repository change")
        ));
    }

    #[test]
    fn quota_warning_followed_by_success_never_requests_an_account() {
        let mut task = test_task();
        let warning = classify_codex_event(
            CodexEventSource::ExecJson,
            &json!({
                "type": "error",
                "error": {
                    "message": "usage limit reached; retrying",
                    "codexErrorInfo": "UsageLimitExceeded"
                }
            }),
        );
        assert!(matches!(
            task.apply_evidence(&warning).as_slice(),
            [CodexSupervisorAction::WaitForTerminal { .. }]
        ));

        let completed = classify_codex_event(
            CodexEventSource::ExecJson,
            &json!({ "type": "turn.completed", "thread_id": "thread-1" }),
        );
        assert!(matches!(
            task.apply_evidence(&completed).as_slice(),
            [CodexSupervisorAction::MarkCompleted]
        ));
        assert_eq!(task.status, ManagedCodexTaskStatus::Completed);
        assert_eq!(task.switch_count, 0);
    }

    #[test]
    fn duplicate_terminal_evidence_only_requests_one_selection() {
        let mut task = test_task();
        let evidence = classify_codex_event(
            CodexEventSource::ExecJson,
            &json!({
                "type": "turn.failed",
                "thread_id": "thread-1",
                "error": {
                    "message": "usage limit reached",
                    "codexErrorInfo": "UsageLimitExceeded"
                }
            }),
        )
        .with_id("terminal-1");
        assert!(matches!(
            task.apply_evidence(&evidence).as_slice(),
            [CodexSupervisorAction::RequestAccountSelection { .. }]
        ));
        assert!(task.apply_evidence(&evidence).is_empty());
    }

    #[test]
    fn resumed_thread_mismatch_needs_attention() {
        let mut task = test_task();
        task.thread_id = Some("thread-original".to_string());
        task.status = ManagedCodexTaskStatus::Switching;
        task.mark_account_selected("account-b")
            .expect("select account");
        task.mark_account_switched("account-b")
            .expect("switch account");

        let evidence =
            CodexTaskEvidence::activity(Some("thread-different".to_string()), "thread.started");
        assert!(matches!(
            task.apply_evidence(&evidence).as_slice(),
            [CodexSupervisorAction::Stop { .. }]
        ));
        assert_eq!(task.status, ManagedCodexTaskStatus::NeedsAttention);
    }

    #[test]
    fn rollout_quota_text_never_confirms_terminal_failure() {
        let evidence = classify_codex_event(
            CodexEventSource::RolloutJsonl,
            &json!({
                "type": "event_msg",
                "payload": {
                    "type": "error",
                    "error": {
                        "type": "usage_limit_reached",
                        "message": "usage limit reached"
                    }
                }
            }),
        );

        assert_eq!(evidence.kind, CodexEvidenceKind::QuotaWarning);
        assert_eq!(evidence.confidence, CodexEvidenceConfidence::Suspected);
        assert!(!evidence.terminal);
    }

    #[test]
    fn explicit_switch_limit_stops_before_requesting_another_account() {
        let mut task = test_task();
        task.config.max_switches = Some(0);
        let evidence = classify_codex_event(
            CodexEventSource::ExecJson,
            &json!({
                "type": "turn.failed",
                "thread_id": "thread-1",
                "error": {
                    "message": "usage limit reached",
                    "codexErrorInfo": "UsageLimitExceeded"
                }
            }),
        );
        let actions = task.apply_evidence(&evidence);
        assert_eq!(task.status, ManagedCodexTaskStatus::NeedsAttention);
        assert_eq!(task.switch_count, 0);
        assert!(matches!(
            actions.as_slice(),
            [CodexSupervisorAction::Stop { .. }]
        ));
    }

    #[test]
    fn rejected_injection_candidate_is_never_reused() {
        let mut task = test_task();
        task.thread_id = Some("thread-1".to_string());
        task.status = ManagedCodexTaskStatus::Switching;
        task.mark_account_selected("account-b")
            .expect("select candidate");
        task.reject_pending_account("account-b", "credential refresh failed")
            .expect("reject candidate");
        assert!(task
            .attempted_account_ids
            .contains(&"account-b".to_string()));
        assert!(task.mark_account_selected("account-b").is_err());
    }

    #[test]
    fn crash_recovery_can_auto_resume_only_once() {
        let mut task = test_task();
        task.thread_id = Some("thread-1".to_string());
        let first = task
            .plan_single_recovery_resume_current()
            .expect("first recovery plan");
        assert!(matches!(
            first.as_slice(),
            [CodexSupervisorAction::ResumeThread { thread_id, .. }] if thread_id == "thread-1"
        ));
        task.mark_needs_attention("simulated second restart");
        let second = task
            .plan_single_recovery_resume_current()
            .expect("second recovery is normalized to attention");
        assert_eq!(task.status, ManagedCodexTaskStatus::NeedsAttention);
        assert!(matches!(
            second.as_slice(),
            [CodexSupervisorAction::Stop { .. }]
        ));
        assert_eq!(task.recovery_attempts, 1);
    }

    #[test]
    fn user_can_explicitly_requeue_a_task_that_never_obtained_a_thread_id() {
        let mut task = test_task();
        task.mark_needs_attention("process exited before thread.started");
        task.requeue_before_first_launch()
            .expect("explicit retry without a thread id");
        assert_eq!(task.status, ManagedCodexTaskStatus::Queued);
        assert_eq!(task.active_account_id.as_deref(), Some("account-a"));
        assert!(task
            .attempted_account_ids
            .contains(&"account-a".to_string()));
    }
}
