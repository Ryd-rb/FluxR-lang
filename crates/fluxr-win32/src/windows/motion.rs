//! Time-based coarse-wheel motion. Millisecond inputs make trajectories testable
//! without clocks, sleeps, desktop input or raster inspection.
#[derive(Default, Debug)]
pub(super) struct Motion {
    start: f64,
    target: f64,
    current: f64,
    started: u64,
    active: bool,
}
impl Motion {
    pub(super) fn active(&self) -> bool {
        self.active
    }
    pub(super) fn target(&self) -> i32 {
        self.target.round() as i32
    }
    pub(super) fn stop(&mut self, current: i32) {
        self.start = current as f64;
        self.current = self.start;
        self.target = self.start;
        self.active = false;
    }
    pub(super) fn request(&mut self, current: i32, target: i32, now: u64, smooth: bool) {
        self.start = current as f64;
        self.current = self.start;
        self.target = target as f64;
        self.started = now;
        self.active = smooth && current != target;
        if !self.active {
            self.current = self.target;
        }
    }
    pub(super) fn sample(&mut self, now: u64) -> i32 {
        if self.active {
            let t = now.saturating_sub(self.started) as f64 / 120.0;
            if t >= 1.0 {
                self.current = self.target;
                self.active = false;
            } else {
                self.current = self.start + (self.target - self.start) * (1.0 - (1.0 - t).powi(3));
            }
        }
        self.current.round() as i32
    }
    pub(super) fn shift(&mut self, delta: i32, limit: i32) {
        let d = delta as f64;
        let max = limit.max(0) as f64;
        self.start = (self.start + d).clamp(0.0, max);
        self.current = (self.current + d).clamp(0.0, max);
        self.target = (self.target + d).clamp(0.0, max);
        if (self.target - self.current).abs() < 0.5 {
            self.active = false;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_trajectories_are_monotone_exact_and_reversible() {
        let mut m = Motion::default();
        m.request(400, 100, 1000, true);
        let mut previous = 400;
        for now in (1000..1120).step_by(7) {
            let at = m.sample(now);
            assert!((100..=previous).contains(&at));
            previous = at;
        }
        assert_eq!(m.sample(1120), 100);
        assert!(!m.active());
        assert_eq!(m.sample(2000), 100);
        m.request(100, 500, 2000, true);
        let middle = m.sample(2040);
        assert!(middle > 100 && middle < 500);
        m.request(middle, middle - 80, 2040, true);
        let mut previous = middle;
        for now in (2040..2160).step_by(9) {
            let at = m.sample(now);
            assert!(at <= previous && at >= middle - 80);
            previous = at;
        }
        assert_eq!(m.sample(2200), middle - 80);
        assert!(!m.active());
        m.request(50, 120, 3000, false);
        assert_eq!(m.sample(3000), 120);
        assert!(!m.active());
        m.request(100, 200, 4000, true);
        m.sample(4030);
        m.shift(70, 240);
        assert_eq!(m.sample(4200), 240);
        assert!(!m.active());
    }
}
