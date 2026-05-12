use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;
use arrow::array::*;
use arrow::datatypes::DataType;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::metadata::ParquetMetaData;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ColumnMeta {
    pub name: String,
    pub dtype: String,
}

#[derive(Debug, Clone)]
pub struct MetaSummary {
    pub file: FileSummary,
    pub columns: Vec<MetaColumnRow>,
}

#[derive(Debug, Clone)]
pub struct FileSummary {
    pub path: String,
    pub size: String,
    pub parquet_version: String,
    pub created_by: String,
    pub total_rows: String,
    pub column_count: String,
    pub row_group_count: String,
}

#[derive(Debug, Clone)]
pub struct MetaColumnRow {
    pub name: String,
    pub dtype: String,
    pub compression: String,
    pub encodings: String,
    pub uncompressed_size: String,
    pub compressed_size: String,
    pub null_count: String,
}

#[derive(Debug, Clone, Default)]
struct PandasColumnMeta {
    pandas_type: Option<String>,
    numpy_type: Option<String>,
}

#[derive(Debug, Clone)]
struct ColumnSummary {
    name: String,
    dtype: String,
    compression: BTreeSet<String>,
    encodings: BTreeSet<String>,
    uncompressed_size: u64,
    compressed_size: u64,
    null_count: Option<u64>,
}

impl ColumnSummary {
    fn new(name: String, dtype: String) -> Self {
        Self {
            name,
            dtype,
            compression: BTreeSet::new(),
            encodings: BTreeSet::new(),
            uncompressed_size: 0,
            compressed_size: 0,
            null_count: Some(0),
        }
    }
}

#[derive(Debug)]
pub struct ParquetData {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<String>>,
    pub row_count: usize,
    pub col_count: usize,
    pub file_size: u64,
    pub file_path: String,
    pub meta_summary: MetaSummary,
    pub meta_text: String,
}

pub enum LoadResult {
    Ok(ParquetData),
    Err(String),
}

pub fn load_async(path: String, tx: mpsc::Sender<LoadResult>) {
    std::thread::spawn(move || {
        tx.send(load_file(&path)).ok();
    });
}

fn load_file(path: &str) -> LoadResult {
    let file_size = std::fs::metadata(path)
        .map(|m| m.len())
        .unwrap_or(0);

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return LoadResult::Err(format!("Cannot open file: {e}")),
    };

    let builder = match ParquetRecordBatchReaderBuilder::try_new(file) {
        Ok(b) => b,
        Err(e) => return LoadResult::Err(format!("Cannot read parquet: {e}")),
    };

    let parquet_metadata = builder.metadata().clone();
    let schema = builder.schema().clone();
    let reader = match builder.build() {
        Ok(r) => r,
        Err(e) => return LoadResult::Err(format!("Cannot build reader: {e}")),
    };

    let columns: Vec<ColumnMeta> = schema
        .fields()
        .iter()
        .map(|f| ColumnMeta {
            name: f.name().clone(),
            dtype: friendly_dtype(f.data_type()),
        })
        .collect();

    let meta_summary = build_meta_summary(path, file_size, parquet_metadata.as_ref(), &columns);
    let meta_text = build_metadata_text(path, file_size, parquet_metadata.as_ref(), &columns);

    let col_count = columns.len();
    let mut all_rows: Vec<Vec<String>> = Vec::new();

    for batch_result in reader {
        let batch = match batch_result {
            Ok(b) => b,
            Err(e) => return LoadResult::Err(format!("Error reading batch: {e}")),
        };

        let n = batch.num_rows();
        for row_idx in 0..n {
            let mut row = Vec::with_capacity(col_count);
            for col_idx in 0..col_count {
                let col = batch.column(col_idx);
                row.push(format_value(col.as_ref(), row_idx));
            }
            all_rows.push(row);
        }
    }

    let row_count = all_rows.len();
    LoadResult::Ok(ParquetData {
        columns,
        rows: all_rows,
        row_count,
        col_count,
        file_size,
        file_path: path.to_string(),
        meta_summary,
        meta_text,
    })
}

