// Codex Local Access：Profile inspection, takeover backup and restore operations。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
fn apply_cost_delta_to_event(event: &mut CodexLocalAccessUsageEvent, delta: f64) {
    if !delta.is_finite() || delta == 0.0 {
        return;
    }
    event.estimated_cost_usd = (event.estimated_cost_usd + delta).max(0.0);
}

fn build_api_port_url(port: u16) -> String {
    format!("http://{CODEX_LOCAL_ACCESS_DEFAULT_CLIENT_URL_HOST}:{port}{CHAT_COMPLETIONS_PATH}")
}

fn build_base_url(port: u16) -> String {
    build_base_url_with_host(port, CodexLocalAccessClientBaseUrlHost::default())
}

fn client_base_url_host_text(host: CodexLocalAccessClientBaseUrlHost) -> &'static str {
    match host {
        CodexLocalAccessClientBaseUrlHost::Localhost => "localhost",
        CodexLocalAccessClientBaseUrlHost::Ipv4Loopback => "127.0.0.1",
    }
}

fn build_base_url_with_host(port: u16, host: CodexLocalAccessClientBaseUrlHost) -> String {
    format!("http://{}:{port}/v1", client_base_url_host_text(host))
}

fn build_collection_base_url(collection: &CodexLocalAccessCollection) -> String {
    build_base_url_with_host(collection.port, collection.client_base_url_host)
}

#[derive(Debug, Clone, Default)]
struct ProfileConfigInspection {
    config_attached: bool,
    model_provider: Option<String>,
    base_url: Option<String>,
    token_matched: bool,
}

fn profile_auth_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join(CODEX_PROFILE_AUTH_FILE)
}

fn profile_config_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join(CODEX_PROFILE_CONFIG_FILE)
}

fn normalize_profile_dir_key(profile_dir: &Path) -> String {
    profile_dir
        .to_string_lossy()
        .trim()
        .trim_end_matches(|item| item == '/' || item == '\\')
        .to_string()
}

fn read_optional_profile_file(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "读取 Codex 配置文件失败: path={}, error={}",
            path.display(),
            e
        )
    })?;
    if path.file_name().and_then(|item| item.to_str()) == Some(CODEX_PROFILE_CONFIG_FILE) {
        let (normalized, _) =
            crate::modules::codex_config_format::normalize_codex_config_input(&content);
        Ok(Some(normalized))
    } else {
        Ok(Some(content))
    }
}

fn write_optional_profile_file(path: &Path, content: Option<&str>) -> Result<(), String> {
    match content {
        Some(content) => {
            let content = if path.file_name().and_then(|item| item.to_str())
                == Some(CODEX_PROFILE_CONFIG_FILE)
            {
                crate::modules::codex_config_format::normalize_config_toml_spacing(content)
            } else {
                content.to_string()
            };
            if path.file_name().and_then(|item| item.to_str()) == Some(CODEX_PROFILE_CONFIG_FILE) {
                crate::modules::codex_config_format::write_codex_config_toml_atomic(path, &content)
            } else {
                write_string_atomic(path, &content)
            }
        }
        None => {
            if path.exists() {
                std::fs::remove_file(path).map_err(|e| {
                    format!(
                        "删除 Codex 配置文件失败: path={}, error={}",
                        path.display(),
                        e
                    )
                })?;
            }
            Ok(())
        }
    }
}

fn is_codex_local_access_config_for_api_key(config_text: &str, api_key: &str) -> bool {
    let Ok(doc) = crate::modules::codex_config_format::read_codex_config_doc_from_str(config_text)
    else {
        return false;
    };
    let provider_selected = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        == Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID);
    if !provider_selected {
        return false;
    }

    doc.get("model_providers")
        .and_then(|item| item.as_table())
        .and_then(|providers| providers.get(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID))
        .and_then(|item| item.as_table())
        .and_then(|provider| provider.get("experimental_bearer_token"))
        .and_then(|item| item.as_str())
        .map(str::trim)
        == Some(api_key.trim())
}

