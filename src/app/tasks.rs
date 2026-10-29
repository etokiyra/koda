//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.
//!
//! Run and test tasks: the project's own commands, detected from its markers
//! and run with bounded output — no configuration, no per-project files.

use super::*;
use crate::app::overlay::TaskRunState;
use crate::background::TaskRunOutcome;
use crate::project::tasks::{self, Task};

impl App {
    /// Open the task picker for the current project.
    pub(super) fn open_task_picker(&mut self) {
        let root = self.workspace.root().to_path_buf();
        let tasks = tasks::detect(&root);
        if tasks.is_empty() {
            self.set_status("No tasks detected for this project");
            return;
        }
        let items = tasks
            .into_iter()
            .map(|task| {
                PickerItem::new(
                    task.label.clone(),
                    task.command_line(),
                    PickerAction::RunTask(task),
                )
                .shortcut("Enter")
            })
            .collect();
        let mut picker = Picker::new("Tasks", "Run a project task…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Start a task on the background worker.
    ///
    /// One task at a time: the result event opens the output view, and a second
    /// run before that would be invisible.
    pub(super) fn run_task(&mut self, task: Task) {
        if self.pending_task {
            self.set_status("A task is already running");
            return;
        }
        self.pending_task = true;
        self.background
            .run_task(self.workspace.root().to_path_buf(), task.clone());
        self.set_status(format!("Running {}…", task.command_line()));
    }

    /// Show the captured output of a finished task.
    pub(super) fn show_task_run(&mut self, outcome: TaskRunOutcome) {
        let TaskRunOutcome {
            task,
            exit_code,
            timed_out,
            spawn_error,
            text,
            truncated,
        } = outcome;
        if let Some(error) = spawn_error {
            self.set_error(error.clone());
            self.push_toast(ToastKind::Error, error);
            return;
        }
        let (success, verdict) = if timed_out {
            (false, "killed — ran past the deadline".to_string())
        } else if exit_code == Some(0) {
            (true, "success".to_string())
        } else {
            match exit_code {
                Some(code) => (false, format!("exit code {code}")),
                None => (false, "terminated by a signal".to_string()),
            }
        };
        let text = if truncated {
            format!("{}\n… output truncated at the cap", text.trim_end())
        } else if text.is_empty() {
            "(no output)".to_string()
        } else {
            text
        };
        self.overlay = Overlay::TaskRun(TaskRunState::from_output(
            self.workspace.root(),
            task.label.clone(),
            &task.command_line(),
            &verdict,
            success,
            &text,
        ));
    }
}
