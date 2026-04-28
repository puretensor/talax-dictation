/// Fixed-size circular buffer for i16 audio samples.
///
/// Stores PCM samples in a contiguous ring. When the buffer is full,
/// new samples overwrite the oldest data. Designed for pre-roll buffering
/// where we want to keep the most recent N samples available.
pub struct RingBuffer {
    buf: Vec<i16>,
    capacity: usize,
    /// Write position (next slot to write into).
    head: usize,
    /// Number of valid samples currently stored.
    len: usize,
}

impl RingBuffer {
    /// Create a new ring buffer that holds at most `capacity` samples.
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0i16; capacity],
            capacity,
            head: 0,
            len: 0,
        }
    }

    /// Append a chunk of samples. If the chunk (combined with existing data)
    /// exceeds capacity, the oldest samples are silently overwritten.
    pub fn push_chunk(&mut self, samples: &[i16]) {
        if self.capacity == 0 {
            return;
        }

        // If the incoming chunk is larger than the entire buffer, only keep
        // the trailing `capacity` samples.
        if samples.len() >= self.capacity {
            let start = samples.len() - self.capacity;
            self.buf.copy_from_slice(&samples[start..]);
            self.head = 0;
            self.len = self.capacity;
            return;
        }

        for &s in samples {
            self.buf[self.head] = s;
            self.head = (self.head + 1) % self.capacity;
            if self.len < self.capacity {
                self.len += 1;
            }
        }
    }

    /// Drain all stored samples in chronological order and reset the buffer.
    pub fn drain(&mut self) -> Vec<i16> {
        if self.len == 0 {
            return Vec::new();
        }

        let mut out = Vec::with_capacity(self.len);

        // The oldest sample sits at (head - len) mod capacity.
        let start = if self.head >= self.len {
            self.head - self.len
        } else {
            self.capacity - (self.len - self.head)
        };

        for i in 0..self.len {
            out.push(self.buf[(start + i) % self.capacity]);
        }

        self.head = 0;
        self.len = 0;
        out
    }

    /// Number of valid samples currently in the buffer.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer contains no samples.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Maximum number of samples the buffer can hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_is_empty() {
        let rb = RingBuffer::new(100);
        assert!(rb.is_empty());
        assert_eq!(rb.len(), 0);
        assert_eq!(rb.capacity(), 100);
    }

    #[test]
    fn push_and_drain_basic() {
        let mut rb = RingBuffer::new(10);
        rb.push_chunk(&[1, 2, 3, 4, 5]);
        assert_eq!(rb.len(), 5);
        assert!(!rb.is_empty());

        let out = rb.drain();
        assert_eq!(out, vec![1, 2, 3, 4, 5]);
        assert!(rb.is_empty());
    }

    #[test]
    fn push_exactly_fills_capacity() {
        let mut rb = RingBuffer::new(4);
        rb.push_chunk(&[10, 20, 30, 40]);
        assert_eq!(rb.len(), 4);
        assert_eq!(rb.drain(), vec![10, 20, 30, 40]);
    }

    #[test]
    fn overflow_keeps_newest() {
        let mut rb = RingBuffer::new(4);
        rb.push_chunk(&[1, 2, 3, 4]);
        rb.push_chunk(&[5, 6]);
        // Oldest (1, 2) should have been overwritten.
        assert_eq!(rb.len(), 4);
        assert_eq!(rb.drain(), vec![3, 4, 5, 6]);
    }

    #[test]
    fn large_chunk_exceeding_capacity() {
        let mut rb = RingBuffer::new(3);
        rb.push_chunk(&[10, 20, 30, 40, 50, 60]);
        // Only the last 3 samples should survive.
        assert_eq!(rb.len(), 3);
        assert_eq!(rb.drain(), vec![40, 50, 60]);
    }

    #[test]
    fn multiple_push_drain_cycles() {
        let mut rb = RingBuffer::new(5);
        rb.push_chunk(&[1, 2, 3]);
        assert_eq!(rb.drain(), vec![1, 2, 3]);

        rb.push_chunk(&[4, 5, 6, 7]);
        assert_eq!(rb.drain(), vec![4, 5, 6, 7]);

        // After two drains the buffer should be clean.
        assert!(rb.is_empty());
        assert_eq!(rb.drain(), Vec::<i16>::new());
    }

    #[test]
    fn wrap_around_correctness() {
        let mut rb = RingBuffer::new(4);
        // Fill partially, drain, then fill again to force wrap-around.
        rb.push_chunk(&[1, 2, 3]);
        let _ = rb.drain();
        // head is now at index 3. Push 5 samples to wrap.
        rb.push_chunk(&[10, 20, 30, 40, 50]);
        // Only last 4 survive: 20, 30, 40, 50
        assert_eq!(rb.drain(), vec![20, 30, 40, 50]);
    }

    #[test]
    fn zero_capacity_buffer() {
        let mut rb = RingBuffer::new(0);
        rb.push_chunk(&[1, 2, 3]);
        assert!(rb.is_empty());
        assert_eq!(rb.drain(), Vec::<i16>::new());
    }
}
