//! macOS meters (placeholder until the Mac half lands).

pub(super) struct Sampler;

impl Sampler {
    pub(super) fn new() -> Sampler {
        Sampler
    }

    pub(super) fn sample(&mut self, _out: &mut super::Sample) {}
}
