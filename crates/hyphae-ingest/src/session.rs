use std::collections::HashSet;

use crate::transcript::SessionRuntime;

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
        session.note_raw_excerpt_line("type: session_meta");

        assert_eq!(session.runtime(), SessionRuntime::Codex);
        assert_eq!(session.session_id(), Some("sess-1"));
        assert_eq!(session.project(), Some("demo-project"));
        assert_eq!(session.message_count(), 1);
        assert!(session.files_modified().contains("src/lib.rs"));
        assert_eq!(session.commands_run(), &["cargo test --quiet".to_string()]);
        assert_eq!(session.errors(), &["boom".to_string()]);
        assert_eq!(session.highlights(), &["hello world".to_string()]);
        assert_eq!(session.raw_excerpt(), &["type: session_meta".to_string()]);
    }
}
