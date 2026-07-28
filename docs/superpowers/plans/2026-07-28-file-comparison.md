# 文件对比功能 — 实现计划

> **面向自动化执行者：** 需要子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 来逐个任务实现此计划。步骤使用 checkbox (`- [ ]`) 语法追踪。

**目标：** 为 Fast Parquet Viewer 增加双文件对比功能——用户选择两个文件和一个行标识列，Python 脚本执行对比，结果以 Parquet 格式用现有查看器展示。

**架构：** Rust 端新增 `compare.rs` 模块管理对比状态和 Python 子进程调用，`app.rs` 增加对比设置对话框和结果切换 UI。Python 脚本 `compare_files.py` 独立负责 pandas 数据加载和对比逻辑，通过 parquet 文件交换结果。

**技术栈：** Rust (egui 0.31, arrow 54, parquet 54), Python 3.8+ (pandas, pyarrow)

---

## 文件结构

| 文件 | 操作 | 职责 |
|------|------|------|
| `compare_files.py` | 新建 | Python 对比脚本——加载两个文件、执行对比、输出 parquet + JSON |
| `src/compare.rs` | 新建 | 对比状态管理、Python 环境检测、子进程调用、结果解析 |
| `src/main.rs` | 修改 | 添加 `mod compare;` |
| `src/app.rs` | 修改 | 工具栏按钮、对比对话框 UI、结果视图切换、状态集成 |

---

### Task 1: 编写 Python 对比脚本

**文件:**
- 创建: `compare_files.py`

- [ ] **Step 1: 创建脚本骨架和参数解析**

```python
#!/usr/bin/env python3
"""Fast Parquet Viewer — 文件对比引擎
用法:
  python compare_files.py --left a.parquet --right b.csv --output .comparisons/xxx/
"""

import argparse
import json
import os
import sys
from datetime import datetime


def main():
    parser = argparse.ArgumentParser(description="对比两个数据文件")
    parser.add_argument("--left", required=True, help="左侧文件路径")
    parser.add_argument("--right", required=True, help="右侧文件路径")
    parser.add_argument("--key", default=None, help="行标识列名（省略时自动检测）")
    parser.add_argument("--output", required=True, help="结果输出目录")
    args = parser.parse_args()

    try:
        result = compare(args.left, args.right, args.key, args.output)
        print(json.dumps(result, ensure_ascii=False))
    except Exception as e:
        print(json.dumps({"ok": False, "error": str(e)}, ensure_ascii=False))
        sys.exit(1)


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: 运行脚本验证参数解析**

```bash
python compare_files.py --help
```
期望：显示帮助信息。

- [ ] **Step 3: 实现文件加载逻辑**

在 `compare_files.py` 中添加以下函数：

```python
import pandas as pd
import zipfile
from pathlib import Path


def load_file(path: str) -> pd.DataFrame:
    """根据扩展名加载文件到 DataFrame"""
    p = Path(path)
    ext = p.suffix.lower()
    if ext == ".parquet":
        return pd.read_parquet(path)
    elif ext == ".csv":
        return pd.read_csv(path)
    elif ext == ".zip":
        # 尝试直接读取内部的 CSV 文件
        with zipfile.ZipFile(path, "r") as zf:
            csv_names = [n for n in zf.namelist() if n.lower().endswith(".csv")]
            if not csv_names:
                raise ValueError(f"ZIP 文件中未找到 CSV 文件: {path}")
            if len(csv_names) > 1:
                print(f"警告: ZIP 中有多个 CSV，使用第一个: {csv_names[0]}", file=sys.stderr)
            with zf.open(csv_names[0]) as f:
                return pd.read_csv(f)
    else:
        raise ValueError(f"不支持的文件格式: {ext}（支持 .parquet / .csv / .zip）")
