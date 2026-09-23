use crate::model::ServiceAction;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, os::unix::process::CommandExt, time::Duration};
use url::Url;

pub fn service_args(action: &ServiceAction, unit: &str, user: bool) -> Result<Vec<String>> {
    ensure!(
        unit.ends_with(".service")
            && unit
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && unit
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.@-".contains(&byte)),
        "Unit must be a plain .service name"
    );
    let mut args = vec!["--no-pager".into(), "--no-ask-password".into()];
    if user {
        args.push("--user".into());
    }
    if *action == ServiceAction::Status {
        args.extend([
            "show".into(),
            "--property=LoadState,ActiveState,SubState,UnitFileState".into(),
        ]);
    } else {
        args.push(action.as_str().into());
    }
    args.extend(["--".into(), unit.into()]);
    Ok(args)
}
pub async fn service(action: &ServiceAction, unit: &str, user: bool) -> Result<Value> {
    let args = service_args(action, unit, user)?;
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new("systemctl")
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Local systemctl timed out; inspect service status")?
    .context("Cannot run local systemctl; check installation and permissions")?;
    ensure!(
        output.status.success(),
        "Local systemctl failed; check the unit and authorize the corresponding systemctl command separately"
    );
    let mut result = json!({"scope":"local","unit":unit});
    if *action == ServiceAction::Status {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some((key, value)) = line.split_once('=') {
                result[key] = json!(value);
            }
        }
    } else {
        result["action"] = json!(action.as_str());
    }
    Ok(result)
}
pub fn proxy_environment(proxy: &str) -> Result<BTreeMap<String, String>> {
    ensure!(
        !proxy
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace()),
        "Invalid proxy URL"
    );
    let url = Url::parse(proxy).map_err(|_| anyhow::anyhow!("Invalid proxy URL"))?;
    ensure!(
        matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
            && url.host().is_some()
            && url.port_or_known_default().is_some_and(|port| port > 0)
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(url.path(), "" | "/")
            && url.query().is_none()
            && url.fragment().is_none(),
        "Proxy must be an HTTP(S) or SOCKS5(H) origin without credentials"
    );
    let mut env = BTreeMap::new();
    for key in [
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
    ] {
        env.insert(key.into(), proxy.into());
    }
    for key in ["no_proxy", "NO_PROXY"] {
        env.insert(key.into(), "localhost,127.0.0.1,::1".into());
    }
    Ok(env)
}
pub fn quote_shell(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
pub fn exec(command: &[String], proxy: &str) -> Result<()> {
    let (program, args) = command
        .split_first()
        .context("exec requires a command after --")?;
    let environment = proxy_environment(proxy)?;
    let _error = std::process::Command::new(program)
        .args(args)
        .envs(environment)
        .env_remove("KAMI_SECRET")
        .exec();
    bail!("Cannot execute requested command; check executable and permissions")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn systemctl_arguments_do_not_allow_injection() {
        for unit in ["--help", "../a.service", "a;id.service", "a.service\n"] {
            assert!(service_args(&ServiceAction::Start, unit, false).is_err());
        }
        let args = service_args(&ServiceAction::Status, "custom.service", true).unwrap();
        assert!(args.contains(&"--no-ask-password".into()));
        assert!(args.contains(&"--user".into()));
        assert_eq!(&args[args.len() - 2..], ["--", "custom.service"]);
    }
    #[test]
    fn proxy_validation_and_shell_quoting() {
        for value in [
            "http://u:p@host:7890",
            "http://host:0",
            "http://host/path",
            "http://host:7890\n",
        ] {
            assert!(proxy_environment(value).is_err());
        }
        assert!(proxy_environment("socks5h://[::1]:7890").is_ok());
        assert_eq!(quote_shell("a'b"), "'a'\\''b'");
    }
}
