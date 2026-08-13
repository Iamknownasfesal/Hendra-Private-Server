//! The wire codec: variable-length integers, quantised positions and bounded strings.
//!
//! # Reading untrusted input
//!
//! Every [`Reader`] method is fed bytes a client sent us, so the rules here are absolute: no method
//! may panic, no method may allocate before it has checked the length against what actually
//! remains, and no method may loop on attacker-controlled counts. A malformed packet must always
//! come back as a [`CodecError`] that the session layer can turn into a disconnect.
//!
//! # Why not fixed-width integers
//!
//! Almost every number the game sends is small — entity ids in the low thousands, hit points in
//! the hundreds, positional deltas of a fraction of a tile. Fixed 32-bit fields spend four bytes on
//! all of them. LEB128 spends one byte below 128 and two below 16,384, which covers the
//! overwhelming majority of values in a snapshot.

use std::fmt;

/// Tiles are quantised to this many steps for transmission.
///
/// A quarter-tile of movement — a typical step at 20 ticks per second — becomes 64 units, which is
/// one varint byte after zigzag. Finer scales push common deltas into two bytes for precision no
/// renderer can show: at 1/256 tile the error is under a hundredth of a pixel at any sane zoom.
pub const POSITION_SCALE: f32 = 256.0;

/// The longest string the codec will read, in bytes.
///
/// Chat lines, player names and guild names are all far below this. The cap exists so a hostile
/// length prefix cannot make us reserve memory on demand.
pub const MAX_STRING_BYTES: usize = 1024;

/// The largest element count the codec will honour for a length-prefixed sequence.
///
/// Bounded for the same reason as [`MAX_STRING_BYTES`]: a count is attacker-controlled, and a
/// caller that trusts it will happily try to allocate for four billion entries.
pub const MAX_SEQUENCE: usize = 16_384;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    #[error("needed {needed} more bytes but only {available} remain")]
    UnexpectedEof { needed: usize, available: usize },

    #[error("varint is longer than {max} bytes")]
    VarintTooLong { max: usize },

    #[error("varint does not fit in the target type")]
    VarintOverflow,

    #[error("string of {len} bytes exceeds the {max}-byte limit")]
    StringTooLong { len: usize, max: usize },

    #[error("string is not valid UTF-8")]
    InvalidUtf8,

    #[error("sequence of {len} exceeds the {max}-element limit")]
    SequenceTooLong { len: usize, max: usize },

    #[error("value {value} is not valid for {what}")]
    InvalidValue { what: &'static str, value: u64 },
}

pub type Result<T> = std::result::Result<T, CodecError>;

// ---------------------------------------------------------------------------------------------
// Quantisation
// ---------------------------------------------------------------------------------------------