fn build_meta_summary(
    path: &str,
    file_size: u64,
    metadata: &ParquetMetaData,
    columns: &[ColumnMeta],
) -> MetaSummary {
    let file_meta = metadata.file_metadata();
    let pandas_columns = extract_pandas_columns(file_meta.key_value_metadata());
    let column_summaries = build_column_summaries(metadata, columns, &pandas_columns);

    MetaSummary {
        file: FileSummary {
            path: path.to_string(),
            size: fmt_size(file_size as f64),
            parquet_version: file_meta.version().to_string(),
            created_by: file_meta.created_by().unwrap_or("unknown").to_string(),
            total_rows: file_meta.num_rows().to_string(),
            column_count: file_meta.schema_descr().num_columns().to_string(),
            row_group_count: metadata.num_row_groups().to_string(),
        },
        columns: column_summaries
            .iter()
            .map(|summary| MetaColumnRow {
                name: summary.name.clone(),
                dtype: summary.dtype.clone(),
                compression: join_set(&summary.compression),
                encodings: join_set(&summary.encodings),
                uncompressed_size: fmt_size(summary.uncompressed_size as f64),
                compressed_size: fmt_size(summary.compressed_size as f64),
                null_count: summary
                    .null_count
                    .map(|count| count.to_string())
                    .unwrap_or_else(|| String::from("unknown")),
            })
            .collect(),
    }
}

