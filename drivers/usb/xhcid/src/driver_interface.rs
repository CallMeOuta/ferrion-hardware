pub extern crate serde;
pub extern crate smallvec;

mod client;
mod protocol;

pub use self::client::{
    DeviceReqData, TransferReadStream, TransferWriteStream, XhciClientHandle,
    XhciClientHandleError, XhciEndpHandle,
};
pub use self::protocol::{
    ConfDesc, ConfigureEndpointsReq, DevDesc, EndpBinaryDirection, EndpDesc, EndpDirection,
    EndpointStatus, IfDesc, Invalid, PortId, PortReq, PortReqDirection, PortReqRecipient,
    PortReqTy, PortState, PortTransferStatus, PortTransferStatusKind, SuperSpeedCmp,
    SuperSpeedPlusIsochCmp, UnknownDesc, XhciEndpCtlDirection, XhciEndpCtlReq, XhciEndpCtlRes,
};
pub use crate::usb::{EndpointTy, ENDP_ATTR_TY_MASK};
