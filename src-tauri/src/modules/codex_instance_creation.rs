// Reservations reject duplicate in-flight names/directories without holding the registry lock.
struct CodexInstanceCreationReservation {
    name: String,
    directory: PathBuf,
}

fn codex_instance_creation_reservations() -> &'static std::sync::Mutex<Vec<(String, PathBuf)>> {
    static RESERVATIONS: std::sync::OnceLock<std::sync::Mutex<Vec<(String, PathBuf)>>> =
        std::sync::OnceLock::new();
    RESERVATIONS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

fn instance_creation_directory_key(directory: &str) -> PathBuf {
    let path = PathBuf::from(instance_store::display_path(Path::new(directory)));
    let resolved = path.canonicalize().unwrap_or_else(|_| {
        path.parent()
            .and_then(|parent| parent.canonicalize().ok())
            .and_then(|parent| path.file_name().map(|name| parent.join(name)))
            .unwrap_or_else(|| path.clone())
    });
    #[cfg(windows)]
    {
        let text = resolved.to_string_lossy().replace('/', "\\").to_lowercase();
        let text = if let Some(tail) = text.strip_prefix(r"\\?\unc\") {
            format!(r"\\{tail}")
        } else {
            text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
        };
        PathBuf::from(text)
    }
    #[cfg(not(windows))]
    {
        resolved
    }
}

impl CodexInstanceCreationReservation {
    fn begin(name: &str, directory: &str) -> Result<Self, String> {
        let directory = instance_creation_directory_key(directory);
        let mut pending = codex_instance_creation_reservations()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pending
            .iter()
            .any(|(pending_name, path)| pending_name == name || path == &directory)
        {
            return Err("同名或同目录的实例正在创建，请稍后重试".to_string());
        }
        pending.push((name.to_string(), directory.clone()));
        Ok(Self {
            name: name.to_string(),
            directory,
        })
    }
}

impl Drop for CodexInstanceCreationReservation {
    fn drop(&mut self) {
        let mut pending = codex_instance_creation_reservations()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.retain(|(name, directory)| name != &self.name || directory != &self.directory);
    }
}

pub fn create_instance(params: CreateInstanceParams) -> Result<InstanceProfile, String> {
    create_instance_with_cancellation(params, &std::sync::atomic::AtomicBool::new(false))
}

pub fn create_instance_with_cancellation(
    params: CreateInstanceParams,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<InstanceProfile, String> {
    create_instance_with_profile_copy(params, cancelled, |source, target| {
        modules::codex_profile_copy::copy_profile_with_cancellation(source, target, cancelled)
    })
}

fn check_instance_creation_cancelled(
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Err("创建实例已取消或超时，请重试".to_string());
    }
    Ok(())
}

fn create_instance_with_profile_copy(
    params: CreateInstanceParams,
    cancelled: &std::sync::atomic::AtomicBool,
    copy_profile: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<InstanceProfile, String> {
    check_instance_creation_cancelled(cancelled)?;
    let _creation_guard = crate::modules::instance_storage_cleanup::protect_instance_creation()?;
    let name = instance_store::normalize_name(&params.name)?;
    let user_data_dir = params.user_data_dir.trim().to_string();
    if user_data_dir.is_empty() {
        return Err("实例目录不能为空".to_string());
    }

    // Resolve directory aliases before taking the registry lock (disk access can be slow).
    let _reservation = CodexInstanceCreationReservation::begin(&name, &user_data_dir)?;
    let _lock = CODEX_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let store = load_instance_store()?;

    instance_store::ensure_unique(&store, &name, &user_data_dir, None)?;
    // Registry readers and other creates must remain available while files are copied.
    drop(_lock);
    check_instance_creation_cancelled(cancelled)?;

    let user_dir_path = PathBuf::from(&user_data_dir);
    let init_mode = params
        .init_mode
        .as_deref()
        .unwrap_or("copy")
        .to_ascii_lowercase();
    let create_empty = init_mode == "empty";
    let use_existing_dir = init_mode == "existingdir" || init_mode == "existing_dir";

    if use_existing_dir {
        if !user_dir_path.exists() {
            let resolved = instance_store::display_path(&user_dir_path);
            return Err(format!("所选目录不存在: {}", resolved));
        }
        if !user_dir_path.is_dir() {
            return Err("所选路径不是目录".to_string());
        }
    } else if create_empty {
        if user_dir_path.exists() {
            let mut has_entries = false;
            if let Ok(mut iter) = fs::read_dir(&user_dir_path) {
                if iter.next().is_some() {
                    has_entries = true;
                }
            }
            if has_entries {
                let resolved_path = instance_store::display_path(&user_dir_path);
                return Err(format!("空白实例需要目标目录为空: {}", resolved_path));
            }
        }
        fs::create_dir_all(&user_dir_path).map_err(|e| format!("创建实例目录失败: {}", e))?;
    } else {
        let source_dir = match params.copy_source_instance_id.as_deref() {
            Some("__default__") | None => get_default_codex_home()?,
            Some(source_id) => {
                let source_instance = store
                    .instances
                    .iter()
                    .find(|item| item.id == source_id)
                    .ok_or("复制来源实例不存在")?;
                PathBuf::from(&source_instance.user_data_dir)
            }
        };

        if user_dir_path.exists() {
            let mut has_entries = false;
            if let Ok(mut iter) = fs::read_dir(&user_dir_path) {
                if iter.next().is_some() {
                    has_entries = true;
                }
            }
            if has_entries {
                let resolved_path = instance_store::display_path(&user_dir_path);
                modules::logger::log_info(&format!(
                    "[Codex Instance] 复制来源实例需要空目录，但目标已存在: {}",
                    resolved_path
                ));
                return Err(format!("复制来源实例需要目标目录为空: {}", resolved_path));
            }
        }

        if !source_dir.exists() {
            return Err("未找到复制来源目录，请先确保来源实例已初始化".to_string());
        }

        copy_profile(&source_dir, &user_dir_path)?;
    }

    check_instance_creation_cancelled(cancelled)?;
    ensure_instance_shared_skills(&user_dir_path)?;

    check_instance_creation_cancelled(cancelled)?;
    let _registration_lock = CODEX_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    instance_store::ensure_unique(&store, &name, &user_data_dir, None)?;
    check_instance_creation_cancelled(cancelled)?;

    let instance = InstanceProfile {
        id: Uuid::new_v4().to_string(),
        name,
        user_data_dir,
        working_dir: params.working_dir,
        extra_args: params.extra_args.trim().to_string(),
        bind_account_id: if create_empty {
            None
        } else {
            params.bind_account_id
        },
        model_routing: if create_empty {
            None
        } else {
            params.model_routing
        },
        launch_mode: params.launch_mode.unwrap_or_default(),
        app_speed: params.app_speed.unwrap_or_default(),
        created_at: Utc::now().timestamp_millis(),
        last_launched_at: None,
        last_pid: None,
    };

    store.instances.push(instance.clone());
    save_instance_store(&store)?;
    Ok(instance)
}