fn build_metadata_text(
    path: &str,
    file_size: u64,
    metadata: &ParquetMetaData,
    columns: &[ColumnMeta],
) -> String {
    let file_meta = metadata.file_metadata();
    let mut lines: Vec<String> = Vec::new();
    let pandas_columns = extract_pandas_columns(file_meta.key_value_metadata());
    let column_summaries = build_column_summaries(metadata, columns, &pandas_columns);

    lines.push(String::from("[Section 1: Summary]"));
    lines.push(String::from("+- File info"));
    lines.push(format!("|  +- Path: {path}"));
    lines.push(format!("|  +- Size: {}", fmt_size(file_size as f64)));
    lines.push(format!("|  +- Parquet version: {}", file_meta.version()));
    lines.push(format!("|  +- Created by: {}", file_meta.created_by().unwrap_or("unknown")));
    lines.push(format!("|  +- Total rows: {}", file_meta.num_rows()));
    lines.push(format!("|  +- Columns: {}", file_meta.schema_descr().num_columns()));
    lines.push(format!("|  +- Row groups: {}", metadata.num_row_groups()));
    lines.push(String::from("+- Column details"));
    append_table(
        &mut lines,
        &["Column", "Data type", "Compression", "Encodings", "Uncompressed", "Compressed", "Null count"],
        &column_summaries
            .iter()
            .map(|summary| {
                vec![
                    summary.name.clone(),
                    summary.dtype.clone(),
                    join_set(&summary.compression),
                    join_set(&summary.encodings),
                    fmt_size(summary.uncompressed_size as f64),
                    fmt_size(summary.compressed_size as f64),
                    summary
                        .null_count
                        .map(|count| count.to_string())
                        .unwrap_or_else(|| String::from("unknown")),
                ]
            })
            .collect::<Vec<_>>(),
    );
    lines.push(String::new());

    lines.push(String::from("Parquet metadata"));
    lines.push(String::from("+- File"));
    lines.push(format!("|  +- Path: {path}"));
    lines.push(format!("|  +- Size: {}", fmt_size(file_size as f64)));
    lines.push(format!("|  +- Parquet format version: {}", file_meta.version()));
    lines.push(format!("|  +- Created by: {}", file_meta.created_by().unwrap_or("unknown")));
    lines.push(format!("|  +- Rows (file metadata): {}", file_meta.num_rows()));
    lines.push(format!("|  +- Columns (schema leaf): {}", file_meta.schema_descr().num_columns()));
    lines.push(format!("|  +- Row groups: {}", metadata.num_row_groups()));

    lines.push(format!("+- Columns ({})", columns.len()));
    for (index, column) in columns.iter().enumerate() {
        lines.push(format!("|  +- [{index}] {}: {}", column.name, column.dtype));
    }

    let key_values = file_meta.key_value_metadata();
    let key_value_count = key_values.map_or(0, |v| v.len());
    lines.push(format!("+- Key-value metadata ({key_value_count})"));

    if let Some(items) = key_values {
        for item in items {
            lines.push(format!("|  +- {}", item.key));
            match item.value.as_deref() {
                Some(value) => lines.push(format!("|  |  +- Value: {value}")),
                None => lines.push(String::from("|  |  +- Value: <null>")),
            }
        }
    }

    lines.push(format!("+- Row groups ({})", metadata.num_row_groups()));

    for index in 0..metadata.num_row_groups() {
        let row_group = metadata.row_group(index);
        let mut codecs: BTreeSet<String> = BTreeSet::new();

        for col_idx in 0..row_group.num_columns() {
            let codec = format!("{:?}", row_group.column(col_idx).compression());
            codecs.insert(codec);
        }

        let codecs_str = if codecs.is_empty() {
            String::from("none")
        } else {
            codecs.into_iter().collect::<Vec<_>>().join(", ")
        };

        lines.push(format!("   +- RG #{index}"));
        lines.push(format!("      +- Rows: {}", row_group.num_rows()));
        lines.push(format!(
            "      +- Compressed size: {}",
            fmt_size(row_group.compressed_size() as f64)
        ));
        lines.push(format!(
            "      +- Uncompressed size: {}",
            fmt_size(row_group.total_byte_size() as f64)
        ));
        lines.push(format!("      +- Codecs: {codecs_str}"));
        lines.push(format!("      +- Column chunks ({})", row_group.num_columns()));

        for col_idx in 0..row_group.num_columns() {
            let chunk = row_group.column(col_idx);
            let col_path = chunk.column_path().string();
            lines.push(format!(
                "      |  +- [{}] path={}, compression={:?}, values={}, compressed={}, uncompressed={}",
                col_idx,
                col_path,
                chunk.compression(),
                chunk.num_values(),
                fmt_size(chunk.compressed_size() as f64),
                fmt_size(chunk.uncompressed_size() as f64),
            ));
        }
    }

    lines.join("\n")
}

fn build_column_summaries(
    metadata: &ParquetMetaData,
    columns: &[ColumnMeta],
    pandas_columns: &BTreeMap<String, PandasColumnMeta>,
) -> Vec<ColumnSummary> {
    let mut summaries: Vec<ColumnSummary> = columns
        .iter()
        .map(|column| {
            let mut dtype = column.dtype.clone();
            if let Some(pandas_meta) = pandas_columns.get(&column.name) {
                let mut extra_parts: Vec<String> = Vec::new();
                if let Some(pandas_type) = &pandas_meta.pandas_type {
                    extra_parts.push(format!("pandas={pandas_type}"));
                }
                if let Some(numpy_type) = &pandas_meta.numpy_type {
                    extra_parts.push(format!("numpy={numpy_type}"));
                }
                if !extra_parts.is_empty() {
                    dtype = format!("{} [{}]", dtype, extra_parts.join(", "));
                }
            }
            ColumnSummary::new(column.name.clone(), dtype)
        })
        .collect();

    for row_group_index in 0..metadata.num_row_groups() {
        let row_group = metadata.row_group(row_group_index);
        for chunk_index in 0..row_group.num_columns() {
            let chunk = row_group.column(chunk_index);
            if let Some(summary) = summaries.get_mut(chunk_index) {
                summary
                    .compression
                    .insert(format!("{:?}", chunk.compression()));
                for encoding in chunk.encodings() {
                    summary.encodings.insert(format!("{encoding:?}"));
                }
                if let Ok(size) = u64::try_from(chunk.uncompressed_size()) {
                    summary.uncompressed_size += size;
                }
                if let Ok(size) = u64::try_from(chunk.compressed_size()) {
                    summary.compressed_size += size;
                }
                match chunk.statistics().and_then(|stats| stats.null_count_opt()) {
                    Some(count) => {
                        if let Some(total) = summary.null_count.as_mut() {
                            *total += count;
                        }
                    }
                    None => summary.null_count = None,
                }
            }
        }
    }

    summaries
}

