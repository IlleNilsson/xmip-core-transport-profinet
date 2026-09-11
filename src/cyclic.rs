//! The real-time class 1 cyclic frame, IEC 61158-6-10 clause 4.7: a `FrameID`
//! in the class's range, the IO data, and the APDU status trailer — cycle
//! counter, data status, transfer status. Every cycle, whether or not
//! anything changed.
//!
//! The IO data is what the application layer laid out; here it carries one
//! chunk of a Stream under a four-byte prologue — the chunk's length, and a
//! flag on the last — so a Stream rides across as many cycles as it takes
//! and the far end knows where it ends.

use transport::error::{Result, protocol_error};

/// The first and last `FrameID` of real-time class 1.
pub const RT_CLASS_1_FIRST: u16 = 0x8000;
pub const RT_CLASS_1_LAST: u16 = 0xbbff;

/// The IO data one cycle carries: the smallest `C_SDU` the class allows.
pub const IO_DATA: usize = 40;
/// The length, the flag and two reserved bytes before a chunk.
const PROLOGUE: usize = 4;
/// The bytes of a Stream one cycle carries.
pub const CHUNK: usize = IO_DATA - PROLOGUE;

/// How far the cycle counter moves each cycle: a send clock of one
/// millisecond in units of 31.25 microseconds.
pub const CYCLE_STEP: u16 = 32;

/// A data status that says primary, valid, run, and nothing to report.
pub const DATA_STATUS_GOOD: u8 = 0x35;

/// One cyclic frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cyclic {
    pub frame_id: u16,
    pub io_data: Vec<u8>,
    pub cycle_counter: u16,
    pub data_status: u8,
    pub transfer_status: u8,
}

impl Cyclic {
    /// The frame's payload: `FrameID`, IO data, APDU status.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 + IO_DATA + 4);
        out.extend_from_slice(&self.frame_id.to_be_bytes());
        out.extend_from_slice(&self.io_data);
        out.resize(2 + IO_DATA, 0);
        out.extend_from_slice(&self.cycle_counter.to_be_bytes());
        out.push(self.data_status);
        out.push(self.transfer_status);
        out
    }

    /// The cyclic frame `payload` carries.
    ///
    /// # Errors
    /// A `FrameID` outside class 1, or a length that is not one cycle's.
    pub fn decode(payload: &[u8]) -> Result<Self> {
        if payload.len() != 2 + IO_DATA + 4 {
            return Err(protocol_error(format!(
                "{} bytes is not one cycle of {IO_DATA} bytes of IO data",
                payload.len()
            )));
        }
        let frame_id = u16::from_be_bytes([payload[0], payload[1]]);
        if !(RT_CLASS_1_FIRST..=RT_CLASS_1_LAST).contains(&frame_id) {
            return Err(protocol_error(format!(
                "FrameID {frame_id:#06x} is not real-time class 1"
            )));
        }
        let trailer = &payload[2 + IO_DATA..];
        Ok(Self {
            frame_id,
            io_data: payload[2..2 + IO_DATA].to_vec(),
            cycle_counter: u16::from_be_bytes([trailer[0], trailer[1]]),
            data_status: trailer[2],
            transfer_status: trailer[3],
        })
    }

    /// The chunk of a Stream this cycle's IO data carries, and whether it
    /// is the last.
    ///
    /// # Errors
    /// A prologue naming more than the IO data holds.
    pub fn chunk(&self) -> Result<(&[u8], bool)> {
        let length = usize::from(u16::from_be_bytes([self.io_data[0], self.io_data[1]]));
        let last = self.io_data[2] & 0x01 != 0;
        let chunk = self
            .io_data
            .get(PROLOGUE..PROLOGUE + length)
            .ok_or_else(|| protocol_error("a chunk longer than the IO data"))?;
        Ok((chunk, last))
    }
}

