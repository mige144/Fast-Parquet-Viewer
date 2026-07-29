use std::process::Command;
use std::path::PathBuf;
use std::sync::mpsc;

/// 对比结果摘要（来自 Python stdout JSON）
#[derive(Debug, Clone)]
pub struct CompareSummary {
    pub total_rows: u64,
    pub diff_rows: u64,
    pub only_left: u64,
    pub only_right: u64,
    pub identical: u64,
    pub output_dir: String,
    pub key_column: String,
}

/// 对比操作结果
pub enum CompareResult {
    Ok(CompareSummary),
    Err(String),
}

/// 列名检测结果
pub enum ColumnDetectResult {
    Ok(Vec<String>),
    Err(String),
}

/// 对比对话框状态
pub enum CompareState {
    Hidden,
    Setup {
        left_path: String,
        right_path: String,
        key_column: String,
        python_path: String,
        show_all: bool,
        available_pythons: Vec<String>,
        available_columns: Vec<String>,
        column_detect_rx: Option<mpsc::Receiver<ColumnDetectResult>>,
    },
    Running {
        rx: mpsc::Receiver<CompareResult>,
        show_all: bool,
    },
}

impl CompareState {
    pub fn new_setup(available_pythons: Vec<String>, default_python: String) -> Self {
        CompareState::Setup {
            left_path: String::new(),
            right_path: String::new(),
            key_column: String::from("time"),
            python_path: default_python,
            show_all: false,
            available_pythons,
            available_columns: Vec::new(),
            column_detect_rx: None,
        }
    }
}

/// 扫描系统上可用的 Python 安装
pub fn detect_python_installations() -> Vec<String> {
    let mut pythons: Vec<String> = Vec::new();

    // 1. 检查系统 PATH 中的 py (Windows launcher), python3 和 python
    if Command::new("py").arg("--version").output().is_ok() {
        pythons.push("py".to_string());
    }
    for name in &["python3", "python"] {
        if Command::new(name).arg("--version").output().is_ok() {
            pythons.push(name.to_string());
        }
    }

    // 2. 扫描 conda 环境
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    if !home.is_empty() {
        // Miniconda / Anaconda envs
        for base in &["miniconda3", "anaconda3"] {
            let envs_dir = PathBuf::from(&home).join(base).join("envs");
            if let Ok(entries) = std::fs::read_dir(&envs_dir) {
                for entry in entries.flatten() {
                    let py = entry.path().join("python.exe");
                    if py.exists() {
                        pythons.push(py.to_string_lossy().to_string());
                    }
                }
            }
        }

        // .conda/environments.txt
        let envs_txt = PathBuf::from(&home).join(".conda").join("environments.txt");
        if let Ok(content) = std::fs::read_to_string(envs_txt) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let py = PathBuf::from(trimmed).join("python.exe");
                if py.exists() {
                    let s = py.to_string_lossy().to_string();
                    if !pythons.contains(&s) {
                        pythons.push(s);
                    }
                }
            }
        }
    }

    pythons
}

/// 生成对比输出目录路径（平台用户数据目录下的 .comparisons/）
pub fn make_output_dir() -> Option<String> {
    let data_dir = user_data_dir()?;
    let dir_name = timestamp_dir_name();
    let output = data_dir.join(".comparisons").join(&dir_name);
    Some(output.to_string_lossy().to_string())
}

fn user_data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var("LOCALAPPDATA").ok().map(|d| PathBuf::from(d).join("FastParquetViewer"))
    } else {
        let home = std::env::var("HOME").ok()?;
        if cfg!(target_os = "macos") {
            Some(PathBuf::from(&home).join("Library/Application Support/FastParquetViewer"))
        } else {
            Some(PathBuf::from(&home).join(".local/share/FastParquetViewer"))
        }
    }
}

fn timestamp_dir_name() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let (y, m, d, hh, mm, ss) = unix_to_ymdhms(secs);
    format!("{y:04}{m:02}{d:02}_{hh:02}{mm:02}{ss:02}")
}

