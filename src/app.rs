use std::sync::mpsc;
use eframe::egui;
use egui::{
    Color32, FontId, RichText, Stroke, Vec2,
    text::LayoutJob,
};
use egui_extras::{Column, TableBuilder};
use crate::loader::{self, LoadResult, MetaSummary, ParquetData};
use crate::table::TableState;
use crate::recent;

// ── Palette ───────────────────────────────────────────────────────────────────

struct Palette {
    bg:         Color32,
    surface:    Color32,
    surface2:   Color32,
    accent:     Color32,
    text:       Color32,
    muted:      Color32,
    row_alt:    Color32,
    row_hover:  Color32,
    header_bg:  Color32,
    border:     Color32,
    null:       Color32,
}

impl Palette {
    fn dark() -> Self {
        Self {
            bg:        Color32::from_rgb(24,  25,  30),
            surface:   Color32::from_rgb(32,  33,  40),
            surface2:  Color32::from_rgb(38,  40,  50),
            accent:    Color32::from_rgb(82,  145, 230),
            text:      Color32::from_rgb(220, 222, 228),
            muted:     Color32::from_rgb(120, 125, 145),
            row_alt:   Color32::from_rgb(30,  31,  38),
            row_hover: Color32::from_rgb(42,  48,  65),
            header_bg: Color32::from_rgb(28,  30,  42),
            border:    Color32::from_rgb(50,  52,  65),
            null:      Color32::from_rgb(80,  85,  105),
        }
    }

    fn light() -> Self {
        Self {
            bg:        Color32::from_rgb(248, 248, 252),
            surface:   Color32::from_rgb(255, 255, 255),
            surface2:  Color32::from_rgb(238, 239, 244),
            accent:    Color32::from_rgb(50,  120, 210),
            text:      Color32::from_rgb(25,  27,  35),
            muted:     Color32::from_rgb(110, 115, 135),
            row_alt:   Color32::from_rgb(242, 243, 247),
            row_hover: Color32::from_rgb(218, 228, 248),
            header_bg: Color32::from_rgb(230, 232, 242),
            border:    Color32::from_rgb(205, 207, 220),
            null:      Color32::from_rgb(170, 175, 195),
        }
    }
}

// ── App ───────────────────────────────────────────────────────────────────────

enum State {
    Empty,
    Loading,
    Loaded(ParquetData, TableState),
    Error(String),
}

enum DropAction {
    OpenPath(String),
    BrowseFromRecent(String),
}

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
}

impl ParquetApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<String>) -> Self {
        let dark_mode = cc.storage
            .and_then(|s| s.get_string("dark_mode"))
            .map(|v| v != "false")
            .unwrap_or(true);

