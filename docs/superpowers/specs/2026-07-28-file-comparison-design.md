# 文件对比功能 — 设计规格

**日期：** 2026-07-28
**状态：** 设计已确认

## 概述

为 Fast Parquet Viewer 增加文件对比功能。用户选择两个数据文件（CSV、CSV.zip 或 Parquet）和行标识列，Python 脚本将两个文件加载到 pandas DataFrame，逐行对比，结果输出为 Parquet 文件——然后用现有的 Parquet 查看器直接展示。

## 架构

```
Rust / egui (桌面应用)                  Python (compare_files.py)
─────────────────────────              ─────────────────────────
对比设置对话框                            pandas DataFrame
  → 文件1、文件2、行标识列、Python环境       → pd.read_csv / read_parquet
  → 启动 Python 子进程                     → 按 key 列做 outer join
  → 解析 stdout JSON                      → 逐单元格对比
  → 加载结果 parquet 文件                  → 保存 compare_total.parquet
  → 用现有表格查看器展示                    → 保存 compare_diff.parquet
```

**通信方式：** Rust 以子进程方式启动 Python，传入 CLI 参数。Python 向 stdout 输出一行 JSON 结果。所有数据通过磁盘上的 Parquet 文件流转——不通过管道传输大数据。

## Python 脚本：`compare_files.py`

**位置：** 与 `FastParquetViewer.exe` 同目录。

**CLI 参数：**

| 参数 | 说明 |
|----------|-------------|
| `--left PATH` | 左侧文件路径（必填） |
| `--right PATH` | 右侧文件路径（必填） |
| `--key COLUMN` | 行标识列名（可选——省略时 Python 自动检测：`time` > `tradeDay` > 报错） |
| `--output DIR` | 结果 parquet 文件输出目录（必填） |

**支持的输入格式：**

- `.parquet` → `pd.read_parquet()`
- `.csv` → `pd.read_csv()`
- `.csv.zip` → 解压后用 `pd.read_csv()`

**对比逻辑：**

1. 将两个文件加载到 pandas DataFrame
2. 按 key 列做 outer join
3. 对每个公共列（非 key 列），逐值对比：
   - `NaN == NaN` → 视为**相同**
   - `value != NaN` 或 `NaN != value` → **不同**
   - 某列只在一个文件中存在 → 缺失侧全部为 null
4. 添加 `_compare_status` 列，取值为：`identical`、`different`、`only_left`、`only_right`
5. 重命名所有非 key 列：加 `left.` 或 `right.` 前缀

**输出列结构：**
```
[key_column] [left.colA] [right.colA] [left.colB] [right.colB] ... [_compare_status]
```

**输出文件（写入 `--output` 目录）：**

| 文件 | 内容 |
|------|---------|
| `compare_total.parquet` | 全部行（identical + different + only_left + only_right） |
| `compare_diff.parquet` | 仅差异行（`_compare_status != "identical"`） |

**Stdout JSON（成功）：**

```json
{
  "ok": true,
  "output_dir": ".comparisons/xxx/",
  "total_rows": 1000,
  "diff_rows": 42,
  "only_left": 5,
  "only_right": 3,
  "identical": 950,
  "key_column": "time"
}
```

**Stdout JSON（错误）：**

```json
{
  "ok": false,
  "error": "无法加载左侧文件: ..."
}
```

## Rust 侧：UI 变更

### 1. 工具栏 — 新增 "Compare…" 按钮

在顶部工具栏新增按钮，排在 "Open…" 和 Ctrl+O 提示之后。

### 2. 对比设置对话框

单个模态对话框（`egui::Window`）包含：

| 字段 | UI 元素 | 行为 |
|-------|-----------|----------|
| 左侧文件 | 文本输入 + 浏览按钮 | 文件对话框，过滤器：`*.parquet`、`*.csv`、`*.zip` |
| 右侧文件 | 文本输入 + 浏览按钮 | 同上 |
| 行标识列 | 文本输入 | 预填 "time" 作为默认值。Python 脚本也会自动检测：`time` > `tradeDay`。用户可键入任意列名。 |
| Python 环境 | 下拉框 / 可编辑文本 | 应用启动时自动检测。选项：系统 python、磁盘上找到的 conda 环境。"重新扫描"按钮。 |
| 显示所有行 | 复选框 | 默认：**不勾选**（仅显示差异） |

