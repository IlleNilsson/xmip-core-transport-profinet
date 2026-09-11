//! The discovery and configuration protocol, IEC 61158-6-10 clause 4.3:
//! how a controller finds the devices on the wire and gives each its name,
//! before any cyclic frame goes.
//!
//! An identify request goes to the DCP multicast address under `FrameID`
//! `0xfefe`; every device answers under `0xfeff` with the blocks the
//! selector asked for, its name of station among them. A get or set goes
//! to one device under `0xfefd`. Every DCP frame is a service, a request or
//! a response, a transaction identifier the response echoes, and blocks of
//! option, suboption, length and data, each padded to an even length.

use ethernet::Mac;
use transport::error::{Result, protocol_error};

/// The `EtherType` every PROFINET frame carries, DCP and cyclic alike.
pub const ETHERTYPE: u16 = 0x8892;

/// The multicast address an identify request goes to.
pub const MULTICAST: Mac = Mac([0x01, 0x0e, 0xcf, 0x00, 0x00, 0x00]);

/// The `FrameID` of an identify request, of an identify response, and of a
/// get or set and its response.
pub const IDENTIFY_REQUEST: u16 = 0xfefe;
pub const IDENTIFY_RESPONSE: u16 = 0xfeff;
pub const GET_SET: u16 = 0xfefd;

/// The device properties option, and its name of station suboption.
pub const OPTION_DEVICE: u8 = 0x02;
pub const SUBOPTION_NAME: u8 = 0x02;
/// The all selector: every option, every suboption.
pub const OPTION_ALL: u8 = 0xff;

/// What a DCP frame asks or answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    Get = 3,
    Set = 4,
    Identify = 5,
}

/// One block: an option, a suboption, and its data — the block info of a
/// response or the qualifier of a set included, where the block has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub option: u8,
    pub suboption: u8,
    pub data: Vec<u8>,
}

/// One DCP frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dcp {
    pub service: Service,
    pub response: bool,
    pub xid: u32,
    pub blocks: Vec<Block>,
}

impl Dcp {
    /// An identify request for everything, under `xid`.
    #[must_use]
    pub fn identify_all(xid: u32) -> Self {
        Self {
            service: Service::Identify,
            response: false,
            xid,
            blocks: vec![Block {
                option: OPTION_ALL,
                suboption: OPTION_ALL,
                data: Vec::new(),
            }],
        }
    }

    /// A set of the name of station to `name`, permanently, under `xid`.
    #[must_use]
    pub fn set_name(xid: u32, name: &str) -> Self {
        let mut data = vec![0x00, 0x01];
        data.extend_from_slice(name.as_bytes());
        Self {
            service: Service::Set,
            response: false,
            xid,
            blocks: vec![Block {
                option: OPTION_DEVICE,
                suboption: SUBOPTION_NAME,
                data,
            }],
        }
    }

    /// The response to this request carrying `blocks`.
    #[must_use]
    pub fn answered_with(&self, blocks: Vec<Block>) -> Self {
        Self {
            service: self.service,
            response: true,
            xid: self.xid,
            blocks,
        }
    }

    /// The name of station this frame carries, past its info or qualifier.
    #[must_use]
    pub fn name(&self) -> Option<String> {
        self.blocks
            .iter()
            .find(|block| block.option == OPTION_DEVICE && block.suboption == SUBOPTION_NAME)
            .and_then(|block| block.data.get(2..))
            .map(|name| String::from_utf8_lossy(name).into_owned())
    }

