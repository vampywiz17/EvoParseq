//! Bounded already-framed application input. No second input mapper.
pub(crate) const RING_DEPTH: usize = 1024;
#[derive(Debug, Clone, Copy)]
pub(crate) struct Request {
    pub bytes: [u8; 256],
    pub len: u16,
}
