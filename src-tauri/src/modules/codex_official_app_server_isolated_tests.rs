// Explicit, offline integration only: the caller must provide a disposable
// HOME/CODEX_HOME and launch this test under an OS-level network deny policy.
#[test]
#[ignore = "requires isolated HOME, native official Codex and OS-denied networking"]
fn isolated_native_official_metadata_rebuild_import_delete_and_failure_recovery() {
    assert_eq!(
        std::env::var("COCKPIT_CODEX_OFFLINE_INTEGRATION").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(
        std::env::var_os("COCKPIT_CODEX_OFFLINE_ROOT").expect("explicit disposable root"),
    );
    let configured_home = PathBuf::from(std::env::var_os("HOME").expect("isolated HOME"));
    let configured_codex =
        PathBuf::from(std::env::var_os("CODEX_HOME").expect("isolated CODEX_HOME"));
    assert!(root.is_absolute());
    assert!(configured_home.starts_with(&root));
    assert!(configured_codex.starts_with(&root));
    let blocked = std::net::TcpStream::connect_timeout(
        &"127.0.0.1:9".parse().unwrap(),
        Duration::from_millis(100),
    )
    .unwrap_err();
    assert!(
        matches!(blocked.raw_os_error(), Some(1 | 13)),
        "OS network deny must be active: {blocked}"
    );
    let native = PathBuf::from(
        std::env::var_os(CODEX_APP_SERVER_EXECUTABLE_ENV).expect("explicit native engine"),
    );
    assert_eq!(official_app_server_executable().unwrap(), native);
    let mut bytes = [0u8; 4];
    std::io::Read::read_exact(&mut std::fs::File::open(&native).unwrap(), &mut bytes).unwrap();
    assert_eq!(
        &bytes[..4],
        &[0xcf, 0xfa, 0xed, 0xfe],
        "never invoke a shell/Node wrapper"
    );
    let codex = root.join(format!("fixture-{}", uuid::Uuid::new_v4()));
    let project = codex.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(codex.join("config.toml"), "cli_auth_credentials_store = \"file\"\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n").unwrap();
    let session_id = uuid::Uuid::new_v4().to_string();
    let rollout = codex.join(format!(
        "sessions/2026/10/11/rollout-2026-10-11T03-00-00-{session_id}.jsonl"
    ));
    std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();
    let records = [
        json!({"timestamp":"2026-10-11T03:00:00Z","type":"session_meta","payload":{"id":session_id,"timestamp":"2026-10-11T03:00:00Z","cwd":project,"originator":"codex_cli_rs","cli_version":"0.162.0-alpha.17.2","source":"cli","model_provider":"openai"}}),
        json!({"timestamp":"2026-10-11T03:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"QA synthetic offline thread","images":[],"local_images":[],"text_elements":[]}}),
        json!({"timestamp":"2026-10-11T03:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"QA synthetic offline thread"}]}}),
    ];
    let original = records
        .iter()
        .map(|record| format!("{record}\n"))
        .collect::<String>()
        .into_bytes();
    std::fs::write(&rollout, &original).unwrap();
    std::fs::write(codex.join("session_index.jsonl"), format!("{}\n", json!({"id":session_id,"thread_name":"QA synthetic offline thread","updated_at":"2026-10-11T03:00:01Z"}))).unwrap();
    let started = Instant::now();
    rebuild_thread_metadata(&codex).expect("production rebuild using real official app-server");
    assert!(started.elapsed() < Duration::from_secs(25));
    assert_eq!(std::fs::read(&rollout).unwrap(), original);
    let databases = std::fs::read_dir(&codex)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("state_")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "sqlite")
        })
        .collect::<Vec<_>>();
    assert!(
        !databases.is_empty(),
        "official engine must create a local state DB"
    );
    let count = || {
        databases
            .iter()
            .map(|path| {
                let db = rusqlite::Connection::open(path).unwrap();
                db.query_row(
                    "SELECT COUNT(*) FROM threads WHERE id=?1",
                    [&session_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
            })
            .sum::<i64>()
    };
    assert!(
        count() > 0,
        "thread must be indexed by the actual official engine"
    );
    let mapped = rebuild_imported_thread_metadata(
        &codex,
        &[(session_id.clone(), project.to_string_lossy().into_owned())],
    );
    if let Err(error) = mapped {
        assert!(
            error.contains("未找到目标目录对应的 Codex 项目"),
            "unexpected mapped rebuild error: {error}"
        );
        println!("Expected missing-project warning: {error}");
    }
    assert_eq!(std::fs::read(&rollout).unwrap(), original);
    assert!(count() > 0);
    let project_id = isolated_native_create_project(&native, &codex, &project);
    rebuild_imported_thread_metadata(
        &codex,
        &[(session_id.clone(), project.to_string_lossy().into_owned())],
    )
    .expect("production mapped rebuild must assign the actual locally registered project");
    let db = rusqlite::Connection::open(&databases[0]).unwrap();
    let saved_project: Option<String> = db
        .query_row(
            "SELECT project_id FROM threads WHERE id=?1",
            [&session_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(saved_project.as_deref(), Some(project_id.as_str()));
    drop(db);
    assert_eq!(std::fs::read(&rollout).unwrap(), original);
    println!(
        "Actual project/create -> mapped production rebuild -> persisted project_id: {project_id}"
    );
    let invalid = delete_threads(&codex, &["not-a-valid-thread-id".into()]).unwrap();
    assert_eq!(invalid, 0);
    assert_eq!(std::fs::read(&rollout).unwrap(), original);
    rebuild_thread_metadata(&codex).expect("retry after failed delete");
    let deleted = delete_threads(&codex, &[session_id.clone(), session_id.clone()]).unwrap();
    assert_eq!(deleted, 1, "deduplicated real thread/delete");
    assert!(!rollout.exists());
    assert_eq!(count(), 0);
    assert!(!codex.join("auth.json").exists());
    println!(
        "Official engine: {}; fixture: {}; DBs: {:?}; elapsed_ms: {}",
        native.display(),
        codex.display(),
        databases,
        started.elapsed().as_millis()
    );
    std::fs::remove_dir_all(codex).unwrap();
}

fn isolated_native_create_project(native: &Path, codex: &Path, project: &Path) -> String {
    let mut child = build_app_server_command(native, codex).spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = sender.send(line);
        }
    });
    let errors = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            println!("[isolated project/create stderr] {line}");
        }
    });
    let result = (|| -> Result<String, String> {
        send_request(
            &mut stdin,
            json!({
                "method":"initialize", "id":1,
                "params":{"clientInfo":{"name":"cockpit-tools-offline-test","version":"1"},"capabilities":{"experimentalApi":true}}
            }),
        )?;
        wait_for_response(&receiver, 1)?;
        // Parameters are taken from the signed engine's generated local JSON
        // schema. This registers only the fixture folder, without a desktop UI.
        send_request(
            &mut stdin,
            json!({
                "method":"project/create", "id":2,
                "params":{"idempotencyKey":uuid::Uuid::new_v4().to_string(),"name":"QA isolated project","roots":[{"path":project}],"metadata":{"qa":"offline"}}
            }),
        )?;
        let response =
            wait_for_response_value(&receiver, 2).map_err(|error| error.message().to_owned())?;
        response["result"]["project"]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("project/create missing id: {response}"))
    })();
    finish_child(&mut child);
    reader.join().unwrap();
    errors.join().unwrap();
    result.expect("actual official local project/create")
}
