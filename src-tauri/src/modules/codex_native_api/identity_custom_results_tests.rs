use super::*;

#[test]
fn custom_results_rebuild_preserves_source_checkpoint_after_restart() {
    let (_temp, path, store) = fixture();
    let original = request(vec![
        message("user", "use the original tool output"),
        json!({"type":"reasoning","summary":[],"encrypted_content":"synthetic-unregistered-cipher"}),
        json!({"type":"custom_tool_call","call_id":"c","id":"call","name":"exec","input":"exact input","status":"completed"}),
        json!({"type":"custom_tool_call_output","call_id":"c","id":"first","output":[{"type":"input_text","text":"first"},{"type":"input_image","image_url":"data:image/png;base64,fixture"}]}),
        json!({"type":"custom_tool_call_output","call_id":"c","id":"second","name":"exec","output":"\nsecond  "}),
        json!({"type":"custom_tool_call_output","call_id":"c","id":"third","name":"exec","output":"third"}),
    ]);
    let saved = original.clone();
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &original)
        .unwrap();
    let plan = store
        .configure_rebuild(&mut context, &HeaderMap::new(), &original, true)
        .unwrap()
        .unwrap();
    assert_eq!(
        context.rebuild_input.as_ref().unwrap(),
        original["input"].as_array().unwrap()
    );
    assert_eq!(
        plan.source_input.as_deref().unwrap(),
        original["input"].as_array().unwrap()
    );
    assert!(paired_tools(plan.body["input"].as_array().unwrap()));
    assert!(!paired_tools(context.rebuild_input.as_ref().unwrap()));
    // Account switching reinspects the wire copy, then restores the source
    // transcript through the same generation hook as production.
    let mut context = store
        .inspect_for_rebuild("caller", &plan.headers, &plan.body)
        .unwrap();
    store
        .configure_rebuild(&mut context, &plan.headers, &plan.body, true)
        .unwrap()
        .unwrap();
    context.start_rebuild_generation(&plan);
    assert_eq!(
        context.rebuild_input.as_ref().unwrap(),
        original["input"].as_array().unwrap()
    );
    let mut headers = plan.headers.clone();
    let mut body = plan.body.clone();
    let binding = store
        .rewrite(&context, "b", &owner("b"), &mut headers, &mut body)
        .unwrap();
    complete(
        &store,
        &binding,
        "custom-b",
        vec![message("assistant", "accepted")],
    );
    let checkpoint_before = checkpoint(&path, "custom-b").unwrap();
    assert!(checkpoint_before
        .2
        .starts_with(original["input"].as_array().unwrap()));
    drop(store);
    let store = IdentityStore::open(path.clone()).unwrap();
    let next = continuation("custom-b", vec![message("user", "continue")]);
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &next)
        .unwrap();
    let plan = store
        .configure_rebuild(&mut context, &HeaderMap::new(), &next, true)
        .unwrap()
        .unwrap();
    assert!(paired_tools(plan.body["input"].as_array().unwrap()));
    assert!(context
        .rebuild_input
        .as_ref()
        .unwrap()
        .starts_with(&checkpoint_before.2));
    assert_eq!(
        checkpoint(&path, "custom-b").unwrap().2,
        checkpoint_before.2
    );
    assert_eq!(original, saved);
}
