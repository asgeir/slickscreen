use std::time::Instant;

#[derive(Copy, Clone, Debug)]
pub(crate) struct SlickscreenTime {
    reference: Instant,
}

impl SlickscreenTime {
    pub fn new(reference: Instant) -> Self {
        SlickscreenTime { reference }
    }

    #[inline]
    pub fn pts_now(&self) -> i64 {
        (self.reference.elapsed().as_micros() & (i64::MAX as u128)) as i64
    }
}