fn extract_pandas_columns(
    key_values: Option<&Vec<parquet::format::KeyValue>>,
) -> BTreeMap<String, PandasColumnMeta> {
    let Some(items) = key_values else {
        return BTreeMap::new();
    };

    let Some(pandas_json) = items
        .iter()
        .find(|item| item.key == "pandas")
        .and_then(|item| item.value.as_deref())
    else {
        return BTreeMap::new();
    };

    let Ok(root) = serde_json::from_str::<Value>(pandas_json) else {
        return BTreeMap::new();
    };

    let Some(columns) = root.get("columns").and_then(Value::as_array) else {
        return BTreeMap::new();
    };

    let mut result = BTreeMap::new();
    for column in columns {
        let field_name = column
            .get("field_name")
            .and_then(Value::as_str)
            .or_else(|| column.get("name").and_then(Value::as_str));
        let Some(field_name) = field_name else {
            continue;
        };

        result.insert(
            field_name.to_string(),
            PandasColumnMeta {
                pandas_type: column
                    .get("pandas_type")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                numpy_type: column
                    .get("numpy_type")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            },
        );
    }

    result
}

fn append_table(lines: &mut Vec<String>, headers: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = headers.iter().map(|header| header.chars().count()).collect();

    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if index < widths.len() {
                widths[index] = widths[index].max(cell.chars().count());
            }
        }
    }

    lines.push(format!("|  {}", format_table_row(headers.iter().map(|item| item.to_string()).collect::<Vec<_>>().as_slice(), &widths)));
    lines.push(format!(
        "|  {}",
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("-+-")
    ));

    for row in rows {
        lines.push(format!("|  {}", format_table_row(row, &widths)));
    }
}

fn format_table_row(cells: &[String], widths: &[usize]) -> String {
    cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let padding = widths[index].saturating_sub(cell.chars().count());
            format!("{}{}", cell, " ".repeat(padding))
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn join_set(values: &BTreeSet<String>) -> String {
    if values.is_empty() {
        return String::from("unknown");
    }
    values.iter().cloned().collect::<Vec<_>>().join(", ")
}

fn fmt_size(bytes: f64) -> String {
    let mut value = bytes;
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if value < 1024.0 {
            if unit == "B" {
                return format!("{value:.0} {unit}");
            }
            return format!("{value:.1} {unit}");
        }
        value /= 1024.0;
    }
    format!("{value:.1} PB")
}

fn friendly_dtype(dt: &DataType) -> String {
    match dt {
        DataType::Boolean => "bool".into(),
        DataType::Int8 => "int8".into(),
        DataType::Int16 => "int16".into(),
        DataType::Int32 => "int32".into(),
        DataType::Int64 => "int64".into(),
        DataType::UInt8 => "uint8".into(),
        DataType::UInt16 => "uint16".into(),
        DataType::UInt32 => "uint32".into(),
        DataType::UInt64 => "uint64".into(),
        DataType::Float16 => "float16".into(),
        DataType::Float32 => "float32".into(),
        DataType::Float64 => "float64".into(),
        DataType::Utf8 | DataType::LargeUtf8 => "string".into(),
        DataType::Binary | DataType::LargeBinary => "bytes".into(),
        DataType::Date32 | DataType::Date64 => "date".into(),
        DataType::Timestamp(u, tz) => {
            let tz_str = tz.as_deref().unwrap_or("no tz");
            format!("timestamp[{u:?}, {tz_str}]")
        }
        DataType::List(f) => format!("list<{}>", friendly_dtype(f.data_type())),
        DataType::Struct(_) => "struct".into(),
        DataType::Dictionary(_, v) => format!("dict<{}>", friendly_dtype(v)),
        DataType::Decimal128(p, s) => format!("decimal({p},{s})"),
        other => format!("{other:?}"),
    }
}

