//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    /// Post a notification that lingers briefly above the statusline.
    ///
    /// Consecutive duplicates are ignored so a burst of work does not spam the
    /// screen, and only the most recent few are kept.
    pub fn push_toast(&mut self, kind: ToastKind, message: impl Into<String>) {
        const MAX: usize = 4;
        let message = message.into();
        if self
            .toasts
            .last()
            .is_some_and(|toast| toast.message == message && toast.kind == kind)
        {
            return;
        }
        self.toasts.push(Toast {
            message,
            kind,
            created: Instant::now(),
        });
        if self.toasts.len() > MAX {
            self.toasts.remove(0);
        }
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status.message = message.into();
        self.status.error = false;
        self.status.expires_at = Some(Instant::now() + Duration::from_secs(4));
    }

    /// Expire stale status messages and notifications.
    ///
    /// Returns `true` when something was cleared, so the caller knows to redraw.
    pub(super) fn tick_status(&mut self) -> bool {
        const TOAST_TTL: Duration = Duration::from_secs(5);
        let mut changed = false;

        let before = self.toasts.len();
        self.toasts
            .retain(|toast| toast.created.elapsed() < TOAST_TTL);
        if self.toasts.len() != before {
            changed = true;
        }

        if let Some(expires_at) = self.status.expires_at
            && Instant::now() >= expires_at
        {
            self.status.message.clear();
            self.status.error = false;
            self.status.expires_at = None;
            changed = true;
        }
        changed
    }

    /// The message shown in the status bar.
    pub fn status_message(&self) -> Option<&str> {
        if self.status.message.is_empty() {
            None
        } else {
            Some(self.status.message.as_str())
        }
    }

    /// A short label for background work in progress, if any.
    pub fn busy(&self) -> Option<&'static str> {
        if self.pending_format.is_some() || self.pending_lsp_format.is_some() {
            Some("formatting")
        } else if self.pending_workspace_symbols.is_some() {
            Some("searching symbols")
        } else if self.pending_project_search.is_some() {
            Some("searching project")
        } else if self.pending_install.is_some() {
            Some("installing")
        } else if self.pending_commit {
            Some("committing")
        } else if self.pending_project {
            Some("creating project")
        } else if self.lsp_status() == LspStatus::Starting {
            Some("connecting")
        } else {
            None
        }
    }

    /// Clear any transient status message.
    pub fn clear_status(&mut self) {
        self.status.message.clear();
        self.status.error = false;
        self.status.expires_at = None;
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.status.message = message.into();
        self.status.error = true;
        self.status.expires_at = Some(Instant::now() + Duration::from_secs(8));
    }
}
