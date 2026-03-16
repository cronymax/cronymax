//! Business-agnostic render scheduling.
//!
//! Decouples frame-rate management (coalescing, timers, ControlFlow)
//! from application-specific event sources (PTY, webview, cursor blink).

use std::time::{Duration, Instant};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::Window;

/// Outcome of a scheduler poll.
pub struct SchedulerPoll {
    /// Whether the caller should call `window.request_redraw()`.
    pub should_redraw: bool,
    /// The ControlFlow the event loop should adopt after this tick.
    pub control_flow: ControlFlow,
}

/// Business-agnostic render scheduling.
///
/// Decouples frame-rate management (coalescing, timers, ControlFlow)
/// from application-specific event sources (PTY, webview, cursor blink).
pub trait RenderSchedule {
    /// Signal that a redraw is needed before the next frame.
    /// Idempotent — multiple calls between polls are coalesced.
    fn mark_dirty(&mut self);

    /// Register a timer wake-up at `deadline`.
    /// The scheduler will return `WaitUntil(min(deadlines))` from `poll()`.
    fn schedule_at(&mut self, deadline: Instant);

    /// Schedule a deferred repaint after `delay`.
    /// Takes the min of any existing deferred deadline and the new one.
    fn schedule_repaint_after(&mut self, delay: Duration);

    /// Evaluate pending state and decide whether to render now.
    ///
    /// Frame coalescing: if dirty but too soon since last render,
    /// returns `should_redraw = false` with `WaitUntil(next_allowed)`.
    /// Also checks deferred repaint deadline; auto-marks dirty if expired.
    fn poll(&mut self, now: Instant) -> SchedulerPoll;

    /// Convenience: call `poll()`, then `request_redraw()` + `set_control_flow()`.
    fn apply(&mut self, now: Instant, window: &Window, event_loop: &ActiveEventLoop) {
        let result = self.poll(now);
        if result.should_redraw {
            window.request_redraw();
        }
        event_loop.set_control_flow(result.control_flow);
    }

    /// Clear all deadlines, dirty flag, and deferred repaint.
    fn reset(&mut self);
}

/// Default frame scheduler with configurable minimum frame interval.
pub struct FrameScheduler {
    dirty: bool,
    last_render_time: Instant,
    min_frame_interval: Duration,
    deadlines: Vec<Instant>,
    deferred_repaint: Option<Instant>,
}

impl FrameScheduler {
    pub fn new(min_frame_interval: Duration) -> Self {
        Self {
            dirty: false,
            last_render_time: Instant::now() - min_frame_interval,
            min_frame_interval,
            deadlines: Vec::new(),
            deferred_repaint: None,
        }
    }
}

impl RenderSchedule for FrameScheduler {
    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn schedule_at(&mut self, deadline: Instant) {
        self.deadlines.push(deadline);
    }

    fn schedule_repaint_after(&mut self, delay: Duration) {
        let deadline = Instant::now() + delay;
        self.deferred_repaint = Some(self.deferred_repaint.map_or(deadline, |d| d.min(deadline)));
    }

    fn poll(&mut self, now: Instant) -> SchedulerPoll {
        // Check deferred repaint deadline.
        if self.deferred_repaint.is_some_and(|d| now >= d) {
            self.dirty = true;
            self.deferred_repaint = None;
        }

        // Register deferred repaint as a timer deadline if still pending.
        if let Some(d) = self.deferred_repaint {
            self.deadlines.push(d);
        }

        // Find earliest deadline (if any) before draining.
        let earliest_deadline = self.deadlines.iter().copied().min();
        self.deadlines.clear();

        if self.dirty {
            let next_allowed = self.last_render_time + self.min_frame_interval;
            if now >= next_allowed {
                // Enough time has passed — render now.
                self.dirty = false;
                self.last_render_time = now;
                let control_flow = match earliest_deadline {
                    Some(d) => ControlFlow::WaitUntil(d),
                    None => ControlFlow::Wait,
                };
                SchedulerPoll {
                    should_redraw: true,
                    control_flow,
                }
            } else {
                // Too soon — schedule wake at frame boundary.
                let wake = match earliest_deadline {
                    Some(d) => next_allowed.min(d),
                    None => next_allowed,
                };
                SchedulerPoll {
                    should_redraw: false,
                    control_flow: ControlFlow::WaitUntil(wake),
                }
            }
        } else {
            // Not dirty.
            let control_flow = match earliest_deadline {
                Some(d) => ControlFlow::WaitUntil(d),
                None => ControlFlow::Wait,
            };
            SchedulerPoll {
                should_redraw: false,
                control_flow,
            }
        }
    }

