//! The words one frame's draws use: their records and slices, and the words several draws share.
use core::hash::{BuildHasherDefault, Hasher};
use std::collections::HashMap;

/// Hashes the small, already-distinct integers that key shared words, without mixing them.
#[derive(Default)]
pub struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(byte);
        }
    }

    fn write_usize(&mut self, i: usize) {
        self.0 = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

/// Identifies a set of shared words: the number of the thing they describe, and which of its forms they hold.
pub type Key = (usize, bool);

/// Words addressed by the draws of a frame, which are uploaded to the GPU together.
///
/// Draw records and slices change every frame. Shared words, such as text outlines, are written once per frame by
/// whichever draw first needs them, and mostly come out the same from frame to frame, so uploading them is skipped.
#[derive(Default)]
pub struct Arena {
    /// Draw records and uploaded slices.
    pub words: Vec<u32>,
    /// Words that several draws share, written with [`resident`](Self::resident).
    pub shared: Vec<u32>,
    resident: HashMap<Key, u32, BuildHasherDefault<KeyHasher>>,
}

impl Arena {
    /// Starts a new frame, forgetting everything written in the last one.
    pub fn begin_frame(&mut self) {
        self.words.clear();
        self.shared.clear();
        self.resident.clear();
    }

    /// Writes shared words once per frame for a `key`, returning what `write` returned the first time. `write`
    /// appends to [`shared`](Self::shared), and may itself call this for other keys.
    pub fn resident(&mut self, key: Key, write: impl FnOnce(&mut Self) -> u32) -> u32 {
        if let Some(&at) = self.resident.get(&key) {
            return at;
        }
        let at = write(self);
        self.resident.insert(key, at);
        at
    }
}