        let recent_files = recent::load();
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
        };
        let palette = if dark_mode { Palette::dark() } else { Palette::light() };
        style_egui(&cc.egui_ctx, &palette, dark_mode);
        if let Some(path) = initial_file {
            app.start_load(path);
        }
        app
    }

    fn start_load(&mut self, path: String) {
        recent::push_and_save(&mut self.recent_files, &path);
        self.state = State::Loading;
        self.search.clear();
        self.show_meta = false;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        loader::load_async(path, tx);
    }

    fn reset_row_filter_for_len(&mut self, len: usize) {
        let max_row = len.saturating_sub(1);
        self.row_from = 0;
        self.row_to = max_row;
        self.row_from_input = String::from("0");
        self.row_to_input = max_row.to_string();
    }

    fn apply_row_filter_inputs(&mut self, len: usize) {
        let max_row = len.saturating_sub(1);
        let from_text = self.row_from_input.trim();
        let to_text = self.row_to_input.trim();

        let parsed_from = if from_text.is_empty() {
            0
        } else {
            from_text.parse::<usize>().ok().unwrap_or(self.row_from)
        };

        let parsed_to = if to_text.is_empty() {
            max_row
        } else {
            to_text.parse::<usize>().ok().unwrap_or(self.row_to)
        };

        let mut from = parsed_from.min(max_row);
        let mut to = parsed_to.min(max_row);
        if from > to {
            std::mem::swap(&mut from, &mut to);
        }

        self.row_from = from;
        self.row_to = to;
        self.row_from_input = from.to_string();
        self.row_to_input = to.to_string();
    }

    fn pick_file_from_recent_folder(recent_path: &str) -> Option<String> {
        let dir = std::path::Path::new(recent_path)
            .parent()
            .map(std::path::Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())?;

        rfd::FileDialog::new()
            .set_directory(dir)
            .add_filter("Parquet", &["parquet", "parq"])
            .pick_file()
            .map(|path| path.to_string_lossy().to_string())
    }

    fn poll_loader(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.rx {
            if let Ok(result) = rx.try_recv() {
                self.rx = None;
                match result {
                    LoadResult::Ok(data) => {
                        self.reset_row_filter_for_len(data.row_count);
                        let ts = TableState::new(data.row_count);
                        self.state = State::Loaded(data, ts);
                    }
                    LoadResult::Err(e) => {
                        self.state = State::Error(e);
                    }
                }
                ctx.request_repaint();
            } else {
                ctx.request_repaint();
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            if let Some(dropped) = i.raw.dropped_files.first() {
                if let Some(path) = &dropped.path {
                    let p = path.to_string_lossy().to_string();
                    if p.to_lowercase().ends_with(".parquet") || p.to_lowercase().ends_with(".parq") {
                        self.start_load(p);
                    }
                }
            }
        });
    }
}