    fn reset(&mut self) {
        self.dirty = false;
        self.deadlines.clear();
        self.deferred_repaint = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_dirty_no_deadlines_returns_wait() {
        let mut s = FrameScheduler::new(Duration::from_millis(4));
        let result = s.poll(Instant::now());
        assert!(!result.should_redraw);
        assert!(matches!(result.control_flow, ControlFlow::Wait));
    }

    #[test]
    fn dirty_after_interval_returns_redraw() {
        let mut s = FrameScheduler::new(Duration::from_millis(4));
        s.mark_dirty();
        // last_render_time is initialized to now - interval, so immediate poll should work.
        let result = s.poll(Instant::now());
        assert!(result.should_redraw);
    }

    #[test]
    fn mark_dirty_is_idempotent() {
        let mut s = FrameScheduler::new(Duration::from_millis(4));
        s.mark_dirty();
        s.mark_dirty();
        s.mark_dirty();
        let result = s.poll(Instant::now());
        assert!(result.should_redraw);
        // After poll, dirty is cleared.
        let result2 = s.poll(Instant::now());
        assert!(!result2.should_redraw);
    }

    #[test]
    fn schedule_at_sets_wait_until() {
        let mut s = FrameScheduler::new(Duration::from_millis(4));
        let deadline = Instant::now() + Duration::from_secs(1);
        s.schedule_at(deadline);
        let result = s.poll(Instant::now());
        assert!(!result.should_redraw);
        match result.control_flow {
            ControlFlow::WaitUntil(d) => assert_eq!(d, deadline),
            _ => panic!("Expected WaitUntil"),
        }
    }

    #[test]
    fn reset_clears_state() {
        let mut s = FrameScheduler::new(Duration::from_millis(4));
        s.mark_dirty();
        s.schedule_at(Instant::now() + Duration::from_secs(1));
        s.reset();
        let result = s.poll(Instant::now());
        assert!(!result.should_redraw);
        assert!(matches!(result.control_flow, ControlFlow::Wait));
    }

    #[test]
    fn schedule_repaint_after_sets_deferred_deadline() {
        let mut s = FrameScheduler::new(Duration::from_millis(4));
        s.schedule_repaint_after(Duration::from_millis(100));
        // Deferred is in the future — poll should NOT mark dirty yet.
        let result = s.poll(Instant::now());
        assert!(!result.should_redraw);
        // But it should set WaitUntil for the deferred deadline.
        assert!(matches!(result.control_flow, ControlFlow::WaitUntil(_)));
    }

    #[test]
    fn poll_auto_marks_dirty_when_deferred_expires() {
        let mut s = FrameScheduler::new(Duration::from_millis(4));
        // Set deferred repaint in the past so it fires immediately.
        s.deferred_repaint = Some(Instant::now() - Duration::from_millis(1));
        let result = s.poll(Instant::now());
        assert!(result.should_redraw);
    }

    #[test]
    fn poll_includes_deferred_in_wait_until_when_pending() {
        let mut s = FrameScheduler::new(Duration::from_millis(4));
        let future = Instant::now() + Duration::from_secs(2);
        s.deferred_repaint = Some(future);
        let result = s.poll(Instant::now());
        assert!(!result.should_redraw);
        match result.control_flow {
            ControlFlow::WaitUntil(d) => assert_eq!(d, future),
            _ => panic!("Expected WaitUntil for deferred repaint"),
        }
    }

    #[test]
    fn schedule_repaint_after_takes_min_of_existing() {
        let mut s = FrameScheduler::new(Duration::from_millis(4));
        s.schedule_repaint_after(Duration::from_secs(10));
        let first = s.deferred_repaint.unwrap();
        // Schedule a shorter delay — should take the min.
        s.schedule_repaint_after(Duration::from_millis(50));
        let second = s.deferred_repaint.unwrap();
        assert!(second < first);
    }

    #[test]
    fn reset_clears_deferred_repaint() {
        let mut s = FrameScheduler::new(Duration::from_millis(4));
        s.schedule_repaint_after(Duration::from_millis(100));
        assert!(s.deferred_repaint.is_some());
        s.reset();
        assert!(s.deferred_repaint.is_none());
        let result = s.poll(Instant::now());
        assert!(!result.should_redraw);
        assert!(matches!(result.control_flow, ControlFlow::Wait));
    }
}
