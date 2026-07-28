#!/usr/bin/env python3
"""Fast Parquet Viewer — 文件对比引擎
用法:
  python compare_files.py --left a.parquet --right b.csv --output .comparisons/xxx/
"""

import argparse
import json
import os
import sys
import zipfile
from datetime import datetime
from pathlib import Path

import numpy as np
import pandas as pd


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
        row_data = {key_col: idx_val}

        # 判断行级别是否存在（key 是否在两侧都存在）
        in_left = idx_val in df_left_idx.index
        in_right = idx_val in df_right_idx.index

        if in_left and not in_right:
            status = "only_left"
        elif not in_left and in_right:
            status = "only_right"
        elif not in_left and not in_right:
            continue  # 不应出现在 outer join 中
        else:
            status = "identical"

        for col in all_common_cols:
            left_col_name = f"{col}_LEFT_"
            right_col_name = f"{col}_RIGHT_"

            left_val = row.get(left_col_name) if left_col_name in joined.columns else np.nan
            right_val = row.get(right_col_name) if right_col_name in joined.columns else np.nan

            row_data[f"left.{col}"] = row.get(left_col_name) if left_col_name in joined.columns else None
            row_data[f"right.{col}"] = row.get(right_col_name) if right_col_name in joined.columns else None

            # 仅在两侧都存在 key 时才比较值
            if status == "identical":
                both_nan = pd.isna(left_val) and pd.isna(right_val)
                if both_nan:
                    continue
                # NaN vs 非 NaN 视为不同
                if pd.isna(left_val) != pd.isna(right_val):
                    status = "different"
                    continue
                try:
                    if left_val != right_val:
                        status = "different"
                except (TypeError, ValueError):
                    if str(left_val) != str(right_val):
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