fn unix_to_ymdhms(ts: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = ts / 86400;
    let secs = ts % 86400;
    let hh = secs / 3600;
    let mm = (secs % 3600) / 60;
    let ss = secs % 60;

    let mut y = 1970i32;
    let mut remaining = days as i64;
    loop {
        let year_days = if is_leap(y) { 366 } else { 365 };
        if remaining < year_days {
            break;
        }
        remaining -= year_days;
        y += 1;
    }
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 1u32;
    for &md in &month_days {
        if remaining < md as i64 {
            break;
        }
        remaining -= md as i64;
        m += 1;
    }
    let d = remaining as u32 + 1;
    (y, m, d, hh as u32, mm as u32, ss as u32)
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

/// 启动异步对比（在后台线程中运行 Python）
pub fn run_compare_async(
    python_path: String,
    left_path: String,
    right_path: String,
    key_column: String,
    output_dir: String,
) -> mpsc::Receiver<CompareResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let script = exe_dir
            .as_ref()
            .map(|d| d.join("compare_files.py"))
            .unwrap_or_else(|| PathBuf::from("compare_files.py"));

        let mut cmd = Command::new(&python_path);
        cmd.arg(script.to_string_lossy().as_ref())
            .arg("--left")
            .arg(&left_path)
            .arg("--right")
            .arg(&right_path)
            .arg("--output")
            .arg(&output_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if !key_column.is_empty() {
            cmd.arg("--key").arg(&key_column);
        }

        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                let _ = tx.send(CompareResult::Err(format!("启动 Python 失败: {e}")));
                return;
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = tx.send(CompareResult::Err(format!("对比失败:\n{stderr}")));
            return;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().last().unwrap_or("");

        match serde_json::from_str::<serde_json::Value>(last_line) {
            Ok(json) => {
                if json.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                    let _ = tx.send(CompareResult::Ok(CompareSummary {
                        total_rows: json["total_rows"].as_u64().unwrap_or(0),
                        diff_rows: json["diff_rows"].as_u64().unwrap_or(0),
                        only_left: json["only_left"].as_u64().unwrap_or(0),
                        only_right: json["only_right"].as_u64().unwrap_or(0),
                        identical: json["identical"].as_u64().unwrap_or(0),
                        output_dir: json["output_dir"].as_str().unwrap_or("").to_string(),
                        key_column: json["key_column"].as_str().unwrap_or("").to_string(),
                    }));
                } else {
                    let err = json["error"].as_str().unwrap_or("未知错误");
                    let _ = tx.send(CompareResult::Err(err.to_string()));
                }
            }
            Err(_) => {
                let _ = tx.send(CompareResult::Err("无法解析 Python 输出".to_string()));
            }
        }
    });

    rx
}

/// 检测两个文件的共有列（异步，后台线程运行 Python --mode columns）
pub fn detect_columns_async(
    python_path: String,
    left_path: String,
    right_path: String,
) -> mpsc::Receiver<ColumnDetectResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let script = exe_dir
            .as_ref()
            .map(|d| d.join("compare_files.py"))
            .unwrap_or_else(|| PathBuf::from("compare_files.py"));

        let output = match Command::new(&python_path)
            .arg(script.to_string_lossy().as_ref())
            .arg("--mode").arg("columns")
            .arg("--left").arg(&left_path)
            .arg("--right").arg(&right_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                let _ = tx.send(ColumnDetectResult::Err(format!("{e}")));
                return;
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = tx.send(ColumnDetectResult::Err(stderr.to_string()));
            return;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().last().unwrap_or("");

        match serde_json::from_str::<serde_json::Value>(last_line) {
            Ok(json) => {
                if let Some(cols) = json.get("columns").and_then(|v| v.as_array()) {
                    let columns: Vec<String> = cols
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                    let _ = tx.send(ColumnDetectResult::Ok(columns));
                } else {
                    let err = json["error"].as_str().unwrap_or("未知错误");
                    let _ = tx.send(ColumnDetectResult::Err(err.to_string()));
                }
            }
            Err(_) => {
                let _ = tx.send(ColumnDetectResult::Err("无法解析列名输出".to_string()));
            }
        }
    });

    rx
}

/// 根据可用列自动选择默认 key 列：time > tradeDay > 第一个
pub fn default_key_column(columns: &[String]) -> String {
    for candidate in &["time", "tradeDay"] {
        if columns.iter().any(|c| c == candidate) {
            return candidate.to_string();
        }
    }
    columns.first().cloned().unwrap_or_default()
}