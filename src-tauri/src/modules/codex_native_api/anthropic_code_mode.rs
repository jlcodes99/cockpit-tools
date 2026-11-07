//! Code-mode APIs documented inside exec are not independent Claude tools.
//! Share the protocol guidance with the bundled Go engine.
use serde_json::Value;

const GUIDE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../resources/codex/code-mode/instructions.txt"));
const PATCH: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../resources/codex/code-mode/apply_patch.txt"));

pub(super) fn instructions(tools: &[Value]) -> Option<String> {
    let tool = tools.iter().find(|tool| {
        let description = tool["description"].as_str().unwrap_or("");
        tool.pointer("/input_schema/properties/input/type")
            .and_then(Value::as_str)
            == Some("string")
            && description.contains("JavaScript")
            && description.contains("global `tools` object")
    })?;
    let mut guide = GUIDE.to_owned();
    if tool["description"]
        .as_str()?
        .contains("apply_patch(input: string)")
    {
        guide.push('\n');
        guide.push_str(PATCH);
    }
    Some(guide.replace("{{EXEC_TOOL}}", tool["name"].as_str()?))
}