fn is_cockpit_managed_local_access_config(config_text: &str) -> bool {
    let Ok(doc) = crate::modules::codex_config_format::read_codex_config_doc_from_str(config_text)
    else {
        return false;
    };
    doc.get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        == Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID)
        && doc
            .get("model_providers")
            .and_then(|item| item.as_table())
            .and_then(|providers| providers.get(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID))
            .and_then(|item| item.as_table())
            .and_then(|provider| provider.get("experimental_bearer_token"))
            .and_then(|item| item.as_str())
            .is_some_and(|key| key.trim().starts_with("agt_codex_"))
}

fn normalize_profile_base_url_for_match(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = Url::parse(raw).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let port = parsed
        .port()
        .map(|value| format!(":{}", value))
        .unwrap_or_default();
    let path = parsed.path().trim_end_matches('/');
    Some(format!(
        "{}://{}{}{}",
        parsed.scheme().to_ascii_lowercase(),
        host,
        port,
        path
    ))
}

fn provider_has_nonempty_static_header(provider: &toml_edit::Table, header_name: &str) -> bool {
    let Some(headers) = provider.get("http_headers") else {
        return false;
    };
    if let Some(inline) = headers.as_inline_table() {
        return inline.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case(header_name)
                && value.as_str().is_some_and(|value| !value.trim().is_empty())
        });
    }
    headers.as_table().is_some_and(|table| {
        table.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case(header_name)
                && value.as_str().is_some_and(|value| !value.trim().is_empty())
        })
    })
}

fn profile_base_url_matches(actual: Option<&str>, expected: &str) -> bool {
    normalize_profile_base_url_for_match(actual)
        .zip(normalize_profile_base_url_for_match(Some(expected)))
        .map(|(actual, expected)| actual == expected)
        .unwrap_or(false)
}

fn inspect_local_access_profile_config(
    config_text: &str,
    expected_base_url: &str,
    expected_api_key: &str,
    uses_bound_oauth_auth: bool,
) -> Result<ProfileConfigInspection, String> {
    let doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(config_text)
        .map_err(|e| format!("解析 Codex config.toml 失败: {}", e))?;
    let model_provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let provider_table = doc
        .get("model_providers")
        .and_then(|item| item.as_table())
        .and_then(|providers| providers.get(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID))
        .and_then(|item| item.as_table());
    let base_url = provider_table
        .and_then(|table| table.get("base_url"))
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let wire_api_matches = provider_table
        .and_then(|table| table.get("wire_api"))
        .and_then(|item| item.as_str())
        .map(str::trim)
        == Some("responses");
    let requires_openai_auth = provider_table
        .and_then(|table| table.get("requires_openai_auth"))
        .and_then(|item| item.as_bool());
    let imagegen_actor_authorized = provider_table.is_some_and(|table| {
        provider_has_nonempty_static_header(table, CODEX_IMAGEGEN_ACTOR_HEADER)
    });
    // 双路径：
    // - 绑定 OAuth：require_openai_auth=true（显示账号）+ actor（生图走本地，避免卡 Confirming）
    // - 纯 API Key：require_openai_auth=false + actor
    let auth_projection_matches = if uses_bound_oauth_auth {
        requires_openai_auth == Some(true) && imagegen_actor_authorized
    } else {
        requires_openai_auth == Some(false) && imagegen_actor_authorized
    };
    let token_matched = provider_table
        .and_then(|table| table.get("experimental_bearer_token"))
        .and_then(|item| item.as_str())
        .map(str::trim)
        == Some(expected_api_key.trim());
    let provider_selected =
        model_provider.as_deref() == Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID);
    let config_attached = provider_selected
        && provider_table.is_some()
        && profile_base_url_matches(base_url.as_deref(), expected_base_url)
        && wire_api_matches
        && auth_projection_matches
        && token_matched;

    Ok(ProfileConfigInspection {
        config_attached,
        model_provider,
        base_url,
        token_matched,
    })
}

