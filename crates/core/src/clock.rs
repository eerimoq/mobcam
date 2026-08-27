const PTS_DISCONTINUITY_US: u64 = 5 * 1000 * 1000;

#[derive(Default)]
pub struct Clock {
    anchored: bool,
    first_pts_us: u64,
    previous_pts_us: u64,
    anchor_ns: u64,
}

impl Clock {
    pub fn timestamp(&mut self, pts_us: u64, now_ns: impl FnOnce() -> u64) -> u64 {
        let distance = pts_us.abs_diff(self.previous_pts_us);
        if !self.anchored || distance > PTS_DISCONTINUITY_US {
            self.anchored = true;
            self.first_pts_us = pts_us;
            self.anchor_ns = now_ns();
        } else if pts_us < self.first_pts_us {
            self.anchor_ns -= (self.first_pts_us - pts_us) * 1000;
            self.first_pts_us = pts_us;
        }
        self.previous_pts_us = pts_us;
        self.anchor_ns + (pts_us - self.first_pts_us) * 1000
    }
}