impl eframe::App for ParquetApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("dark_mode", self.dark_mode.to_string());
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let palette = if self.dark_mode { Palette::dark() } else { Palette::light() };

        self.poll_loader(ctx);
        self.handle_dropped_files(ctx);

        // Keyboard shortcuts
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Escape) {
                self.search.clear();
            }
        });

        let loaded_row_count = match &self.state {
            State::Loaded(data, _) => Some(data.row_count),
            _ => None,
        };

        // Top menu bar
        egui::TopBottomPanel::top("menubar")
            .frame(egui::Frame::new().fill(palette.surface).inner_margin(egui::Margin::symmetric(12, 6)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.visuals_mut().widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
                    if ui.add(egui::Button::new(
                        RichText::new("Open…").color(palette.text).size(13.0)
                    ).frame(false)).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Parquet", &["parquet", "parq"])
                            .pick_file()
                        {
                            self.start_load(path.to_string_lossy().to_string());
                        }
                    }

                    // Ctrl+O
                    ctx.input(|i| {
                        if i.key_pressed(egui::Key::O) && i.modifiers.ctrl {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Parquet", &["parquet", "parq"])
                                .pick_file()
                            {
                                let _ = path;
                            }
                        }
                    });

                    ui.add_space(4.0);
                    ui.add(egui::Separator::default().vertical().spacing(8.0));
                    ui.add_space(4.0);

                    ui.label(RichText::new("Ctrl+O  open").color(palette.muted).size(11.0));

                    if let Some(row_count) = loaded_row_count {
                        ui.add_space(10.0);
                        ui.add(egui::Separator::default().vertical().spacing(8.0));
                        ui.add_space(8.0);
                        ui.label(RichText::new("Rows").color(palette.muted).size(11.0));
                        ui.label(RichText::new("from").color(palette.muted).size(11.0));

                        let from_resp = ui.add(
                            egui::TextEdit::singleline(&mut self.row_from_input)
                                .desired_width(60.0)
                                .font(FontId::monospace(12.0))
                        );

                        ui.label(RichText::new("to").color(palette.muted).size(11.0));

                        let to_resp = ui.add(
                            egui::TextEdit::singleline(&mut self.row_to_input)
                                .desired_width(60.0)
                                .font(FontId::monospace(12.0))
                        );

                        let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let editing_row_filter = from_resp.has_focus()
                            || to_resp.has_focus()
                            || from_resp.lost_focus()
                            || to_resp.lost_focus();
                        if enter_pressed && editing_row_filter {
                            self.apply_row_filter_inputs(row_count);
                        }

                        ui.add_space(8.0);
                        ui.label(RichText::new("Filter").color(palette.muted).size(11.0));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .desired_width(220.0)
                                .hint_text("type to filter rows…")
                                .font(FontId::monospace(12.0))
                        );
                        if ui.small_button("clear").clicked() {
                            self.search.clear();
                            self.reset_row_filter_for_len(row_count);
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let has_data = matches!(self.state, State::Loaded(_, _));
                        let label = if self.dark_mode { "☀ Light" } else { "🌙 Dark" };
                        if ui.add(egui::Button::new(
                            RichText::new(label).color(palette.muted).size(12.0)
                        ).frame(false)).clicked() {
                            self.dark_mode = !self.dark_mode;
                            let new_palette = if self.dark_mode { Palette::dark() } else { Palette::light() };
                            style_egui(ctx, &new_palette, self.dark_mode);
                        }

                        ui.add_space(8.0);

                        if ui.add_enabled(
                            has_data,
                            egui::Button::new(RichText::new("Meta").color(palette.muted).size(12.0)).frame(false),
                        ).clicked() {
                            self.show_meta = true;
                        }
                    });
                });
            });

        // Status bar
        egui::TopBottomPanel::bottom("statusbar")
            .frame(egui::Frame::new().fill(palette.surface).inner_margin(egui::Margin::symmetric(12, 5)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    match &self.state {
                        State::Empty => {
                            ui.label(RichText::new("Drop a .parquet file here or use Open…").color(palette.muted).size(12.0));
                        }
                        State::Loading => {
                            ui.spinner();
                            ui.add_space(6.0);
                            ui.label(RichText::new("Loading…").color(palette.muted).size(12.0));
                        }
                        State::Loaded(data, ts) => {
                            let visible = if !self.search.is_empty() {
                                let q = self.search.to_lowercase();
                                ts.row_order.iter().filter(|&&ri| {
                                    ri >= self.row_from
                                        && ri <= self.row_to
                                        && data.rows[ri].iter().any(|c| c.to_lowercase().contains(&q))
                                }).count()
                            } else {
                                ts.row_order
                                    .iter()
                                    .filter(|&&ri| ri >= self.row_from && ri <= self.row_to)
                                    .count()
                            };
                            let size_str = fmt_size(data.file_size);
                            ui.label(
                                RichText::new(format!(
                                    "{} rows  ×  {} cols   │   {}   │   {}",
                                    fmt_num(visible), data.col_count, size_str, data.file_path
                                ))
                                .color(palette.muted)
                                .size(12.0)
                            );
                        }
                        State::Error(e) => {
                            ui.label(RichText::new(format!("Error: {e}")).color(Color32::from_rgb(230, 80, 80)).size(12.0));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let resp = ui.add(
                            egui::Label::new(
                                RichText::new("Created by Jakob Aungiers")
                                    .color(palette.muted)
                                    .size(11.0)
                            ).sense(egui::Sense::click())
                        ).on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab("https://jakob-aungiers.com"));
                        }
                    });
                });
            });

        // Central panel
        let mut drop_action: Option<DropAction> = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.bg))
            .show(ctx, |ui| {
                match &self.state {
                    State::Empty => {
                        if let Some(action) = draw_drop_zone(ui, &palette, &self.recent_files) {
                            drop_action = Some(action);
                        }
                    }
                    State::Loading => draw_loading(ui, &palette),
                    State::Error(e) => draw_error(ui, &palette, e),
                    State::Loaded(_, _) => {}
                }
            });

        if let Some(action) = drop_action {
            match action {
                DropAction::OpenPath(path) => self.start_load(path),
                DropAction::BrowseFromRecent(path) => {
                    if let Some(selected) = Self::pick_file_from_recent_folder(&path) {
                        self.start_load(selected);
                    }
                }
            }
        }

        // Draw table separately so we can mutate self
        if let State::Loaded(data, ts) = &mut self.state {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(palette.bg))
                .show(ctx, |ui| {
                    draw_table(
                        ui,
                        &palette,
                        data,
                        ts,
                        &self.search,
                        self.row_from,
                        self.row_to,
                    );
                });
        }

        if self.show_meta {
            let metadata = match &self.state {
                State::Loaded(data, _) => Some((&data.meta_summary, data.meta_text.as_str())),
                _ => None,
            };

            egui::Window::new("Parquet Metadata")
                .open(&mut self.show_meta)
                .resizable(true)
                .default_size(Vec2::new(980.0, 720.0))
                .show(ctx, |ui| {
                    ui.visuals_mut().override_text_color = Some(palette.text);

                    match metadata {
                        Some((summary, text)) => {
                            egui::ScrollArea::vertical()
                                .id_salt("metadata_scroll")
                                .show(ui, |ui| {
                                    draw_meta_summary(ui, &palette, summary);
                                    ui.add_space(10.0);
                                    ui.separator();
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new("Full Metadata")
                                            .size(13.0)
                                            .color(palette.muted),
                                    );
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new(text)
                                            .monospace()
                                            .size(12.0)
                                            .color(palette.text),
                                    );
                                });
                        }
                        None => {
                            ui.label(RichText::new("No file is currently loaded.").color(palette.muted));
                        }
                    }
                });
        }
    }
}

