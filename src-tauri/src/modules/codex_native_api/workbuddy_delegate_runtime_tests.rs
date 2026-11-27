use super::*;

fn node_path() -> Option<PathBuf> {
    std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(if cfg!(windows) { "node.exe" } else { "node" }))
                .find(|file| file.is_file())
        })
        .and_then(|file| file.canonicalize().ok())
}

fn fake_cli(root: &std::path::Path) -> PathBuf {
    let cli = root.join("fake-full-cli.cjs");
    std::fs::write(&cli, r#"const readline=require('node:readline');
const send=f=>process.stdout.write(JSON.stringify({jsonrpc:'2.0',...f})+'\n');
const ok=(id,result)=>send({id,result});
let alias, prompt, calls=0;
readline.createInterface({input:process.stdin}).on('line',line=>{
const f=JSON.parse(line);
if(f.method==='initialize')return ok(f.id,{agentCapabilities:{delegateToolsSupport:true}});
if(f.method==='session/new'){
if(!require('node:path').isAbsolute(f.params.cwd))process.exit(88);
return ok(f.id,{sessionId:'session-fixture',models:{availableModels:[{modelId:'fixture-model',_meta:{effortOptions:['low']}}],currentModelId:'fixture-model'},configOptions:[{id:'thought_level',currentValue:'low',options:[{value:'low'}]}]});}
if(f.method==='_codebuddy.ai/delegateToolsChanged'){alias=f.params.tools[0].id;return ok(f.id,{});}
if(f.method==='session/prompt'){
prompt=f.id;calls++;
if(calls===1){return send({id:6007,method:'session/request_permission',params:{sessionId:'session-fixture',toolCall:{toolCallId:'model-permission-fixture-original',_meta:{'codebuddy.ai/toolName':'DelegateTool'},rawInput:{toolId:alias,input:{path:'fixture.txt'}}},options:[{kind:'allow_once',optionId:'fixture-allow-once'}]}});}
send({method:'session/update',params:{sessionId:'session-fixture',update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text:'SECOND_FROM_SAME_CLI'}}}});return ok(prompt,{stopReason:'end_turn'});
}
if(f.id===6007){
if(f.result?.outcome?.outcome!=='selected'||f.result?.outcome?.optionId!=='fixture-allow-once')process.exit(90);
return send({id:7123,method:'_codebuddy.ai/delegateTool',params:{sessionId:'',toolCallId:'delegate-codex_fixture-1795664485847',toolId:alias,input:{path:'fixture.txt'},timeout:30000}});
}
if(f.id===7123){
if(f.result.status!=='success'||JSON.stringify(f).includes('delegate-codex_fixture-1795664485847')||JSON.stringify(f).includes('model-permission-fixture-original'))process.exit(89);
send({method:'session/update',params:{sessionId:'session-fixture',update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text:String(f.result.output)}}}});return ok(prompt,{stopReason:'end_turn',usage:{inputTokens:11,outputTokens:7}});
}
if(f.method==='session/cancel')process.exit(0);
});
"#).unwrap();
    cli
}

