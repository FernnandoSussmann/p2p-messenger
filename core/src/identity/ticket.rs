//! The DD-02 seam for the shareable connection ticket.
//!
//! A ticket is `p2pm:` followed by an iroh `EndpointTicket` string (NodeId plus
//! relay and direct addresses). The legacy `p2pm:<nodeid>` form still parses,
//! as a NodeId-only address. [`NodeIdTicketProvider`] produces that legacy form
//! and is the fallback before a live endpoint exists. A ticket carries public
//! data only.

use std::fmt;

use iroh_base::{EndpointAddr, TransportAddr};
use iroh_tickets::endpoint::EndpointTicket;
use qrcode::render::svg;
use qrcode::QrCode;

pub use iroh_base::PublicKey as NodeId;

const TICKET_PREFIX: &str = "p2pm:";

/// Longest accepted ticket string; checked before any decoding.
pub const MAX_TICKET_LEN: usize = 4096;
const MAX_PARSED_TRANSPORT_ADDRS: usize = 16;
const MAX_PARSED_RELAY_URLS: usize = 4;
const MAX_PRODUCED_IP_ADDRS: usize = 8;

#[derive(Debug)]
pub enum TicketError {
    /// The string is not a ticket produced by this app.
    Malformed,
    /// The ticket exceeds [`MAX_TICKET_LEN`].
    TooLarge,
    /// The ticket could not be rendered as a QR code.
    Qr(String),
}

impl fmt::Display for TicketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TicketError::Malformed => write!(f, "malformed ticket"),
            TicketError::TooLarge => write!(f, "ticket too large"),
            TicketError::Qr(msg) => write!(f, "QR rendering failed: {msg}"),
        }
    }
}

impl std::error::Error for TicketError {}

/// A shareable, string-serialisable connection ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket(String);

impl Ticket {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parses pasted ticket text. Trims ASCII whitespace, bounds the length
    /// before decoding, and never panics on arbitrary input.
    pub fn parse(input: &str) -> Result<ParsedTicket, TicketError> {
        if input.len() > MAX_TICKET_LEN {
            return Err(TicketError::TooLarge);
        }
        let body = input
            .trim_matches(|c: char| c.is_ascii_whitespace())
            .strip_prefix(TICKET_PREFIX)
            .ok_or(TicketError::Malformed)?;
        let addr = match body.parse::<EndpointTicket>() {
            Ok(ticket) => ticket.endpoint_addr().clone(),
            Err(_) => {
                EndpointAddr::new(body.parse::<NodeId>().map_err(|_| TicketError::Malformed)?)
            }
        };
        let relays = addr
            .addrs
            .iter()
            .filter(|a| matches!(a, TransportAddr::Relay(_)))
            .count();
        if addr.addrs.len() > MAX_PARSED_TRANSPORT_ADDRS || relays > MAX_PARSED_RELAY_URLS {
            return Err(TicketError::Malformed);
        }
        Ok(ParsedTicket {
            node_id: addr.id,
            addr,
        })
    }

    /// Builds a ticket from a live endpoint address, keeping at most 4 relay
    /// URLs and 8 IP addresses so it stays small enough for a QR code.
    pub fn from_endpoint_addr(addr: &EndpointAddr) -> Result<Ticket, TicketError> {
        let mut ips = 0;
        let mut relays = 0;
        let kept = addr.addrs.iter().filter(|a| match a {
            TransportAddr::Relay(_) => {
                relays += 1;
                relays <= MAX_PARSED_RELAY_URLS
            }
            TransportAddr::Ip(_) => {
                ips += 1;
                ips <= MAX_PRODUCED_IP_ADDRS
            }
            _ => false,
        });
        let trimmed = EndpointAddr::from_parts(addr.id, kept.cloned());
        let text = format!("{TICKET_PREFIX}{}", EndpointTicket::new(trimmed));
        if text.len() > MAX_TICKET_LEN {
            return Err(TicketError::TooLarge);
        }
        Ok(Ticket(text))
    }
}

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTicket {
    pub node_id: NodeId,
    pub addr: EndpointAddr,
}

pub trait TicketProvider {
    /// Assembles the current shareable ticket for `node_id`.
    fn current_ticket(&self, node_id: &NodeId) -> Result<Ticket, TicketError>;
}

#[derive(Debug, Default)]
pub struct NodeIdTicketProvider;

impl TicketProvider for NodeIdTicketProvider {
    fn current_ticket(&self, node_id: &NodeId) -> Result<Ticket, TicketError> {
        Ok(Ticket(format!("{TICKET_PREFIX}{node_id}")))
    }
}

/// Renders the ticket as an SVG QR code for display.
pub fn ticket_to_qr_svg(ticket: &Ticket) -> Result<String, TicketError> {
    let code =
        QrCode::new(ticket.as_str().as_bytes()).map_err(|e| TicketError::Qr(e.to_string()))?;
    Ok(code.render::<svg::Color>().build())
}

#[cfg(test)]
mod tests {
    use iroh_base::SecretKey;

    use super::*;

    fn known_node_id() -> NodeId {
        SecretKey::from_bytes(&[7u8; 32]).public()
    }

    fn known_ticket() -> Ticket {
        NodeIdTicketProvider
            .current_ticket(&known_node_id())
            .unwrap()
    }