fn inspect_local_access_profile_attachment(
    profile_dir: &Path,
    collection: Option<&CodexLocalAccessCollection>,
) -> CodexLocalAccessProfileAttachment {
    let profile_dir_text = normalize_profile_dir_key(profile_dir);
    let Some(collection) = collection else {
        return CodexLocalAccessProfileAttachment {
            profile_dir: profile_dir_text,
            attached: false,
            config_attached: false,
            auth_attached: false,
            model_provider: None,
            base_url: None,
            expected_base_url: None,
            error: None,
        };
    };

    let expected_base_url = build_collection_base_url(collection);
    let expected_api_key = collection.api_key.trim();
    let has_bound_oauth =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref()).is_some();
    let mut attachment = CodexLocalAccessProfileAttachment {
        profile_dir: profile_dir_text,
        attached: false,
        config_attached: false,
        auth_attached: false,
        model_provider: None,
        base_url: None,
        expected_base_url: Some(expected_base_url.clone()),
        error: None,
    };

    let auth_text = match read_optional_profile_file(&profile_auth_path(profile_dir)) {
        Ok(content) => content,
        Err(error) => {
            attachment.error = Some(error);
            None
        }
    };
    let uses_bound_oauth_auth = auth_text
        .as_deref()
        .is_some_and(|text| has_bound_oauth && is_codex_oauth_auth_text(text));
    attachment.auth_attached = auth_text.as_deref().is_some_and(|text| {
        uses_bound_oauth_auth || is_codex_local_access_auth_text(text, expected_api_key)
    });

    match read_optional_profile_file(&profile_config_path(profile_dir)) {
        Ok(Some(config_text)) => match inspect_local_access_profile_config(
            &config_text,
            &expected_base_url,
            expected_api_key,
            uses_bound_oauth_auth,
        ) {
            Ok(inspection) => {
                attachment.config_attached = inspection.config_attached;
                attachment.model_provider = inspection.model_provider;
                attachment.base_url = inspection.base_url;
                if !inspection.token_matched && attachment.config_attached {
                    attachment.error = Some("Codex API 服务接管密钥不匹配".to_string());
                }
            }
            Err(error) => {
                attachment.error = Some(match attachment.error.take() {
                    Some(existing) => format!("{}；{}", existing, error),
                    None => error,
                });
            }
        },
        Ok(None) => {}
        Err(error) => {
            attachment.error = Some(match attachment.error.take() {
                Some(existing) => format!("{}；{}", existing, error),
                None => error,
            });
        }
    }

    attachment.attached = attachment.config_attached;
    attachment
}

fn provider_header_value(
    provider: &toml_edit::Table,
    header_name: &str,
) -> Option<toml_edit::Value> {
    let headers = provider.get("http_headers")?;
    if let Some(headers) = headers.as_inline_table() {
        return headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(header_name))
            .map(|(_, value)| value.clone());
    }
    headers.as_table().and_then(|headers| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(header_name))
            .and_then(|(_, item)| item.as_value().cloned())
    })
}

fn set_provider_header_value(
    provider: &mut toml_edit::Table,
    header_name: &str,
    header_value: toml_edit::Value,
) {
    if provider.get("http_headers").is_none() {
        provider["http_headers"] =
            toml_edit::Item::Value(toml_edit::Value::InlineTable(toml_edit::InlineTable::new()));
    }
    let headers = provider
        .get_mut("http_headers")
        .expect("http_headers should exist after initialization");
    if let Some(headers) = headers.as_inline_table_mut() {
        headers.insert(header_name, header_value);
    } else if let Some(headers) = headers.as_table_mut() {
        headers[header_name] = toml_edit::Item::Value(header_value);
    }
}