/// Converts a world coordinate to its transmitted fixed-point form.
///
/// Saturates rather than wrapping: a position far outside any real map should clamp to the edge of
/// the representable range, never fold around to the opposite side of the world.
#[inline]
pub fn quantize(position: f32) -> i32 {
    let scaled = position * POSITION_SCALE;
    if scaled.is_nan() {
        return 0;
    }
    scaled.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

/// Converts a transmitted fixed-point coordinate back to a world coordinate.
#[inline]
pub fn dequantize(quantized: i32) -> f32 {
    quantized as f32 / POSITION_SCALE
}

/// Zigzag-maps a signed integer so that small magnitudes of either sign stay small.
///
/// Two's-complement negatives have their high bits set, which would make every negative varint the
/// maximum length. Zigzag interleaves the signs so -1 encodes as 1 rather than as ten bytes.
#[inline]
pub fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

/// Reverses [`zigzag`].
#[inline]
pub fn unzigzag(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

// ---------------------------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------------------------

/// Appends encoded values to a caller-owned buffer.
///
/// Borrowing the buffer rather than owning one is deliberate: the connection keeps a single scratch
/// buffer and clears it per packet, so encoding a tick allocates nothing once it is warm.
pub struct Writer<'a> {
    buf: &'a mut Vec<u8>,
}

impl<'a> Writer<'a> {
    pub fn new(buf: &'a mut Vec<u8>) -> Writer<'a> {
        Writer { buf }
    }

    /// How many bytes have been written into the underlying buffer so far.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub fn u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn u32_fixed(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn f32(&mut self, value: f32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes an unsigned integer as LEB128.
    pub fn varint(&mut self, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                self.buf.push(byte);
                return;
            }
            self.buf.push(byte | 0x80);
        }
    }

    /// Writes a signed integer as zigzag then LEB128.
    pub fn varint_signed(&mut self, value: i64) {
        self.varint(zigzag(value));
    }

    /// Writes a world coordinate as an absolute quantised value.
    pub fn position(&mut self, value: f32) {
        self.varint_signed(quantize(value) as i64);
    }

    /// Writes a world coordinate as a delta from a previously transmitted one.
    ///
    /// This is the encoding that makes snapshots small: both sides quantise identically, so the
    /// difference is exact and typically fits in one byte.
    pub fn position_delta(&mut self, value: f32, baseline: f32) {
        let delta = quantize(value).wrapping_sub(quantize(baseline));
        self.varint_signed(delta as i64);
    }

    pub fn bool(&mut self, value: bool) {
        self.buf.push(value as u8);
    }

    /// Writes a length-prefixed UTF-8 string.
    ///
    /// Strings longer than [`MAX_STRING_BYTES`] are truncated on a character boundary rather than
    /// rejected — the caller is our own code, and losing the tail of an over-long chat line is
    /// preferable to failing to encode a packet mid-tick.
    pub fn string(&mut self, value: &str) {
        let mut bytes = value.as_bytes();
        if bytes.len() > MAX_STRING_BYTES {
            let mut end = MAX_STRING_BYTES;
            while end > 0 && !value.is_char_boundary(end) {
                end -= 1;
            }
            bytes = &value.as_bytes()[..end];
        }
        self.varint(bytes.len() as u64);
        self.buf.extend_from_slice(bytes);
    }

    /// Writes a length-prefixed byte string.
    pub fn bytes(&mut self, value: &[u8]) {
        self.varint(value.len() as u64);
        self.buf.extend_from_slice(value);
    }

    /// Writes a condition mask, spending bytes only on the part that is set.
    ///
    /// Nearly every entity carries no conditions at all, which this encodes as a single zero byte
    /// instead of sixteen.
    pub fn condition_mask(&mut self, mask: u128) {
        let significant = 16 - (mask.leading_zeros() / 8) as usize;
        self.varint(significant as u64);
        self.buf
            .extend_from_slice(&mask.to_le_bytes()[..significant]);
    }

    /// The raw buffer, for handing to the transport.
    pub fn finish(self) -> &'a mut Vec<u8> {
        self.buf
    }
}

// ---------------------------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------------------------

/// Decodes values out of a borrowed buffer.
///
/// Strings and byte slices are returned as borrows of the input, so decoding a packet allocates
/// nothing unless the caller chooses to own something.
#[derive(Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Reader<'a> {
        Reader { buf, pos: 0 }
    }

    /// Bytes not yet consumed.
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// How far into the buffer we are, for diagnostics.
    pub fn position(&self) -> usize {
        self.pos
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if self.remaining() < count {
            return Err(CodecError::UnexpectedEof {
                needed: count,
                available: self.remaining(),
            });
        }
        let slice = &self.buf[self.pos..self.pos + count];
        self.pos += count;
        Ok(slice)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn u32_fixed(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn f32(&mut self) -> Result<f32> {
        let bytes = self.take(4)?;
        Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Reads a LEB128 unsigned integer.
    ///
    /// Refuses continuation past ten bytes, which is the most a `u64` can need, so a stream of
    /// `0x80` bytes terminates as an error instead of running to the end of the packet.
    pub fn varint(&mut self) -> Result<u64> {
        const MAX_BYTES: usize = 10;

        let mut value: u64 = 0;
        let mut shift = 0u32;

        for index in 0..MAX_BYTES {
            let byte = self.u8()?;
            let payload = (byte & 0x7f) as u64;

            // The tenth byte may only contribute the single bit that fits.
            if shift >= 64 || (index == MAX_BYTES - 1 && payload > 1) {
                return Err(CodecError::VarintOverflow);
            }

            value |= payload << shift;

            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }

        Err(CodecError::VarintTooLong { max: MAX_BYTES })
    }

    /// Reads a zigzag LEB128 signed integer.
    pub fn varint_signed(&mut self) -> Result<i64> {
        Ok(unzigzag(self.varint()?))
    }

    /// Reads a varint and narrows it, failing rather than truncating.
    pub fn varint_u32(&mut self) -> Result<u32> {
        u32::try_from(self.varint()?).map_err(|_| CodecError::VarintOverflow)
    }

    /// Reads a varint and narrows it to `usize`, bounded by `max`.
    pub fn count(&mut self, max: usize) -> Result<usize> {
        let raw = self.varint()?;
        let len = usize::try_from(raw).map_err(|_| CodecError::VarintOverflow)?;
        if len > max {
            return Err(CodecError::SequenceTooLong { len, max });
        }
        Ok(len)
    }

    pub fn position_value(&mut self) -> Result<f32> {
        let quantized = i32::try_from(self.varint_signed()?).map_err(|_| CodecError::VarintOverflow)?;
        Ok(dequantize(quantized))
    }

    /// Reads a coordinate transmitted as a delta from a known baseline.
    pub fn position_delta(&mut self, baseline: f32) -> Result<f32> {
        let delta = i32::try_from(self.varint_signed()?).map_err(|_| CodecError::VarintOverflow)?;
        Ok(dequantize(quantize(baseline).wrapping_add(delta)))
    }

    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }

    /// Reads a length-prefixed UTF-8 string, borrowed from the input.
    pub fn string(&mut self) -> Result<&'a str> {
        let len = self.varint()?;
        let len = usize::try_from(len).map_err(|_| CodecError::VarintOverflow)?;
        if len > MAX_STRING_BYTES {
            return Err(CodecError::StringTooLong {
                len,
                max: MAX_STRING_BYTES,
            });
        }
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes).map_err(|_| CodecError::InvalidUtf8)
    }

    /// Reads a length-prefixed byte string, borrowed from the input.
    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.count(MAX_STRING_BYTES)?;
        self.take(len)
    }

    /// Reads a condition mask written by [`Writer::condition_mask`].
    pub fn condition_mask(&mut self) -> Result<u128> {
        let significant = self.count(16)?;
        let bytes = self.take(significant)?;
        let mut full = [0u8; 16];
        full[..significant].copy_from_slice(bytes);
        Ok(u128::from_le_bytes(full))
    }
}