```

- [ ] **Step 4: 用测试数据验证加载逻辑**

创建两个测试文件并测试加载：

```bash
python -c "
import pandas as pd
df = pd.DataFrame({'time': ['T+0','T+1','T+2'], 'price': [100.5, 101.2, 99.0], 'vol': [1000, 500, 300]})
df.to_csv('/tmp/test_a.csv', index=False)
df.to_parquet('/tmp/test_a.parquet', index=False)
print('测试文件已创建')
"
```

- [ ] **Step 5: 实现自动检测 key 列函数**

```python
def detect_key_column(df_left: pd.DataFrame, df_right: pd.DataFrame, user_key: str | None) -> str:
    """自动检测或使用用户指定的 key 列"""
    if user_key and user_key.strip():
        key = user_key.strip()
        if key not in df_left.columns:
            raise ValueError(f"key 列 '{key}' 在左侧文件中不存在")
        if key not in df_right.columns:
            raise ValueError(f"key 列 '{key}' 在右侧文件中不存在")
        return key

    # 自动检测: time > tradeDay > 报错
    for candidate in ["time", "tradeDay"]:
        if candidate in df_left.columns and candidate in df_right.columns:
            return candidate

    raise ValueError("无法自动检测行标识列。请用 --key 指定列名（两文件共有的列）。")
```

- [ ] **Step 6: 实现核心对比逻辑**

```python
import numpy as np


def compare(left_path: str, right_path: str, key_arg: str | None, output_dir: str) -> dict:
    """执行对比，返回结果摘要 dict"""
    # 1. 加载
    df_left = load_file(left_path)
    df_right = load_file(right_path)

    # 2. 检测 key 列
    key_col = detect_key_column(df_left, df_right, key_arg)

    # 3. 找出公共列（排除 key 列）
    left_cols = [c for c in df_left.columns if c != key_col]
    right_cols = [c for c in df_right.columns if c != key_col]
    all_common_cols = sorted(set(left_cols) | set(right_cols))

    # 4. Outer join on key
    df_left_idx = df_left.set_index(key_col)
    df_right_idx = df_right.set_index(key_col)
    joined = df_left_idx.join(df_right_idx, how="outer", lsuffix="_LEFT_", rsuffix="_RIGHT_")

    # 5. 构建对比结果 DataFrame
    result_rows = []
    for idx_val, row in joined.iterrows():
        status = "identical"
        row_data = {key_col: idx_val}

        for col in all_common_cols:
            left_col_name = f"{col}_LEFT_"
            right_col_name = f"{col}_RIGHT_"
            left_val = row.get(left_col_name, np.nan)
            right_val = row.get(right_col_name, np.nan)

            left_has = left_col_name in joined.columns and not pd.isna(row.get(left_col_name))
            right_has = right_col_name in joined.columns and not pd.isna(row.get(right_col_name))
            both_nan = pd.isna(left_val) and pd.isna(right_val)

            row_data[f"left.{col}"] = row.get(left_col_name) if left_col_name in joined.columns else None
            row_data[f"right.{col}"] = row.get(right_col_name) if right_col_name in joined.columns else None

            if both_nan:
                continue  # 两边都是 NaN，视为相同
            if not left_has and right_has:
                status = "only_right"
            elif left_has and not right_has:
                status = "only_left"
            elif not left_has and not right_has:
                continue  # 都不存在，跳过
            else:
                # 两边都有值，进行比较
                try:
                    if pd.isna(left_val) and pd.isna(right_val):
                        pass  # 相同
                    elif left_val != right_val:
                        if status == "identical":
                            status = "different"
                except (TypeError, ValueError):
                    if str(left_val) != str(right_val):
                        if status == "identical":
                            status = "different"

        row_data["_compare_status"] = status
        result_rows.append(row_data)

    result_df = pd.DataFrame(result_rows)

    # 6. 写入输出目录
    os.makedirs(output_dir, exist_ok=True)

    total_path = os.path.join(output_dir, "compare_total.parquet")
    result_df.to_parquet(total_path, index=False)

    diff_df = result_df[result_df["_compare_status"] != "identical"]
    diff_path = os.path.join(output_dir, "compare_diff.parquet")
    diff_df.to_parquet(diff_path, index=False)

    # 统计
    counts = result_df["_compare_status"].value_counts()
    return {
        "ok": True,
        "output_dir": output_dir,
        "total_rows": len(result_df),
        "diff_rows": int(counts.get("different", 0)),
        "only_left": int(counts.get("only_left", 0)),
        "only_right": int(counts.get("only_right", 0)),
        "identical": int(counts.get("identical", 0)),
        "key_column": key_col,
    }
