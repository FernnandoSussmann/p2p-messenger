//! Hand-rolled argument parsing for `messenger-headless run` / `ctl`.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use messenger_core::identity::{validate_display_name, validate_relay_url, DisplayName};

#[derive(Debug, PartialEq, Eq)]
pub struct CliError(pub String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

fn err<T>(msg: impl Into<String>) -> Result<T, CliError> {
    Err(CliError(msg.into()))
}

#[derive(Debug, PartialEq, Eq)]
pub enum RelayArg {
    Default,
    Disabled,
    /// A URL already accepted by `validate_relay_url` (canonical form).
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanArg {
    Mdns,
    Off,
}

#[derive(Debug)]
pub struct RunArgs {
    pub data_dir: PathBuf,
    pub name: DisplayName,
    pub relay: RelayArg,
    pub lan: LanArg,
    pub bind: Option<SocketAddr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitEvent {
    IncomingRequest,
    ConnectionState,
    LanPeersChanged,
}

impl WaitEvent {
    /// The Tauri event name this selector matches.
    pub fn name(self) -> &'static str {
        match self {
            WaitEvent::IncomingRequest => "incoming_request",
            WaitEvent::ConnectionState => "connection_state",
            WaitEvent::LanPeersChanged => "lan_peers_changed",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct WaitArgs {
    pub event: WaitEvent,
    pub node: Option<String>,
    pub state: Option<String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CtlCommand {
    NodeId,
    Ticket,
    Presence(Option<bool>),
    Peers,
    Connect(String),
    Respond {
        node_id: String,
        accept: bool,
        nickname: Option<String>,
    },
    Connected,
    Contacts,
    Wait(WaitArgs),
    Shutdown,
}

#[derive(Debug)]
pub enum Command {
    Run(RunArgs),
    Ctl {
        data_dir: PathBuf,
        command: CtlCommand,
    },
}

/// Parses the arguments that follow the program name.
pub fn parse<I>(args: I) -> Result<Command, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut it = args.into_iter();
    match it.next().as_deref() {
        Some("run") => parse_run(it).map(Command::Run),
        Some("ctl") => parse_ctl(it),
        Some(other) => err(format!("unknown subcommand: {other}")),
        None => err("expected a subcommand: run | ctl"),
    }
}

fn value(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, CliError> {
    it.next()
        .ok_or_else(|| CliError(format!("{flag} requires a value")))
}

fn set_once<T>(slot: &mut Option<T>, flag: &str, v: T) -> Result<(), CliError> {
    if slot.replace(v).is_some() {
        return err(format!("{flag} given more than once"));
    }
    Ok(())
}

fn parse_run(mut it: impl Iterator<Item = String>) -> Result<RunArgs, CliError> {
    let mut data_dir = None;
    let mut keyring = None;
    let mut name = None;
    let mut relay = None;
    let mut lan = None;
    let mut bind = None;
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--data-dir" => set_once(&mut data_dir, &flag, PathBuf::from(value(&mut it, &flag)?))?,
            "--keyring" => set_once(&mut keyring, &flag, value(&mut it, &flag)?)?,
            "--name" => set_once(&mut name, &flag, value(&mut it, &flag)?)?,
            "--relay" => set_once(&mut relay, &flag, value(&mut it, &flag)?)?,
            "--lan" => set_once(&mut lan, &flag, value(&mut it, &flag)?)?,
            "--bind" => set_once(&mut bind, &flag, value(&mut it, &flag)?)?,
            other => return err(format!("unknown argument: {other}")),
        }
    }

    match keyring.as_deref() {
        Some("file") => {}
        Some(_) => {
            return err("--keyring must be `file`: the headless node never uses the OS keychain")
        }
        None => return err("--keyring file is required"),
    }
    let data_dir = data_dir.ok_or_else(|| CliError("--data-dir is required".into()))?;
    let name = name.ok_or_else(|| CliError("--name is required".into()))?;
    let name =
        validate_display_name(&name).map_err(|e| CliError(format!("invalid --name: {e}")))?;
    let relay = match relay.as_deref() {
        None | Some("default") => RelayArg::Default,
        Some("disabled") => RelayArg::Disabled,
        Some(url) => RelayArg::Custom(
            validate_relay_url(url).map_err(|_| CliError("invalid --relay URL".into()))?,
        ),
    };
    let lan = match lan.as_deref() {
        None | Some("mdns") => LanArg::Mdns,
        Some("off") => LanArg::Off,
        Some(_) => return err("--lan must be `mdns` or `off`"),
    };
    let bind = bind
        .map(|b| {
            b.parse::<SocketAddr>()
                .map_err(|_| CliError("invalid --bind: expected IP:PORT".into()))
        })
        .transpose()?;
    Ok(RunArgs {
        data_dir,
        name,
        relay,
        lan,
        bind,
    })
}

fn parse_ctl(mut it: impl Iterator<Item = String>) -> Result<Command, CliError> {
    if it.next().as_deref() != Some("--data-dir") {
        return err("ctl requires --data-dir <DIR> before the command");
    }
    let data_dir = PathBuf::from(value(&mut it, "--data-dir")?);
    let name = it
        .next()
        .ok_or_else(|| CliError("ctl requires a command".into()))?;
    let command = match name.as_str() {
        "node-id" => CtlCommand::NodeId,
        "ticket" => CtlCommand::Ticket,
        "peers" => CtlCommand::Peers,
        "connected" => CtlCommand::Connected,
        "contacts" => CtlCommand::Contacts,
        "shutdown" => CtlCommand::Shutdown,
        "presence" => CtlCommand::Presence(match it.next().as_deref() {
            None => None,
            Some("on") => Some(true),
            Some("off") => Some(false),
            Some(_) => return err("presence takes `on` or `off`"),
        }),
        "connect" => CtlCommand::Connect(value(&mut it, "connect")?),
        "respond" => {
            let node_id = value(&mut it, "respond")?;
            let (accept, nickname) = match it.next().as_deref() {
                Some("accept") => (true, it.next()),
                Some("decline") => (false, None),
                _ => return err("respond takes <NODE_ID> accept [NICKNAME] | decline"),
            };
            CtlCommand::Respond {
                node_id,
                accept,
                nickname,
            }
        }
        "wait" => CtlCommand::Wait(parse_wait(&mut it)?),
        other => return err(format!("unknown command: {other}")),
    };
    if let Some(extra) = it.next() {
        return err(format!("unexpected argument: {extra}"));
    }
    Ok(Command::Ctl { data_dir, command })
}

fn parse_wait(it: &mut impl Iterator<Item = String>) -> Result<WaitArgs, CliError> {
    let event = match it.next().as_deref() {
        Some("incoming_request") => WaitEvent::IncomingRequest,
        Some("connection_state") => WaitEvent::ConnectionState,
        Some("lan_peers_changed") => WaitEvent::LanPeersChanged,
        _ => return err("wait takes incoming_request | connection_state | lan_peers_changed"),
    };
    let (mut node, mut state, mut timeout_secs) = (None, None, None);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--node" => set_once(&mut node, &flag, value(it, &flag)?)?,
            "--state" => set_once(&mut state, &flag, value(it, &flag)?)?,
            "--timeout-secs" => {
                let n = value(it, &flag)?
                    .parse::<u64>()
                    .map_err(|_| CliError("--timeout-secs must be a number".into()))?;
                set_once(&mut timeout_secs, &flag, n)?;
            }
            other => return err(format!("unknown argument: {other}")),
        }
    }
    Ok(WaitArgs {
        event,
        node,
        state,
        timeout_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: &str) -> Result<Command, CliError> {
        parse(line.split_whitespace().map(str::to_owned))
    }

    fn ctl(cmd: &str) -> CtlCommand {
        match p(&format!("ctl --data-dir /d {cmd}")).unwrap() {
            Command::Ctl { data_dir, command } => {
                assert_eq!(data_dir, PathBuf::from("/d"));
                command
            }
            other => panic!("not ctl: {other:?}"),
        }
    }

    fn run(line: &str) -> Result<RunArgs, CliError> {
        match p(line)? {
            Command::Run(r) => Ok(r),
            other => panic!("not run: {other:?}"),
        }
    }

    #[test]
    fn run_parses_all_flags() {
        let r = run("run --data-dir /d --keyring file --name Alice --relay disabled --lan off --bind 127.0.0.1:0")
            .unwrap();
        assert_eq!(r.data_dir, PathBuf::from("/d"));
        assert_eq!(r.name.as_str(), "Alice");
        assert_eq!(r.relay, RelayArg::Disabled);
        assert_eq!(r.lan, LanArg::Off);
        assert_eq!(r.bind, Some("127.0.0.1:0".parse().unwrap()));
    }

    #[test]
    fn run_defaults() {
        let r = run("run --data-dir /d --keyring file --name Alice").unwrap();
        assert_eq!(r.relay, RelayArg::Default);
        assert_eq!(r.lan, LanArg::Mdns);
        assert_eq!(r.bind, None);
    }

    #[test]
    fn missing_keyring_is_rejected() {
        assert!(run("run --data-dir /d --name Alice").is_err());
    }

    #[test]
    fn non_file_keyring_is_rejected() {
        assert!(run("run --data-dir /d --keyring os --name Alice").is_err());
        assert!(run("run --data-dir /d --keyring FILE --name Alice").is_err());
    }

    #[test]
    fn bad_name_is_rejected() {
        let blank = parse(
            [
                "run",
                "--data-dir",
                "/d",
                "--keyring",
                "file",
                "--name",
                "  ",
            ]
            .map(str::to_owned),
        );
        assert!(blank.is_err());
        let bidi = parse(
            [
                "run",
                "--data-dir",
                "/d",
                "--keyring",
                "file",
                "--name",
                "a\u{202E}b",
            ]
            .map(str::to_owned),
        );
        assert!(bidi.is_err());
        assert!(run("run --data-dir /d --keyring file").is_err());
    }

    #[test]
    fn relay_variants() {
        let base = "run --data-dir /d --keyring file --name A --relay";
        assert_eq!(
            run(&format!("{base} default")).unwrap().relay,
            RelayArg::Default
        );
        assert!(matches!(
            run(&format!("{base} https://relay.example.com"))
                .unwrap()
                .relay,
            RelayArg::Custom(_)
        ));
        assert!(run(&format!("{base} not-a-url")).is_err());
        assert!(run(&format!("{base} http://relay.example.com")).is_err());
    }

    #[test]
    fn bad_lan_bind_and_unknown_flags_are_rejected() {
        let base = "run --data-dir /d --keyring file --name A";
        assert!(run(&format!("{base} --lan wifi")).is_err());
        assert!(run(&format!("{base} --bind nope")).is_err());
        assert!(run(&format!("{base} --bogus x")).is_err());
        assert!(run(&format!("{base} --lan")).is_err());
        assert!(run(&format!("{base} --lan off --lan mdns")).is_err());
        assert!(run("run --keyring file --name A").is_err());
    }

    #[test]
    fn ctl_simple_commands() {
        assert_eq!(ctl("node-id"), CtlCommand::NodeId);
        assert_eq!(ctl("ticket"), CtlCommand::Ticket);
        assert_eq!(ctl("peers"), CtlCommand::Peers);
        assert_eq!(ctl("connected"), CtlCommand::Connected);
        assert_eq!(ctl("contacts"), CtlCommand::Contacts);
        assert_eq!(ctl("shutdown"), CtlCommand::Shutdown);
    }

    #[test]
    fn ctl_presence() {
        assert_eq!(ctl("presence"), CtlCommand::Presence(None));
        assert_eq!(ctl("presence on"), CtlCommand::Presence(Some(true)));
        assert_eq!(ctl("presence off"), CtlCommand::Presence(Some(false)));
        assert!(p("ctl --data-dir /d presence maybe").is_err());
    }

    #[test]
    fn ctl_connect_and_respond() {
        assert_eq!(
            ctl("connect p2pm:abc"),
            CtlCommand::Connect("p2pm:abc".into())
        );
        assert_eq!(
            ctl("respond n1 accept"),
            CtlCommand::Respond {
                node_id: "n1".into(),
                accept: true,
                nickname: None
            }
        );
        assert_eq!(
            ctl("respond n1 accept Bobby"),
            CtlCommand::Respond {
                node_id: "n1".into(),
                accept: true,
                nickname: Some("Bobby".into())
            }
        );
        assert_eq!(
            ctl("respond n1 decline"),
            CtlCommand::Respond {
                node_id: "n1".into(),
                accept: false,
                nickname: None
            }
        );
        assert!(p("ctl --data-dir /d respond n1 decline Bobby").is_err());
        assert!(p("ctl --data-dir /d respond n1 accept Bobby extra").is_err());
        assert!(p("ctl --data-dir /d respond n1").is_err());
        assert!(p("ctl --data-dir /d respond n1 maybe").is_err());
        assert!(p("ctl --data-dir /d connect").is_err());
    }

    #[test]
    fn ctl_wait() {
        assert_eq!(
            ctl("wait incoming_request"),
            CtlCommand::Wait(WaitArgs {
                event: WaitEvent::IncomingRequest,
                node: None,
                state: None,
                timeout_secs: None
            })
        );
        assert_eq!(
            ctl("wait connection_state --node n1 --state connected --timeout-secs 7"),
            CtlCommand::Wait(WaitArgs {
                event: WaitEvent::ConnectionState,
                node: Some("n1".into()),
                state: Some("connected".into()),
                timeout_secs: Some(7)
            })
        );
        assert!(matches!(
            ctl("wait lan_peers_changed"),
            CtlCommand::Wait(WaitArgs {
                event: WaitEvent::LanPeersChanged,
                ..
            })
        ));
        assert!(p("ctl --data-dir /d wait nope").is_err());
        assert!(p("ctl --data-dir /d wait connection_state --timeout-secs x").is_err());
        assert_eq!(WaitEvent::ConnectionState.name(), "connection_state");
    }

    #[test]
    fn unknown_and_malformed_commands_are_rejected() {
        assert!(p("ctl --data-dir /d frobnicate").is_err());
        assert!(p("ctl --data-dir /d").is_err());
        assert!(p("ctl node-id").is_err());
        assert!(p("ctl --data-dir /d node-id extra").is_err());
        assert!(p("bogus").is_err());
        assert!(p("").is_err());
    }
}