/// `payload` as the cycles that carry it under `frame_id`, counting from
/// `first_counter`: at least one, so an empty Stream is one empty cycle
/// with the last flag set.
#[must_use]
pub fn cycles(frame_id: u16, payload: &[u8], first_counter: u16) -> Vec<Cyclic> {
    let chunks: Vec<&[u8]> = if payload.is_empty() {
        vec![&[]]
    } else {
        payload.chunks(CHUNK).collect()
    };
    let total = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(n, chunk)| {
            let mut io_data = Vec::with_capacity(IO_DATA);
            io_data.extend_from_slice(&u16::try_from(chunk.len()).unwrap_or(0).to_be_bytes());
            io_data.push(u8::from(n + 1 == total));
            io_data.push(0);
            io_data.extend_from_slice(chunk);
            io_data.resize(IO_DATA, 0);
            Cyclic {
                frame_id,
                io_data,
                cycle_counter: first_counter
                    .wrapping_add(CYCLE_STEP.wrapping_mul(u16::try_from(n).unwrap_or(0))),
                data_status: DATA_STATUS_GOOD,
                transfer_status: 0,
            }
        })
        .collect()
}

/// The Stream a run of cycles carries, checked for continuity: each cycle
/// counter one step past the last, and the run ending on a last flag.
///
/// # Errors
/// A cycle missed, a data status that is not good, or a run with no end.
pub fn assemble(cycles: impl IntoIterator<Item = Cyclic>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut expected = None;
    for cycle in cycles {
        if let Some(counter) = expected
            && cycle.cycle_counter != counter
        {
            return Err(protocol_error("a cycle was missed"));
        }
        if cycle.data_status != DATA_STATUS_GOOD {
            return Err(protocol_error(format!(
                "a data status of {:#04x} is not good",
                cycle.data_status
            )));
        }
        let (chunk, last) = cycle.chunk()?;
        bytes.extend_from_slice(chunk);
        if last {
            return Ok(bytes);
        }
        expected = Some(cycle.cycle_counter.wrapping_add(CYCLE_STEP));
    }
    Err(protocol_error("the cycles ended before the last flag"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_rides_as_many_cycles_as_it_takes_and_assembles_back() {
        let payload: Vec<u8> = (0..100u8).collect();
        let run = cycles(0x8000, &payload, 64);
        assert_eq!(run.len(), 3, "36 + 36 + 28");
        assert_eq!(run[0].cycle_counter, 64);
        assert_eq!(run[2].cycle_counter, 128);
        assert_eq!(run[2].chunk().expect("chunk"), (&payload[72..], true));
        assert!(!run[0].chunk().expect("chunk").1);
        let wire = run[0].encode();
        assert_eq!(wire.len(), 46);
        assert_eq!(&wire[..2], &[0x80, 0x00]);
        assert_eq!(&wire[42..], &[0, 64, DATA_STATUS_GOOD, 0]);
        assert_eq!(Cyclic::decode(&wire).expect("decode"), run[0]);
        assert_eq!(assemble(run).expect("assemble"), payload);
        let empty = cycles(0x8001, &[], 0);
        assert_eq!(empty.len(), 1);
        assert_eq!(assemble(empty).expect("empty"), Vec::<u8>::new());
    }

    #[test]
    fn a_missed_cycle_a_bad_status_and_no_end_are_refused() {
        let payload = [7u8; 80];
        let mut run = cycles(0x8000, &payload, 0);
        run.remove(1);
        assert!(assemble(run).is_err(), "a cycle was missed");
        let mut run = cycles(0x8000, &payload, 0);
        run[0].data_status = 0x15;
        assert!(assemble(run).is_err(), "not valid");
        let mut run = cycles(0x8000, &payload, 0);
        run.pop();
        assert!(assemble(run).is_err(), "no end");
        let mut lying = cycles(0x8000, &[1], 0);
        lying[0].io_data[1] = 200;
        assert!(assemble(lying).is_err(), "a chunk longer than the IO data");
    }

    #[test]
    fn what_is_not_a_class_1_cycle_is_refused() {
        assert!(Cyclic::decode(&[0x80, 0x00, 1]).is_err(), "short");
        let mut wire = cycles(0x8000, &[], 0)[0].encode();
        wire[0] = 0xc0;
        assert!(Cyclic::decode(&wire).is_err(), "class 3");
        wire[0] = 0xfe;
        wire[1] = 0xfe;
        assert!(Cyclic::decode(&wire).is_err(), "DCP");
    }
}
