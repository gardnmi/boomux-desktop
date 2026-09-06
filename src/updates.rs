//! Read-only release discovery. No installs or daemon lifecycle operations.
use semver::Version;
use serde_json::Value;
use std::{
    io::Read,
    process::{Command, Stdio},
};

const LIMIT: u64 = 128 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub current: String,
    pub latest: String,
    pub url: String,
}

impl Notice {
    pub fn visible(&self, dismissed: &str) -> bool {
        self.latest != dismissed
    }
}

#[derive(Default)]
pub struct Check {
    pub desktop: Option<Notice>,
    pub boomux: Option<Notice>,
    pub bundled: bool,
    pub unavailable: bool,
}

pub fn valid_dismissal(text: &str) -> bool {
    text.is_empty() || (text.len() <= 64 && Version::parse(text).is_ok())
}

fn notice(current: &str, tag: &str, repository: &str) -> Option<Notice> {
    if current.len() > 64 || tag.len() > 64 {
        return None;
    }
    let current = Version::parse(current).ok()?;
    let latest = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
    if !latest.pre.is_empty() || !latest.cmp_precedence(&current).is_gt() {
        return None;
    }
    Some(Notice {
        current: current.to_string(),
        latest: latest.to_string(),
        url: format!("https://github.com/gardnmi/{repository}/releases/tag/{tag}"),
    })
}

fn desktop_release(raw: &[u8], current: &str) -> Option<Notice> {
    let release: Value = serde_json::from_slice(raw).ok()?;
    if release.get("draft")?.as_bool()? || release.get("prerelease")?.as_bool()? {
        return None;
    }
    notice(
        current,
        release.get("tag_name")?.as_str()?,
        "boomux-desktop",
    )
}

fn boomux_status(raw: &[u8]) -> Option<Notice> {
    let envelope: Value = serde_json::from_slice(raw).ok()?;
    if envelope.get("schema")?.as_str()? != "boomux.cli/v1"
        || envelope.get("command")?.as_str()? != "update.status"
    {
        return None;
    }
    let data = envelope.get("data")?;
    let current = data.get("current")?.as_str()?;
    let latest = data.get("latest")?.as_str()?;
    notice(current, &format!("v{latest}"), "boomux")
}

// Both process lifetime and retained output are bounded. Call only on a worker.
fn output(program: &str, args: &[&str]) -> Option<Vec<u8>> {
    let mut child = Command::new("timeout")
        .args(["--kill-after=1s", "20s", program])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut bytes = Vec::new();
    let read = child.stdout.take()?.take(LIMIT + 1).read_to_end(&mut bytes);
    let status = child.wait().ok()?;
    (read.is_ok() && status.success() && bytes.len() as u64 <= LIMIT).then_some(bytes)
}

pub fn check() -> Check {
    let desktop_raw = output(
        "curl",
        &[
            "--disable",
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "5",
            "--max-time",
            "15",
            "--max-filesize",
            "131072",
            "--header",
            "Accept: application/vnd.github+json",
            "--user-agent",
            "boomux-desktop-update-check",
            "https://api.github.com/repos/gardnmi/boomux-desktop/releases/latest",
        ],
    );
    let boomux_raw = output("boomux", &["--json", "update", "status"]);
    let desktop = desktop_raw
        .as_deref()
        .and_then(|raw| desktop_release(raw, env!("CARGO_PKG_VERSION")));
    let boomux = boomux_raw.as_deref().and_then(boomux_status);
    let unavailable = desktop_raw
        .as_deref()
        .and_then(|raw| serde_json::from_slice::<Value>(raw).ok())
        .is_none_or(|value| !value["tag_name"].is_string())
        || boomux_raw
            .as_deref()
            .and_then(|raw| serde_json::from_slice::<Value>(raw).ok())
            .is_none_or(|value| !value["data"]["latest"].is_string());
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.parent()?
                .parent()
                .map(|parent| parent.join("release.txt"))
        })
        .is_some_and(|path| path.is_file());
    Check {
        desktop,
        boomux,
        bundled,
        unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_newer_stable_releases_are_offered() {
        for tag in ["v0.1.0", "v0.0.9", "v0.2.0-beta.1", "bad", "v0.1.0+rebuild"] {
            assert!(notice("0.1.0", tag, "boomux-desktop").is_none());
        }
        assert!(notice("0.9.0", "v0.10.0", "boomux-desktop").is_some());
        assert!(
            desktop_release(
                br#"{"tag_name":"v0.2.0","draft":true,"prerelease":false}"#,
                "0.1.0"
            )
            .is_none()
        );
        assert!(desktop_release(b"invalid", "0.1.0").is_none());
    }
    #[test]
    fn dismissal_is_version_specific_and_urls_are_fixed_to_the_owner() {
        let update = desktop_release(br#"{"tag_name":"v0.2.0","draft":false,"prerelease":false,"html_url":"https://untrusted.example"}"#, "0.1.0").unwrap();
        assert!(!update.visible("0.2.0"));
        assert!(update.visible("0.1.1"));
        assert_eq!(
            update.url,
            "https://github.com/gardnmi/boomux-desktop/releases/tag/v0.2.0"
        );
        assert!(!valid_dismissal("unbounded or malformed text"));
    }
    #[test]
    fn boomux_discovery_requires_the_supported_envelope() {
        let raw = br#"{"schema":"boomux.cli/v1","command":"update.status","data":{"current":"1.9.5","latest":"1.10.0","install_kind":"source_build"}}"#;
        assert_eq!(boomux_status(raw).unwrap().latest, "1.10.0");
        assert!(boomux_status(br#"{"schema":"unknown","command":"update.status","data":{"current":"1.9.5","latest":"1.10.0"}}"#).is_none());
        assert!(boomux_status(br#"{"schema":"boomux.cli/v1","command":"update.status","data":{"current":"1.9.5","latest":null}}"#).is_none());
    }
}
