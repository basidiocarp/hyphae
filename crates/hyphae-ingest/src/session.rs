use std::collections::HashSet;

use crate::transcript::SessionRuntime;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexLifecycleState {
    pub session_started: bool,
    pub session_ended: bool,
    pub turns_completed: usize,
    pub approvals_requested: usize,
    pub approvals_resolved: usize,
    pub pending_approvals: usize,
    pub last_event: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexLifecyclePhase {
    SessionActive,
    AwaitingApproval,
    ApprovalResolved,
    TurnComplete,
    SessionEnded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCodexLifecycleEvent {
    pub normalized_event_type: String,
    pub note: String,
}

/// Shared normalized session state for Claude and Codex inputs.
#[derive(Debug, Clone)]
pub struct NormalizedSession {
    runtime: SessionRuntime,
    session_id: Option<String>,
    project: Option<String>,
    message_count: usize,
    files_modified: HashSet<String>,
    commands_run: Vec<String>,
    errors: Vec<String>,
    highlights: Vec<String>,
    lifecycle_events: Vec<String>,
    codex_lifecycle_state: Option<CodexLifecycleState>,
    raw_excerpt: Vec<String>,
}

impl NormalizedSession {
    pub fn new(runtime: SessionRuntime) -> Self {
        Self {
            runtime,
            session_id: None,
            project: None,
            message_count: 0,
            files_modified: HashSet::new(),
            commands_run: Vec::new(),
            errors: Vec::new(),
            highlights: Vec::new(),
            lifecycle_events: Vec::new(),
            codex_lifecycle_state: matches!(runtime, SessionRuntime::Codex)
                .then(CodexLifecycleState::default),
            raw_excerpt: Vec::new(),
        }
    }

    pub fn runtime(&self) -> SessionRuntime {
        self.runtime
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn project(&self) -> Option<&str> {
        self.project.as_deref()
    }

    pub fn message_count(&self) -> usize {
        self.message_count
    }

    pub fn files_modified(&self) -> &HashSet<String> {
        &self.files_modified
    }

    pub fn commands_run(&self) -> &[String] {
        &self.commands_run
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    pub fn highlights(&self) -> &[String] {
        &self.highlights
    }

    pub fn lifecycle_events(&self) -> &[String] {
        &self.lifecycle_events
    }

    pub fn codex_lifecycle_state(&self) -> Option<&CodexLifecycleState> {
        self.codex_lifecycle_state.as_ref()
    }

    pub fn raw_excerpt(&self) -> &[String] {
        &self.raw_excerpt
    }

    pub fn note_session_id(&mut self, session_id: impl Into<String>) {
        if self.session_id.is_none() {
            self.session_id = Some(session_id.into());
        }
    }

    pub fn note_project(&mut self, project: impl Into<String>) {
        if self.project.is_none() {
            self.project = Some(project.into());
        }
    }

    pub fn note_message(&mut self) {
        self.message_count += 1;
    }

    pub fn note_file_modified(&mut self, path: impl Into<String>) {
        self.files_modified.insert(path.into());
    }

    pub fn note_command(&mut self, command: impl Into<String>) {
        if self.commands_run.len() < 50 {
            self.commands_run.push(command.into());
        }
    }

    pub fn note_error(&mut self, error: impl Into<String>) {
        if self.errors.len() < 20 {
            self.errors.push(error.into());
        }
    }

    pub fn note_highlight(&mut self, text: &str) {
        if self.highlights.len() >= 5 {
            return;
        }

        let snippet = truncate_snippet(text, 160);
        if !snippet.is_empty() {
            self.highlights.push(snippet);
        }
    }

    pub fn note_lifecycle_event(
        &mut self,
        event_type: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.record_codex_lifecycle_event(&event_type.into(), &detail.into());
    }

    pub fn record_codex_lifecycle_event(
        &mut self,
        event_type: &str,
        detail: &str,
    ) -> Option<RecordedCodexLifecycleEvent> {
        let normalized_event_type = normalize_codex_event_type(event_type);
        self.update_codex_lifecycle_state(&normalized_event_type);

        let note = format_codex_lifecycle_note(&normalized_event_type, detail);
        if note.is_empty() {
            return None;
        }

        if self.lifecycle_events.len() < 8 {
            self.lifecycle_events.push(note.clone());
        }

        Some(RecordedCodexLifecycleEvent {
            normalized_event_type,
            note,
        })
    }

    pub fn note_codex_session_started(&mut self) {
        if let Some(state) = &mut self.codex_lifecycle_state {
            state.session_started = true;
            state.last_event = Some("session-started".to_string());
        }
    }

    pub fn note_raw_excerpt_line(&mut self, line: impl Into<String>) {
        if self.raw_excerpt.len() < 20 {
            self.raw_excerpt.push(line.into());
        }
    }

    pub fn note_project_from_cwd(&mut self, cwd: &str) {
        if self.project.is_some() {
            return;
        }

        if let Some(project) = project_from_cwd(cwd) {
            self.project = Some(project);
        }
    }

    pub fn codex_lifecycle_state_summary(&self) -> Option<String> {
        self.codex_lifecycle_state
            .as_ref()
            .map(summarize_codex_lifecycle_state)
            .filter(|summary| !summary.is_empty())
    }

    pub fn codex_lifecycle_state_keyword(&self) -> Option<String> {
        self.codex_lifecycle_state
            .as_ref()
            .and_then(codex_lifecycle_state_keyword)
    }

    fn update_codex_lifecycle_state(&mut self, normalized_event_type: &str) {
        let Some(state) = &mut self.codex_lifecycle_state else {
            return;
        };

        state.last_event = Some(normalized_event_type.to_string());

        match normalized_event_type {
            "session-started" => {
                state.session_started = true;
                state.session_ended = false;
            }
            "agent-turn-complete" => {
                state.session_started = true;
                state.turns_completed += 1;
            }
            "approval-requested" => {
                state.session_started = true;
                state.approvals_requested += 1;
                state.pending_approvals += 1;
            }
            "approval-approved" | "approval-denied" | "approval-rejected" => {
                state.session_started = true;
                state.approvals_resolved += 1;
                state.pending_approvals = state.pending_approvals.saturating_sub(1);
            }
            "tool-use" | "tool-result" => {
                state.session_started = true;
            }
            "session-ended" => {
                state.session_started = true;
                state.session_ended = true;
            }
            _ => {
                if normalized_event_type.contains("session")
                    && normalized_event_type.contains("end")
                {
                    state.session_started = true;
                    state.session_ended = true;
                }
            }
        }
    }
}

pub fn project_from_cwd(cwd: &str) -> Option<String> {
    std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
}

pub fn truncate_snippet(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    let mut chars = trimmed.chars();
    let snippet: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{snippet}...")
    } else {
        snippet
    }
}

pub fn normalize_codex_event_type(event_type: &str) -> String {
    let normalized = event_type
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-")
        .replace(' ', "-");

    match normalized.as_str() {
        "session-start" | "session-started" | "session-begin" | "session-began" => {
            "session-started".to_string()
        }
        "session-end" | "session-ended" | "session-stop" | "session-stopped"
        | "session-complete" | "session-completed" => "session-ended".to_string(),
        "tool-use" => "tool-use".to_string(),
        "tool-result" => "tool-result".to_string(),
        _ => normalized,
    }
}

pub fn format_codex_lifecycle_note(event_type: &str, detail: &str) -> String {
    let event_type = normalize_codex_event_type(event_type);
    let detail = truncate_snippet(detail, 160);
    if detail.is_empty() {
        event_type
    } else {
        format!("{event_type}: {detail}")
    }
}

pub fn summarize_codex_lifecycle_state(state: &CodexLifecycleState) -> String {
    let mut parts = Vec::new();

    if let Some(phase) = codex_lifecycle_phase(state) {
        parts.push(format!("phase {}", phase_label(phase)));
    }
    if state.session_started {
        parts.push("session started".to_string());
    }
    if state.turns_completed > 0 {
        parts.push(format!("{} turn(s) completed", state.turns_completed));
    }
    if state.approvals_requested > 0 {
        parts.push(format!("{} approval request(s)", state.approvals_requested));
    }
    if state.approvals_resolved > 0 {
        parts.push(format!("{} approval decision(s)", state.approvals_resolved));
    }
    if state.pending_approvals > 0 {
        parts.push(format!("{} pending approval(s)", state.pending_approvals));
    }
    if state.session_ended {
        parts.push("session ended".to_string());
    }
    if let Some(last_event) = state.last_event.as_deref() {
        parts.push(format!("last event {last_event}"));
    }

    parts.join(" · ")
}

pub fn codex_lifecycle_phase(state: &CodexLifecycleState) -> Option<CodexLifecyclePhase> {
    if state.session_ended {
        return Some(CodexLifecyclePhase::SessionEnded);
    }

    if state.pending_approvals > 0 {
        return Some(CodexLifecyclePhase::AwaitingApproval);
    }

    match state.last_event.as_deref() {
        Some("approval-approved" | "approval-denied" | "approval-rejected") => {
            Some(CodexLifecyclePhase::ApprovalResolved)
        }
        Some("agent-turn-complete") => Some(CodexLifecyclePhase::TurnComplete),
        Some(_) if state.session_started => Some(CodexLifecyclePhase::SessionActive),
        None if state.session_started => Some(CodexLifecyclePhase::SessionActive),
        _ => None,
    }
}

pub fn phase_label(phase: CodexLifecyclePhase) -> &'static str {
    match phase {
        CodexLifecyclePhase::SessionActive => "session-active",
        CodexLifecyclePhase::AwaitingApproval => "awaiting-approval",
        CodexLifecyclePhase::ApprovalResolved => "approval-resolved",
        CodexLifecyclePhase::TurnComplete => "turn-complete",
        CodexLifecyclePhase::SessionEnded => "session-ended",
    }
}

pub fn codex_lifecycle_state_keyword(state: &CodexLifecycleState) -> Option<String> {
    codex_lifecycle_phase(state).map(|phase| format!("state:{}", phase_label(phase)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalized_session_accumulates_data() {
        let mut session = NormalizedSession::new(SessionRuntime::Codex);
        session.note_session_id("sess-1");
        session.note_project_from_cwd("/Users/test/demo-project");
        session.note_message();
        session.note_file_modified("src/lib.rs");
        session.note_command("cargo test --quiet");
        session.note_error("boom");
        session.note_highlight("hello world");
        session.note_codex_session_started();
        session.note_lifecycle_event("approval_requested", "needs approval before writing files");
        session.note_lifecycle_event("agent-turn-complete", "turn wrapped");
        session.note_raw_excerpt_line("type: session_meta");

        assert_eq!(session.runtime(), SessionRuntime::Codex);
        assert_eq!(session.session_id(), Some("sess-1"));
        assert_eq!(session.project(), Some("demo-project"));
        assert_eq!(session.message_count(), 1);
        assert!(session.files_modified().contains("src/lib.rs"));
        assert_eq!(session.commands_run(), &["cargo test --quiet".to_string()]);
        assert_eq!(session.errors(), &["boom".to_string()]);
        assert_eq!(session.highlights(), &["hello world".to_string()]);
        assert_eq!(
            session.lifecycle_events(),
            &[
                "approval-requested: needs approval before writing files".to_string(),
                "agent-turn-complete: turn wrapped".to_string()
            ]
        );
        assert_eq!(
            session.codex_lifecycle_state(),
            Some(&CodexLifecycleState {
                session_started: true,
                session_ended: false,
                turns_completed: 1,
                approvals_requested: 1,
                approvals_resolved: 0,
                pending_approvals: 1,
                last_event: Some("agent-turn-complete".to_string()),
            })
        );
        assert_eq!(session.raw_excerpt(), &["type: session_meta".to_string()]);
    }

    #[test]
    fn test_summarize_codex_lifecycle_state_mentions_key_transitions() {
        let summary = summarize_codex_lifecycle_state(&CodexLifecycleState {
            session_started: true,
            session_ended: true,
            turns_completed: 2,
            approvals_requested: 1,
            approvals_resolved: 1,
            pending_approvals: 0,
            last_event: Some("session-ended".to_string()),
        });

        assert!(summary.contains("phase session-ended"));
        assert!(summary.contains("session started"));
        assert!(summary.contains("2 turn(s) completed"));
        assert!(summary.contains("1 approval request(s)"));
        assert!(summary.contains("1 approval decision(s)"));
        assert!(summary.contains("session ended"));
        assert!(summary.contains("last event session-ended"));
    }

    #[test]
    fn test_codex_lifecycle_phase_tracks_pending_approvals() {
        let mut session = NormalizedSession::new(SessionRuntime::Codex);
        session.note_codex_session_started();
        session.note_lifecycle_event("approval-requested", "needs approval");
        assert_eq!(
            session
                .codex_lifecycle_state()
                .and_then(codex_lifecycle_phase),
            Some(CodexLifecyclePhase::AwaitingApproval)
        );

        session.note_lifecycle_event("approval-approved", "approved");
        assert_eq!(
            session
                .codex_lifecycle_state()
                .and_then(codex_lifecycle_phase),
            Some(CodexLifecyclePhase::ApprovalResolved)
        );
        assert_eq!(
            session.codex_lifecycle_state(),
            Some(&CodexLifecycleState {
                session_started: true,
                session_ended: false,
                turns_completed: 0,
                approvals_requested: 1,
                approvals_resolved: 1,
                pending_approvals: 0,
                last_event: Some("approval-approved".to_string()),
            })
        );
    }

    #[test]
    fn test_record_codex_lifecycle_event_returns_normalized_note() {
        let mut session = NormalizedSession::new(SessionRuntime::Codex);
        let recorded = session
            .record_codex_lifecycle_event("approval_requested", "needs approval")
            .unwrap();

        assert_eq!(recorded.normalized_event_type, "approval-requested");
        assert_eq!(recorded.note, "approval-requested: needs approval");
        assert_eq!(
            session.codex_lifecycle_state_keyword().as_deref(),
            Some("state:awaiting-approval")
        );
        assert_eq!(
            session.codex_lifecycle_state_summary().as_deref(),
            Some(
                "phase awaiting-approval · session started · 1 approval request(s) · 1 pending approval(s) · last event approval-requested"
            )
        );
    }

    #[test]
    fn test_normalize_codex_event_type_canonicalizes_session_and_tool_variants() {
        assert_eq!(
            normalize_codex_event_type("session_start"),
            "session-started"
        );
        assert_eq!(normalize_codex_event_type("session end"), "session-ended");
        assert_eq!(normalize_codex_event_type("tool_use"), "tool-use");
        assert_eq!(normalize_codex_event_type("tool result"), "tool-result");
    }
}