fn format_value(array: &dyn Array, idx: usize) -> String {
    if array.is_null(idx) {
        return String::new();
    }
    use arrow::array::*;
    use arrow::datatypes::DataType::*;
    match array.data_type() {
        Boolean => {
            let a = array.as_any().downcast_ref::<BooleanArray>().unwrap();
            a.value(idx).to_string()
        }
        Int8 => array.as_any().downcast_ref::<Int8Array>().unwrap().value(idx).to_string(),
        Int16 => array.as_any().downcast_ref::<Int16Array>().unwrap().value(idx).to_string(),
        Int32 => array.as_any().downcast_ref::<Int32Array>().unwrap().value(idx).to_string(),
        Int64 => array.as_any().downcast_ref::<Int64Array>().unwrap().value(idx).to_string(),
        UInt8 => array.as_any().downcast_ref::<UInt8Array>().unwrap().value(idx).to_string(),
        UInt16 => array.as_any().downcast_ref::<UInt16Array>().unwrap().value(idx).to_string(),
        UInt32 => array.as_any().downcast_ref::<UInt32Array>().unwrap().value(idx).to_string(),
        UInt64 => array.as_any().downcast_ref::<UInt64Array>().unwrap().value(idx).to_string(),
        Float32 => {
            let v = array.as_any().downcast_ref::<Float32Array>().unwrap().value(idx) as f64;
            fmt_float(v)
        }
        Float64 => {
            let v = array.as_any().downcast_ref::<Float64Array>().unwrap().value(idx);
            fmt_float(v)
        }
        Utf8 => array.as_any().downcast_ref::<StringArray>().unwrap().value(idx).to_string(),
        LargeUtf8 => array.as_any().downcast_ref::<LargeStringArray>().unwrap().value(idx).to_string(),
        Date32 => {
            let days = array.as_any().downcast_ref::<Date32Array>().unwrap().value(idx);
            format_date32(days)
        }
        Date64 => {
            let ms = array.as_any().downcast_ref::<Date64Array>().unwrap().value(idx);
            format_date64(ms)
        }
        Timestamp(arrow::datatypes::TimeUnit::Millisecond, _) => {
            let v = array.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap().value(idx);
            format_timestamp_ms(v)
        }
        Timestamp(arrow::datatypes::TimeUnit::Microsecond, _) => {
            let v = array.as_any().downcast_ref::<TimestampMicrosecondArray>().unwrap().value(idx);
            format_timestamp_us(v)
        }
        Timestamp(arrow::datatypes::TimeUnit::Nanosecond, _) => {
            let v = array.as_any().downcast_ref::<TimestampNanosecondArray>().unwrap().value(idx);
            format_timestamp_ns(v)
        }
        Timestamp(arrow::datatypes::TimeUnit::Second, _) => {
            let v = array.as_any().downcast_ref::<TimestampSecondArray>().unwrap().value(idx);
            format_timestamp_s(v)
        }
        _ => {
            // Fallback: use arrow's built-in display
            use arrow::util::display::ArrayFormatter;
            use arrow::util::display::FormatOptions;
            let opts = FormatOptions::default();
            ArrayFormatter::try_new(array, &opts)
                .map(|f| f.value(idx).to_string())
                .unwrap_or_else(|_| "<?>".to_string())
        }
    }
}

// ── Float formatting (like Python's {:.6g}) ──────────────────────────────────

