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
    df_left[key_col] = df_left[key_col].astype(str)
    df_right[key_col] = df_right[key_col].astype(str)

    if df_left[key_col].duplicated().any():
        raise ValueError(f"左侧文件 key 列 '{key_col}' 包含重复值，无法唯一标识行")
    if df_right[key_col].duplicated().any():
        raise ValueError(f"右侧文件 key 列 '{key_col}' 包含重复值，无法唯一标识行")

    df_left_idx = df_left.set_index(key_col)
    df_right_idx = df_right.set_index(key_col)
    joined = df_left_idx.join(df_right_idx, how="outer", lsuffix="_LEFT_", rsuffix="_RIGHT_")

    joined_columns_set = set(joined.columns)

    # 5. 构建对比结果 DataFrame（向量化）
    in_left_series = joined.index.isin(df_left_idx.index)
    in_right_series = joined.index.isin(df_right_idx.index)

    status_series = pd.Series("identical", index=joined.index)
    status_series[in_left_series & ~in_right_series] = "only_left"
    status_series[~in_left_series & in_right_series] = "only_right"

    # 对比公共列：仅在 status 仍为 "identical" 的行上比较
    for col in all_common_cols:
        mask = status_series == "identical"  # recompute each iteration

        left_col_name = f"{col}_LEFT_"
        right_col_name = f"{col}_RIGHT_"

        left_vals = joined[left_col_name] if left_col_name in joined_columns_set else pd.Series(np.nan, index=joined.index)
        right_vals = joined[right_col_name] if right_col_name in joined_columns_set else pd.Series(np.nan, index=joined.index)

        # Compare values: both-NaN treated as identical
        both_null = left_vals.isna() & right_vals.isna()
        try:
            not_equal = (left_vals != right_vals) & mask & ~both_null
        except (TypeError, ValueError):
            # Fallback to string comparison for mixed types
            not_equal = (left_vals.astype(str) != right_vals.astype(str)) & mask & ~both_null
        status_series[not_equal] = "different"

    # 构建结果 DataFrame
    result_data = {key_col: joined.index.values}
    for col in all_common_cols:
        left_col_name = f"{col}_LEFT_"
        right_col_name = f"{col}_RIGHT_"

        result_data[f"left.{col}"] = (
            joined[left_col_name].values if left_col_name in joined_columns_set
            else np.full(len(joined), np.nan)
        )
        result_data[f"right.{col}"] = (
            joined[right_col_name].values if right_col_name in joined_columns_set
            else np.full(len(joined), np.nan)
        )

    result_data["_compare_status"] = status_series.values
    result_df = pd.DataFrame(result_data)

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
