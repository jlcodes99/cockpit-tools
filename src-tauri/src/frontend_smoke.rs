//! Explicit, isolated verification of the exact release executable's embedded UI.
//! No production command handlers, account stores, plugins, or workers are started.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{ipc::InvokeBody, Manager, WebviewUrl, WebviewWindowBuilder};

const FLAG: &str = "--frontend-smoke-test";
const MARKER: &str = "cockpit-frontend-smoke-v1";

struct Evidence {
    path: PathBuf,
    value: Value,
    denied: BTreeSet<String>,
    finished: bool,
    ready_since: Option<Instant>,
    consecutive_valid_snapshots: u32,
}

impl Evidence {
    fn save(&mut self) {
        self.value["deniedBusinessCommands"] = json!(self.denied);
        self.value["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.value) {
            let _ = fs::write(&self.path, bytes);
        }
    }
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let size = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    Ok(format!("{:X}", hash.finalize()))
}

fn validate_output(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || !path.is_dir() {
        return Err("Smoke directory must already exist and be absolute".into());
    }
    for ancestor in path.ancestors() {
        let meta = fs::symlink_metadata(ancestor).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            return Err("Smoke directory cannot contain links".into());
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("Smoke directory cannot contain reparse points".into());
            }
        }
    }
    let entries = fs::read_dir(path)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if entries.len() != 1
        || entries[0].file_name() != "frontend-smoke.allowed"
        || fs::read_to_string(path.join("frontend-smoke.allowed"))
            .map_err(|e| e.to_string())?
            .trim()
            != MARKER
    {
        return Err("Smoke directory must contain only the explicit consent marker".into());
    }
    Ok(())
}

pub(crate) fn try_run() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(FLAG)) {
        return false;
    }
    let result = match (args.next(), args.next()) {
        (Some(dir), None) => run(PathBuf::from(dir)),
        _ => Err("Expected --frontend-smoke-test <isolated absolute directory>".into()),
    };
    if let Err(error) = result {
        eprintln!("Frontend smoke: {error}");
        std::process::exit(2);
    }
    true
}

fn run(output: PathBuf) -> Result<(), String> {
    validate_output(&output)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let state = Arc::new(Mutex::new(Evidence {
        path: output.join("webview-evidence.json"),
        value: json!({
            "schemaVersion":1,"status":"Starting","pid":std::process::id(),
            "executable":exe,"artifactSha256":hash_file(&exe)?,
            "scope":"Exact embedded frontend in minimal isolated WebView; business API is not exercised",
            "frontendReady":false,"frontendStages":[],"pageLoads":[],
            "productionHandlersRegistered":false,"productionPluginsRegistered":false,
            "webviewDataDirectory":output.join("webview"),"snapshots":[]
        }),
        denied: BTreeSet::new(),
        finished: false,
        ready_since: None,
        consecutive_valid_snapshots: 0,
    }));
    state.lock().unwrap().save();
    let mut context = crate::create_app_context();
    context.config_mut().identifier = "com.jlcodes.cockpit-tools.frontend-smoke".into();
    context.config_mut().app.windows.clear();
    let handler_state = state.clone();
    let setup_state = state.clone();
    let page_state = state.clone();
    let app = tauri::Builder::default()
        .invoke_handler(move |invoke| {
            let command = invoke.message.command().to_string();
            let body = match invoke.message.payload() { InvokeBody::Json(value) => value.clone(), _ => Value::Null };
            let handle = invoke.message.webview().app_handle().clone();
            let mut record = handler_state.lock().unwrap();
            match command.as_str() {
                "diagnostics_frontend_stage" => {
                    if let Some(stage) = body.get("stage").and_then(Value::as_str) {
                        if let Some(stages) = record.value["frontendStages"].as_array_mut() { stages.push(json!(stage)); }
                    }
                    record.save(); invoke.resolver.resolve(());
                }
                "diagnostics_frontend_ready" => {
                    record.value["frontendReady"] = json!(body["stage"] == "react_mounted");
                    record.save(); invoke.resolver.resolve(());
                }
                "frontend_smoke_snapshot" => {
                    if !record.finished {
                        let snapshot = body.get("snapshot").cloned().unwrap_or(Value::Null);
                        let valid = record.value["frontendReady"] == true
                            && record.value["frontendStages"].as_array().is_some_and(|stages| stages.iter().any(|stage| stage == "script_loaded"))
                            && snapshot["readyState"] == "complete"
                            && snapshot["rootChildren"].as_u64().unwrap_or(0) > 0
                            && snapshot["appVisible"] == true && snapshot["mainVisible"] == true
                            && snapshot["hasIndexScript"] == true && snapshot["indexScriptLoaded"] == true
                            && snapshot["styleSheets"].as_u64().unwrap_or(0) > 0
                            && snapshot["resourceErrors"].as_array().is_some_and(Vec::is_empty)
                            && snapshot["fatalErrors"].as_array().is_some_and(Vec::is_empty);
                        if let Some(snapshots) = record.value["snapshots"].as_array_mut() {
                            snapshots.push(snapshot);
                            if snapshots.len() > 5 { snapshots.remove(0); }
                        }
                        if valid {
                            record.consecutive_valid_snapshots += 1;
                            record.ready_since.get_or_insert_with(Instant::now);
                        } else {
                            record.consecutive_valid_snapshots = 0;
                            record.ready_since = None;
                        }
                        let stable_for = record.ready_since.map(|start| start.elapsed()).unwrap_or_default();
                        record.value["consecutiveValidSnapshots"] = json!(record.consecutive_valid_snapshots);
                        record.value["stableForMs"] = json!(stable_for.as_millis() as u64);
                        if record.consecutive_valid_snapshots >= 2 && stable_for >= Duration::from_secs(2) {
                            record.finished = true; record.value["status"] = json!("Passed");
                            record.value["frontendAssetMarkerFound"] = json!(true);
                            let closer = handle.clone();
                            std::thread::spawn(move || { closer.exit(0); });
                        }
                        record.save();
                    }
                    invoke.resolver.resolve(());
                }
                _ => {
                    record.denied.insert(command);
                    invoke.resolver.reject("FRONTEND_SMOKE_BUSINESS_COMMAND_DISABLED");
                }
            }
            true
        })
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Cockpit Tools — isolated frontend verification")
                .inner_size(1280.0, 800.0)
                .visible(false)
                .data_directory(output.join("webview"))
                .initialization_script(SCRIPT)
                .on_navigation(|url| url.scheme() == "tauri" || ((url.scheme() == "http" || url.scheme() == "https") && url.host_str() == Some("tauri.localhost")))
                .on_page_load(move |_window, payload| {
                    let mut record = page_state.lock().unwrap();
                    if let Some(loads) = record.value["pageLoads"].as_array_mut() {
                        loads.push(json!({"url":payload.url().as_str(),"event":format!("{:?}",payload.event())}));
                    }
                    record.save();
                })
                .build()?;
            let handle = app.handle().clone();
            let timeout_state = setup_state.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(45));
                let mut record = timeout_state.lock().unwrap();
                if !record.finished {
                    record.finished = true; record.value["status"] = json!("FailedTimeout"); record.save();
                    handle.exit(2);
                }
            });
            Ok(())
        })
        .build(context).map_err(|e| e.to_string())?;
    let exit_state = state.clone();
    app.run(move |_app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            let mut record = exit_state.lock().unwrap();
            if !record.finished {
                record.value["status"] = json!("ClosedBeforeVerification");
            }
            record.value["eventLoopExited"] = json!(true);
            record.save();
        }
    });
    let record = state.lock().map_err(|e| e.to_string())?;
    if record.value["status"] != "Passed" || record.value["eventLoopExited"] != true {
        return Err(format!(
            "WebView verification did not pass: {}",
            record.value["status"]
        ));
    }
    Ok(())
}

