use super::*;
use crate::modules::codex_native_api::chat::{response_to_chat, ChatStreamAdapter, StreamOutput};

#[test]
fn long_tool_names_match_go_and_restore_json_stream_and_followup() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/chat_tools_go.json")).unwrap();
    for case in cases.into_iter().filter(|case| case["roundtrip"] == true) {
        let prepared = prepare_request(case["input"].clone(), ChatFlavor::ChatCompletions).unwrap();
        let tools = prepared.responses["tools"].as_array().unwrap();
        let aliases: Vec<_> = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert!(aliases
            .iter()
            .all(|name| name.len() <= 64 && name.is_ascii()));
        assert_eq!(
            aliases.iter().copied().collect::<HashSet<_>>().len(),
            aliases.len()
        );
        let mut output = Vec::new();
        for (index, tool) in tools.iter().enumerate() {
            let custom = tool["type"] == "custom";
            let mut item = json!({"type":if custom{"custom_tool_call"}else{"function_call"},"id":format!("item_{index}"),"call_id":format!("call_{index}"),"name":tool["name"],"status":"completed"});
            item[if custom { "input" } else { "arguments" }] = json!("raw 250ms cipher");
            output.push(item);
        }
        let response = json!({"id":"response_fixture","object":"response","status":"completed","model":"fixture","output":output});
        let chat = response_to_chat(&response, &prepared.context).unwrap();
        let calls = chat["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        let names: Vec<_> = calls
            .iter()
            .map(|call| {
                call[call["type"].as_str().unwrap()]["name"]
                    .as_str()
                    .unwrap()
            })
            .collect();
        assert_eq!(
            json!(names),
            case["expected_names"],
            "{} JSON",
            case["name"]
        );
        let mut stream = ChatStreamAdapter::new(prepared.context.clone());
        let mut streamed_names = Vec::new();
        for (index, item) in response["output"].as_array().unwrap().iter().enumerate() {
            for frame in stream
                .push(
                    &json!({"type":"response.output_item.added","output_index":index,"item":item}),
                )
                .unwrap()
            {
                if let StreamOutput::Event(frame) = frame {
                    for call in frame
                        .pointer("/choices/0/delta/tool_calls")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        let kind = call["type"].as_str().unwrap();
                        streamed_names.push(call[kind]["name"].clone());
                    }
                }
            }
        }
        stream
            .push(&json!({"type":"response.completed","response":response}))
            .unwrap();
        assert_eq!(
            json!(streamed_names),
            case["expected_names"],
            "{} stream",
            case["name"]
        );
        let mut followup = case["input"].clone();
        followup["messages"]
            .as_array_mut()
            .unwrap()
            .push(chat["choices"][0]["message"].clone());
        let next = prepare_request(followup, ChatFlavor::ChatCompletions).unwrap();
        let replay: Vec<_> = next.responses["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| {
                matches!(
                    item["type"].as_str(),
                    Some("function_call" | "custom_tool_call")
                )
            })
            .map(|item| item["name"].as_str().unwrap())
            .collect();
        assert_eq!(replay, aliases, "{} next request", case["name"]);
        assert!(next.responses["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item.get("arguments").or_else(|| item.get("input")))
            .all(|value| value == "raw 250ms cipher"));
    }
}

#[test]
fn allowed_choice_and_namespace_use_the_same_request_aliases() {
    let long = "function-name".repeat(8);
    let prepared = prepare_request(json!({"model":"fixture","messages":[],"tools":[{"type":"namespace","name":"ns","tools":[{"type":"function","name":long,"parameters":{"type":"object"}}]}],"tool_choice":{"type":"allowed_tools","tools":[{"type":"function","name":long}]}}),ChatFlavor::ChatCompletions).unwrap();
    assert_eq!(
        prepared.responses["tools"][0]["tools"][0]["name"],
        prepared.responses["tool_choice"]["tools"][0]["name"]
    );
    assert_eq!(
        prepared.context.tool_name(
            prepared.responses["tools"][0]["tools"][0]["name"]
                .as_str()
                .unwrap()
        ),
        long
    );
}
