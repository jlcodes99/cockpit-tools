use super::*;
use serde_json::json;
#[test]
fn gemini_replayed_thought_signatures_do_not_become_codex_state() {
    let req = prepare_request(json!({"contents":[
        {"role":"model","parts":[{"thought":true,"text":"private reasoning","thoughtSignature":"opaque-gemini"},
        {"text":"answer","thoughtSignature":"ignored-signature"},
        {"functionCall":{"id":"call-existing","name":"f","args":{}},"thoughtSignature":"signed-call"}]},
        {"role":"user","parts":[{"functionResponse":{"id":"call-existing","name":"f","response":{}}}]}
    ]}), CompatFlavor::Gemini, Some("m"), None).unwrap();
    assert!(!req.responses.to_string().contains("private reasoning"));
    assert!(!req.responses.to_string().contains("Signature"));
    assert!(req.responses.to_string().contains("answer"));
    assert_eq!(req.responses["input"][2]["call_id"], "call-existing");
}
#[test]
fn gemini_catalog_advertises_the_supported_stream_action() {
    let list = models(&json!({"data":[{"id":"m"}]}), CompatFlavor::Gemini).unwrap();
    assert!(list["models"][0]["supportedGenerationMethods"]
        .as_array()
        .unwrap()
        .contains(&json!("streamGenerateContent")));
    assert!(list["models"][0].get("inputTokenLimit").is_none());
}
#[test]
fn native_compat_claude_tool_image_and_usage() {
    let request=prepare_request(json!({"model":"m","messages":[{"role":"assistant","content":[{"type":"tool_use","id":"a","name":"f","input":{"x":1}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"ok"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AA"}}]}],"tools":[{"name":"f","input_schema":{"type":"object"}}]}),CompatFlavor::Claude,None,None).unwrap();
    assert_eq!(request.responses["input"][0]["arguments"], "{\"x\":1}");
    assert_eq!(request.responses["input"][1]["call_id"], "a");
    assert_eq!(
        request.responses["input"][2]["content"][0]["image_url"],
        "data:image/png;base64,AA"
    );
    let out=response_from_responses(&json!({"id":"r","status":"completed","output":[{"type":"function_call","name":"f","call_id":"a","arguments":"{\"x\":1}"}],"usage":{"input_tokens":10,"output_tokens":4,"input_tokens_details":{"cached_tokens":6}}}),&request.context).unwrap();
    assert_eq!(out["stop_reason"], "tool_use");
    assert_eq!(out["usage"]["input_tokens"], 4);
    assert_eq!(out["usage"]["cache_read_input_tokens"], 6);
}
#[test]
fn native_compat_gemini_function_pair_and_schema() {
    let req=prepare_request(json!({"contents":[{"role":"model","parts":[{"functionCall":{"name":"f","args":{"a":1}}}]},{"role":"user","parts":[{"functionResponse":{"name":"f","response":{"ok":true}}}]}],"generationConfig":{"responseMimeType":"application/json","responseSchema":{"type":"OBJECT","properties":{"a":{"type":"STRING","nullable":true}}}}}),CompatFlavor::Gemini,Some("models/m"),Some(true)).unwrap();
    assert_eq!(req.responses["model"], "m");
    assert_eq!(
        req.responses["input"][0]["call_id"],
        req.responses["input"][1]["call_id"]
    );
    assert_eq!(
        req.responses["text"]["format"]["schema"]["properties"]["a"]["type"],
        json!(["string", "null"])
    );
}
#[test]
fn native_compat_ollama_tool_ids_are_matched() {
    let req=prepare_request(json!({"model":"m","messages":[{"role":"assistant","content":"","tool_calls":[{"function":{"name":"f","arguments":{"a":2}}}]},{"role":"tool","tool_name":"f","content":"ok"}],"options":{"num_predict":10},"format":"json"}),CompatFlavor::Ollama,None,None).unwrap();
    assert!(req.stream);
    assert_eq!(req.responses["max_output_tokens"], 10);
    let input = req.responses["input"].as_array().unwrap();
    let call = input.iter().find(|i| i["type"] == "function_call").unwrap();
    let result = input
        .iter()
        .find(|i| i["type"] == "function_call_output")
        .unwrap();
    assert_eq!(call["call_id"], result["call_id"]);
}

#[test]
fn native_compat_out_of_order_explicit_tool_results_preserve_pending_calls() {
    let gemini = prepare_request(
        json!({"contents":[
            {"role":"model","parts":[
                {"functionCall":{"id":"call-a","name":"f","args":{}}},
                {"functionCall":{"id":"call-b","name":"f","args":{}}}
            ]},
            {"role":"user","parts":[
                {"functionResponse":{"id":"call-b","name":"f","response":{"value":"b"}}},
                {"functionResponse":{"name":"f","response":{"value":"a"}}}
            ]}
        ]}),
        CompatFlavor::Gemini,
        Some("m"),
        None,
    )
    .unwrap();
    let ollama = prepare_request(
        json!({"model":"m","messages":[
            {"role":"assistant","tool_calls":[
                {"id":"call-a","function":{"name":"f","arguments":{}}},
                {"id":"call-b","function":{"name":"f","arguments":{}}}
            ]},
            {"role":"tool","tool_call_id":"call-b","tool_name":"f","content":"b"},
            {"role":"tool","tool_name":"f","content":"a"}
        ]}),
        CompatFlavor::Ollama,
        None,
        None,
    )
    .unwrap();
    for request in [gemini, ollama] {
        let ids: Vec<_> = request.responses["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .map(|item| item["call_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["call-b", "call-a"]);
    }
}
#[test]
fn native_compat_token_count_never_estimates() {
    assert!(count_tokens_response(&json!({}), CompatFlavor::Claude).is_err());
    assert_eq!(
        count_tokens_response(&json!({"input_tokens":123}), CompatFlavor::Gemini).unwrap(),
        json!({"totalTokens":123})
    );
    let req = count_tokens_request(
        json!({"model":"m","messages":[{"role":"user","content":"hi"}],"max_tokens":30}),
        CompatFlavor::Claude,
        None,
    )
    .unwrap();
    assert!(req.get("max_output_tokens").is_none());
}
#[test]
fn native_compat_stream_finishes_and_preserves_tool_json() {
    for flavor in [
        CompatFlavor::Claude,
        CompatFlavor::Gemini,
        CompatFlavor::Ollama,
    ] {
        let mut stream = CompatStreamAdapter::new(CompatContext {
            flavor,
            model: "m".into(),
            tool_names: Default::default(),
        });
        let mut out = Vec::new();
        for e in [
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"a","name":"f","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"x\":"}),
            json!({"type":"response.function_call_arguments.delta","output_index":0,"delta":"1}"}),
            json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[{"type":"function_call","call_id":"a","name":"f","arguments":"{\"x\":1}"}],"usage":{"input_tokens":3,"output_tokens":5}}}),
        ] {
            out.extend(stream.push(&e).unwrap());
        }
        assert!(stream.finish().is_ok());
        match flavor {
            CompatFlavor::Claude => {
                assert_eq!(out.last().unwrap().event.as_deref(), Some("message_stop"));
                assert!(out.iter().any(|f| f.data["delta"]["partial_json"] == "1}"));
            }
            CompatFlavor::Gemini => assert_eq!(
                out.last().unwrap().data["candidates"][0]["content"]["parts"][0]["functionCall"]
                    ["args"]["x"],
                1
            ),
            CompatFlavor::Ollama => {
                assert_eq!(out.last().unwrap().data["done"], true);
                assert_eq!(out.last().unwrap().data["eval_count"], 5);
            }
        }
    }
}
#[test]
fn native_compat_stream_error_is_not_success() {
    for flavor in [
        CompatFlavor::Claude,
        CompatFlavor::Gemini,
        CompatFlavor::Ollama,
    ] {
        let mut stream = CompatStreamAdapter::new(CompatContext {
            flavor,
            model: "m".into(),
            tool_names: Default::default(),
        });
        assert!(stream
            .push(&json!({"type":"error","error":{"message":"failed"}}))
            .is_err());
        assert!(stream.finish().is_err());
    }
}
#[test]
fn native_compat_models_do_not_invent_weights() {
    let list = json!({"data":[{"id":"m","context_length":100}]});
    let models = models(&list, CompatFlavor::Ollama).unwrap();
    assert!(models["models"][0].get("size").is_none());
    assert_eq!(
        ollama_show(&list["data"][0]).unwrap()["model_info"]["context_length"],
        100
    );
}

#[test]
fn native_compat_reasoning_budgets_map_to_supported_effort_levels() {
    for (budget, effort) in [(1024, "low"), (4096, "medium"), (16000, "high")] {
        let claude = prepare_request(
            json!({"model":"m","messages":[],"thinking":{"type":"enabled","budget_tokens":budget}}),
            CompatFlavor::Claude,
            None,
            None,
        )
        .unwrap();
        assert_eq!(claude.responses["reasoning"]["effort"], effort);
        let gemini = prepare_request(
            json!({"contents":[],"generationConfig":{"thinkingConfig":{"thinkingBudget":budget}}}),
            CompatFlavor::Gemini,
            Some("m"),
            None,
        )
        .unwrap();
        assert_eq!(gemini.responses["reasoning"]["effort"], effort);
    }
}