// ── Metadata dialog ───────────────────────────────────────────────────────────

fn draw_meta_summary(ui: &mut egui::Ui, p: &Palette, summary: &MetaSummary) {
    ui.label(RichText::new("Summary").size(16.0).color(p.text));
    ui.add_space(6.0);

    egui::Grid::new("meta_file_summary")
        .num_columns(2)
        .spacing(Vec2::new(18.0, 6.0))
        .show(ui, |ui| {
            meta_kv(ui, p, "File path", &summary.file.path);
            meta_kv(ui, p, "File size", &summary.file.size);
            ui.end_row();
            meta_kv(ui, p, "Parquet version", &summary.file.parquet_version);
            meta_kv(ui, p, "Created by", &summary.file.created_by);
            ui.end_row();
            meta_kv(ui, p, "Total rows", &summary.file.total_rows);
            meta_kv(ui, p, "Columns", &summary.file.column_count);
            ui.end_row();
            meta_kv(ui, p, "Row groups", &summary.file.row_group_count);
            ui.label(egui::RichText::new("").size(12.0));
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.label(RichText::new("Column details").size(13.0).color(p.muted));
    ui.add_space(6.0);

    let text_height = ui.text_style_height(&egui::TextStyle::Body);
    let row_height = text_height + 8.0;
    let table_height = row_height * (summary.columns.len() as f32 + 1.5);

    egui::ScrollArea::horizontal()
        .id_salt("meta_columns_table")
        .show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(true)
                .min_scrolled_height(table_height.min(260.0))
                .max_scroll_height(table_height.min(260.0))
                .column(Column::initial(140.0).at_least(100.0).resizable(true))
                .column(Column::initial(280.0).at_least(180.0).resizable(true))
                .column(Column::initial(120.0).at_least(90.0).resizable(true))
                .column(Column::initial(220.0).at_least(140.0).resizable(true))
                .column(Column::initial(110.0).at_least(90.0).resizable(true))
                .column(Column::initial(110.0).at_least(90.0).resizable(true))
                .column(Column::initial(90.0).at_least(70.0).resizable(true))
                .header(row_height, |mut header| {
                    for title in ["Column", "Data type", "Compression", "Encodings", "Uncompressed", "Compressed", "Null count"] {
                        header.col(|ui| {
                            ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, p.header_bg);
                            ui.label(RichText::new(title).color(p.text).size(12.0).strong());
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_height, summary.columns.len(), |mut row| {
                        let item = &summary.columns[row.index()];
                        row.col(|ui| { ui.label(RichText::new(&item.name).color(p.text).size(12.0)); });
                        row.col(|ui| { ui.label(RichText::new(&item.dtype).color(p.text).size(12.0)); });
                        row.col(|ui| { ui.label(RichText::new(&item.compression).color(p.text).size(12.0)); });
                        row.col(|ui| { ui.label(RichText::new(&item.encodings).color(p.text).size(12.0)); });
                        row.col(|ui| { ui.label(RichText::new(&item.uncompressed_size).color(p.text).size(12.0)); });
                        row.col(|ui| { ui.label(RichText::new(&item.compressed_size).color(p.text).size(12.0)); });
                        row.col(|ui| { ui.label(RichText::new(&item.null_count).color(p.text).size(12.0)); });
                    });
                });
        });
}

fn meta_kv(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str) {
    ui.label(RichText::new(format!("{label}:  {value}")).color(p.text).size(12.0));
}

// ── Table rendering ───────────────────────────────────────────────────────────

fn draw_table(
    ui: &mut egui::Ui,
    p: &Palette,
    data: &ParquetData,
    ts: &mut TableState,
    search: &str,
    row_from: usize,
    row_to: usize,
) {
    let query = if !search.is_empty() {
        Some(search.to_lowercase())
    } else {
        None
    };

    let filtered: Vec<usize> = ts
        .row_order
        .iter()
        .copied()
        .filter(|&ri| {
            if ri < row_from || ri > row_to {
                return false;
            }
            if let Some(q) = &query {
                data.rows[ri].iter().any(|c| c.to_lowercase().contains(q.as_str()))
            } else {
                true
            }
        })
        .collect();

    let col_count = data.col_count;
    let row_count = filtered.len();

    let text_height = ui.text_style_height(&egui::TextStyle::Body);
    let row_height = text_height + 8.0;
    let header_height = 48.0;

    let mut sort_request: Option<usize> = None;

    egui::ScrollArea::horizontal()
        .id_salt("table_hscroll")
        .show(ui, |ui| {
    let mut builder = TableBuilder::new(ui)
        .striped(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(64.0))
        .resizable(true);

    for _ in 0..col_count {
        builder = builder.column(Column::initial(120.0).at_least(40.0).clip(true));
    }

    builder
        .header(header_height, |mut header| {
            header.col(|ui| {
                ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, p.header_bg);
                ui.add_space(2.0);
            });

            for col_idx in 0..col_count {
                let meta = &data.columns[col_idx];
                let is_sorted = ts.sort_col == Some(col_idx);
                let sort_asc = ts.sort_asc;

                header.col(|ui| {
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, p.header_bg);
                    ui.painter().line_segment(
                        [rect.right_top(), rect.right_bottom()],
                        Stroke::new(1.0, p.border),
                    );
                    ui.painter().line_segment(
                        [rect.left_bottom(), rect.right_bottom()],
                        Stroke::new(1.0, p.border),
                    );

                    let response = ui.allocate_rect(rect, egui::Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 0.0, p.row_hover);
                    }
                    if response.clicked() {
                        sort_request = Some(col_idx);
                    }

                    let pad = 8.0;
                    let name_rect = egui::Rect::from_min_size(
                        rect.min + Vec2::new(pad, 6.0),
                        Vec2::new(rect.width() - pad * 2.0 - 16.0, 20.0),
                    );
                    let dtype_rect = egui::Rect::from_min_size(
                        rect.min + Vec2::new(pad, 26.0),
                        Vec2::new(rect.width() - pad * 2.0, 16.0),
                    );

                    let name_color = if is_sorted { p.accent } else { p.text };
                    ui.painter().text(
                        name_rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        &meta.name,
                        FontId::new(13.0, egui::FontFamily::Proportional),
                        name_color,
                    );

                    ui.painter().text(
                        dtype_rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        &meta.dtype,
                        FontId::new(11.0, egui::FontFamily::Monospace),
                        p.muted,
                    );

                    if is_sorted {
                        let arrow = if sort_asc { "▲" } else { "▼" };
                        ui.painter().text(
                            egui::Pos2::new(rect.right() - 14.0, rect.top() + 14.0),
                            egui::Align2::CENTER_CENTER,
                            arrow,
                            FontId::new(10.0, egui::FontFamily::Proportional),
                            p.accent,
                        );
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_height, row_count, |mut row| {
                let row_idx = row.index();
                let data_row = filtered[row_idx];
                let bg = if row_idx % 2 == 0 { p.bg } else { p.row_alt };

                row.col(|ui| {
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, p.surface);
                    ui.painter().line_segment(
                        [rect.right_top(), rect.right_bottom()],
                        Stroke::new(1.0, p.border),
                    );
                    ui.painter().text(
                        rect.right_center() - Vec2::new(8.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        data_row.to_string(),
                        FontId::new(11.0, egui::FontFamily::Monospace),
                        p.muted,
                    );
                });

                for col_idx in 0..col_count {
                    row.col(|ui| {
                        let rect = ui.available_rect_before_wrap();
                        ui.painter().rect_filled(rect, 0.0, bg);
                        ui.painter().line_segment(
                            [rect.right_top(), rect.right_bottom()],
                            Stroke::new(1.0, p.border),
                        );

                        let cell_val = data.rows[data_row].get(col_idx).map(String::as_str).unwrap_or("");
                        let is_null = cell_val.is_empty();

                        if is_null {
                            ui.painter().text(
                                rect.left_center() + Vec2::new(6.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                "null",
                                FontId::new(12.0, egui::FontFamily::Monospace),
                                p.null,
                            );
                        } else {
                            if let Some(q) = &query {
                                if cell_val.to_lowercase().contains(q.as_str()) {
                                    let mut job = LayoutJob::default();
                                    highlight_matches(&mut job, cell_val, q, p.text);
                                    ui.painter().galley(
                                        rect.left_center() + Vec2::new(6.0, 0.0),
                                        ui.fonts(|f| f.layout_job(job)),
                                        Color32::WHITE,
                                    );
                                    return;
                                }
                            }

                            ui.painter().text(
                                rect.left_center() + Vec2::new(6.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                cell_val,
                                FontId::new(12.0, egui::FontFamily::Proportional),
                                p.text,
                            );
                        }
                    });
                }
            });
        });

    }); // end ScrollArea::horizontal

    if let Some(col) = sort_request {
        ts.sort_by(col, &data.rows);
    }
}

// ── Highlight search matches ──────────────────────────────────────────────────

fn highlight_matches(job: &mut LayoutJob, text: &str, query: &str, text_color: Color32) {
    let lower = text.to_lowercase();
    let mut last = 0usize;
    let mut search_start = 0usize;

    while let Some(pos) = lower[search_start..].find(query) {
        let abs = search_start + pos;
        if abs > last {
            job.append(&text[last..abs], 0.0, egui::TextFormat {
                font_id: FontId::new(12.0, egui::FontFamily::Proportional),
                color: text_color,
                ..Default::default()
            });
        }
        let end = abs + query.len();
        job.append(&text[abs..end], 0.0, egui::TextFormat {
            font_id: FontId::new(12.0, egui::FontFamily::Proportional),
            color: Color32::BLACK,
            background: Color32::from_rgb(255, 200, 50),
            ..Default::default()
        });
        last = end;
        search_start = end;
        if search_start >= lower.len() { break; }
    }
    if last < text.len() {
        job.append(&text[last..], 0.0, egui::TextFormat {
            font_id: FontId::new(12.0, egui::FontFamily::Proportional),
            color: text_color,
            ..Default::default()
        });
    }
}

// ── Empty / loading / error states ───────────────────────────────────────────

fn draw_drop_zone(ui: &mut egui::Ui, p: &Palette, recent: &[String]) -> Option<DropAction> {
    let mut action: Option<DropAction> = None;
    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        ui.label(RichText::new("⬇").size(56.0).color(p.border));
        ui.add_space(16.0);
        ui.label(RichText::new("Drop a .parquet file here").size(20.0).color(p.muted));
        ui.add_space(8.0);
        ui.label(RichText::new("or use  Open…  in the menu bar").size(13.0).color(p.null));

        if !recent.is_empty() {
            ui.add_space(32.0);
            ui.separator();
            ui.add_space(12.0);
            ui.label(RichText::new("Recent files").size(13.0).color(p.muted));
            ui.add_space(8.0);

            for path in recent {
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(path).size(13.0).color(p.accent).monospace()
                        ).sense(egui::Sense::click())
                    ).on_hover_cursor(egui::CursorIcon::PointingHand);
                    if resp.clicked() {
                        action = Some(DropAction::OpenPath(path.clone()));
                    }

                    let folder = ui.add(
                        egui::Button::new(RichText::new("📁").size(12.0).color(p.muted)).frame(false)
                    ).on_hover_text("Browse from this folder").on_hover_cursor(egui::CursorIcon::PointingHand);
                    if folder.clicked() {
                        action = Some(DropAction::BrowseFromRecent(path.clone()));
                    }
                });
                ui.add_space(4.0);
            }
        }
    });
    action
}