    /// The frame's payload: `FrameID`, header, blocks.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut blocks = Vec::new();
        for block in &self.blocks {
            blocks.push(block.option);
            blocks.push(block.suboption);
            blocks.extend_from_slice(&u16::try_from(block.data.len()).unwrap_or(0).to_be_bytes());
            blocks.extend_from_slice(&block.data);
            if !block.data.len().is_multiple_of(2) {
                blocks.push(0);
            }
        }
        let frame_id = match (self.service, self.response) {
            (Service::Identify, false) => IDENTIFY_REQUEST,
            (Service::Identify, true) => IDENTIFY_RESPONSE,
            _ => GET_SET,
        };
        let mut out = Vec::with_capacity(12 + blocks.len());
        out.extend_from_slice(&frame_id.to_be_bytes());
        out.push(self.service as u8);
        out.push(u8::from(self.response));
        out.extend_from_slice(&self.xid.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&u16::try_from(blocks.len()).unwrap_or(0).to_be_bytes());
        out.extend_from_slice(&blocks);
        out
    }

    /// The DCP frame `payload` carries, or `None` where the `FrameID` is not
    /// DCP's — a cyclic frame, say.
    ///
    /// # Errors
    /// A DCP frame shorter than its header, a service that is not one of
    /// the three, or a block length past the end.
    pub fn decode(payload: &[u8]) -> Result<Option<Self>> {
        let frame_id = payload
            .get(..2)
            .map(|id| u16::from_be_bytes([id[0], id[1]]))
            .ok_or_else(|| protocol_error("shorter than a FrameID"))?;
        if !matches!(frame_id, IDENTIFY_REQUEST | IDENTIFY_RESPONSE | GET_SET) {
            return Ok(None);
        }
        let head = payload
            .get(2..12)
            .ok_or_else(|| protocol_error("a DCP frame shorter than its header"))?;
        let service = match head[0] {
            3 => Service::Get,
            4 => Service::Set,
            5 => Service::Identify,
            other => return Err(protocol_error(format!("DCP service {other} is not one"))),
        };
        let response = head[1] == 1;
        let xid = u32::from_be_bytes([head[2], head[3], head[4], head[5]]);
        let length = usize::from(u16::from_be_bytes([head[8], head[9]]));
        let body = payload
            .get(12..12 + length)
            .ok_or_else(|| protocol_error("a DCP data length past the end"))?;
        let mut blocks = Vec::new();
        let mut at = 0;
        while at < body.len() {
            let fixed = body
                .get(at..at + 4)
                .ok_or_else(|| protocol_error("a DCP block shorter than its header"))?;
            let len = usize::from(u16::from_be_bytes([fixed[2], fixed[3]]));
            let data = body
                .get(at + 4..at + 4 + len)
                .ok_or_else(|| protocol_error("a DCP block length past the end"))?;
            blocks.push(Block {
                option: fixed[0],
                suboption: fixed[1],
                data: data.to_vec(),
            });
            at += 4 + len + usize::from(!len.is_multiple_of(2));
        }
        Ok(Some(Self {
            service,
            response,
            xid,
            blocks,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identify_request_and_its_response_round_trip() {
        let request = Dcp::identify_all(0x0102_0304);
        let payload = request.encode();
        assert_eq!(&payload[..2], &[0xfe, 0xfe]);
        assert_eq!(&payload[2..12], &[5, 0, 1, 2, 3, 4, 0, 0, 0, 4]);
        assert_eq!(&payload[12..], &[0xff, 0xff, 0, 0]);
        assert_eq!(
            Dcp::decode(&payload).expect("decode"),
            Some(request.clone())
        );
        let mut data = vec![0, 0];
        data.extend_from_slice(b"valve");
        let response = request.answered_with(vec![Block {
            option: OPTION_DEVICE,
            suboption: SUBOPTION_NAME,
            data,
        }]);
        let payload = response.encode();
        assert_eq!(&payload[..2], &[0xfe, 0xff]);
        assert_eq!(payload.len(), 12 + 4 + 8, "padded to even");
        let back = Dcp::decode(&payload).expect("decode").expect("dcp");
        assert_eq!(back, response);
        assert_eq!(back.name().as_deref(), Some("valve"));
        assert!(request.name().is_none());
    }

    #[test]
    fn a_set_names_the_station_and_a_cyclic_frame_is_not_dcp() {
        let set = Dcp::set_name(7, "drive-1");
        let payload = set.encode();
        assert_eq!(&payload[..2], &[0xfe, 0xfd]);
        assert_eq!(&payload[12..16], &[2, 2, 0, 9]);
        assert_eq!(&payload[16..18], &[0, 1], "permanent");
        assert_eq!(
            Dcp::decode(&payload)
                .expect("decode")
                .expect("dcp")
                .name()
                .as_deref(),
            Some("drive-1")
        );
        assert_eq!(Dcp::decode(&[0x80, 0x00, 1, 2]).expect("cyclic"), None);
    }

    #[test]
    fn what_is_not_dcp_is_refused() {
        assert!(Dcp::decode(&[0xfe]).is_err(), "no FrameID");
        assert!(Dcp::decode(&[0xfe, 0xfe, 5, 0]).is_err(), "short header");
        assert!(
            Dcp::decode(&[0xfe, 0xfe, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err(),
            "service 9"
        );
        assert!(
            Dcp::decode(&[0xfe, 0xfe, 5, 0, 0, 0, 0, 0, 0, 0, 0, 8]).is_err(),
            "past the end"
        );
        assert!(
            Dcp::decode(&[0xfe, 0xfe, 5, 0, 0, 0, 0, 0, 0, 0, 0, 4, 2, 2, 0, 9]).is_err(),
            "block past"
        );
    }
}