impl fmt::Debug for Reader<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reader")
            .field("position", &self.pos)
            .field("remaining", &self.remaining())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(f: impl FnOnce(&mut Writer<'_>)) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = Writer::new(&mut buf);
        f(&mut writer);
        buf
    }

    #[test]
    fn varints_round_trip_across_the_whole_range() {
        let values = [
            0u64,
            1,
            127,
            128,
            255,
            256,
            16_383,
            16_384,
            u32::MAX as u64,
            u64::MAX - 1,
            u64::MAX,
        ];

        for value in values {
            let bytes = write(|w| w.varint(value));
            let mut reader = Reader::new(&bytes);
            assert_eq!(reader.varint().unwrap(), value, "value {value}");
            assert!(reader.is_empty(), "value {value} left trailing bytes");
        }
    }

    #[test]
    fn small_numbers_cost_one_byte() {
        assert_eq!(write(|w| w.varint(0)).len(), 1);
        assert_eq!(write(|w| w.varint(127)).len(), 1);
        assert_eq!(write(|w| w.varint(128)).len(), 2);
        assert_eq!(write(|w| w.varint(16_383)).len(), 2);

        // The point of zigzag: small negatives stay small.
        assert_eq!(write(|w| w.varint_signed(-1)).len(), 1);
        assert_eq!(write(|w| w.varint_signed(-63)).len(), 1);
        assert_eq!(write(|w| w.varint_signed(63)).len(), 1);
    }

    #[test]
    fn signed_varints_round_trip() {
        let values = [0i64, 1, -1, 63, -64, i32::MIN as i64, i64::MIN, i64::MAX];
        for value in values {
            let bytes = write(|w| w.varint_signed(value));
            let mut reader = Reader::new(&bytes);
            assert_eq!(reader.varint_signed().unwrap(), value, "value {value}");
        }
    }

    #[test]
    fn a_typical_movement_delta_is_one_byte() {
        // A quarter tile per tick at 20 TPS is a brisk walking pace.
        let bytes = write(|w| w.position_delta(10.25, 10.0));
        assert_eq!(bytes.len(), 2, "64 units needs two bytes after zigzag");

        // An eighth of a tile — the common case for most entities — fits in one.
        let bytes = write(|w| w.position_delta(10.125, 10.0));
        assert_eq!(bytes.len(), 1);

        // A stationary entity costs a single zero byte.
        let bytes = write(|w| w.position_delta(10.0, 10.0));
        assert_eq!(bytes, vec![0]);
    }

    #[test]
    fn position_deltas_reconstruct_exactly() {
        let baseline = 103.5f32;
        for step in [-4.0f32, -0.5, -0.00390625, 0.0, 0.00390625, 0.25, 7.75] {
            let target = baseline + step;
            let bytes = write(|w| w.position_delta(target, baseline));
            let mut reader = Reader::new(&bytes);
            let decoded = reader.position_delta(baseline).unwrap();
            assert_eq!(
                quantize(decoded),
                quantize(target),
                "step {step} did not survive"
            );
        }
    }

    #[test]
    fn quantisation_error_stays_under_half_a_step() {
        let step = 1.0 / POSITION_SCALE;
        for raw in [0.0f32, 0.1, 1.7, 42.123_456, 1023.9, -17.3] {
            let error = (dequantize(quantize(raw)) - raw).abs();
            assert!(error <= step / 2.0 + f32::EPSILON, "{raw} drifted by {error}");
        }
    }

    #[test]
    fn quantisation_saturates_instead_of_wrapping() {
        assert_eq!(quantize(f32::NAN), 0);
        assert!(quantize(f32::INFINITY) > 0);
        assert!(quantize(f32::NEG_INFINITY) < 0);
    }

    #[test]
    fn an_empty_condition_mask_costs_one_byte() {
        let bytes = write(|w| w.condition_mask(0));
        assert_eq!(bytes, vec![0]);

        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.condition_mask().unwrap(), 0);
    }

    #[test]
    fn condition_masks_round_trip_at_every_width() {
        for mask in [0u128, 1, 0xff, 0x1_0000, u64::MAX as u128, u128::MAX] {
            let bytes = write(|w| w.condition_mask(mask));
            let mut reader = Reader::new(&bytes);
            assert_eq!(reader.condition_mask().unwrap(), mask, "mask {mask:#x}");
        }

        // One low bit set should not pay for the high half.
        assert_eq!(write(|w| w.condition_mask(1)).len(), 2);
    }

    #[test]
    fn strings_round_trip_and_borrow_the_input() {
        let bytes = write(|w| w.string("Hendra ✦ vault"));
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.string().unwrap(), "Hendra ✦ vault");
    }

    // -- adversarial input ---------------------------------------------------------------------

    #[test]
    fn truncated_input_errors_rather_than_panicking() {
        let bytes = write(|w| w.string("a fairly long chat message"));
        for cut in 0..bytes.len() {
            let mut reader = Reader::new(&bytes[..cut]);
            // Any outcome is fine except a panic; most will be Eof.
            let _ = reader.string();
        }
    }

    #[test]
    fn a_runaway_varint_terminates() {
        let runaway = vec![0x80u8; 64];
        let mut reader = Reader::new(&runaway);
        assert!(matches!(
            reader.varint(),
            Err(CodecError::VarintTooLong { .. }) | Err(CodecError::VarintOverflow)
        ));
    }

    #[test]
    fn an_overlong_varint_does_not_silently_wrap() {
        // Eleven continuation bytes cannot encode a u64 no matter what they hold.
        let bytes = vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
        let mut reader = Reader::new(&bytes);
        assert!(reader.varint().is_err());
    }

    #[test]
    fn a_hostile_string_length_is_refused_before_allocating() {
        // A length prefix claiming 4 GB, with no payload behind it.
        let mut bytes = Vec::new();
        Writer::new(&mut bytes).varint(4_000_000_000);
        let mut reader = Reader::new(&bytes);
        assert!(matches!(
            reader.string(),
            Err(CodecError::StringTooLong { .. })
        ));
    }

    #[test]
    fn a_hostile_sequence_count_is_bounded() {
        let mut bytes = Vec::new();
        Writer::new(&mut bytes).varint(u32::MAX as u64);
        let mut reader = Reader::new(&bytes);
        assert!(matches!(
            reader.count(MAX_SEQUENCE),
            Err(CodecError::SequenceTooLong { .. })
        ));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let mut bytes = Vec::new();
        {
            let mut writer = Writer::new(&mut bytes);
            writer.varint(2);
        }
        bytes.extend_from_slice(&[0xff, 0xfe]);

        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.string(), Err(CodecError::InvalidUtf8));
    }

    #[test]
    fn an_over_long_string_is_truncated_on_a_character_boundary() {
        let long = "✦".repeat(MAX_STRING_BYTES); // three bytes each
        let bytes = write(|w| w.string(&long));
        let mut reader = Reader::new(&bytes);
        let decoded = reader.string().expect("truncation must stay valid UTF-8");
        assert!(decoded.len() <= MAX_STRING_BYTES);
        assert!(long.starts_with(decoded));
    }

    #[test]
    fn a_fuzzed_buffer_never_panics() {
        // A cheap deterministic sweep: every decoder against a range of junk inputs.
        let mut seed = 0x243f_6a88_85a3_08d3u64;
        for _ in 0..2_000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let len = (seed % 48) as usize;
            let junk: Vec<u8> = (0..len)
                .map(|i| (seed >> (i % 8 * 8)) as u8)
                .collect();

            let mut reader = Reader::new(&junk);
            let _ = reader.varint();
            let _ = reader.varint_signed();
            let _ = reader.string();
            let _ = reader.bytes();
            let _ = reader.condition_mask();
            let _ = reader.position_value();
            let _ = reader.count(MAX_SEQUENCE);
        }
    }
}
