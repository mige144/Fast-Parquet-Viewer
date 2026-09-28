# 元数据对话框文字可复制 — 设计文档

日期：2026-09-28

## 问题

Parquet Metadata 对话框中的 meta 信息无法选中文字复制到粘贴板。原因：对话框内文本
放在 `egui::ScrollArea` 中，ScrollArea 默认开启 `drag_to_scroll`，鼠标拖动被滚动区
拦截，label 无法拖选文字。

## 需求（用户已确认）

- 拖选 + 复制按钮：既支持鼠标拖选后 Ctrl+C，也提供一键复制完整元数据的按钮。

## 设计

### 改动 1：允许拖选文字

- `src/app.rs` 元数据对话框外层 `ScrollArea::vertical()`（"metadata_scroll"）加
  `.drag_to_scroll(false)`。
- 列详情表格的内层 `ScrollArea::horizontal()`（"meta_columns_table"）同样加
  `.drag_to_scroll(false)`。
- 效果：鼠标拖动不再被滚动区拦截，可以选中 label 里的文字后 Ctrl+C 复制。滚轮和
  滚动条滚动不受影响。

### 改动 2：「Copy」按钮

- 对话框顶部 "Summary" 标题同一行右侧加一个 frameless "Copy" 按钮，视觉风格与
  状态栏 "Meta" 按钮一致。
- 点击后调用 `ui.ctx().copy_text(...)` 复制完整元数据文本
  （summary + meta_text 组合）到系统粘贴板。
- 按钮文字短暂变为 "Copied!"（约 1.5 秒后恢复），给出复制成功反馈。
- 反馈状态用对话框渲染时的时间戳比较实现，无需持久化状态。

## 不做的事

- 不改列详情表格单元格的选择行为（拖选修复后自然支持）。
- 不复制摘要部分（摘要内容已包含在完整元数据文本里）。
- 不加右键菜单（egui label 拖选 + Ctrl+C 已足够）。

## 验证

- `cargo check` 通过。
- 运行程序，打开一个 parquet 文件 → Meta：鼠标拖选文字、Ctrl+C 粘贴验证；
  点 Copy 按钮，在记事本粘贴验证完整元数据。