fn fmt_float(v: f64) -> String {
    if v == 0.0 { return "0".to_string(); }
    let abs = v.abs();
    // Use exponential if very large or very small
    if abs >= 1e6 || abs < 1e-4 {
        // 5 decimal places in exponential = 6 sig figs
        let s = format!("{v:.5e}");
        // Clean up trailing zeros in mantissa
        if let Some(e_pos) = s.find('e') {
            let mantissa = s[..e_pos].trim_end_matches('0').trim_end_matches('.');
            let exp = &s[e_pos..];
            return format!("{mantissa}{exp}");
        }
        s
    } else {
        // Fixed: choose decimal places to show ~6 sig figs
        let mag = abs.log10().floor() as i32;
        let decimals = (5 - mag).max(0) as usize;
        let s = format!("{v:.decimals$}");
        // Trim trailing zeros
        if s.contains('.') {
            let s = s.trim_end_matches('0').trim_end_matches('.');
            s.to_string()
        } else {
            s
        }
    }
}

// ── Date/time helpers ────────────────────────────────────────────────────────

fn format_date32(days: i32) -> String {
    // days since 1970-01-01
    let epoch = 2440588i64; // Julian day of 1970-01-01
    let jd = days as i64 + epoch;
    let (y, m, d) = julian_to_ymd(jd);
    format!("{y:04}-{m:02}-{d:02}")
}

fn format_date64(ms: i64) -> String {
    format_date32((ms / 86_400_000) as i32)
}

fn format_timestamp_s(s: i64) -> String {
    let days = s.div_euclid(86_400) as i32;
    let time = s.rem_euclid(86_400);
    let date = format_date32(days);
    let h = time / 3600;
    let m = (time % 3600) / 60;
    let sec = time % 60;
    format!("{date} {h:02}:{m:02}:{sec:02}")
}

fn format_timestamp_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000) as i32;
    let rem = ms.rem_euclid(86_400_000);
    let date = format_date32(days);
    let h = rem / 3_600_000;
    let m = (rem % 3_600_000) / 60_000;
    let s = (rem % 60_000) / 1_000;
    let millis = rem % 1_000;
    format!("{date} {h:02}:{m:02}:{s:02}.{millis:03}")
}

fn format_timestamp_us(us: i64) -> String {
    let days = us.div_euclid(86_400_000_000) as i32;
    let rem = us.rem_euclid(86_400_000_000);
    let date = format_date32(days);
    let h = rem / 3_600_000_000;
    let m = (rem % 3_600_000_000) / 60_000_000;
    let s = (rem % 60_000_000) / 1_000_000;
    let us_part = rem % 1_000_000;
    format!("{date} {h:02}:{m:02}:{s:02}.{us_part:06}")
}

fn format_timestamp_ns(ns: i64) -> String {
    let days = ns.div_euclid(86_400_000_000_000) as i32;
    let rem = ns.rem_euclid(86_400_000_000_000);
    let date = format_date32(days);
    let h = rem / 3_600_000_000_000;
    let m = (rem % 3_600_000_000_000) / 60_000_000_000;
    let s = (rem % 60_000_000_000) / 1_000_000_000;
    let ns_part = rem % 1_000_000_000;
    format!("{date} {h:02}:{m:02}:{s:02}.{ns_part:09}")
}

fn julian_to_ymd(jd: i64) -> (i32, u32, u32) {
    // Algorithm from https://en.wikipedia.org/wiki/Julian_day#Julian_day_number_calculation
    let l = jd + 68569;
    let n = 4 * l / 146097;
    let l = l - (146097 * n + 3) / 4;
    let i = 4000 * (l + 1) / 1461001;
    let l = l - 1461 * i / 4 + 31;
    let j = 80 * l / 2447;
    let day = l - 2447 * j / 80;
    let l = j / 11;
    let month = j + 2 - 12 * l;
    let year = 100 * (n - 49) + i + l;
    (year as i32, month as u32, day as u32)
}
