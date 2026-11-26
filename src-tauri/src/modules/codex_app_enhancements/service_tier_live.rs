use super::{ContextKey, Session, COMMAND_TIMEOUT};
use serde_json::{json, Value};

const SCRIPT: &str = include_str!("service_tier_live.js");
pub(super) const NOTICE: &str = "__cockpitServiceTierCaptured";
const OBJECT_GROUP: &str = "cockpit-service-tier";

pub(super) enum Attach {
    Pending,
    Active,
    Armed,
}

pub(super) async fn attach(session: &mut Session, context: ContextKey) -> Result<Attach, String> {
    let result = session
        .evaluate(
            format!("{SCRIPT}\nwindow.__cockpitServiceTierLive.prepare()"),
            context,
            COMMAND_TIMEOUT,
        )
        .await?;
    let bindings = match result.get("state").and_then(Value::as_str) {
        Some("pending") => return Ok(Attach::Pending),
        Some("active") => return Ok(Attach::Active),
        Some("ready") => result["bindings"].as_array(),
        Some("requirements-unavailable") => return Err("speed-requirements-unavailable".into()),
        _ => return Err("speed-contract-unsupported".into()),
    };
    let bindings = bindings
        .filter(|items| items.len() == 2)
        .ok_or("speed-contract-unsupported")?;
    let bindings = bindings
        .iter()
        .map(|binding| {
            binding
                .as_str()
                .filter(|name| valid_identifier(name))
                .ok_or("speed-contract-unsupported")
        })
        .collect::<Result<Vec<_>, _>>()?;
    if session.context() != Some(context) {
        return Err("context-changed".into());
    }
    if let Err(error) = arm(session, context, &bindings).await {
        detach(session).await;
        return Err(error);
    }
    Ok(Attach::Armed)
}

fn valid_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
}

async fn arm(session: &mut Session, context: ContextKey, bindings: &[&str]) -> Result<(), String> {
    session
        .command(
            "Runtime.addBinding",
            json!({"name":NOTICE,"executionContextId":context.1}),
            COMMAND_TIMEOUT,
        )
        .await?;
    session
        .command("Debugger.enable", json!({}), COMMAND_TIMEOUT)
        .await?;
    for (index, binding) in bindings.iter().enumerate() {
        let target = session
            .command(
                "Runtime.evaluate",
                json!({
                    "expression":format!("window.__cockpitServiceTierLive.target({index})"),
                    "contextId":context.1,"objectGroup":OBJECT_GROUP,
                }),
                COMMAND_TIMEOUT,
            )
            .await?;
        let object = target
            .pointer("/result/result/objectId")
            .and_then(Value::as_str)
            .ok_or("speed-target-unavailable")?;
        // The condition binds one native import on its next call and always
        // returns false, so the renderer continues through the original code.
        let condition = format!(
            "({binding}=globalThis.__cockpitServiceTierLive?.capture({index},{binding})??{binding},false)"
        );
        session
            .command(
                "Debugger.setBreakpointOnFunctionCall",
                json!({
                    "objectId":object,"condition":condition,
                }),
                COMMAND_TIMEOUT,
            )
            .await?;
    }
    if session.context() != Some(context) {
        return Err("context-changed".into());
    }
    let armed = session
        .evaluate(
            "window.__cockpitServiceTierLive.arm()".into(),
            context,
            COMMAND_TIMEOUT,
        )
        .await?;
    if armed != Value::Bool(true) {
        return Err("speed-target-unavailable".into());
    }
    Ok(())
}

pub(super) async fn detach(session: &mut Session) {
    let _ = session
        .command("Debugger.disable", json!({}), COMMAND_TIMEOUT)
        .await;
    let _ = session
        .command(
            "Runtime.releaseObjectGroup",
            json!({"objectGroup":OBJECT_GROUP}),
            COMMAND_TIMEOUT,
        )
        .await;
    let _ = session
        .command(
            "Runtime.removeBinding",
            json!({"name":NOTICE}),
            COMMAND_TIMEOUT,
        )
        .await;
}
