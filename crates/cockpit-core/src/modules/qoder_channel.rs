use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QoderChannel {
    QoderIde,
    QoderApp,
    QoderCnIde,
    QoderCnApp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QoderCredentialKind {
    StateVscdb, // VS Code SQLite ItemTable: secret://aicoding.auth.*
    AuthV1Dat,  // Electron 裸二进制 v10 信封 AES-256-GCM
}

impl QoderChannel {
    pub const ALL: [Self; 4] = [
        Self::QoderIde,
        Self::QoderApp,
        Self::QoderCnIde,
        Self::QoderCnApp,
    ];

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub fn as_str(self) -> &'static str {
        self.provider_key()
    }

    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        let normalized = raw
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("qoder")
            .to_ascii_lowercase()
            .replace('-', "_");

        match normalized.as_str() {
            "qoder" | "qoder_ide" => Ok(Self::QoderIde),
            "qoder_app" => Ok(Self::QoderApp),
            "qoder_cn_ide" => Ok(Self::QoderCnIde),
            "qoder_cn_app" => Ok(Self::QoderCnApp),
            other => Err(format!("不支持的 Qoder 渠道: {}", other)),
        }
    }

    pub fn provider_key(self) -> &'static str {
        match self {
            Self::QoderIde => "qoder",
            Self::QoderApp => "qoder_app",
            Self::QoderCnIde => "qoder_cn_ide",
            Self::QoderCnApp => "qoder_cn_app",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::QoderIde => "Qoder IDE",
            Self::QoderApp => "Qoder App",
            Self::QoderCnIde => "Qoder CN IDE",
            Self::QoderCnApp => "Qoder CN App",
        }
    }

    pub fn is_cn(self) -> bool {
        matches!(self, Self::QoderCnIde | Self::QoderCnApp)
    }

    pub fn openapi_base_url(self) -> &'static str {
        if self.is_cn() {
            "https://openapi.qoder.com.cn"
        } else {
            "https://openapi.qoder.sh"
        }
    }

    pub fn credential_kind(self) -> QoderCredentialKind {
        match self {
            Self::QoderIde | Self::QoderCnIde => QoderCredentialKind::StateVscdb,
            Self::QoderApp | Self::QoderCnApp => QoderCredentialKind::AuthV1Dat,
        }
    }

    pub fn accounts_index_filename(self) -> &'static str {
        match self {
            Self::QoderIde => "qoder_accounts.json",
            Self::QoderApp => "qoder_app_accounts.json",
            Self::QoderCnIde => "qoder_cn_ide_accounts.json",
            Self::QoderCnApp => "qoder_cn_app_accounts.json",
        }
    }

    pub fn accounts_dir_name(self) -> &'static str {
        match self {
            Self::QoderIde => "qoder_accounts",
            Self::QoderApp => "qoder_app_accounts",
            Self::QoderCnIde => "qoder_cn_ide_accounts",
            Self::QoderCnApp => "qoder_cn_app_accounts",
        }
    }

    pub fn instances_filename(self) -> &'static str {
        match self {
            Self::QoderIde => "qoder_instances.json",
            Self::QoderApp => "qoder_app_instances.json",
            Self::QoderCnIde => "qoder_cn_ide_instances.json",
            Self::QoderCnApp => "qoder_cn_app_instances.json",
        }
    }

    /// 专属的数据根目录名称（相对 APPDATA / Application Support / .config）
    pub fn app_support_dir_name(self) -> &'static str {
        match self {
            Self::QoderIde => "Qoder",
            Self::QoderApp => "com.qoder.app.stable",
            Self::QoderCnIde => "QoderCN",
            Self::QoderCnApp => "com.qodercn.app.stable",
        }
    }

    /// 默认数据目录全路径
    pub fn default_user_data_dir(self) -> Result<PathBuf, String> {
        #[cfg(target_os = "windows")]
        {
            let appdata = std::env::var("APPDATA")
                .map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
            let base = PathBuf::from(appdata);

            if self == Self::QoderCnIde {
                let primary = base.join("QoderCN");
                if primary.exists() {
                    return Ok(primary);
                }
                let alt = base.join("Qoder CN");
                if alt.exists() {
                    return Ok(alt);
                }
                return Ok(primary);
            }

            return Ok(base.join(self.app_support_dir_name()));
        }

        #[cfg(target_os = "macos")]
        {
            let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
            let support = home.join("Library/Application Support");
            return Ok(support.join(self.app_support_dir_name()));
        }

        #[cfg(target_os = "linux")]
        {
            let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
            let config = home.join(".config");
            return Ok(config.join(self.app_support_dir_name()));
        }

        #[allow(unreachable_code)]
        Err("Qoder 仅支持 Windows、macOS 和 Linux".to_string())
    }

    /// 进程探测和查杀时匹配的二进制文件名白名单
    pub fn expected_exe_names(self) -> &'static [&'static str] {
        match self {
            Self::QoderIde => &["Qoder IDE.exe", "Qoder.exe"],
            Self::QoderApp => &["Qoder.exe", "Qoder Launcher.exe"],
            Self::QoderCnIde => &["Qoder CN IDE.exe", "Qoder.exe"],
            Self::QoderCnApp => &["Qoder CN.exe", "Qoder Launcher.exe"],
        }
    }

    /// Windows 默认安装与执行文件候选列表
    pub fn default_exe_candidates(self) -> Vec<PathBuf> {
        let mut candidates = Vec::new();

        #[cfg(target_os = "windows")]
        {
            let local_appdata = std::env::var("LOCALAPPDATA").ok().map(PathBuf::from);
            let program_files = std::env::var("PROGRAMFILES").ok().map(PathBuf::from);
            let program_files_x86 = std::env::var("PROGRAMFILES(X86)").ok().map(PathBuf::from);
            let program_data = std::env::var("PROGRAMDATA").ok().map(PathBuf::from);

            match self {
                Self::QoderIde => {
                    // 1. Qoder IDE 独立通道
                    if let Some(ref pf) = program_files {
                        candidates.push(pf.join("Qoder IDE").join("Qoder IDE.exe"));
                    }
                    if let Some(ref la) = local_appdata {
                        candidates.push(la.join("Programs").join("Qoder IDE").join("Qoder IDE.exe"));
                    }
                    // 2. 原 Qoder (老通道)
                    if let Some(ref pf) = program_files {
                        candidates.push(pf.join("Qoder").join("Qoder.exe"));
                    }
                    if let Some(ref la) = local_appdata {
                        candidates.push(la.join("Programs").join("Qoder").join("Qoder.exe"));
                    }
                    if let Some(ref pfx86) = program_files_x86 {
                        candidates.push(pfx86.join("Qoder IDE").join("Qoder IDE.exe"));
                    }
                }
                Self::QoderApp => {
                    // 优先检查 install.ini 指向的主程序
                    if let Some(ref pd) = program_data {
                        let ini_path = pd.join("Qoder").join("Qoder Launcher").join("install.ini");
                        if let Some((dir, exe)) = parse_launcher_install_ini(&ini_path) {
                            candidates.push(PathBuf::from(dir).join(exe));
                        }
                    }
                    if let Some(ref pf) = program_files {
                        candidates.push(pf.join("Qoder").join("Qoder").join("Qoder.exe"));
                    }
                    if let Some(ref pd) = program_data {
                        candidates.push(pd.join("Qoder").join("Qoder Launcher").join("Qoder Launcher.exe"));
                    }
                }
                Self::QoderCnIde => {
                    if let Some(ref pf) = program_files {
                        candidates.push(pf.join("Qoder CN IDE").join("Qoder CN IDE.exe"));
                    }
                    if let Some(ref la) = local_appdata {
                        candidates.push(la.join("Programs").join("Qoder CN IDE").join("Qoder CN IDE.exe"));
                    }
                }
                Self::QoderCnApp => {
                    if let Some(ref pd) = program_data {
                        let ini_file = pd.join("Qoder CN").join("install.ini");
                        if let Some((dir, exe)) = parse_launcher_install_ini(&ini_file) {
                            candidates.push(PathBuf::from(dir).join(exe));
                        }
                    }
                    if let Some(ref pf) = program_files {
                        candidates.push(pf.join("Qoder CN").join("Qoder CN").join("Qoder CN.exe"));
                        candidates.push(pf.join("Qoder CN").join("Qoder CN.exe"));
                    }
                    if let Some(ref pd) = program_data {
                        candidates.push(pd.join("Qoder CN").join("Qoder Launcher.exe"));
                    }
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            match self {
                Self::QoderIde => {
                    candidates.push(PathBuf::from("/Applications/Qoder.app/Contents/MacOS/Qoder"));
                    candidates.push(PathBuf::from("/Applications/Qoder IDE.app/Contents/MacOS/Qoder IDE"));
                }
                Self::QoderCnIde => {
                    candidates.push(PathBuf::from("/Applications/Qoder CN IDE.app/Contents/MacOS/Qoder CN IDE"));
                }
                _ => {}
            }
        }

        #[cfg(target_os = "linux")]
        {
            match self {
                Self::QoderIde => {
                    candidates.push(PathBuf::from("/usr/bin/qoder"));
                    candidates.push(PathBuf::from("/usr/local/bin/qoder"));
                    candidates.push(PathBuf::from("/opt/qoder/qoder"));
                }
                _ => {}
            }
        }

        candidates
    }

    /// 探测当前渠道已安装的执行文件路径
    pub fn detect_installed_exe(self) -> Option<PathBuf> {
        for candidate in self.default_exe_candidates() {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }
}

/// 解析 Qoder Launcher 的 install.ini (支持 UTF-16 与 UTF-8)，提取 installDir 与 appExecutable
#[cfg(target_os = "windows")]
fn parse_launcher_install_ini(path: &Path) -> Option<(String, String)> {
    if !path.exists() {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let text = if bytes.len() >= 2 && (bytes[0] == 0xFF && bytes[1] == 0xFE) {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&u16s)
    } else if bytes.len() >= 2 && bytes[1] == 0x00 {
        let u16s: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&u16s)
    } else {
        String::from_utf8_lossy(&bytes).to_string()
    };

    let mut install_dir = None;
    let mut app_exe = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') || trimmed.is_empty() {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            if k.eq_ignore_ascii_case("installDir") {
                install_dir = Some(v.to_string());
            } else if k.eq_ignore_ascii_case("appExecutable") {
                app_exe = Some(v.to_string());
            }
        }
    }

    match (install_dir, app_exe) {
        (Some(d), Some(e)) if !d.is_empty() && !e.is_empty() => Some((d, e)),
        _ => None,
    }
}