fn remove_codex_local_access_config(config_text: &str) -> Result<String, String> {
    if config_text.trim().is_empty() {
        return Ok(String::new());
    }

    let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(config_text)
        .map_err(|e| format!("解析 Codex config.toml 失败: {}", e))?;
    if doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        != Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID)
    {
        return Ok(config_text.to_string());
    }

    let _ = doc.remove("model_provider");
    if doc
        .get("model_catalog_json")
        .and_then(|item| item.as_str())
        .is_some_and(is_cockpit_managed_model_catalog_name)
    {
        let _ = doc.remove("model_catalog_json");
    }
    let should_remove_model_providers = doc
        .get_mut("model_providers")
        .and_then(|item| item.as_table_mut())
        .map(|model_providers| {
            if let Some(provider) = model_providers
                .get_mut(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID)
                .and_then(|item| item.as_table_mut())
            {
                for key in [
                    "name",
                    "base_url",
                    "wire_api",
                    "requires_openai_auth",
                    "experimental_bearer_token",
                    "supports_websockets",
                ] {
                    let _ = provider.remove(key);
                }
                let remove_headers = provider
                    .get_mut("http_headers")
                    .map(|headers| {
                        if let Some(headers) = headers.as_inline_table_mut() {
                            let managed_keys = headers
                                .iter()
                                .filter(|(key, _)| {
                                    key.eq_ignore_ascii_case(CODEX_IMAGEGEN_ACTOR_HEADER)
                                        || key.eq_ignore_ascii_case(
                                            CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER,
                                        )
                                        || key.eq_ignore_ascii_case(
                                            codex_account::CODEX_CLIENT_INSTANCE_ID_HEADER,
                                        )
                                })
                                .map(|(key, _)| key.to_string())
                                .collect::<Vec<_>>();
                            for key in managed_keys {
                                let _ = headers.remove(&key);
                            }
                            headers.is_empty()
                        } else if let Some(headers) = headers.as_table_mut() {
                            let managed_keys = headers
                                .iter()
                                .filter(|(key, _)| {
                                    key.eq_ignore_ascii_case(CODEX_IMAGEGEN_ACTOR_HEADER)
                                        || key.eq_ignore_ascii_case(
                                            CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER,
                                        )
                                        || key.eq_ignore_ascii_case(
                                            codex_account::CODEX_CLIENT_INSTANCE_ID_HEADER,
                                        )
                                })
                                .map(|(key, _)| key.to_string())
                                .collect::<Vec<_>>();
                            for key in managed_keys {
                                let _ = headers.remove(&key);
                            }
                            headers.is_empty()
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if remove_headers {
                    let _ = provider.remove("http_headers");
                }
                if provider.is_empty() {
                    let _ = model_providers.remove(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID);
                }
            }
            model_providers.is_empty()
        })
        .unwrap_or(false);
    if should_remove_model_providers {
        let _ = doc.remove("model_providers");
    }

    Ok(crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc))
}

fn restore_config_toml_from_takeover_backup(
    current_config: Option<&str>,
    backup_config: Option<&str>,
) -> Result<Option<String>, String> {
    let current_config = current_config.unwrap_or_default();
    let mut current_doc = if current_config.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(current_config)
            .map_err(|e| format!("解析当前 Codex config.toml 失败: {}", e))?
    };
    let mut backup_doc = match backup_config.filter(|content| !content.trim().is_empty()) {
        Some(content) => Some(
            crate::modules::codex_config_format::read_codex_config_doc_from_str(content)
                .map_err(|e| format!("解析 Codex API 服务接管备份 config.toml 失败: {}", e))?,
        ),
        None => None,
    };
    if backup_doc
        .as_ref()
        .and_then(|doc| doc.get("model_catalog_json"))
        .and_then(|item| item.as_str())
        .is_some_and(is_cockpit_managed_model_catalog_name)
    {
        if let Some(doc) = backup_doc.as_mut() {
            let _ = doc.remove("model_catalog_json");
        }
    }

    let current_selected_local_access = current_doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        == Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID);
    let current_uses_local_catalog = current_doc
        .get("model_catalog_json")
        .and_then(|item| item.as_str())
        .is_some_and(is_cockpit_managed_model_catalog_name);
    if current_selected_local_access {
        let cleaned = remove_codex_local_access_config(
            &crate::modules::codex_config_format::codex_config_doc_to_string(&mut current_doc),
        )?;
        current_doc = if cleaned.trim().is_empty() {
            Document::new()
        } else {
            crate::modules::codex_config_format::read_codex_config_doc_from_str(&cleaned)
                .map_err(|e| format!("解析清理后的 Codex config.toml 失败: {}", e))?
        };

        if let Some(backup_provider) = backup_doc
            .as_ref()
            .and_then(|doc| doc.get("model_providers"))
            .and_then(|item| item.as_table())
            .and_then(|providers| providers.get(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID))
            .and_then(|item| item.as_table())
        {
            if current_doc.get("model_providers").is_none() {
                current_doc["model_providers"] = toml_edit::table();
            }
            let providers = current_doc["model_providers"]
                .as_table_mut()
                .ok_or("config.toml 中 model_providers 不是合法表结构")?;
            if !providers.contains_key(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID) {
                providers[CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID] = toml_edit::table();
            }
            let provider = providers[CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID]
                .as_table_mut()
                .ok_or("config.toml 中 codex_local_access provider 不是合法表结构")?;
            for key in [
                "name",
                "base_url",
                "wire_api",
                "requires_openai_auth",
                "experimental_bearer_token",
                "supports_websockets",
            ] {
                if let Some(item) = backup_provider.get(key) {
                    provider[key] = item.clone();
                }
            }
            for header_name in [
                CODEX_IMAGEGEN_ACTOR_HEADER,
                CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER,
                codex_account::CODEX_CLIENT_INSTANCE_ID_HEADER,
            ] {
                if let Some(header_value) = provider_header_value(backup_provider, header_name) {
                    set_provider_header_value(provider, header_name, header_value);
                }
            }
        }

        if let Some(model_provider) = backup_doc
            .as_ref()
            .and_then(|doc| doc.get("model_provider"))
        {
            current_doc["model_provider"] = model_provider.clone();
        }
    }

    if current_uses_local_catalog {
        match backup_doc
            .as_ref()
            .and_then(|doc| doc.get("model_catalog_json"))
        {
            Some(item) => current_doc["model_catalog_json"] = item.clone(),
            None => {
                let _ = current_doc.remove("model_catalog_json");
            }
        }
    }

    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut current_doc);
    if content.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(content))
    }
}