点击"开始对比"按钮后验证输入，然后启动 Python 子进程。

### 3. 对比执行中

Python 运行时，对话框显示旋转动画和"正在对比…"文字。应用通过 channel 非阻塞轮询子进程输出（与 `loader.rs` 模式一致）。

### 4. 对比完成后

- 解析 stdout JSON
- 如果 ok：将结果 parquet 文件加载到现有 `State::Loaded` 状态
  - 如果勾选了"显示所有行" → 加载 `compare_total.parquet`
  - 如果未勾选（默认） → 加载 `compare_diff.parquet`
- 如果出错：显示错误信息

### 5. 结果视图 — Diff/Total 切换

查看对比结果时，工具栏新增：

- **"⬅ 返回"按钮** — 返回之前的文件视图（或空状态）
- **Diff / Total 切换开关** — 在 `compare_diff.parquet` 和 `compare_total.parquet` 之间即时切换（两个文件已在磁盘上）
- **汇总文字** — 如 "42 处差异 / 1000 行"

### 6. Python 环境检测

应用启动时，Rust 扫描 Python 安装：

| 来源 | 路径模式 |
|--------|-------------|
| 系统 PATH | `python3`、`python` |
| Miniconda | `%USERPROFILE%/miniconda3/envs/*/python.exe` |
| Anaconda | `%USERPROFILE%/anaconda3/envs/*/python.exe` |
| `.conda` | `%USERPROFILE%/.conda/environments.txt` |

结果填充到下拉框。选择通过 `eframe::Storage` 持久化。用户可输入自定义路径。

## 目录结构

```
FastParquetViewer.exe
compare_files.py                     ← Python 对比脚本（与 exe 一起分发）
.comparisons/                        ← 对比输出根目录（由 Python 创建）
└── YYYYMMDD_HHMMSS_left_vs_right/
    ├── compare_total.parquet        ← 全部对比行
    └── compare_diff.parquet         ← 仅差异行
```

`.comparisons/` 命名包含时间戳和两个文件名，避免冲突。

## Rust 状态变更

### 新枚举：`CompareState`

```rust
enum CompareState {
    Hidden,                           // 对话框未显示
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
```

### `ParquetApp` 新增字段

```rust
compare_state: CompareState,
compare_output_dir: Option<String>,  // 对比结果目录，用于 total/diff 切换
compare_data: Option<CompareSummary>, // 行数统计等，用于工具栏显示
previous_file: Option<String>,        // 用户点击返回时恢复的文件路径
```

## 文件加载变更

现有 `loader.rs` 已支持 Parquet 加载。无需修改——对比结果本身就是 Parquet 文件，现有加载和显示代码可直接使用。

CSV/CSV.zip 的支持完全由 Python 脚本处理——Rust 永远不需要直接读取 CSV。

## 错误处理

| 场景 | 处理方式 |
|----------|----------|
| 未找到 Python | 对话框显示警告："未找到 Python，请安装 Python 3.8+ 和 pandas。" |
| 未安装 pandas | Python 脚本报 ImportError → 显示 "pip install pandas pyarrow" |
| 左侧文件不可读 | Python 脚本报错 → 在对话框中显示 |
| 右侧文件不可读 | 同上 |
| key 列在某个文件中不存在 | Python 脚本报错 → "列 'X' 在文件 Y 中未找到" |
| Python 进程崩溃 | Rust 检测到非零退出码 → 显示"对比失败"及 stderr |
| 输出目录写入错误 | Python 脚本报错 → 显示给用户 |

## 关键设计决策

1. **Parquet 作为交换格式** — 不需要新的表格渲染代码。Python 输出 Parquet，Rust 用现有查看器加载。
2. **子进程优于嵌入** — 构建更简单，无 Rust-Python FFI 复杂度。
3. **两个输出文件，即时切换** — 用户无需重新计算即可在 total/diff 之间切换。
4. **`_compare_status` 列** — 机器可读的状态列，便于未来扩展（按状态筛选、颜色标记行）。
5. **列前缀约定** — `left.` 和 `right.` 前缀明确标识每个值的来源文件。