fn tools() -> Value {
    json!([{"type":"function","name":"read_fixture","description":"Read a synthetic fixture", "parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}}])
}

fn owner() -> Owner {
    Owner {
        caller_id: "caller-a".into(),
        thread_id: "thread-a".into(),
    }
}
fn config() -> LaunchConfig {
    LaunchConfig {
        account_id: "synthetic-account".into(),
        source_generation: "synthetic-generation".into(),
        model: "fixture-model".into(),
        effort: "low".into(),
        access_token: "DUMMY_NOT_A_REAL_TOKEN".into(),
        domestic: true,
    }
}
fn body() -> Value {
    json!({"model":"workbuddy/fixture-model","tools":tools(),"input":[{"role":"user","content":"Read the fixture using the declared tool"}]})
}

fn fixture() -> (Arc<Registry>, Arc<Entry>, Arc<Actor>, mpsc::Receiver<Value>) {
    let registry = Arc::new(Registry::default());
    let (cancel, _) = watch::channel(false);
    let entry = Arc::new(Entry {
        id: "actor-fixture".into(),
        owner: owner(),
        source: Source::from_config(&config()),
        catalog: ToolCatalog::from_tools(Some(&tools())).unwrap(),
        actor: OnceCell::new(),
        gate: Arc::new(AsyncMutex::new(())),
        cancel: cancel.clone(),
        created: Instant::now(),
    });
    let (outgoing, receiver) = mpsc::channel(32);
    let mut core = Core::default();
    core.session = "session-fixture".into();
    let actor = Arc::new(Actor {
        core: Mutex::new(core),
        notify: Notify::new(),
        outgoing,
        cancel,
        entry: Arc::downgrade(&entry),
        registry: Arc::downgrade(&registry),
        cleanup: Notify::new(),
        workspace: PathBuf::from("/synthetic-private-workspace"),
        expected_model: "fixture-model".into(),
        expected_effort: "low".into(),
        cleaned: std::sync::atomic::AtomicBool::new(false),
    });
    assert!(entry.actor.set(Ok(actor.clone())).is_ok());
    {
        let mut index = registry.index.lock().unwrap();
        index
            .threads
            .insert(("caller-a".into(), "thread-a".into()), entry.id.clone());
        index.entries.insert(entry.id.clone(), entry.clone());
    }
    (registry, entry, actor, receiver)
}

fn stop() -> (watch::Sender<bool>, StopSignal) {
    let (send, receive) = watch::channel(false);
    (send, StopSignal(receive))
}
fn callback(actor: &Actor, entry: &Entry, rpc: Value) {
    let alias = entry.catalog.delegate_definitions()[0]["id"].clone();
    actor.handle_frame(json!({"jsonrpc":"2.0","id":rpc,"method":"_codebuddy.ai/delegateTool","params":{"sessionId":"","toolCallId":"delegate-codex_fixture-1795664485847","toolId":alias,"input":{"path":"fixture.txt"},"timeout":30000}}));
}
fn finish(actor: &Actor, prompt_id: Value, text: &str) {
    actor.handle_frame(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"session-fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}}}));
    actor.handle_frame(json!({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":"end_turn"}}));
}

#[tokio::test]
async fn handoff_keeps_prompt_pending_and_feeds_actual_result_once() {
    let (registry, entry, actor, mut receive) = fixture();
    let (_stop, signal) = stop();
    let first = body();
    let task = {
        let registry = registry.clone();
        let first = first.clone();
        let signal = signal.clone();
        tokio::spawn(async move { registry.respond(owner(), config(), &first, signal).await })
    };
    let prompt = receive.recv().await.unwrap();
    assert_eq!(prompt["method"], "session/prompt");
    callback(&actor, &entry, json!(127));
    let handoff = task.await.unwrap().unwrap();
    let call = handoff["output"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call")
        .unwrap()
        .clone();
    assert!(call["call_id"].as_str().unwrap().starts_with("call_wb_"));
    assert!(handoff["id"].as_str().unwrap().starts_with("resp_wb_"));
    assert_eq!(handoff["model"], "workbuddy/fixture-model");
    assert!(!actor.core.lock().unwrap().prompt_done);
    assert!(
        receive.try_recv().is_err(),
        "handoff must not manufacture a tool result"
    );
    let nonce = "UNPREDICTABLE_TEST_RESULT_57d962";
    let mut input = first["input"].as_array().unwrap().clone();
    input.push(call.clone());
    input.push(json!({"type":"function_call_output","call_id":call["call_id"],"output":nonce}));
    let second = json!({"model":"workbuddy/fixture-model","input":input});
    let task = {
        let registry = registry.clone();
        let second = second.clone();
        let signal = signal.clone();
        tokio::spawn(async move { registry.respond(owner(), config(), &second, signal).await })
    };
    let result = receive.recv().await.unwrap();
    assert_eq!(result["id"], 127);
    assert_eq!(result["result"]["output"], nonce);
    finish(&actor, prompt["id"].clone(), nonce);
    let completed = task.await.unwrap().unwrap();
    assert_eq!(completed["output"][0]["content"][0]["text"], nonce);
    assert!(completed.get("usage").is_none(), "never invent usage");
    let retry = registry
        .respond(owner(), config(), &second, signal.clone())
        .await
        .unwrap();
    assert_eq!(retry, completed);
    assert!(receive.try_recv().is_err());
    let mut conflict_body = second.clone();
    conflict_body["input"][2]["output"] = json!("conflicting result");
    assert!(registry
        .respond(owner(), config(), &conflict_body, signal.clone())
        .await
        .is_err());
    assert!(receive.try_recv().is_err());
    // Full-history second user turn finds the Ready actor through historical IDs.
    let mut history = second["input"].as_array().unwrap().clone();
    history.extend(completed["output"].as_array().unwrap().clone());
    history.push(json!({"role":"user","content":"A second user turn"}));
    let third = json!({"model":"workbuddy/fixture-model","input":history});
    let continuation = registry
        .continuation("caller-a", &third, None)
        .unwrap()
        .unwrap();
    assert_eq!(continuation.thread_id, "thread-a");
    let task = {
        let registry = registry.clone();
        let third = third.clone();
        tokio::spawn(async move { registry.respond(owner(), config(), &third, signal).await })
    };
    let second_prompt = receive.recv().await.unwrap();
    assert_eq!(second_prompt["method"], "session/prompt");
    finish(&actor, second_prompt["id"].clone(), "SECOND_USER_TURN");
    assert_eq!(
        task.await.unwrap().unwrap()["output"][0]["content"][0]["text"],
        "SECOND_USER_TURN"
    );
}

#[tokio::test]
async fn previous_response_delta_and_exact_rpc_id_are_supported() {
    let (registry, entry, actor, mut receive) = fixture();
    let (_send, signal) = stop();
    let first = body();
    let task = {
        let registry = registry.clone();
        let signal = signal.clone();
        tokio::spawn(async move { registry.respond(owner(), config(), &first, signal).await })
    };
    let prompt = receive.recv().await.unwrap();
    callback(&actor, &entry, json!("opaque-rpc-id"));
    let first = task.await.unwrap().unwrap();
    let call = &first["output"][0];
    let next = json!({"model":"workbuddy/fixture-model","previous_response_id":first["id"],"input":[{"type":"function_call_output","call_id":call["call_id"],"output":{"content":["preserve","structure"]}}]});
    assert!(registry.continuation("caller-b", &next, None).is_err());
    assert!(registry
        .continuation("caller-a", &next, Some("wrong-thread"))
        .is_err());
    let task = {
        let registry = registry.clone();
        tokio::spawn(async move { registry.respond(owner(), config(), &next, signal).await })
    };
    let result = receive.recv().await.unwrap();
    assert_eq!(result["id"], "opaque-rpc-id");
    assert_eq!(
        result["result"]["output"],
        json!({"content":["preserve","structure"]})
    );
    finish(&actor, prompt["id"].clone(), "done");
    assert!(task.await.unwrap().is_ok());
}

#[tokio::test]
async fn request_drop_signals_actor_cancellation() {
    let (registry, entry, _actor, mut receive) = fixture();
    let (_send, signal) = stop();
    let task =
        tokio::spawn(async move { registry.respond(owner(), config(), &body(), signal).await });
    assert_eq!(receive.recv().await.unwrap()["method"], "session/prompt");
    task.abort();
    let _ = task.await;
    assert!(
        *entry.cancel.borrow(),
        "HTTP future cancellation must cancel upstream ownership"
    );
}

#[tokio::test]
async fn source_changes_concurrent_requests_and_unknown_history_fail_closed() {
    let (registry, entry, _actor, _receive) = fixture();
    let (_send, signal) = stop();
    let mut changed = config();
    changed.source_generation = "rebound".into();
    assert!(registry
        .respond(owner(), changed, &body(), signal.clone())
        .await
        .is_err());
    let _gate = entry.gate.clone().lock_owned().await;
    assert_eq!(
        registry
            .respond(owner(), config(), &body(), signal)
            .await
            .unwrap_err()
            .code,
        "workbuddy_request_in_progress"
    );
    assert!(registry
        .continuation(
            "caller-a",
            &json!({"input":[],"previous_response_id":"resp_wb_missing"}),
            None
        )
        .is_err());
    assert!(Registry::default().continuation("caller-a",&json!({"input":[{"type":"function_call_output","call_id":"call_wb_lost","output":"do not replay"}]}),None).is_err());
}

#[test]
fn forbidden_fs_terminal_and_unknown_permissions_are_denied() {
    for method in ["fs/read_text_file", "terminal/create", "_unknown/request"] {
        let (_registry, _entry, actor, mut receive) = fixture();
        actor.handle_frame(json!({"jsonrpc":"2.0","id":1,"method":method,"params":{}}));
        assert_eq!(receive.try_recv().unwrap()["error"]["code"], -32601);
        assert!(*actor.cancel.borrow());
    }
    let (_registry, entry, actor, mut receive) = fixture();
    let alias = entry.catalog.delegate_definitions()[0]["id"].clone();
    actor.handle_frame(json!({"jsonrpc":"2.0","id":2,"method":"session/request_permission","params":{"sessionId":"session-fixture","toolCall":{"title":"DelegateTool","_meta":{"codebuddy.ai/toolName":"Bash"},"rawInput":{"toolId":alias,"input":{"path":"fixture.txt"}}},"options":[{"kind":"allow_once","optionId":"yes"}]}}));
    assert_eq!(
        receive.try_recv().unwrap()["result"]["outcome"]["outcome"],
        "cancelled"
    );
    assert!(*actor.cancel.borrow());
}

#[test]
fn matching_permission_meta_and_declared_schema_only_are_approved() {
    let (_registry, entry, actor, mut receive) = fixture();
    let alias = entry.catalog.delegate_definitions()[0]["id"].clone();
    actor.handle_frame(json!({"jsonrpc":"2.0","id":"permission-rpc","method":"session/request_permission","params":{"sessionId":"session-fixture","toolCall":{"_meta":{"codebuddy.ai/toolName":"DelegateTool"},"rawInput":{"toolId":alias,"input":{"path":"fixture.txt"}}},"options":[{"kind":"allow_always","optionId":"never"},{"kind":"allow_once","optionId":"single"}]}}));
    assert_eq!(
        receive.try_recv().unwrap()["result"]["outcome"],
        json!({"outcome":"selected","optionId":"single"})
    );
}

#[test]
fn catalog_aliases_are_not_pinned_models_even_when_available_and_current() {
    for alias in [
        "auto", "AUTO", "Auto", "default", "DEFAULT", "enabled", "Enabled",
    ] {
        let catalog = json!({"models":{"availableModels":[{"modelId":alias,"_meta":{"effortOptions":["low"]}}],"currentModelId":alias},"configOptions":[{"id":"thought_level","currentValue":"low","options":[{"value":"low"}]}]});
        for effort in ["low", ""] {
            let rejected = validate_catalog(&catalog, alias, effort).unwrap_err();
            assert_eq!(rejected.status, 400);
            assert_eq!(rejected.code, "workbuddy_model_unavailable");
        }
    }
}

#[test]
fn model_and_effort_require_actual_active_catalog_entries() {
    let catalog = json!({"models":{"availableModels":[{"modelId":"fixture-model","_meta":{"effortOptions":["low"]}}],"currentModelId":"fixture-model"},"configOptions":[{"id":"effort","currentValue":"low","options":[{"value":"low"}]}]});
    assert!(validate_catalog(&catalog, "fixture-model", "low").is_ok());
    assert!(validate_catalog(&catalog, "fixture-model", "").is_ok());
    assert!(validate_catalog(&catalog, "default", "low").is_err());
    assert!(validate_catalog(&catalog, "fixture-model", "high").is_err());
    assert!(validate_catalog(&json!({}), "fixture-model", "low").is_err());
    assert!(
        actual_usage(&json!({"usage":{"inputTokens":7,"outputTokens":3}}))
            .unwrap()
            .get("total_tokens")
            .is_none()
    );
    assert!(actual_usage(&json!({})).is_none());
}

#[test]
fn semantic_cursor_ignores_only_transport_metadata() {
    let a = semantic(
        &json!({"id":"different","type":"function_call","status":"completed","call_id":"call_wb_x","name":"tool","arguments":"{\"x\": 1}"}),
    );
    let b = semantic(
        &json!({"type":"function_call","call_id":"call_wb_x","name":"tool","arguments":"{\"x\":1}"}),
    );
    assert_eq!(a, b);
    assert_ne!(
        a,
        semantic(
            &json!({"type":"function_call","call_id":"call_wb_other","name":"tool","arguments":"{\"x\":1}"})
        )
    );
    assert!(history_delta(
        &[a.clone()],
        &[b.clone(), json!({"role":"user","content":"next"})],
        false
    )
    .is_ok());
    assert!(history_delta(
        &[a],
        &[json!({"role":"user","content":"missing history"})],
        false
    )
    .is_err());
}

#[test]
fn launcher_never_uses_auth_files_and_has_strict_process_flags() {
    assert!(!LAUNCHER.contains("auth-secret.json"));
    for flag in [
        "--tools",
        "--strict-mcp-config",
        "--setting-sources",
        "--no-session-persistence",
        "--settings",
        "dontAsk",
        "DelegateTool",
    ] {
        assert!(LAUNCHER.contains(flag));
    }
    assert!(LAUNCHER.contains("stream.unshift(remainder)"));
    assert!(LAUNCHER.contains("CODEBUDDY_DISABLE_TRACE_COLLECTOR"));
}

#[tokio::test]
async fn anonymous_bootstrap_preserves_first_acp_frame_and_cleans_private_profile() {
    let Some(node) = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(if cfg!(windows) { "node.exe" } else { "node" }))
            .find(|file| file.is_file())
    }) else {
        return;
    };
    let private = tempfile::Builder::new()
        .prefix("cockpit-workbuddy-delegate-")
        .tempdir()
        .unwrap();
    let root = private.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let cli = root.join("fake-cli.cjs");
    std::fs::write(&cli,r#"const readline=require('node:readline');
const settingsIndex=process.argv.indexOf('--settings');
const settingsPath=settingsIndex>=0?process.argv[settingsIndex+1]:'';
const settings=settingsPath?JSON.parse(require('node:fs').readFileSync(settingsPath,'utf8')):{};
const sourcesIndex=process.argv.indexOf('--setting-sources');
const correct=process.env.CODEBUDDY_AUTH_TOKEN==='DUMMY_NOT_A_REAL_TOKEN' && !process.env.PROBE_UNSAFE && process.argv.includes('--strict-mcp-config') && process.argv.includes('--no-session-persistence') && process.argv.includes('dontAsk')
 && require('node:path').isAbsolute(settingsPath) && sourcesIndex>=0 && process.argv[sourcesIndex+1]===''
 && settings.autoCompactEnabled===false && settings.promptSuggestionEnabled===false && settings.fileCheckpointingEnabled===false && settings.autoUpdates===false
 && settings.memory?.enabled===false && settings.memory?.autoMemoryEnabled===false && settings.memory?.parentDiscovery===false
 && settings.daemon?.preventSleep===false && process.env.CODEBUDDY_CODE_DISABLE_FILE_CHECKPOINTING==='1'
 && !process.env.CODEBUDDY_IS_SANDBOX && !process.argv.includes('--dangerously-skip-permissions');
readline.createInterface({input:process.stdin}).on('line',line=>{const f=JSON.parse(line);process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:f.id,result:{bootstrapAndFramePreserved:correct,method:f.method}})+'\n');});
"#).unwrap();
    let launcher = root.join("launcher.cjs");
    std::fs::write(&launcher, LAUNCHER).unwrap();
    let mut command = Command::new(node);
    command
        .arg(&launcher)
        .env_clear()
        .env("HOME", root.join("home"))
        .current_dir(&workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let bootstrap = json!({"accessToken":"DUMMY_NOT_A_REAL_TOKEN","cliPath":cli,"model":"fixture-model","effort":"low","domestic":true});
    let frame = json!({"jsonrpc":"2.0","id":"first-frame","method":"initialize","params":{}});
    stdin
        .write_all(format!("{bootstrap}\n{frame}\n").as_bytes())
        .await
        .unwrap();
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(5), stdout.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let result: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(result["id"], "first-frame");
    assert_eq!(result["result"]["bootstrapAndFramePreserved"], true);
    child.start_kill().unwrap();
    child.wait().await.unwrap();
    let settings = std::fs::read_to_string(root.join("profile/settings.json")).unwrap();
    assert!(!settings.contains("DUMMY_NOT_A_REAL_TOKEN"));
    cleanup_profile(private);
    assert!(!root.exists());
}

#[tokio::test]
async fn delivery_guard_and_accounting_are_idempotent_across_retries() {
    let (registry, entry, actor, mut receive) = fixture();
    let (_send, signal) = stop();
    let task = {
        let registry = registry.clone();
        tokio::spawn(async move { registry.respond(owner(), config(), &body(), signal).await })
    };
    receive.recv().await.unwrap();
    callback(&actor, &entry, json!("rpc-delivery"));
    let document = task.await.unwrap().unwrap();
    let id = document["id"].as_str().unwrap();
    assert!(registry.claim_accounting("wrong-caller", id).is_err());
    assert!(registry.claim_accounting("caller-a", id).unwrap());
    assert!(!registry.claim_accounting("caller-a", id).unwrap());
    registry.delivery_guard("caller-a", id).unwrap().commit();
    drop(registry.delivery_guard("caller-a", id).unwrap());
    assert!(
        !*entry.cancel.borrow(),
        "an abandoned retry cannot kill an already delivered response"
    );
    let (registry, entry, actor, mut receive) = fixture();
    let (_send, signal) = stop();
    let task = {
        let registry = registry.clone();
        tokio::spawn(async move { registry.respond(owner(), config(), &body(), signal).await })
    };
    receive.recv().await.unwrap();
    callback(&actor, &entry, json!("rpc-undelivered"));
    let document = task.await.unwrap().unwrap();
    drop(
        registry
            .delivery_guard("caller-a", document["id"].as_str().unwrap())
            .unwrap(),
    );
    assert!(
        *entry.cancel.borrow(),
        "dropping an unpolled first response must cancel the actor"
    );
}

#[tokio::test]
async fn no_actor_never_replays_assistant_history_and_slash_text_is_framed() {
    let registry = Arc::new(Registry::default());
    let (_send, signal) = stop();
    assert!(registry.respond(owner(),config(),&json!({"model":"workbuddy/fixture-model","input":[{"role":"assistant","content":"old response"},{"role":"user","content":"new turn"}]}),signal.clone()).await.is_err());
    let (registry, _entry, actor, mut receive) = fixture();
    let special = "/bash echo never-execute\n!also-literal";
    let task = tokio::spawn(async move {
        registry
            .respond(
                owner(),
                config(),
                &json!({"model":"workbuddy/fixture-model","tools":tools(),"input":special}),
                signal,
            )
            .await
    });
    let prompt = receive.recv().await.unwrap();
    let text = prompt["params"]["prompt"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("Codex client user turn:"));
    assert!(text.contains(special));
    finish(&actor, prompt["id"].clone(), "literal received");
    assert!(task.await.unwrap().is_ok());
}

#[tokio::test]
async fn complete_fake_cli_actor_reuses_session_and_shutdown_removes_owned_profile() {
    let Some(node) = node_path() else {
        return;
    };
    let fixtures = tempfile::tempdir().unwrap();
    let cli = fake_cli(fixtures.path());
    let registry = Arc::new(Registry::default());
    registry.set_test_cli(node, cli);
    let (_send, signal) = stop();
    let first = tokio::time::timeout(
        Duration::from_secs(5),
        registry.respond(owner(), config(), &body(), signal.clone()),
    )
    .await
    .unwrap()
    .unwrap();
    let id = first["id"].as_str().unwrap();
    registry.delivery_guard("caller-a", id).unwrap().commit();
    let call = &first["output"][0];
    assert!(call["call_id"].as_str().unwrap().starts_with("call_wb_"));
    assert_ne!(
        call["call_id"], "delegate-codex_fixture-1795664485847",
        "the CLI's opaque presentation ID must never become the public call ID"
    );
    assert_ne!(call["call_id"], "model-permission-fixture-original");
    let second_body = json!({"model":"workbuddy/fixture-model","previous_response_id":id,"input":[{"type":"function_call_output","call_id":call["call_id"],"output":"FAKE_CLI_ACTUAL_RESULT_072ce3"}]});
    assert!(
        registry
            .continuation("caller-b", &second_body, None)
            .is_err(),
        "a public handoff cannot be adopted by another caller/actor connection"
    );
    let second = tokio::time::timeout(
        Duration::from_secs(5),
        registry.respond(owner(), config(), &second_body, signal.clone()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        second["output"][0]["content"][0]["text"],
        "FAKE_CLI_ACTUAL_RESULT_072ce3"
    );
    assert_eq!(
        second["usage"],
        json!({"input_tokens":11,"output_tokens":7})
    );
    registry
        .delivery_guard("caller-a", second["id"].as_str().unwrap())
        .unwrap()
        .commit();
    let third = tokio::time::timeout(Duration::from_secs(5), registry.respond(owner(),config(),&json!({"model":"workbuddy/fixture-model","previous_response_id":second["id"],"input":"second turn"}),signal)).await.unwrap().unwrap();
    assert_eq!(
        third["output"][0]["content"][0]["text"],
        "SECOND_FROM_SAME_CLI"
    );
    let actor = registry
        .index
        .lock()
        .unwrap()
        .entries
        .values()
        .next()
        .unwrap()
        .actor
        .get()
        .unwrap()
        .as_ref()
        .unwrap()
        .clone();
    let root = actor.workspace.parent().unwrap().to_path_buf();
    assert!(root.exists());
    registry.shutdown().await;
    assert!(
        !root.exists(),
        "shutdown must reap the CLI and remove only its owned profile"
    );
    assert!(registry
        .continuation(
            "caller-a",
            &json!({"input":[],"previous_response_id":third["id"]}),
            None
        )
        .is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn startup_drop_during_backpressured_bootstrap_kills_owned_group() {
    use std::os::unix::process::CommandExt;
    let Some(node) = node_path() else {
        return;
    };
    let fixtures = tempfile::tempdir().unwrap();
    let script = fixtures.path().join("startup-nonreading-parent.cjs");
    let ready = fixtures.path().join("descendant-ready.json");
    std::fs::write(&script, r#"const fs=require('node:fs');
const c=require('node:child_process').spawn(process.execPath,['-e',`process.on('SIGTERM',()=>{});process.stdout.write('READY');setInterval(()=>{},1000);`],{stdio:['ignore','pipe','ignore']});
c.stdout.once('data',()=>fs.writeFileSync(process.argv[2],JSON.stringify({leader:process.pid,descendant:c.pid})));
process.on('SIGTERM',()=>process.exit(0));setInterval(()=>{},1000);
// Intentionally never attach stdin: the synthetic multi-MB bootstrap must block.
"#).unwrap();
    let mut launch = prepare_process_with_cli((node.clone(), script.clone())).unwrap();
    let root = launch.workspace.parent().unwrap().to_path_buf();
    let mut command = Command::new(node);
    command
        .arg(&script)
        .arg(&ready)
        .env_clear()
        .current_dir(&launch.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command.as_std_mut().process_group(0);
    launch.command = command;
    let mut config = config();
    config.access_token = "SYNTHETIC_BACKPRESSURE_ONLY_".repeat(131072);
    let (cancel, _) = watch::channel(false);
    let (_send, signal) = stop();
    let task = tokio::spawn(Actor::launch_prepared(
        config,
        Weak::new(),
        Weak::new(),
        cancel.clone(),
        signal,
        launch,
    ));
    let ids: Value = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(raw) = std::fs::read_to_string(&ready) {
                if let Ok(ids) = serde_json::from_str(&raw) {
                    break ids;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let descendant = ids["descendant"].as_i64().unwrap() as libc::pid_t;
    let leader = ids["leader"].as_i64().unwrap() as libc::pid_t;
    assert!(
        !task.is_finished(),
        "nonreading pipe must hold the initial bootstrap write"
    );
    assert_eq!(unsafe { libc::kill(descendant, 0) }, 0);
    task.abort();
    let _ = task.await;
    assert!(
        *cancel.borrow(),
        "dropping startup must trigger registered cancellation ownership"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while root.exists()
            || unsafe { libc::kill(leader, 0) } == 0
            || unsafe { libc::kill(descendant, 0) } == 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("startup cancellation must reap leader, kill descendant and remove private profile");
}

#[test]
fn metadata_processes_have_two_global_nonwaiting_slots() {
    let first = metadata_slot().unwrap();
    let second = metadata_slot().unwrap();
    let blocked = metadata_slot().err().unwrap();
    assert_eq!(blocked.status, 503);
    assert_eq!(blocked.code, "workbuddy_catalog_busy");
    drop(first);
    assert!(metadata_slot().is_ok());
    drop(second);
}

#[cfg(unix)]
#[tokio::test]
async fn cleanup_kills_term_ignoring_descendant_after_leader_exits_quickly() {
    let Some(node) = node_path() else {
        return;
    };
    let fixtures = tempfile::tempdir().unwrap();
    let cli = fixtures.path().join("fake-forking-cli.cjs");
    std::fs::write(&cli, r#"const readline=require('node:readline');
const send=(id,result)=>process.stdout.write(JSON.stringify({jsonrpc:'2.0',id,result})+'\n');
readline.createInterface({input:process.stdin}).on('line',line=>{
const f=JSON.parse(line);
if(f.method==='initialize')return send(f.id,{agentCapabilities:{delegateToolsSupport:true}});
if(f.method==='session/new'){
const heartbeat=require('node:path').join(__dirname,'descendant-heartbeat');
const c=require('node:child_process').spawn(process.execPath,['-e',`const fs=require('node:fs');process.on('SIGTERM',()=>{});fs.writeFileSync(process.argv[1],'READY');process.stdout.write('READY');setInterval(()=>fs.appendFileSync(process.argv[1],'x'),20);`,heartbeat],{stdio:['ignore','pipe','ignore']});
c.stdout.once('data',()=>send(f.id,{sessionId:'forking',models:{availableModels:[{modelId:'fixture-model',_meta:{effortOptions:['low']}}],currentModelId:'fixture-model',syntheticDescendantPid:c.pid},configOptions:[{id:'thought_level',currentValue:'low',options:[{value:'low'}]}]}));
}
if(f.method==='session/cancel')process.exit(0);
});
process.on('SIGTERM',()=>process.exit(0));
"#).unwrap();
    let launch = prepare_process_with_cli((node, cli)).unwrap();
    let root = launch.workspace.parent().unwrap().to_path_buf();
    let (cancel, _) = watch::channel(false);
    let (_send, signal) = stop();
    let actor = Actor::launch_prepared(
        config(),
        Weak::new(),
        Weak::new(),
        cancel.clone(),
        signal,
        launch,
    )
    .await
    .unwrap();
    let descendant = actor.core.lock().unwrap().rpc_results["catalog"]["models"]
        ["syntheticDescendantPid"]
        .as_i64()
        .unwrap() as libc::pid_t;
    assert_eq!(unsafe { libc::kill(descendant, 0) }, 0);
    cancel.send_replace(true);
    actor.wait_cleanup().await;
    assert!(!root.exists());
    // macOS may retain an orphan zombie briefly, for which kill(pid, 0) still
    // succeeds. A zombie is not a surviving executable descendant: validate
    // that its pre-registered 20ms heartbeat stops permanently after cleanup.
    let heartbeat = fixtures.path().join("descendant-heartbeat");
    let stopped = std::fs::read(&heartbeat).unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        std::fs::read(&heartbeat).unwrap(),
        stopped,
        "TERM-ignoring descendant is still executing after owned-group cleanup"
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while unsafe { libc::kill(descendant, 0) } == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the killed orphan must be reaped without a surviving descendant");
}

#[tokio::test]
async fn metadata_only_fake_cli_does_not_submit_prompt() {
    let Some(node) = node_path() else {
        return;
    };
    let fixtures = tempfile::tempdir().unwrap();
    let cli = fake_cli(fixtures.path());
    let launch = prepare_process_with_cli((node, cli)).unwrap();
    let root = launch.workspace.parent().unwrap().to_path_buf();
    let (cancel, _) = watch::channel(false);
    let (_send, signal) = stop();
    let actor = Actor::launch_prepared(
        config(),
        Weak::new(),
        Weak::new(),
        cancel.clone(),
        signal,
        launch,
    )
    .await
    .unwrap();
    let core = actor.core.lock().unwrap();
    assert!(core.ready);
    assert!(core.prompt_id.is_none());
    assert_eq!(
        core.rpc_results["catalog"]["models"]["currentModelId"],
        "fixture-model"
    );
    drop(core);
    cancel.send_replace(true);
    actor.wait_cleanup().await;
    assert!(!root.exists());
}

#[tokio::test]
async fn serial_tool_mode_keeps_additional_callbacks_pending_without_reprompt() {
    let (registry, entry, actor, mut receive) = fixture();
    let (_send, signal) = stop();
    let mut first = body();
    first["parallel_tool_calls"] = json!(false);
    let task = {
        let registry = registry.clone();
        let signal = signal.clone();
        tokio::spawn(async move { registry.respond(owner(), config(), &first, signal).await })
    };
    let prompt = receive.recv().await.unwrap();
    callback(&actor, &entry, json!("one"));
    callback(&actor, &entry, json!("two"));
    let first = task.await.unwrap().unwrap();
    assert_eq!(first["output"].as_array().unwrap().len(), 1);
    let next = json!({"model":"workbuddy/fixture-model","previous_response_id":first["id"],"parallel_tool_calls":false,"input":[{"type":"function_call_output","call_id":first["output"][0]["call_id"],"output":"first actual result"}]});
    let second = registry
        .respond(owner(), config(), &next, signal)
        .await
        .unwrap();
    let result = receive.recv().await.unwrap();
    assert_eq!(result["id"], "one");
    assert_eq!(second["output"].as_array().unwrap().len(), 1);
    assert_ne!(
        second["output"][0]["call_id"],
        first["output"][0]["call_id"]
    );
    assert!(receive.try_recv().is_err());
    assert_eq!(
        actor.core.lock().unwrap().prompt_id.as_deref(),
        prompt["id"].as_str()
    );
}