const SCRIPT: &str = r#"(() => {
  const resourceErrors=[], fatalErrors=[], expectedRejections=[];
  const short=x=>String(x ?? '').slice(0,600);
  const expected=x=>short(x).includes('FRONTEND_SMOKE_');
  window.addEventListener('error', e=>{
    const target=e.target;
    if(target && target!==window && (target.src||target.href)) resourceErrors.push(short(target.src||target.href));
    else if(!expected(e.message)) fatalErrors.push(short(e.message));
  },true);
  window.addEventListener('unhandledrejection',e=>{
    if(expected(e.reason)) expectedRejections.push(short(e.reason)); else fatalErrors.push(short(e.reason));
  });
  window.addEventListener('cockpit:boot-failed',()=>fatalErrors.push('cockpit:boot-failed'));
  const local=url=>{try{const u=new URL(url,location.href);return ['tauri:','asset:','ipc:'].includes(u.protocol)||['tauri.localhost','ipc.localhost','asset.localhost'].includes(u.hostname);}catch{return false}};
  const originalFetch=window.fetch.bind(window);
  window.fetch=(input,init)=>local(typeof input==='string'||input instanceof URL?input:input.url)?originalFetch(input,init):Promise.reject(new Error('FRONTEND_SMOKE_EXTERNAL_REQUEST_BLOCKED'));
  window.WebSocket=class{constructor(){throw new Error('FRONTEND_SMOKE_EXTERNAL_REQUEST_BLOCKED')}};
  const visible=el=>{if(!el)return false;const r=el.getBoundingClientRect();const s=getComputedStyle(el);return r.width>200&&r.height>100&&s.display!=='none'&&s.visibility!=='hidden';};
  let ticks=0;
  const timer=setInterval(()=>{
    if(++ticks>40){clearInterval(timer);return;}
    const root=document.getElementById('root'), app=document.querySelector('.app-container'), main=document.querySelector('.main-wrapper');
    const resources=performance.getEntriesByType('resource');
    const snapshot={href:location.href,readyState:document.readyState,rootChildren:root?.childElementCount||0,
      appVisible:visible(app),mainVisible:visible(main),rootText:short(root?.innerText),
      hasIndexScript:[...document.scripts].some(s=>/\/assets\/index-/.test(s.src)),
      indexScriptLoaded:resources.some(r=>/\/assets\/index-/.test(r.name)&&r.responseEnd>0),
      scriptSources:[...document.scripts].map(s=>s.src).filter(Boolean),styleSheets:document.styleSheets.length,
      resourceErrors:[...resourceErrors],fatalErrors:[...fatalErrors],expectedRejections:[...expectedRejections],
      viewport:{width:innerWidth,height:innerHeight},resources:resources.map(r=>({name:r.name,initiatorType:r.initiatorType,duration:r.duration,responseEnd:r.responseEnd,decodedBodySize:r.decodedBodySize})).slice(0,100)};
    window.__TAURI_INTERNALS__?.invoke('frontend_smoke_snapshot',{snapshot}).catch(()=>{});
  },1000);
})();"#;
