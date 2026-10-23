use super::*;
use serde_json::json;

#[test]
fn chat_ignored_options_do_not_block_responses_or_mutate_original() {
    let body = json!({"model":"fixture","messages":[{"role":"user","name":"participant","content":"raw 250ms cipher"}],"n":2,"stop":["end"],"seed":1,"logit_bias":{"1":1},"prediction":{"content":"raw"},"audio":{"voice":"a"},"presence_penalty":0.5,"frequency_penalty":1,"modalities":["audio"]});
    let original = body.clone();
    let converted = super::super::chat::prepare_request(
        chat(body.clone(), false),
        super::super::chat::ChatFlavor::ChatCompletions,
    )
    .unwrap();
    assert_eq!(
        converted.responses["input"][0]["content"][0]["text"],
        "raw 250ms cipher"
    );
    assert_eq!(body, original);
    assert_eq!(body["n"], 2);
}

#[test]
fn claude_ignored_options_preserve_document_data() {
    let body = json!({"model":"fixture","messages":[{"role":"user","content":[{"type":"document","source":{"type":"text","data":"document cipher"},"citations":{"enabled":true}}]}],"stop_sequences":["end"],"top_k":1,"container":{"id":"c"},"context_management":{"edits":[]},"mcp_servers":[{"url":"unused"}]});
    let converted = super::super::compat::prepare_request(
        compatibility(body, CompatFlavor::Claude),
        CompatFlavor::Claude,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        converted.responses["input"][0]["content"][0]["text"],
        "document cipher"
    );
}

#[test]
fn gemini_ignored_options_preserve_supported_generation_settings() {
    let body = json!({"model":"fixture","contents":[{"role":"user","parts":[{"text":"text"}]}],"safetySettings":[{}],"cachedContent":"unused","generationConfig":{"candidateCount":4,"stopSequences":["end"],"topK":10,"speechConfig":{},"responseModalities":["AUDIO"],"mediaResolution":"HIGH","responseMimeType":"audio/wav","temperature":0.2,"thinkingConfig":{"thinkingLevel":"high"}}});
    let converted = super::super::compat::prepare_request(
        compatibility(body, CompatFlavor::Gemini),
        CompatFlavor::Gemini,
        None,
        None,
    )
    .unwrap();
    assert_eq!(converted.responses["temperature"], 0.2);
    assert_eq!(converted.responses["reasoning"]["effort"], "high");
}

#[test]
fn ollama_local_sampling_hints_are_ignored() {
    let body = json!({"model":"fixture","messages":[{"role":"user","content":"text"}],"options":{"temperature":0.2,"top_p":0.8,"num_predict":-1,"num_ctx":8192,"top_k":40,"repeat_penalty":1.1,"seed":123,"stop":["end"],"mirostat":2}});
    let prepared = compatibility(body, CompatFlavor::Ollama);
    assert_eq!(prepared["options"], json!({"temperature":0.2,"top_p":0.8}));
    assert!(
        super::super::compat::prepare_request(prepared, CompatFlavor::Ollama, None, None).is_ok()
    );
}

#[test]
fn malformed_core_fields_are_not_silently_repaired() {
    let body = json!({"model":"fixture","messages":"invalid","stop":["end"]});
    assert!(super::super::chat::prepare_request(
        chat(body, false),
        super::super::chat::ChatFlavor::ChatCompletions
    )
    .is_err());
}
