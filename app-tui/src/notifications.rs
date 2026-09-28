//! Bottom-right transient notification stack. Conceptually the same idea
//! as `app-egui/`'s `ToastQueue` (`design::components::ToastQueue`),
//! rebuilt here since that code is `egui::Ui`-bound - only the *idea* (a
//! queue of timed, dismissible notices) is reused, not the code.

use std::time::{Duration, Instant};

use crate::theme::StatusTone;

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub tone: StatusTone,
    expires_at: Instant,
}

#[derive(Debug, Clone, Default)]
pub struct NotificationQueue {
    toasts: Vec<Toast>,
}

impl NotificationQueue {
    pub fn push(&mut self, message: impl Into<String>, tone: StatusTone) {
        self.toasts.push(Toast {
            message: message.into(),
            tone,
            expires_at: Instant::now() + Duration::from_secs(4),
        });
    }

    /// Drops expired toasts. Called once per `Tick` from the main loop,
    /// not from render, so rendering stays a pure read of current state.
    pub fn expire(&mut self) {
        let now = Instant::now();
        self.toasts.retain(|t| t.expires_at > now);
    }

    pub fn visible(&self) -> &[Toast] {
        &self.toasts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_toasts_are_dropped() {
        let mut queue = NotificationQueue::default();
        queue.push("hello", StatusTone::Info);
        assert_eq!(queue.visible().len(), 1);
        queue.toasts[0].expires_at = Instant::now() - Duration::from_secs(1);
        queue.expire();
        assert!(queue.visible().is_empty());
    }

    #[test]
    fn unexpired_toasts_survive_expire() {
        let mut queue = NotificationQueue::default();
        queue.push("still here", StatusTone::Success);
        queue.expire();
        assert_eq!(queue.visible().len(), 1);
    }
}