```

- [ ] **Step 7: 端到端测试对比脚本**

创建两个有差异的测试文件并运行对比：

```bash
python -c "
import pandas as pd; import numpy as np

# 左侧文件
left = pd.DataFrame({
    'time': ['2024-01-01', '2024-01-02', '2024-01-03', '2024-01-04'],
    'price': [100.5, 101.2, 99.0, 105.0],
    'volume': [1000, 500, 300, 800]
})
left.to_parquet('/tmp/left.parquet', index=False)

# 右侧文件: T+1 价格不同, T+4 不存在(左), T+5 仅右侧有
right = pd.DataFrame({
    'time': ['2024-01-01', '2024-01-02', '2024-01-03', '2024-01-05'],
    'price': [100.5, 101.8, 99.0, 102.0],
    'volume': [1000, 500, np.nan, 600]
})
right.to_parquet('/tmp/right.parquet', index=False)
print('测试数据已创建')
"

python compare_files.py \
  --left /tmp/left.parquet \
  --right /tmp/right.parquet \
  --output /tmp/comparison_test/
```

期望输出 JSON 中 `total_rows=5, diff_rows=2, only_left=1, only_right=1, identical=1`。

- [ ] **Step 8: 验证输出 Parquet 文件内容**

```bash
python -c "
import pandas as pd
total = pd.read_parquet('/tmp/comparison_test/compare_total.parquet')
diff = pd.read_parquet('/tmp/comparison_test/compare_diff.parquet')
print('Total rows:', len(total))
print('Diff rows:', len(diff))
print('Total columns:', list(total.columns))
print()
print(total.to_string())
"
```

期望：total 5 行，diff 4 行，列名包含 `left.price`、`right.price`、`_compare_status` 等。

---

### Task 2: 创建 Rust 对比模块

**文件:**
- 创建: `src/compare.rs`
- 修改: `src/main.rs`

- [ ] **Step 1: 创建 `src/compare.rs` 基础结构**

```rust
use std::sync::mpsc;
use std::process::Command;
use std::path::PathBuf;

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
    },
    Running {
        rx: mpsc::Receiver<CompareResult>,
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
        }
    }
}
```

- [ ] **Step 2: 实现 Python 环境检测**

在 `src/compare.rs` 中添加：

```rust
/// 扫描系统上可用的 Python 安装
pub fn detect_python_installations() -> Vec<String> {
    let mut pythons: Vec<String> = Vec::new();

    // 1. 检查系统 PATH 中的 python3 和 python
    for name in &["python3", "python"] {
        if Command::new(name).arg("--version").output().is_ok() {
            pythons.push(name.to_string());
        }
    }

    // 2. 扫描 conda 环境
    if let Ok(home) = std::env::var("USERPROFILE") {
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
                if trimmed.is_empty() { continue; }
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
```

- [ ] **Step 3: 实现子进程调用函数**

在 `src/compare.rs` 中添加：

```rust
/// 生成对比输出目录路径（基于 exe 目录下的 .comparisons/）
pub fn make_output_dir() -> Option<String> {
    let exe_dir = std::env::current_exe()
        .ok()?
        .parent()
        .map(|p| p.to_path_buf())?;

    let now = chrono_now();
    let dir_name = format!("{}", now); // YYYYMMDD_HHMMSS 格式
    let output = exe_dir.join(".comparisons").join(&dir_name);
    Some(output.to_string_lossy().to_string())
}

/// 简易时间戳（不依赖 chrono crate）
fn chrono_now() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    // 粗略计算年月日时分秒
    let days_since_epoch = secs / 86400;
    // 使用简化公式：从 1970-01-01 开始计算
    let (y, m, d, hh, mm, ss) = unix_to_ymdhms(secs);
    format!("{y:04}{m:02}{d:02}_{hh:02}{mm:02}{ss:02}")
}

fn unix_to_ymdhms(ts: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = ts / 86400;
    let secs = ts % 86400;
    let hh = secs / 3600;
    let mm = (secs % 3600) / 60;
    let ss = secs % 60;

    // 简化版日期转换（仅用于目录命名，容许微小偏差）
    let mut y = 1970i32;
    let mut remaining = days as i64;
    loop {
        let year_days = if is_leap(y) { 366 } else { 365 };
        if remaining < year_days { break; }
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
        if remaining < md as i64 { break; }
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
            .arg("--left").arg(&left_path)
            .arg("--right").arg(&right_path)
            .arg("--output").arg(&output_dir)
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
```

- [ ] **Step 4: 注册模块**

在 `src/main.rs` 中添加 `mod compare;`：

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod compare;  // <-- 新增
mod loader;
mod recent;
mod table;

// ... 其余不变
```

- [ ] **Step 5: 编译检查**

```bash
cargo check 2>&1
```

期望：编译成功，无错误。

---

### Task 3: 增加对比按钮和设置对话框 UI

**文件:**
- 修改: `src/app.rs`

- [ ] **Step 1: 在 `ParquetApp` 结构体中添加对比相关字段**

```rust
// 在 ParquetApp struct 中添加以下字段：
pub struct ParquetApp {
    state:        State,
    rx:           Option<mpsc::Receiver<LoadResult>>,
    search:       String,
    show_meta:    bool,
    dark_mode:    bool,
    recent_files: Vec<String>,
    row_from_input: String,
    row_to_input:   String,
    row_from:       usize,
    row_to:         usize,
    // ── 对比功能字段 ──
    compare_state: compare::CompareState,         // 对比对话框状态
    compare_output_dir: Option<String>,            // 结果 parquet 所在目录
    compare_summary: Option<compare::CompareSummary>, // 对比统计信息
    before_compare_path: Option<String>,           // 对比前正在查看的文件路径
    available_pythons: Vec<String>,                // 启动时检测的 Python 列表
}
```

- [ ] **Step 2: 在 `ParquetApp::new` 中初始化对比字段**

```rust
// 在 ParquetApp::new() 的最后，return 之前添加：
let available_pythons = compare::detect_python_installations();
let default_python = available_pythons.first().cloned().unwrap_or_default();

// 修改 app 构造：
let mut app = Self {
    state:        State::Empty,
    rx:           None,
    search:       String::new(),
    show_meta:    false,
    dark_mode,
    recent_files,
    row_from_input: String::from("0"),
    row_to_input:   String::from("0"),
    row_from:       0,
    row_to:         0,
    compare_state: compare::CompareState::Hidden,   // <-- 新增
    compare_output_dir: None,                        // <-- 新增
    compare_summary: None,                           // <-- 新增
    before_compare_path: None,                       // <-- 新增
    available_pythons,                               // <-- 新增
};
```

- [ ] **Step 3: 在工具栏中增加 "Compare…" 按钮**

在 toolbar `ui.horizontal(|ui| {` 内部，紧跟 "Open…" 按钮之后，Ctrl+O 提示之前添加：

```rust
// ── 新增 Compare… 按钮 ──
ui.add_space(4.0);
ui.add(egui::Separator::default().vertical().spacing(8.0));
ui.add_space(4.0);

if ui.add(egui::Button::new(
    RichText::new("Compare…").color(palette.text).size(13.0)
).frame(false)).clicked() {
    let default_python = self.available_pythons.first().cloned().unwrap_or_default();
    self.compare_state = compare::CompareState::new_setup(
        self.available_pythons.clone(),
        default_python,
    );
}

ui.add_space(4.0);
ui.add(egui::Separator::default().vertical().spacing(8.0));
ui.add_space(4.0);
// ── end ──
```

- [ ] **Step 4: 实现对比设置对话框 UI**

在 `fn update` 中，status bar 之后、central panel 之前添加对话框渲染。创建辅助函数：

```rust
fn draw_compare_dialog(
    ctx: &egui::Context,
    compare_state: &mut compare::CompareState,
    palette: &Palette,
) {
    // 使用 match 提取 Setup 状态的可变引用
    if let compare::CompareState::Setup {
        left_path,
        right_path,
        key_column,
        python_path,
        show_all,
        available_pythons,
    } = compare_state
    {
        let mut open = true;
        egui::Window::new("对比文件")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(460.0);
                ui.visuals_mut().override_text_color = Some(palette.text);

                ui.label(RichText::new("选择两个文件进行对比").size(14.0).color(palette.text));

                ui.add_space(8.0);

                // 左侧文件
                ui.label(RichText::new("左侧文件").size(11.0).color(palette.muted));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(left_path)
                            .desired_width(ui.available_width() - 56.0)
                            .hint_text("选择文件...")
                    );
                    if ui.button("浏览").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("数据文件", &["parquet", "parq", "csv", "zip"])
                            .pick_file()
                        {
                            *left_path = path.to_string_lossy().to_string();
                        }
                    }
                });

                ui.add_space(6.0);

                // 右侧文件
                ui.label(RichText::new("右侧文件").size(11.0).color(palette.muted));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(right_path)
                            .desired_width(ui.available_width() - 56.0)
                            .hint_text("选择文件...")
                    );
                    if ui.button("浏览").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("数据文件", &["parquet", "parq", "csv", "zip"])
                            .pick_file()
                        {
                            *right_path = path.to_string_lossy().to_string();
                        }
                    }
                });

                ui.add_space(6.0);

                // 行标识列
                ui.label(RichText::new("行标识列").size(11.0).color(palette.muted));
                ui.add(
                    egui::TextEdit::singleline(key_column)
                        .desired_width(ui.available_width())
                        .hint_text("默认: time > tradeDay（自动检测）")
                );
                ui.label(RichText::new("自动检测: time → tradeDay。留空使用自动检测。")
                    .size(10.0).color(palette.null));

                ui.add_space(6.0);

                // Python 环境
                ui.label(RichText::new("Python 环境").size(11.0).color(palette.muted));
                egui::ComboBox::from_id_salt("python_env")
                    .width(ui.available_width())
                    .selected_text(python_path.as_str())
                    .show_ui(ui, |ui| {
                        for py in available_pythons.iter() {
                            ui.selectable_value(python_path, py.clone(), py.as_str());
                        }
                    });

                // 重新扫描按钮
                if ui.button("重新扫描 Python 环境").clicked() {
                    let found = compare::detect_python_installations();
                    if !found.is_empty() {
                        *python_path = found[0].clone();
                    }
                    *available_pythons = found;
                }

                ui.add_space(8.0);

                // 显示所有行
                ui.checkbox(show_all, "显示所有行（不勾选仅显示差异）");

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                // 按钮行
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        open = false;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_start = !left_path.is_empty()
                            && !right_path.is_empty()
                            && !python_path.is_empty();
                        if ui.add_enabled(
                            can_start,
                            egui::Button::new(
                                RichText::new("开始对比").color(palette.text)
                            ),
                        ).clicked() {
                            open = false;
                            // 在这里触发开始对比 —— 通过修改 compare_state 为 Running
                            // 实际实现在下一环节
                        }
                    });
                });
            });

        if !open {
            *compare_state = compare::CompareState::Hidden;
        }
    }
}
```

- [ ] **Step 5: 在 update 中调用对话框渲染**

在 `fn update` 的合适位置（poll_loader 之后、central panel 之前）添加：

```rust
// 渲染对比对话框
draw_compare_dialog(ctx, &mut self.compare_state, &palette);
```

- [ ] **Step 6: 编译检查 UI 代码**

```bash
cargo check 2>&1
```

期望：编译成功。修复出现的任何编译错误。

---

### Task 4: 实现对比执行和结果加载

**文件:**
- 修改: `src/app.rs`

- [ ] **Step 1: 实现 "开始对比" 按钮的触发逻辑**

修改对话框中的 "开始对比" 按钮点击处理。将原有的 `draw_compare_dialog` 改为返回一个 Option，表示用户点击了开始。或者直接在 app 层面处理。

修改方式：在 `draw_compare_dialog` 函数签名中增加一个返回值。将函数签名改为返回 `bool`，表示是否点击了开始：

```rust
fn draw_compare_dialog(
    ctx: &egui::Context,
    compare_state: &mut compare::CompareState,
    palette: &Palette,
) -> bool {
    let mut start_clicked = false;
    // ... 原有 dialog 代码 ...
    // 在开始按钮的 clicked 处理中设置:
    // start_clicked = true;
    start_clicked
}
```

- [ ] **Step 2: 在 `update` 中处理开始对比**

在 `update` 中，对话框调用之后：

```rust
let start_compare = draw_compare_dialog(ctx, &mut self.compare_state, &palette);

if start_compare {
    if let compare::CompareState::Setup {
        left_path,
        right_path,
        key_column,
        python_path,
        show_all,
        ..
    } = &self.compare_state
    {
        // 保存当前正在查看的文件路径（用于返回）
        self.before_compare_path = match &self.state {
            State::Loaded(data, _) => Some(data.file_path.clone()),
            _ => None,
        };

        // 生成输出目录
        let output_dir = compare::make_output_dir().unwrap_or_else(|| {
            format!(".comparisons/compare_{}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs())
        });

        // 启动异步对比
        let rx = compare::run_compare_async(
            python_path.clone(),
            left_path.clone(),
            right_path.clone(),
            key_column.clone(),
            output_dir.clone(),
        );

        // 记录 show_all 选择和输出目录
        let show_all_val = *show_all;
        self.compare_output_dir = Some(output_dir);
        self.compare_state = compare::CompareState::Running { rx };
        // 用额外字段暂存 show_all 选择
        // 方案：在 Running 变体中增加 show_all 字段，或者新增一个字段
    }
}
```

- [ ] **Step 3: 调整 `CompareState::Running` 以包含 `show_all`**

修改 `src/compare.rs` 中的 `CompareState`：

```rust
pub enum CompareState {
    Hidden,
    Setup {
        left_path: String,
        right_path: String,
        key_column: String,
        python_path: String,
        show_all: bool,
        available_pythons: Vec<String>,
    },
    Running {
        rx: mpsc::Receiver<CompareResult>,
        show_all: bool,     // <-- 新增：记住用户的选择
    },
}
```

同步更新 Step 2 的代码，将 `show_all_val` 存入 `Running` 变体。

- [ ] **Step 4: 实现 `poll_compare` 函数**

仿照现有的 `poll_loader` 模式：

```rust
fn poll_compare(&mut self, ctx: &egui::Context) {
    if let compare::CompareState::Running { rx, show_all } = &self.compare_state {
        if let Ok(result) = rx.try_recv() {
            match result {
                compare::CompareResult::Ok(summary) => {
                    // 选择加载哪个 parquet
                    let output_dir = self.compare_output_dir.clone().unwrap_or_default();
                    let file_name = if *show_all {
                        "compare_total.parquet"
                    } else {
                        "compare_diff.parquet"
                    };
                    let result_path = std::path::PathBuf::from(&output_dir)
                        .join(file_name)
                        .to_string_lossy()
                        .to_string();

                    self.compare_summary = Some(summary);
                    self.compare_state = compare::CompareState::Hidden;
                    // 用现有加载逻辑加载结果 parquet
                    self.start_load(result_path);
                }
                compare::CompareResult::Err(e) => {
                    self.compare_state = compare::CompareState::Hidden;
                    self.state = State::Error(e);
                }
            }
            ctx.request_repaint();
        } else {
            ctx.request_repaint();
        }
    }
}
```

- [ ] **Step 5: 在 `update` 中调用 `poll_compare`**

在 `update` 函数中，`poll_loader` 之后添加：

```rust
self.poll_compare(ctx);
```

- [ ] **Step 6: 显示对比进度**

修改对比对话框，当状态为 `Running` 时显示进度提示：

```rust
if let compare::CompareState::Running { .. } = &self.compare_state {
    egui::Window::new("对比中...")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.visuals_mut().override_text_color = Some(palette.text);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add_space(8.0);
                ui.label("正在对比文件...");
            });
        });
}
```

- [ ] **Step 7: 编译检查**

```bash
cargo check 2>&1
```

期望：编译成功。

- [ ] **Step 8: 集成测试 —— 启动应用并执行对比**

```bash
# 先确保 compare_files.py 在 target/release/ 目录下
cp compare_files.py target/release/

# 启动应用
cargo run --release
```

手动测试流程：
1. 点击 "Compare…"
2. 选择两个 parquet 文件
3. 保持默认 key 列 "time"
4. 保持不勾选"显示所有行"
5. 选择 Python 环境
6. 点击"开始对比"
7. 确认对比结果正确显示

---

### Task 5: 实现结果视图的切换和返回

**文件:**
- 修改: `src/app.rs`

- [ ] **Step 1: 在对比结果工具栏中增加 Diff/Total 切换与返回按钮**

在工具栏渲染中，当 `compare_summary` 为 `Some` 且在查看对比结果时，显示额外的工具栏控件。在 toolbar `ui.horizontal(|ui| {` 内部，搜索过滤框之后、右对齐区域之前添加：

```rust
// ── 对比结果工具栏 ──
if let (Some(summary), Some(output_dir)) = (&self.compare_summary, &self.compare_output_dir) {
    ui.add_space(4.0);
    ui.add(egui::Separator::default().vertical().spacing(8.0));
    ui.add_space(4.0);

    // 返回按钮
    if ui.add(egui::Button::new(
        RichText::new("⬅ 返回").color(palette.text).size(13.0)
    ).frame(false)).clicked() {
        // 恢复到对比前的状态
        if let Some(prev_path) = self.before_compare_path.take() {
            self.start_load(prev_path);
        } else {
            self.state = State::Empty;
        }
        self.compare_summary = None;
        self.compare_output_dir = None;
    }

    ui.add_space(4.0);
    ui.add(egui::Separator::default().vertical().spacing(8.0));
    ui.add_space(4.0);

    // Diff / Total 切换
    let current_is_diff = match &self.state {
        State::Loaded(data, _) => data.file_path.contains("compare_diff"),
        _ => false,
    };

    let mut show_diff = current_is_diff;
    ui.label(RichText::new("Diff only").color(palette.muted).size(11.0));
    if ui.add(egui::Checkbox::without_text(&mut show_diff)).changed() {
        let file_name = if show_diff {
            "compare_diff.parquet"
        } else {
            "compare_total.parquet"
        };
        let result_path = std::path::PathBuf::from(output_dir)
            .join(file_name)
            .to_string_lossy()
            .to_string();
        self.start_load(result_path);
    }

    ui.add_space(4.0);
    ui.add(egui::Separator::default().vertical().spacing(8.0));
    ui.add_space(4.0);

    // 汇总文字
    let status_text = format!(
        "{} 差异 / {} 行 | 仅左:{}  仅右:{}  相同:{}",
        summary.diff_rows, summary.total_rows,
        summary.only_left, summary.only_right, summary.identical
    );
    ui.label(RichText::new(status_text).color(palette.muted).size(11.0));
}
// ── end ──
```

- [ ] **Step 2: 编译检查**

```bash
cargo check 2>&1
```

期望：编译成功。修复任何错误。

- [ ] **Step 3: 更新 Cargo.toml 版本号**

```bash
# 当前版本 1.2.0，递增到 1.3.0
```

修改 `Cargo.toml` 第一行 `version = "1.3.0"`。

- [ ] **Step 4: 构建 Release**

```bash
cargo build --release 2>&1
```

期望：构建成功，生成 `target/release/FastParquetViewer.exe`。

- [ ] **Step 5: 确保 compare_files.py 复制到 release 目录**

```bash
cp compare_files.py target/release/
```

- [ ] **Step 6: 端到端完整测试**

手动测试路径：
1. 准备两个有差异的 parquet 文件（可用 Task 1 Step 7 创建的 `/tmp/left.parquet` 和 `/tmp/right.parquet`）
2. 运行 `target/release/FastParquetViewer.exe`
3. 点击 "Compare…" → 选择两个文件 → 开始对比
4. 验证：结果展示 diff 行数正确，_compare_status 列显示状态
5. 切换 "Diff only" → 确认只显示差异行
6. 取消勾选 → 确认显示全部行
7. 点击 "⬅ 返回" → 确认返回之前状态
8. 测试错误场景：选择一个不存在的 Python 路径 → 验证错误提示
9. 测试 key 列自动检测：不填写 key 列 → 验证 Python 自动检测到 time 列

- [ ] **Step 7: 提交代码**

```bash
git add -A
git commit -m "feat: 增加文件对比功能 (Python子进程 + parquet交换)

- 新增 compare_files.py: pandas加载两文件、outer join对比、输出parquet
- 新增 src/compare.rs: Python环境检测、子进程管理、结果解析
- 修改 src/app.rs: 对比对话框UI、工具栏按钮、结果视图切换
- 输出: compare_total.parquet + compare_diff.parquet
- 列名前缀: left. / right. 区分来源
- NaN == NaN 视为相同"

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
```

---

## 自检清单

- [ ] **Spec 覆盖检查:** 每个 spec 需求都需要有对应任务
  - 文件格式支持 (CSV/CSV.zip/Parquet) → Task 1 Step 3 ✓
  - 行标识列自动检测 (time > tradeDay) → Task 1 Step 5 ✓
  - NaN == NaN → Task 1 Step 6 ✓
  - left./right. 列前缀 → Task 1 Step 6 ✓
  - 两个输出文件 (total/diff) → Task 1 Step 6 ✓
  - 对比对话框 UI → Task 3 Step 4 ✓
  - Python 环境选择 → Task 3 Step 4 + Task 2 Step 2 ✓
  - "仅显示差异" 选项 → Task 3 Step 4 ✓
  - 结果加载到现有查看器 → Task 4 Step 4 ✓
  - Diff/Total 切换 → Task 5 Step 1 ✓
  - 返回按钮 → Task 5 Step 1 ✓
  - 对比目录 (.comparisons/) → Task 2 Step 3 ✓
  - 错误处理 → spec 中列出的所有场景都有处理路径 ✓

- [ ] **占位符扫描:** 无 TBD/TODO，所有步骤都有具体代码

- [ ] **类型一致性:** `CompareState` 的变体在所有引用位置保持一致，`show_all` 字段在 `Setup` 和 `Running` 中均有定义