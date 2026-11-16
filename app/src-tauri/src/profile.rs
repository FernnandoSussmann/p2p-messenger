//! Developer/test profile selection (`--profile <name>` / `MESSENGER_PROFILE`).
//!
//! An invalid value is an error, never a silent fall-back to the default
//! profile, because that would mix two testers' identities.

use std::path::{Path, PathBuf};

use messenger_core::identity::{Profile, ProfileError};

/// Resolves the active profile. The CLI wins over the environment.
pub fn profile_from(
    args: impl IntoIterator<Item = String>,
    env: Option<String>,
) -> Result<Option<Profile>, ProfileError> {
    let mut args = args.into_iter();
    let mut from_cli = None;
    while let Some(arg) = args.next() {
        if arg == "--profile" {
            from_cli = Some(args.next().unwrap_or_default());
            break;
        }
        if let Some(value) = arg.strip_prefix("--profile=") {
            from_cli = Some(value.to_owned());
            break;
        }
    }
    from_cli.or(env).map(|raw| Profile::parse(&raw)).transpose()
}

pub struct ProfileLayout {
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub profile: Option<Profile>,
}

pub fn layout(base_data: &Path, base_log: &Path, profile: Option<Profile>) -> ProfileLayout {
    let (data_dir, log_dir) = match &profile {
        Some(p) => (p.data_dir(base_data), p.data_dir(base_log)),
        None => (base_data.to_path_buf(), base_log.to_path_buf()),
    };
    ProfileLayout {
        data_dir,
        log_dir,
        profile,
    }
}

pub fn window_title(base: &str, profile: Option<&Profile>) -> String {
    match profile {
        Some(p) => format!("{base} \u{2014} profile {}", p.as_str()),
        None => base.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn name(p: Result<Option<Profile>, ProfileError>) -> Option<String> {
        p.unwrap().map(|p| p.as_str().to_owned())
    }

    #[test]
    fn no_override_is_none() {
        assert_eq!(name(profile_from(args(&["app"]), None)), None);
    }

    #[test]
    fn env_is_used_when_no_cli() {
        assert_eq!(
            name(profile_from(args(&["app"]), Some("Env".into()))),
            Some("env".into())
        );
    }

    #[test]
    fn cli_wins_over_env_in_both_forms() {
        let env = || Some("env".to_string());
        assert_eq!(
            name(profile_from(args(&["app", "--profile", "b"]), env())),
            Some("b".into())
        );
        assert_eq!(
            name(profile_from(args(&["app", "--profile=c"]), env())),
            Some("c".into())
        );
        assert_eq!(
            name(profile_from(
                args(&["app", "--profile", "ok"]),
                Some("bad/".into())
            )),
            Some("ok".into())
        );
    }

    #[test]
    fn invalid_values_are_errors_not_the_default() {
        assert!(profile_from(args(&["app", "--profile", "../x"]), None).is_err());
        assert!(profile_from(args(&["app", "--profile=default"]), None).is_err());
        assert!(profile_from(args(&["app", "--profile"]), None).is_err());
        assert!(profile_from(args(&["app"]), Some("a b".into())).is_err());
    }

    #[test]
    fn layout_isolates_profiles() {
        let base = Path::new("/data");
        assert_eq!(layout(base, Path::new("/logs"), None).data_dir, base);
        let (a, b) = (Profile::parse("a").unwrap(), Profile::parse("b").unwrap());
        assert_ne!(a.keychain_service(), b.keychain_service());
        let logs = Path::new("/logs");
        let (la, lb) = (layout(base, logs, Some(a)), layout(base, logs, Some(b)));
        assert_ne!(la.data_dir, lb.data_dir);
        assert!(la.data_dir.starts_with(base));
    }

    #[test]
    fn log_dir_is_profile_namespaced() {
        let (data, logs) = (Path::new("/data"), Path::new("/logs"));
        assert_eq!(layout(data, logs, None).log_dir, logs);
        let (a, b) = (Profile::parse("a").unwrap(), Profile::parse("b").unwrap());
        let (la, lb) = (layout(data, logs, Some(a)), layout(data, logs, Some(b)));
        assert_ne!(la.log_dir, lb.log_dir);
        assert!(la.log_dir.starts_with(logs.join("profiles")));
        assert_ne!(la.log_dir, la.data_dir);
    }

    #[test]
    fn title_only_changes_with_a_profile() {
        assert_eq!(window_title("p2p-messenger", None), "p2p-messenger");
        let b = Profile::parse("b").unwrap();
        assert!(window_title("p2p-messenger", Some(&b)).ends_with("profile b"));
    }
}
