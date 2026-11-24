use std::sync::LazyLock;

use regex::Regex;
use syscall::{Error, Result, ENOENT};

use super::{EndpNum, PortId};

static REGEX_PORT_CONFIGURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^port([\d\.]+)/configure$")
        .expect("Failed to create the regex for the port<n>/configure scheme.")
});
static REGEX_PORT_ATTACH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^port([\d\.]+)/attach$")
        .expect("Failed to create the regex for the port<n>/attach scheme.")
});
static REGEX_PORT_DETACH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^port([\d\.]+)/detach$")
        .expect("Failed to create the regex for the port<n>/detach scheme.")
});
static REGEX_PORT_DESCRIPTORS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^port([\d\.]+)/descriptors$")
        .expect("Failed to create the regex for the port<n>/descriptors")
});
static REGEX_PORT_STATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^port([\d\.]+)/state$")
        .expect("Failed to create the regex for the port<n>/state scheme")
});
static REGEX_PORT_REQUEST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^port([\d\.]+)/request$")
        .expect("Failed to create the regex for the port<n>/request scheme")
});
static REGEX_PORT_ENDPOINTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^port([\d\.]+)/endpoints$")
        .expect("Failed to create the regex for the port<n>/endpoints scheme")
});
static REGEX_PORT_SPECIFIC_ENDPOINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^port([\d\.]+)/endpoints/(\d{1,3})$")
        .expect("Failed to create the regex for the port<n>/endpoints/<n> scheme")
});
static REGEX_PORT_SUB_ENDPOINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"port([\d\.]+)/endpoints/(\d{1,3})/(ctl|data)$")
        .expect("Failed to create the regex for the port<n>/endpoints/<n>/<sub_endpoint> scheme")
});
static REGEX_PORT_ROOT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^port([\d\.]+)$").expect("Failed to create the regex for the port<n> scheme.")
});
static REGEX_TOP_LEVEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^$").expect("Failed to create the regex for the top-level scheme")
});

/// Parameters to a handle that were extracted from a scheme.
///
/// This structure is used to easily convert a scheme filesystem path to
/// the parameters that we care about when constructing a handle.
#[derive(Debug)]
pub(super) enum SchemeParameters {
    /// The scheme references the top-level XHCI driver endpoint
    TopLevel,
    /// /port<n>
    Port(PortId), // port number
    /// /port<n>/descriptors
    PortDesc(PortId), // port number
    /// /port<n>/state
    PortState(PortId), // port number
    /// /port<n>/request
    PortReq(PortId), // port number
    /// /port<n>/endpoints
    Endpoints(PortId), // port number
    /// /port<n>/endpoints/<n>/(data|ctl)
    ///
    /// This can also represent
    /// /port<n>/endpoints/<n>
    Endpoint(PortId, EndpNum, String), // port number, endpoint number, handle type
    /// /port<n>/configure
    ConfigureEndpoints(PortId), // port number
    /// /port<n>/attach
    AttachDevice(PortId), // port number
    /// /port<n>/detach
    DetachDevice(PortId), // port number
}