fn is_codex_local_access_auth_text(auth_text: &str, api_key: &str) -> bool {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return false;
    }

    let Ok(value) = serde_json::from_str::<Value>(auth_text) else {
        return false;
    };
    let auth_mode = value
        .get("auth_mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_ascii_lowercase);
    let openai_api_key = value
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim);

    auth_mode.as_deref() == Some("apikey")
        && openai_api_key
            .map(|key| key == api_key || key.starts_with("agt_codex_"))
            .unwrap_or(false)
}

fn is_exact_codex_local_access_auth_text(auth_text: &str, api_key: &str) -> bool {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(auth_text) else {
        return false;
    };
    value
        .get("auth_mode")
        .and_then(Value::as_str)
        .is_some_and(|mode| mode.trim().eq_ignore_ascii_case("apikey"))
        && value
            .get("OPENAI_API_KEY")
            .and_then(Value::as_str)
            .is_some_and(|key| key.trim() == api_key)
}

fn is_codex_oauth_auth_text(auth_text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(auth_text) else {
        return false;
    };
    value
        .get("tokens")
        .and_then(|tokens| tokens.get("id_token"))
        .and_then(Value::as_str)
        .is_some_and(|token| !token.trim().is_empty())
}

fn load_takeover_backups() -> Result<CodexLocalAccessTakeoverBackups, String> {
    let path = local_access_takeover_backups_path()?;
    if !path.exists() {
        return Ok(CodexLocalAccessTakeoverBackups {
            version: CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION,
            profiles: Vec::new(),
        });
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取 Codex API 服务接管备份失败: {}", e))?;
    match serde_json::from_str::<CodexLocalAccessTakeoverBackups>(&content) {
        Ok(mut backups) => {
            backups.version = CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION;
            Ok(backups)
        }
        Err(error) => {
            match crate::modules::atomic_write::quarantine_file(&path, "invalid-json") {
                Ok(Some(backup_path)) => logger::log_codex_api_warn(&format!(
                    "Codex API 服务接管备份解析失败，已隔离: path={}, backup={}, error={}",
                    path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => logger::log_codex_api_warn(&format!(
                    "Codex API 服务接管备份解析失败，文件已不存在: path={}, error={}",
                    path.display(),
                    error
                )),
                Err(backup_error) => logger::log_codex_api_warn(&format!(
                    "Codex API 服务接管备份解析失败且隔离失败: path={}, parse_error={}, backup_error={}",
                    path.display(),
                    error,
                    backup_error
                )),
            }
            Ok(CodexLocalAccessTakeoverBackups {
                version: CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION,
                profiles: Vec::new(),
            })
        }
    }
}

fn save_takeover_backups(backups: &CodexLocalAccessTakeoverBackups) -> Result<(), String> {
    let path = local_access_takeover_backups_path()?;
    if backups.profiles.is_empty() {
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("删除 Codex API 服务接管备份失败: {}", e))?;
        }
        return Ok(());
    }

    let content = serde_json::to_string_pretty(backups)
        .map_err(|e| format!("序列化 Codex API 服务接管备份失败: {}", e))?;
    write_string_atomic(&path, &content)
        .map_err(|e| format!("写入 Codex API 服务接管备份失败: {}", e))
}

fn save_profile_takeover_backup(profile_dir: &Path, api_key: &str) -> Result<(), String> {
    let profile_key = normalize_profile_dir_key(profile_dir);
    if profile_key.is_empty() {
        return Err("Codex API 服务接管目录为空".to_string());
    }

    let config_toml = read_optional_profile_file(&profile_config_path(profile_dir))?;
    let mut backups = load_takeover_backups()?;
    let existing_backup = backups
        .profiles
        .iter_mut()
        .find(|item| item.profile_dir == profile_key);

    if config_toml
        .as_deref()
        .map(|content| {
            is_codex_local_access_config_for_api_key(content, api_key)
                || is_cockpit_managed_local_access_config(content)
        })
        .unwrap_or(false)
    {
        if existing_backup.is_none() {
            logger::log_codex_api_warn(&format!(
                "Codex API 服务接管前发现目标目录已绑定运行时 provider，未把该状态保存为恢复备份: profile_dir={}",
                profile_key
            ));
        }
        return Ok(());
    }

    let auth_json = read_optional_profile_file(&profile_auth_path(profile_dir))?;
    let now = now_ms();
    match existing_backup {
        Some(existing) => {
            existing.auth_json = auth_json;
            existing.config_toml = config_toml;
            existing.updated_at = now;
        }
        None => backups
            .profiles
            .push(CodexLocalAccessProfileTakeoverBackup {
                profile_dir: profile_key,
                auth_json,
                config_toml,
                created_at: now,
                updated_at: now,
            }),
    }

    backups.version = CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION;
    save_takeover_backups(&backups)
}

fn cleanup_profile_takeover_artifacts(profile_dir: &Path) -> Result<bool, String> {
    let mut changed = false;
    for (file_name, label) in [
        (
            CODEX_LOCAL_ACCESS_AUTH_PROJECTION_FILE,
            "Codex API 服务账号投影",
        ),
        (
            CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE,
            "Codex API 服务模型目录",
        ),
        (CODEX_MODEL_CACHE_FILE, "Codex 模型缓存"),
    ] {
        let path = profile_dir.join(file_name);
        match std::fs::remove_file(&path) {
            Ok(()) => changed = true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "删除{}失败: path={}, error={}",
                    label,
                    path.display(),
                    error
                ))
            }
        }
    }
    Ok(changed)
}

