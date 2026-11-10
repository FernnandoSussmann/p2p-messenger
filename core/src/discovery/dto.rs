//! String-only DTOs and the single event-to-JSON mapping, shared by the Tauri
//! shell and the headless node so both emit identical JSON.

use serde::Serialize;
use serde_json::Value;

use super::events::{ConnectionState, Direction, FailureReason, IncomingRequest, NetEvent};
use crate::identity::NodeId;

#[derive(Debug, Serialize)]
pub struct MyTicketDto {
    pub ticket: String,
    pub qr_svg: String,
}

#[derive(Debug, Serialize)]
pub struct IncomingRequestDto {
    pub node_id: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectionStateDto {
    pub node_id: String,
    pub direction: &'static str,
    pub state: &'static str,
    pub reason: Option<&'static str>,
}

impl From<&IncomingRequest> for IncomingRequestDto {
    fn from(req: &IncomingRequest) -> Self {
        Self {
            node_id: req.node_id.to_string(),
            display_name: req.display_name.as_ref().map(|n| n.as_str().to_owned()),
        }
    }
}

impl ConnectionStateDto {
    pub fn new(node_id: &NodeId, direction: &Direction, state: &ConnectionState) -> Self {
        let direction = match direction {
            Direction::Inbound => "inbound",
            Direction::Outbound => "outbound",
        };
        let (state, reason) = match state {
            ConnectionState::Connecting => ("connecting", None),
            ConnectionState::Connected => ("connected", None),
            ConnectionState::Disconnected => ("disconnected", None),
            ConnectionState::Failed(r) => ("failed", Some(reason_str(r))),
        };
        Self {
            node_id: node_id.to_string(),
            direction,
            state,
            reason,
        }
    }
}

fn reason_str(reason: &FailureReason) -> &'static str {
    match reason {
        FailureReason::Declined => "declined",
        FailureReason::Timeout => "timeout",
        FailureReason::Unreachable => "unreachable",
        FailureReason::Protocol => "protocol",
        FailureReason::Expired => "expired",
        FailureReason::Closed => "closed",
    }
}

/// The external event name and JSON payload for a core event. The exhaustive
/// match makes a new `NetEvent` variant fail to compile until it is named.
pub fn event_json(ev: &NetEvent) -> (&'static str, Value) {
    match ev {
        NetEvent::IncomingRequest(req) => {
            ("incoming_request", to_value(&IncomingRequestDto::from(req)))
        }
        NetEvent::ConnectionState {
            node_id,
            direction,
            state,
        } => (
            "connection_state",
            to_value(&ConnectionStateDto::new(node_id, direction, state)),
        ),
        NetEvent::MyTicketChanged => ("my_ticket_changed", Value::Null),
    }
}

fn to_value<T: Serialize>(dto: &T) -> Value {
    // DTOs hold only strings and options, so serialization cannot fail.
    serde_json::to_value(dto).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::validate_display_name;
    use serde_json::json;

    fn node_id(seed: u8) -> NodeId {
        iroh_base::SecretKey::from_bytes(&[seed; 32]).public()
    }

    #[test]
    fn incoming_request_json_shape() {
        let id = node_id(1);
        let with_name = NetEvent::IncomingRequest(IncomingRequest {
            node_id: id,
            display_name: Some(validate_display_name("Ada").unwrap()),
        });
        let (name, v) = event_json(&with_name);
        assert_eq!(name, "incoming_request");
        assert_eq!(v, json!({"node_id": id.to_string(), "display_name": "Ada"}));

        let no_name = NetEvent::IncomingRequest(IncomingRequest {
            node_id: id,
            display_name: None,
        });
        let (_, v) = event_json(&no_name);
        assert_eq!(v, json!({"node_id": id.to_string(), "display_name": null}));
    }

    #[test]
    fn connection_state_json_shape() {
        let id = node_id(2);
        let cases = [
            (ConnectionState::Connecting, "connecting", None),
            (ConnectionState::Connected, "connected", None),
            (ConnectionState::Disconnected, "disconnected", None),
            (
                ConnectionState::Failed(FailureReason::Declined),
                "failed",
                Some("declined"),
            ),
            (
                ConnectionState::Failed(FailureReason::Timeout),
                "failed",
                Some("timeout"),
            ),
            (
                ConnectionState::Failed(FailureReason::Unreachable),
                "failed",
                Some("unreachable"),
            ),
            (
                ConnectionState::Failed(FailureReason::Protocol),
                "failed",
                Some("protocol"),
            ),
            (
                ConnectionState::Failed(FailureReason::Expired),
                "failed",
                Some("expired"),
            ),
            (
                ConnectionState::Failed(FailureReason::Closed),
                "failed",
                Some("closed"),
            ),
        ];
        for (direction, dir_str) in [
            (Direction::Inbound, "inbound"),
            (Direction::Outbound, "outbound"),
        ] {
            for (state, state_str, reason) in &cases {
                let ev = NetEvent::ConnectionState {
                    node_id: id,
                    direction,
                    state: *state,
                };
                let (name, v) = event_json(&ev);
                assert_eq!(name, "connection_state");
                assert_eq!(
                    v,
                    json!({
                        "node_id": id.to_string(),
                        "direction": dir_str,
                        "state": state_str,
                        "reason": reason,
                    })
                );
            }
        }
    }

    #[test]
    fn my_ticket_changed_has_null_payload() {
        assert_eq!(
            event_json(&NetEvent::MyTicketChanged),
            ("my_ticket_changed", Value::Null)
        );
    }

    #[test]
    fn my_ticket_dto_json_shape() {
        let dto = MyTicketDto {
            ticket: "p2pm:x".into(),
            qr_svg: "<svg/>".into(),
        };
        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            json!({"ticket": "p2pm:x", "qr_svg": "<svg/>"})
        );
    }

    #[test]
    fn event_names_match_dd02_api_table() {
        let id = node_id(3);
        let events = [
            NetEvent::IncomingRequest(IncomingRequest {
                node_id: id,
                display_name: None,
            }),
            NetEvent::ConnectionState {
                node_id: id,
                direction: Direction::Outbound,
                state: ConnectionState::Connected,
            },
            NetEvent::MyTicketChanged,
        ];
        for ev in &events {
            // Exhaustive: a new variant must be added here and to the table.
            let expected = match ev {
                NetEvent::IncomingRequest(_) => "incoming_request",
                NetEvent::ConnectionState { .. } => "connection_state",
                NetEvent::MyTicketChanged => "my_ticket_changed",
            };
            assert_eq!(event_json(ev).0, expected);
        }
    }
}