impl SchemeParameters {
    /// This function gets a partially populated handle from a scheme string.
    ///
    /// This function is intended to be used by the driver's 'open' filesystem
    /// hook to determine if the given string value represents a valid scheme
    ///
    /// # Arguments
    /// 'scheme: &[str]' - A scheme in string format.
    ///
    /// # Returns
    /// A [Result] containing:
    /// - A [SchemeParameters] object representing the scheme that was passed, populated with the input parameters
    /// - [ENOENT] if the passed scheme path is not valid for this driver.
    ///
    /// # Notes
    /// ENOENT is returned so that it can easily be forwarded to the caller of open(). It cleans
    /// up the function considerably to be able to use the ? syntax.
    pub fn from_scheme(scheme: &str) -> Result<Self> {
        fn get_string_from_regex(
            rgx: &Regex,
            scheme: &str,
            capture_idx: usize,
        ) -> syscall::Result<String> {
            if let Some(capture_list) = rgx.captures(scheme) {
                if let Some(value) = capture_list.get(capture_idx + 1) {
                    return Ok(value.as_str().to_string());
                }
            }

            Err(Error::new(ENOENT))
        };

        fn get_port_id_from_regex(
            rgx: &Regex,
            scheme: &str,
            capture_idx: usize,
        ) -> syscall::Result<PortId> {
            if let Some(capture_list) = rgx.captures(scheme) {
                if let Some(value) = capture_list.get(capture_idx + 1) {
                    if let Ok(port_id) = value.as_str().parse::<PortId>() {
                        return Ok(port_id);
                    }
                }
            }

            Err(Error::new(ENOENT))
        };

        fn get_u8_from_regex(rgx: &Regex, scheme: &str, capture_idx: usize) -> syscall::Result<u8> {
            if let Some(capture_list) = rgx.captures(scheme) {
                if let Some(value) = capture_list.get(capture_idx + 1) {
                    if let Ok(integer) = value.as_str().parse::<u8>() {
                        return Ok(integer);
                    }
                }
            }

            Err(Error::new(ENOENT))
        };

        //We don't implement From::<&path::Path> because we don't want to make this a part of
        //the public interface. This function does not guarantee that the handle is VALID, only
        //that the scheme is valid. open() will validate the contents of the enumeration instance,
        //and store it if it's valid.

        //Generate the regular expressions for all of our valid schemes.

        //Check if we have a match and either return a partially initialized scheme, OR ENOENT
        if REGEX_PORT_CONFIGURE.is_match(scheme) {
            let port_num = get_port_id_from_regex(&REGEX_PORT_CONFIGURE, scheme, 0)?;

            Ok(Self::ConfigureEndpoints(port_num))
        } else if REGEX_PORT_ATTACH.is_match(scheme) {
            let port_num = get_port_id_from_regex(&REGEX_PORT_ATTACH, scheme, 0)?;

            Ok(Self::AttachDevice(port_num))
        } else if REGEX_PORT_DETACH.is_match(scheme) {
            let port_num = get_port_id_from_regex(&REGEX_PORT_DETACH, scheme, 0)?;

            Ok(Self::DetachDevice(port_num))
        } else if REGEX_PORT_DESCRIPTORS.is_match(scheme) {
            let port_num = get_port_id_from_regex(&REGEX_PORT_DESCRIPTORS, scheme, 0)?;

            Ok(Self::PortDesc(port_num))
        } else if REGEX_PORT_STATE.is_match(scheme) {
            let port_num = get_port_id_from_regex(&REGEX_PORT_STATE, scheme, 0)?;

            Ok(Self::PortState(port_num))
        } else if REGEX_PORT_REQUEST.is_match(scheme) {
            let port_num = get_port_id_from_regex(&REGEX_PORT_REQUEST, scheme, 0)?;

            Ok(Self::PortReq(port_num))
        } else if REGEX_PORT_ENDPOINTS.is_match(scheme) {
            let port_num = get_port_id_from_regex(&REGEX_PORT_ENDPOINTS, scheme, 0)?;

            Ok(Self::Endpoints(port_num))
        } else if REGEX_PORT_SPECIFIC_ENDPOINT.is_match(scheme) {
            let port_num = get_port_id_from_regex(&REGEX_PORT_SPECIFIC_ENDPOINT, scheme, 0)?;
            let endpoint_num =
                EndpNum::new(get_u8_from_regex(&REGEX_PORT_SPECIFIC_ENDPOINT, scheme, 1)?);

            Ok(Self::Endpoint(port_num, endpoint_num, String::from("root")))
        } else if REGEX_PORT_SUB_ENDPOINT.is_match(scheme) {
            let port_num = get_port_id_from_regex(&REGEX_PORT_SUB_ENDPOINT, scheme, 0)?;
            let endpoint_num =
                EndpNum::new(get_u8_from_regex(&REGEX_PORT_SUB_ENDPOINT, scheme, 1)?);
            let handle_type = get_string_from_regex(&REGEX_PORT_SUB_ENDPOINT, scheme, 2)?;

            Ok(Self::Endpoint(port_num, endpoint_num, handle_type))
        } else if REGEX_PORT_ROOT.is_match(scheme) {
            let port_num = get_port_id_from_regex(&REGEX_PORT_ROOT, scheme, 0)?;
            Ok(Self::Port(port_num))
        } else if REGEX_TOP_LEVEL.is_match(scheme) {
            Ok(Self::TopLevel)
        } else {
            Err(Error::new(ENOENT))
        }
    }
}