fn draw_loading(ui: &mut egui::Ui, p: &Palette) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.spinner();
            ui.add_space(16.0);
            ui.label(RichText::new("Loading…").size(16.0).color(p.muted));
        });
    });
}

fn draw_error(ui: &mut egui::Ui, p: &Palette, msg: &str) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.label(RichText::new("✕").size(48.0).color(Color32::from_rgb(200, 70, 70)));
            ui.add_space(12.0);
            ui.label(RichText::new("Failed to open file").size(18.0).color(p.text));
            ui.add_space(8.0);
            ui.label(RichText::new(msg).size(12.0).color(p.muted).monospace());
        });
    });
}

// ── Styling ───────────────────────────────────────────────────────────────────

fn style_egui(ctx: &egui::Context, p: &Palette, dark: bool) {
    let mut style = (*ctx.style()).clone();

    style.visuals.dark_mode = dark;
    style.visuals.panel_fill = p.bg;
    style.visuals.window_fill = p.surface;
    style.visuals.faint_bg_color = p.row_alt;
    style.visuals.extreme_bg_color = p.bg;
    style.visuals.override_text_color = Some(p.text);
    style.visuals.widgets.noninteractive.bg_fill = p.surface;
    style.visuals.widgets.inactive.bg_fill = p.surface2;
    style.visuals.widgets.hovered.bg_fill = p.row_hover;
    style.visuals.widgets.active.bg_fill = p.accent;
    style.visuals.selection.bg_fill = Color32::from_rgba_premultiplied(
        p.accent.r(), p.accent.g(), p.accent.b(), 60,
    );
    style.visuals.selection.stroke = Stroke::new(1.0, p.accent);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);

    style.spacing.item_spacing = Vec2::new(8.0, 4.0);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    style.spacing.scroll = egui::style::ScrollStyle::solid();

    ctx.set_style(style);
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn fmt_size(n: u64) -> String {
    let mut v = n as f64;
    for unit in &["B", "KB", "MB", "GB"] {
        if v < 1024.0 {
            return if *unit == "B" { format!("{v:.0} {unit}") } else { format!("{v:.1} {unit}") };
        }
        v /= 1024.0;
    }
    format!("{v:.1} TB")
}

fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 { result.push(','); }
        result.push(ch);
    }
    result.chars().rev().collect()
}
