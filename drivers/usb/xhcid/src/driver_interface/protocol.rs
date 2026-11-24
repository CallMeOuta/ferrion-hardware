use std::convert::TryFrom;
use std::num::NonZeroU8;
use std::{fmt, result, str};

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use syscall::{Error, Result, EINVAL};
use thiserror::Error;

use crate::usb::{EndpointTy, ENDP_ATTR_TY_MASK};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConfigureEndpointsReq {
    /// Index into the configuration descriptors of the device descriptor.
    pub config_desc: u8,
    pub interface_desc: Option<u8>,
    pub alternate_setting: Option<u8>,
    pub hub_ports: Option<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DevDesc {
    pub kind: u8,
    pub usb: u16,
    pub class: u8,
    pub sub_class: u8,
    pub protocol: u8,
    pub packet_size: u8,
    pub vendor: u16,
    pub product: u16,
    pub release: u16,
    pub manufacturer_str: Option<String>,
    pub product_str: Option<String>,
    pub serial_str: Option<String>,
    pub config_descs: SmallVec<[ConfDesc; 1]>,
}

impl DevDesc {
    pub fn major_version(&self) -> u8 {
        ((self.usb & 0xFF00) >> 8) as u8
    }
    pub fn minor_version(&self) -> u8 {
        self.usb as u8
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfDesc {
    pub kind: u8,
    pub configuration_value: u8,
    pub configuration: Option<String>,
    pub attributes: u8,
    pub max_power: u8,
    pub interface_descs: SmallVec<[IfDesc; 1]>,
    pub unknown_descs: SmallVec<[UnknownDesc; 1]>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct EndpDesc {
    pub kind: u8,
    pub address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
    pub ssc: Option<SuperSpeedCmp>,
    pub sspc: Option<SuperSpeedPlusIsochCmp>,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EndpDirection {
    Out,
    In,
    Bidirectional,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EndpBinaryDirection {
    Out,
    In,
}

impl From<PortReqDirection> for EndpBinaryDirection {
    fn from(d: PortReqDirection) -> Self {
        match d {
            PortReqDirection::DeviceToHost => Self::In,
            PortReqDirection::HostToDevice => Self::Out,
        }
    }
}

impl From<EndpBinaryDirection> for EndpDirection {
    fn from(b: EndpBinaryDirection) -> Self {
        match b {
            EndpBinaryDirection::In => Self::In,
            EndpBinaryDirection::Out => Self::Out,
        }
    }
}

impl From<PortReqDirection> for EndpDirection {
    fn from(d: PortReqDirection) -> Self {
        match d {
            PortReqDirection::HostToDevice => Self::Out,
            PortReqDirection::DeviceToHost => Self::In,
        }
    }
}

impl EndpDesc {
    pub fn ty(self) -> EndpointTy {
        match self.attributes & ENDP_ATTR_TY_MASK {
            0 => EndpointTy::Ctrl,
            1 => EndpointTy::Isoch,
            2 => EndpointTy::Bulk,
            3 => EndpointTy::Interrupt,
            _ => unreachable!(),
        }
    }
    pub fn is_control(&self) -> bool {
        self.ty() == EndpointTy::Ctrl
    }
    pub fn is_interrupt(&self) -> bool {
        self.ty() == EndpointTy::Interrupt
    }
    pub fn is_bulk(&self) -> bool {
        self.ty() == EndpointTy::Bulk
    }
    pub fn is_isoch(&self) -> bool {
        self.ty() == EndpointTy::Isoch
    }
    pub fn direction(&self) -> EndpDirection {
        if self.is_control() {
            return EndpDirection::Bidirectional;
        }
        if self.address & 0x80 != 0 {
            EndpDirection::In
        } else {
            EndpDirection::Out
        }
    }
    pub fn xhci_ep_type(&self) -> Result<u8> {
        Ok(match self.direction() {
            EndpDirection::Out if self.is_isoch() => 1,
            EndpDirection::Out if self.is_bulk() => 2,
            EndpDirection::Out if self.is_interrupt() => 3,
            EndpDirection::Bidirectional if self.is_control() => 4,
            EndpDirection::In if self.is_isoch() => 5,
            EndpDirection::In if self.is_bulk() => 6,
            EndpDirection::In if self.is_interrupt() => 7,
            _ => return Err(Error::new(EINVAL)),
        })
    }
    pub fn xhci_dci(&self) -> u8 {
        if self.is_control() || self.direction() == EndpDirection::In {
            (self.address & 0x7F) * 2 + 1
        } else if self.direction() == EndpDirection::Out {
            (self.address & 0x7F) * 2
        } else {
            unreachable!()
        }
    }
    pub fn is_superspeed(&self) -> bool {
        self.ssc.is_some()
    }
    pub fn is_superspeedplus(&self) -> bool {
        self.sspc.is_some()
    }
    fn interrupt_usage_bits(&self) -> u8 {
        assert!(self.is_interrupt());
        (self.attributes & 0x20) >> 4
    }
    pub fn is_periodic(&self) -> bool {
        #[repr(u8)]
        enum InterruptUsageBits {
            Periodic,
            Notification,
            Rsvd2,
            Rsvd3,
        }

        if self.is_interrupt() {
            self.interrupt_usage_bits() == InterruptUsageBits::Periodic as u8
        } else {
            self.is_isoch()
        }
    }
    pub fn log_max_streams(&self) -> Option<NonZeroU8> {
        self.ssc
            .as_ref()
            .map(|ssc| {
                if self.is_bulk() {
                    let raw = ssc.attributes & 0x1F;
                    NonZeroU8::new(raw)
                } else {
                    None
                }
            })
            .flatten()
    }
    pub fn isoch_mult(&self, lec: bool) -> u8 {
        if !lec && self.is_isoch() {
            if self.is_superspeedplus() {
                return 0;
            }
            self.ssc
                .as_ref()
                .map(|ssc| ssc.attributes & 0x3)
                .unwrap_or(0)
        } else {
            0
        }
    }
    pub fn max_burst(&self) -> u8 {
        self.ssc.map(|ssc| ssc.max_burst).unwrap_or(0)
    }
    pub fn has_ssp_companion(&self) -> bool {
        self.ssc
            .map(|ssc| ssc.attributes & (1 << 7) != 0)
            .unwrap_or(false)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IfDesc {
    pub kind: u8,
    pub number: u8,
    pub alternate_setting: u8,
    pub class: u8,
    pub sub_class: u8,
    pub protocol: u8,
    pub interface_str: Option<String>,
    pub endpoints: SmallVec<[EndpDesc; 4]>,
    pub unknown_descs: SmallVec<[UnknownDesc; 1]>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct SuperSpeedCmp {
    pub kind: u8,
    pub max_burst: u8,
    pub attributes: u8,
    pub bytes_per_interval: u16,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct SuperSpeedPlusIsochCmp {
    pub kind: u8,
    pub bytes_per_interval: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UnknownDesc {
    pub len: u8,
    pub kind: u8,
    pub all_bytes: Vec<u8>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PortReq {
    pub direction: PortReqDirection,
    pub req_type: PortReqTy,
    pub req_recipient: PortReqRecipient,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
    pub transfers_data: bool,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum PortReqDirection {
    HostToDevice,
    DeviceToHost,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum PortReqTy {
    Class,
    Vendor,
    Standard,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum PortReqRecipient {
    Device,
    Interface,
    Endpoint,
    Other,
    VendorSpecific,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PortId {
    pub root_hub_port_num: u8,
    pub route_string: u32,
}

impl PortId {
    pub fn root_hub_port_index(&self) -> usize {
        self.root_hub_port_num.checked_sub(1).unwrap().into()
    }

    pub fn hub_depth(&self) -> u8 {
        let mut hub_depth = 0;
        let mut route_string = self.route_string;
        while route_string != 0 {
            route_string >>= 4;
            hub_depth += 1;
        }
        hub_depth
    }

    pub fn child(&self, value: u8) -> Result<Self, String> {
        let depth = self.hub_depth();
        if depth >= 5 {
            return Err(format!("too many route string components"));
        }
        if value & 0xF0 != 0 {
            return Err(format!(
                "value {:?} is too large for route string component",
                value
            ));
        }
        Ok(Self {
            root_hub_port_num: self.root_hub_port_num,
            route_string: self.route_string | u32::from(value) << (depth * 4),
        })
    }

    pub fn parent(&self) -> Option<(Self, u8)> {
        let depth = self.hub_depth();
        let parent_depth = depth.checked_sub(1)?;
        let parent_shift = parent_depth * 4;
        let parent_mask = 0xF << parent_shift;
        Some((
            Self {
                root_hub_port_num: self.root_hub_port_num,
                route_string: self.route_string & !parent_mask,
            },
            u8::try_from((self.route_string & parent_mask) >> parent_shift).unwrap(),
        ))
    }
}

impl fmt::Display for PortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.root_hub_port_num)?;
        // USB 3.1 Revision 1.1 Specification Section 8.9 Route String Field
        // The Route String is a 20-bit field in downstream directed packets that the hub uses to route
        // each packet to the designated downstream port. It is composed of a concatenation of the
        // downstream port numbers (4 bits per hub) for each hub traversed to reach a device.
        let mut route_string = self.route_string;
        while route_string != 0 {
            write!(f, ".{}", route_string & 0xF)?;
            route_string >>= 4;
        }
        Ok(())
    }
}

impl str::FromStr for PortId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut root_hub_port_num = 0;
        let mut route_string = 0;
        for (i, part) in s.split('.').enumerate() {
            let value: u8 = part
                .parse()
                .map_err(|e| format!("failed to parse {:?}: {}", part, e))?;

            // Neither root hub port number nor route string support 0 components
            // to identify downstream ports
            if value == 0 {
                return Err(format!("zero is not a valid port ID component"));
            }

            // Parse root hub port number
            if i == 0 {
                root_hub_port_num = value;
                continue;
            }

            // Parse route string component
            let depth = i - 1;
            if depth >= 5 {
                return Err(format!("too many route string components"));
            }
            if value & 0xF0 != 0 {
                return Err(format!(
                    "value {:?} is too large for route string component",
                    value
                ));
            }
            route_string |= u32::from(value) << (depth * 4);
        }
        Ok(Self {
            root_hub_port_num,
            route_string,
        })
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PortState {
    EnabledOrDisabled,
    Default,
    Addressed,
    Configured,
}
impl PortState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EnabledOrDisabled => "enabled_or_disabled",
            Self::Default => "default",
            Self::Addressed => "addressed",
            Self::Configured => "configured",
        }
    }
}
#[derive(Debug, Error)]
#[error("invalid input")]
pub struct Invalid(pub &'static str);

impl str::FromStr for PortState {
    type Err = Invalid;

    fn from_str(s: &str) -> result::Result<Self, Self::Err> {
        Ok(match s {
            "enabled_or_disabled" | "enabled/disabled" => Self::EnabledOrDisabled,
            "default" => Self::Default,
            "addressed" => Self::Addressed,
            "configured" => Self::Configured,
            _ => return Err(Invalid("read reserved port state")),
        })
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum EndpointStatus {
    Disabled,
    Enabled,
    Halted,
    Stopped,
    Error,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PortTransferStatus {
    pub kind: PortTransferStatusKind,
    pub bytes_transferred: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum PortTransferStatusKind {
    Success,
    ShortPacket,
    Stalled,
    Unknown,
}
impl Default for PortTransferStatusKind {
    fn default() -> Self {
        Self::Success
    }
}

impl EndpointStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::Halted => "halted",
            Self::Stopped => "stopped",
            Self::Error => "error",
        }
    }
}

impl str::FromStr for EndpointStatus {
    type Err = Invalid;

    fn from_str(s: &str) -> result::Result<Self, Self::Err> {
        Ok(match s {
            "disabled" => Self::Disabled,
            "enabled" => Self::Enabled,
            "halted" => Self::Halted,
            "stopped" => Self::Stopped,
            "error" => Self::Error,
            _ => return Err(Invalid("read reserved endpoint state")),
        })
    }
}

/// The direction of a transfer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum XhciEndpCtlDirection {
    /// Host to device
    Out,
    /// Device to host
    In,
    /// No data, and hence no I/O on the Data interface file at all.
    NoData,
}

/// A request to an endpoint Ctl interface file. Currently serialized with JSON.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum XhciEndpCtlReq {
    // TODO: Reduce the number of direction enums from 5 to perhaps 2.
    // TODO: Allow to send multiple buffers in one transfer.
    /// Tells xhcid that a buffer is about to be sent from the Data interface file, to the
    /// endpoint.
    Transfer {
        /// The direction of the transfer. If the direction is `XhciEndpCtlDirection::NoData`, no
        /// bytes will be transferred, and therefore no reads or writes shall be done to the Data
        /// driver interface file.
        direction: XhciEndpCtlDirection,

        /// The number of bytes to be read or written. This field must be set to zero if the
        /// direction is `XhciEndpCtlDirection::NoData`. When all bytes have been read or written,
        /// the transfer will be considered complete by xhcid, and a non-pending status will be
        /// returned.
        count: u32,
    },
    // TODO: Allow clients to specify what to reset.
    /// Tells xhcid that the endpoint is going to be reset.
    Reset {
        /// Only issue the Reset Endpoint and Set TR Dequeue Pointer commands, and let the client
        /// itself send a potential ClearFeature(ENDPOINT_HALT).
        no_clear_feature: bool,
    },

    /// Tells xhcid that the endpoint status is going to be retrieved from the Ctl interface file.
    Status,
}
/// A response from an endpoint Ctl interface file. Currently serialized with JSON.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum XhciEndpCtlRes {
    /// Xhcid responded with the current state of an endpoint.
    Status(EndpointStatus),

    /// Xhci sent the result of a transfer.
    TransferResult(PortTransferStatus),

    /// Xhcid is waiting for data to be sent or received on the Data interface file.
    Pending,

    /// No Ctl request is currently being processed by xhcid.
    Idle,
}