fn restore_profile_takeover_backup(
    backup: &CodexLocalAccessProfileTakeoverBackup,
    api_key: &str,
    allow_rotated_managed_key: bool,
) -> Result<bool, String> {
    let profile_dir = PathBuf::from(&backup.profile_dir);
    let config_path = profile_config_path(&profile_dir);
    let auth_path = profile_auth_path(&profile_dir);
    let current_config = read_optional_profile_file(&config_path)?;
    let current_auth = read_optional_profile_file(&auth_path)?;
    let config_is_managed = current_config
        .as_deref()
        .map(|content| {
            is_codex_local_access_config_for_api_key(content, api_key)
                || (allow_rotated_managed_key && is_cockpit_managed_local_access_config(content))
        })
        .unwrap_or(false);
    let auth_is_managed = current_auth
        .as_deref()
        .map(|content| {
            is_exact_codex_local_access_auth_text(content, api_key)
                || (allow_rotated_managed_key && is_codex_local_access_auth_text(content, api_key))
        })
        .unwrap_or(false);

    if !config_is_managed && !auth_is_managed {
        return Ok(false);
    }

    let restored_config = restore_config_toml_from_takeover_backup(
        current_config.as_deref(),
        backup.config_toml.as_deref(),
    )?;
    write_optional_profile_file(&auth_path, backup.auth_json.as_deref())?;
    write_optional_profile_file(&config_path, restored_config.as_deref())?;
    let _ = cleanup_profile_takeover_artifacts(&profile_dir)?;
    Ok(true)
}