    #[test]
    fn ticket_round_trips_node_id() {
        let parsed = Ticket::parse(known_ticket().as_str()).unwrap();
        assert_eq!(parsed.node_id, known_node_id());
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(matches!(Ticket::parse("nope"), Err(TicketError::Malformed)));
        assert!(matches!(
            Ticket::parse("p2pm:zzzz"),
            Err(TicketError::Malformed)
        ));
    }

    fn full_addr() -> EndpointAddr {
        EndpointAddr::new(known_node_id())
            .with_relay_url("https://relay.example.com".parse().unwrap())
            .with_ip_addr("192.168.1.5:4433".parse().unwrap())
            .with_ip_addr("[2001:db8::1]:4433".parse().unwrap())
    }

    fn raw_ticket(addr: EndpointAddr) -> String {
        format!("{TICKET_PREFIX}{}", EndpointTicket::new(addr))
    }

    #[test]
    fn live_ticket_round_trips_relay_and_addrs() {
        let addr = full_addr();
        let ticket = Ticket::from_endpoint_addr(&addr).unwrap();
        assert!(ticket.as_str().starts_with("p2pm:"));
        let parsed = Ticket::parse(ticket.as_str()).unwrap();
        assert_eq!(parsed.node_id, known_node_id());
        assert_eq!(parsed.addr, addr);
    }

    #[test]
    fn legacy_ticket_parses_as_node_id_only() {
        let parsed = Ticket::parse(known_ticket().as_str()).unwrap();
        assert_eq!(parsed.addr, EndpointAddr::new(known_node_id()));
    }

    #[test]
    fn parse_trims_ascii_whitespace() {
        let t = format!("  \n{}\t ", known_ticket());
        assert_eq!(Ticket::parse(&t).unwrap().node_id, known_node_id());
    }

    #[test]
    fn parse_rejects_malformed_inputs() {
        let live = Ticket::from_endpoint_addr(&full_addr()).unwrap();
        let truncated = &live.as_str()[..live.as_str().len() - 5];
        let wrong_prefix = live.as_str().replacen("p2pm:", "p2px:", 1);
        for bad in [
            "",
            "   \n",
            "p2pm:",
            "p2pm:endpoint",
            truncated,
            &wrong_prefix,
        ] {
            assert!(
                matches!(Ticket::parse(bad), Err(TicketError::Malformed)),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn parse_rejects_oversize_before_decoding() {
        let big = format!("p2pm:{}", "a".repeat(MAX_TICKET_LEN));
        assert!(matches!(Ticket::parse(&big), Err(TicketError::TooLarge)));
        let exact = "a".repeat(MAX_TICKET_LEN);
        assert!(matches!(Ticket::parse(&exact), Err(TicketError::Malformed)));
    }

    #[test]
    fn parse_rejects_too_many_addrs_and_relays() {
        let mut addr = EndpointAddr::new(known_node_id());
        for i in 0..17u16 {
            addr = addr.with_ip_addr(format!("10.0.0.1:{}", 1000 + i).parse().unwrap());
        }
        assert!(matches!(
            Ticket::parse(&raw_ticket(addr)),
            Err(TicketError::Malformed)
        ));

        let mut addr = EndpointAddr::new(known_node_id());
        for i in 0..5 {
            addr = addr.with_relay_url(format!("https://r{i}.example.com").parse().unwrap());
        }
        assert!(matches!(
            Ticket::parse(&raw_ticket(addr)),
            Err(TicketError::Malformed)
        ));
    }

    #[test]
    fn produced_ticket_is_trimmed_and_renders_qr() {
        let mut addr = EndpointAddr::new(known_node_id());
        for i in 0..12 {
            addr = addr.with_ip_addr(format!("[2001:db8::{i:x}]:{}", 4000 + i).parse().unwrap());
        }
        for i in 0..6 {
            addr = addr.with_relay_url(format!("https://relay{i}.example.com").parse().unwrap());
        }
        let ticket = Ticket::from_endpoint_addr(&addr).unwrap();
        let parsed = Ticket::parse(ticket.as_str()).unwrap();
        assert_eq!(parsed.addr.ip_addrs().count(), 8);
        assert_eq!(parsed.addr.relay_urls().count(), 4);
        assert!(ticket.as_str().len() <= MAX_TICKET_LEN);
        assert!(ticket_to_qr_svg(&ticket).unwrap().contains("<svg"));
    }

    #[test]
    fn parse_never_panics_on_arbitrary_input() {
        let live = Ticket::from_endpoint_addr(&full_addr()).unwrap();
        let live = live.as_str().as_bytes();
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..10_000 {
            let len = (next() % 200) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            let _ = Ticket::parse(&String::from_utf8_lossy(&bytes));
            let _ = Ticket::parse(&format!("p2pm:{}", String::from_utf8_lossy(&bytes)));
            let mut mutated = live.to_vec();
            mutated.truncate((next() as usize) % (mutated.len() + 1));
            if !mutated.is_empty() {
                let i = (next() as usize) % mutated.len();
                mutated[i] = next() as u8;
            }
            let _ = Ticket::parse(&String::from_utf8_lossy(&mutated));
        }
    }

    #[test]
    fn qr_svg_is_wellformed() {
        let svg = ticket_to_qr_svg(&known_ticket()).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(svg.len() > 200);
    }

    #[test]
    fn ticket_contains_no_secret() {
        let secret = [7u8; 32];
        let secret_hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
        let ticket = known_ticket();
        let svg = ticket_to_qr_svg(&ticket).unwrap();
        assert!(!ticket.as_str().contains(&secret_hex));
        assert!(!svg.contains(&secret_hex));
    }
}
