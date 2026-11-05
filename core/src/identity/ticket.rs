//! The DD-02 seam for the shareable connection ticket.
//!
//! [`NodeIdTicketProvider`] is a placeholder that encodes only the NodeId.
//! Relay and direct addresses are absent until DD-02 supplies a live provider.
//! A ticket carries public data only.

use std::fmt;

use qrcode::render::svg;
use qrcode::QrCode;

pub use iroh_base::PublicKey as NodeId;

const TICKET_PREFIX: &str = "p2pm:";

#[derive(Debug)]
pub enum TicketError {
    /// The string is not a ticket produced by this app.
    Malformed,
    /// The ticket could not be rendered as a QR code.
    Qr(String),
}

impl fmt::Display for TicketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TicketError::Malformed => write!(f, "malformed ticket"),
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

    pub fn parse(input: &str) -> Result<ParsedTicket, TicketError> {
        let encoded = input
            .strip_prefix(TICKET_PREFIX)
            .ok_or(TicketError::Malformed)?;
        let node_id = encoded
            .parse::<NodeId>()
            .map_err(|_| TicketError::Malformed)?;
        Ok(ParsedTicket { node_id })
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