fn cleanup_profile_takeover_without_backup(
    profile_dir: &Path,
    api_key: &str,
    allow_rotated_managed_key: bool,
) -> Result<bool, String> {
    let config_path = profile_config_path(profile_dir);
    let auth_path = profile_auth_path(profile_dir);
    let mut changed = false;
    let mut managed = false;

    if let Some(config_text) = read_optional_profile_file(&config_path)? {
        if is_codex_local_access_config_for_api_key(&config_text, api_key)
            || (allow_rotated_managed_key && is_cockpit_managed_local_access_config(&config_text))
        {
            managed = true;
            let cleaned = remove_codex_local_access_config(&config_text)?;
            let cleaned_content = if cleaned.trim().is_empty() {
                None
            } else {
                Some(cleaned)
            };
            write_optional_profile_file(&config_path, cleaned_content.as_deref())?;
            changed = true;
        }
    }

    if let Some(auth_text) = read_optional_profile_file(&auth_path)? {
        if is_exact_codex_local_access_auth_text(&auth_text, api_key)
            || (allow_rotated_managed_key && is_codex_local_access_auth_text(&auth_text, api_key))
        {
            managed = true;
            write_optional_profile_file(&auth_path, None)?;
            changed = true;
        }
    }

    if managed && cleanup_profile_takeover_artifacts(profile_dir)? {
        changed = true;
    }

    Ok(changed)
}

fn restore_takeover_profiles_after_disable(
    collection: &CodexLocalAccessCollection,
) -> Result<(), String> {
    let backups = load_takeover_backups()?;
    let default_profile = codex_account::get_codex_home();
    let default_key = normalize_profile_dir_key(&default_profile);
    let protect_default_profile = account::is_dev_profile();
    let mut target_profiles = collect_local_access_profile_takeover_dirs()
        .into_iter()
        .map(|profile_dir| (normalize_profile_dir_key(&profile_dir), profile_dir))
        .collect::<HashMap<_, _>>();
    if !protect_default_profile {
        let default_is_exact_takeover =
            read_optional_profile_file(&profile_config_path(&default_profile))?
                .as_deref()
                .is_some_and(|content| {
                    is_codex_local_access_config_for_api_key(content, &collection.api_key)
                })
                || read_optional_profile_file(&profile_auth_path(&default_profile))?
                    .as_deref()
                    .is_some_and(|content| {
                        is_exact_codex_local_access_auth_text(content, &collection.api_key)
                    });
        if default_is_exact_takeover {
            target_profiles.insert(default_key.clone(), default_profile.clone());
        }
    }

    let mut restored_count = 0usize;
    let mut restored_profiles = HashSet::new();
    let mut remaining_backups = Vec::new();
    for backup in backups.profiles {
        if protect_default_profile && backup.profile_dir == default_key {
            remaining_backups.push(backup);
            continue;
        }
        if !target_profiles.contains_key(&backup.profile_dir) {
            remaining_backups.push(backup);
            continue;
        }
        if restore_profile_takeover_backup(&backup, &collection.api_key, true)? {
            restored_count += 1;
            restored_profiles.insert(backup.profile_dir);
        } else {
            remaining_backups.push(backup);
        }
    }

    save_takeover_backups(&CodexLocalAccessTakeoverBackups {
        version: CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION,
        profiles: remaining_backups,
    })?;

    let mut cleaned_without_backup = 0usize;
    for (profile_key, profile_dir) in target_profiles {
        if restored_profiles.contains(&profile_key) {
            continue;
        }
        if cleanup_profile_takeover_without_backup(&profile_dir, &collection.api_key, true)? {
            cleaned_without_backup += 1;
        }
    }

    if restored_count > 0 || cleaned_without_backup > 0 {
        logger::log_codex_api_info(&format!(
            "Codex API 服务停用后已恢复 Live 配置: restored_profiles={}, cleaned_without_backup={}",
            restored_count, cleaned_without_backup
        ));
    }

    Ok(())
}

