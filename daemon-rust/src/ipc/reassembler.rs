use tracing::warn;

pub const MAX_UTTERANCE_CAP: usize = 960 * 1024; // 960 KiB (~30s of 16 kHz 16-bit mono PCM)

#[derive(Default, Debug)]
pub struct PcmReassembler {
    active_turn_id: Option<u32>,
    overflowed_turn_id: Option<u32>,
    buffer: Vec<u8>,
}

impl PcmReassembler {
    pub fn new() -> Self {
        Self {
            active_turn_id: None,
            overflowed_turn_id: None,
            buffer: Vec::with_capacity(64 * 1024),
        }
    }

    /// Feeds an incoming PCM frame into the reassembler.
    ///
    /// - If `chunk.is_empty()`, marks EOF for the specified `turn_id`.
    ///   If valid and not overflowed, returns the complete reassembled PCM buffer.
    /// - If `turn_id` does not match the active turn, stale buffers are discarded.
    /// - If total buffered bytes exceed 960 KiB, the buffer is dropped, `overflowed_turn_id`
    ///   is set, and all subsequent chunks and EOF for that turn_id are ignored.
    pub fn handle_frame(&mut self, turn_id: u32, chunk: &[u8]) -> Option<Vec<u8>> {
        if turn_id == 0 {
            // Audio frames must have a non-zero monotonic turn_id
            return None;
        }

        // If this turn_id previously overflowed, ignore all subsequent chunks and EOF
        if self.overflowed_turn_id == Some(turn_id) {
            return None;
        }

        // If a new turn_id arrives, clear prior overflow tracking
        if self.overflowed_turn_id.is_some() && self.overflowed_turn_id != Some(turn_id) {
            self.overflowed_turn_id = None;
        }

        // EOF Frame (0-byte payload)
        if chunk.is_empty() {
            if self.active_turn_id == Some(turn_id) {
                let completed = std::mem::take(&mut self.buffer);
                self.active_turn_id = None;

                if completed.is_empty() {
                    return None;
                }
                return Some(completed);
            }
            return None;
        }

        // Stale turn_id replacement: discard stale buffer if mid-stream turn changes
        if self.active_turn_id != Some(turn_id) {
            if self.active_turn_id.is_some() {
                warn!(
                    "Discarding incomplete audio buffer for turn_id={:?} (superseded by turn_id={})",
                    self.active_turn_id, turn_id
                );
            }
            self.active_turn_id = Some(turn_id);
            self.buffer.clear();
        }

        // Enforce hard cap (960 KiB)
        if self.buffer.len().saturating_add(chunk.len()) > MAX_UTTERANCE_CAP {
            warn!(
                "Audio utterance for turn_id={} exceeded max cap ({} bytes); discarding to prevent OOM",
                turn_id, MAX_UTTERANCE_CAP
            );
            self.buffer.clear();
            self.active_turn_id = None;
            self.overflowed_turn_id = Some(turn_id);
            return None;
        }

        self.buffer.extend_from_slice(chunk);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multi_chunk_reassembly() {
        let mut reassembler = PcmReassembler::new();
        let turn_id = 1;

        let chunk1 = vec![1u8; 32 * 1024];
        let chunk2 = vec![2u8; 32 * 1024];
        let chunk3 = vec![3u8; 16 * 1024];

        assert!(reassembler.handle_frame(turn_id, &chunk1).is_none());
        assert!(reassembler.handle_frame(turn_id, &chunk2).is_none());
        assert!(reassembler.handle_frame(turn_id, &chunk3).is_none());

        // EOF frame (0 bytes) triggers completion
        let result = reassembler.handle_frame(turn_id, &[]).expect("Reassembly complete");
        assert_eq!(result.len(), (32 + 32 + 16) * 1024);
        assert_eq!(&result[..32 * 1024], &chunk1[..]);
        assert_eq!(&result[32 * 1024..64 * 1024], &chunk2[..]);
        assert_eq!(&result[64 * 1024..], &chunk3[..]);
    }

    #[test]
    fn test_stale_turn_id_replacement() {
        let mut reassembler = PcmReassembler::new();

        // Feed chunks for turn 1
        let chunk_turn1 = vec![0xAA; 1024];
        assert!(reassembler.handle_frame(1, &chunk_turn1).is_none());

        // New turn 2 arrives before turn 1 EOF
        let chunk_turn2 = vec![0xBB; 2048];
        assert!(reassembler.handle_frame(2, &chunk_turn2).is_none());

        // EOF for turn 2
        let result = reassembler.handle_frame(2, &[]).expect("Turn 2 completed");
        assert_eq!(result.len(), 2048);
        assert_eq!(result[0], 0xBB);
    }

    #[test]
    fn test_cap_rejection_over_960kib() {
        let mut reassembler = PcmReassembler::new();
        let turn_id = 5;

        // Feed 15 chunks of 64 KiB = 960 KiB (at limit)
        let chunk_64k = vec![0x11; 64 * 1024];
        for _ in 0..15 {
            assert!(reassembler.handle_frame(turn_id, &chunk_64k).is_none());
        }

        // 16th chunk pushes it over 960 KiB -> must reject
        let extra_chunk = vec![0x22; 1024];
        assert!(reassembler.handle_frame(turn_id, &extra_chunk).is_none());

        // EOF after cap breach returns None
        assert!(reassembler.handle_frame(turn_id, &[]).is_none());
    }

    #[test]
    fn test_overflow_tail_leak_prevention() {
        let mut reassembler = PcmReassembler::new();
        let turn_id = 42;

        // Feed up to 960 KiB
        let chunk_64k = vec![0x33; 64 * 1024];
        for _ in 0..15 {
            assert!(reassembler.handle_frame(turn_id, &chunk_64k).is_none());
        }

        // Overflow chunk
        assert!(reassembler.handle_frame(turn_id, &vec![0x44; 1024]).is_none());

        // Subsequent chunks arriving for the overflowed turn_id MUST be ignored
        assert!(reassembler.handle_frame(turn_id, &vec![0x55; 2048]).is_none());
        assert!(reassembler.handle_frame(turn_id, &vec![0x66; 4096]).is_none());

        // EOF for the overflowed turn MUST return None (no partial utterance tail)
        assert!(reassembler.handle_frame(turn_id, &[]).is_none());

        // Subsequent new turn_id functions normally
        let new_turn = 43;
        let valid_chunk = vec![0x77; 1024];
        assert!(reassembler.handle_frame(new_turn, &valid_chunk).is_none());
        let res = reassembler.handle_frame(new_turn, &[]).expect("New turn succeeds");
        assert_eq!(res.len(), 1024);
        assert_eq!(res[0], 0x77);
    }

    #[test]
    fn test_empty_utterance_returns_none() {
        let mut reassembler = PcmReassembler::new();
        assert!(reassembler.handle_frame(10, &[]).is_none());
    }
}
