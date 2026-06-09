use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

use anyhow::Result;
use chrono::{Local, Timelike};
use cyber_runner::{
    build_packets, build_run_count_packet, default_run_start,
    http_client::{
        fetch_venues, send_packet, send_packet_result, FetchVenuesOptions, DEFAULT_VENUE_PATH,
    },
    load_venues,
    models::{DEFAULT_BASE_URL, DEFAULT_REFERER, DEFAULT_USER_AGENT},
    packets::{build_http_text, calculate_sign, rewrite_headers},
    serialize_body, GeneratedPacket, ReplayConfig, Venue,
};
use eframe::egui;
use serde_json::Value;
use walkers::{
    lon_lat,
    sources::{Attribution, TileSource},
    HttpTiles, Map, MapMemory, Plugin, Position, Projector, TileId,
};

pub fn run_gui() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 860.0]),
        run_and_return: false,
        ..Default::default()
    };
    eframe::run_native(
        "TJ CyberRunner",
        options,
        Box::new(|cc| {
            install_cjk_fonts(&cc.egui_ctx);
            Ok(Box::new(CyberRunnerApp::new(cc)))
        }),
    )?;
    Ok(())
}

fn install_cjk_fonts(ctx: &egui::Context) {
    let Some(font) = load_system_cjk_font().or_else(load_embedded_cjk_font) else {
        eprintln!("No CJK-capable font found; using egui default fonts.");
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "cjk".to_owned(),
        Arc::new(egui::FontData::from_owned(font.bytes)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "cjk".to_owned());
    }
    ctx.set_fonts(fonts);
    eprintln!("Loaded CJK font: {}", font.label);
}

struct LoadedFont {
    label: String,
    bytes: Vec<u8>,
}

const FONT_SAMPLE_CHARS: &[char] = &[
    'A', 'z', '0', '中', '文', '跑', '步', '场', '地', '请', '求', '回', '应', '登', '录', '学',
    '期',
];

const PREFERRED_CJK_FAMILIES: &[&str] = &[
    "Heiti SC",
    "SimHei",
    "PingFang SC",
    "Microsoft YaHei",
    "Noto Sans CJK SC",
    "Source Han Sans SC",
    "WenQuanYi Micro Hei",
    "Arial Unicode MS",
    "Songti SC",
    "STHeiti",
    "Noto Sans SC",
];

fn load_system_cjk_font() -> Option<LoadedFont> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let mut best: Option<(i32, fontdb::ID, String)> = None;
    for face in db.faces() {
        if !face_supports_required_chars(&db, face.id) {
            continue;
        }

        let score = face_score(face);
        let label = face_label(face);
        let should_replace = match &best {
            Some((best_score, _, _)) => score < *best_score,
            None => true,
        };
        if should_replace {
            best = Some((score, face.id, label));
        }
    }

    let (_, id, label) = best?;
    db.with_face_data(id, |data, _| LoadedFont {
        label: format!("system: {label}"),
        bytes: data.to_vec(),
    })
}

fn face_supports_required_chars(db: &fontdb::Database, id: fontdb::ID) -> bool {
    db.with_face_data(id, |data, face_index| {
        ttf_parser::Face::parse(data, face_index)
            .map(|face| {
                FONT_SAMPLE_CHARS
                    .iter()
                    .all(|ch| face.glyph_index(*ch).is_some())
            })
            .unwrap_or(false)
    })
    .unwrap_or(false)
}

fn face_score(face: &fontdb::FaceInfo) -> i32 {
    let family_score = face_family_priority(face).unwrap_or(1_000);
    let style_penalty = if face.style == fontdb::Style::Normal {
        0
    } else {
        200
    };
    let weight_penalty = (i32::from(face.weight.0) - i32::from(fontdb::Weight::NORMAL.0)).abs();
    let monospace_penalty = if face.monospaced { 100 } else { 0 };

    family_score * 1_000 + style_penalty + weight_penalty + monospace_penalty
}

fn face_family_priority(face: &fontdb::FaceInfo) -> Option<i32> {
    face.families.iter().find_map(|(family, _)| {
        PREFERRED_CJK_FAMILIES
            .iter()
            .position(|preferred| family.eq_ignore_ascii_case(preferred))
            .map(|index| index as i32)
    })
}

fn face_label(face: &fontdb::FaceInfo) -> String {
    face.families
        .first()
        .map(|(family, _)| family.clone())
        .filter(|family| !family.is_empty())
        .unwrap_or_else(|| face.post_script_name.clone())
}

fn load_embedded_cjk_font() -> Option<LoadedFont> {
    embedded_cjk_font_bytes().and_then(|bytes| {
        if font_bytes_support_required_chars(bytes) {
            Some(LoadedFont {
                label: "embedded: CyberRunnerFallbackCJK.otf".to_owned(),
                bytes: bytes.to_vec(),
            })
        } else {
            eprintln!("Embedded CJK fallback font exists but does not cover required characters.");
            None
        }
    })
}

fn font_bytes_support_required_chars(bytes: &[u8]) -> bool {
    ttf_parser::Face::parse(bytes, 0)
        .map(|face| {
            FONT_SAMPLE_CHARS
                .iter()
                .all(|ch| face.glyph_index(*ch).is_some())
        })
        .unwrap_or(false)
}

#[cfg(cyber_runner_embedded_cjk_font)]
fn embedded_cjk_font_bytes() -> Option<&'static [u8]> {
    Some(include_bytes!("../assets/fonts/CyberRunnerFallbackCJK.otf"))
}

#[cfg(not(cyber_runner_embedded_cjk_font))]
fn embedded_cjk_font_bytes() -> Option<&'static [u8]> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_cjk_font_covers_required_chars() {
        let Some(bytes) = embedded_cjk_font_bytes() else {
            panic!("embedded CJK font is missing");
        };

        assert!(font_bytes_support_required_chars(bytes));
    }
}

#[derive(Clone, Copy, Debug)]
struct CartoDarkSource;
#[derive(Clone, Copy, Debug)]
struct CartoLightSource;

impl TileSource for CartoDarkSource {
    fn tile_url(&self, tile_id: TileId) -> String {
        format!(
            "https://a.basemaps.cartocdn.com/dark_all/{}/{}/{}.png",
            tile_id.zoom, tile_id.x, tile_id.y
        )
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            text: "OpenStreetMap contributors, CARTO",
            url: "https://carto.com/attributions",
            logo_light: None,
            logo_dark: None,
        }
    }
}

impl TileSource for CartoLightSource {
    fn tile_url(&self, tile_id: TileId) -> String {
        format!(
            "https://a.basemaps.cartocdn.com/light_all/{}/{}/{}.png",
            tile_id.zoom, tile_id.x, tile_id.y
        )
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            text: "OpenStreetMap contributors, CARTO",
            url: "https://carto.com/attributions",
            logo_light: None,
            logo_dark: None,
        }
    }
}

struct CyberRunnerApp {
    venues_path: String,
    base_url: String,
    authorization: String,
    user_agent: String,
    login_name: String,
    semester_start_year: i32,
    semester_term: u8,
    semester_id: String,
    semester_name: String,
    sex: String,
    run_status: String,
    standard_pace: String,
    result_km: String,
    track_km: String,
    duration_minutes: String,
    inset_m: String,
    seed: String,
    referer: String,
    token_query: bool,
    send_enabled: bool,
    venues: Vec<Venue>,
    selected_venue: usize,
    packets: Vec<GeneratedPacket>,
    request_log: String,
    status_log: String,
    replay_rx: Option<mpsc::Receiver<ReplayEvent>>,
    replay_cancel: Option<Arc<AtomicBool>>,
    replay_running: bool,
    route: Vec<(f64, f64)>,
    route_key: String,
    map_error: Option<String>,
    map_memory: MapMemory,
    tiles_dark: HttpTiles,
    tiles_light: HttpTiles,
}

enum ReplayEvent {
    Request(String),
    Status(String),
    Finished,
}

impl CyberRunnerApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let tiles_dark = HttpTiles::new(CartoDarkSource, _cc.egui_ctx.clone());
        let tiles_light = HttpTiles::new(CartoLightSource, _cc.egui_ctx.clone());
        let mut app = Self {
            venues_path: "CyberRunner/output/health_run_venues.json".to_owned(),
            base_url: DEFAULT_BASE_URL.to_owned(),
            authorization: String::new(),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            login_name: String::new(),
            semester_start_year: 2025,
            semester_term: 2,
            semester_id: "121".to_owned(),
            semester_name: "2025-2026学年第2学期".to_owned(),
            sex: "0".to_owned(),
            run_status: "0".to_owned(),
            standard_pace: "8.00".to_owned(),
            result_km: "2.00".to_owned(),
            track_km: "2.00".to_owned(),
            duration_minutes: "10".to_owned(),
            inset_m: "15".to_owned(),
            seed: "20260601".to_owned(),
            referer: DEFAULT_REFERER.to_owned(),
            token_query: false,
            send_enabled: false,
            venues: Vec::new(),
            selected_venue: 0,
            packets: Vec::new(),
            request_log: String::new(),
            status_log: String::new(),
            replay_rx: None,
            replay_cancel: None,
            replay_running: false,
            route: Vec::new(),
            route_key: String::new(),
            map_error: None,
            map_memory: MapMemory::default(),
            tiles_dark,
            tiles_light,
        };
        let _ = app.load_venues();
        app
    }

    fn append_status(&mut self, text: impl AsRef<str>) {
        self.status_log.push_str(text.as_ref());
        if !self.status_log.ends_with('\n') {
            self.status_log.push('\n');
        }
    }

    fn append_request(&mut self, text: impl AsRef<str>) {
        self.request_log.push_str(text.as_ref());
        if !self.request_log.ends_with('\n') {
            self.request_log.push('\n');
        }
    }

    fn load_venues(&mut self) -> Result<()> {
        self.venues = load_venues(PathBuf::from(self.venues_path.trim()))?;
        self.selected_venue = self.selected_venue.min(self.venues.len().saturating_sub(1));
        self.route_key.clear();
        self.append_status(format!("Loaded {} venues.", self.venues.len()));
        Ok(())
    }

    fn fetch_venues(&mut self) -> Result<()> {
        let out = PathBuf::from("CyberRunner/output/health_run_venues.json");
        let count = fetch_venues(FetchVenuesOptions {
            base_url: self.base_url.trim(),
            authorization: non_empty(&self.authorization),
            out: &out,
            path: DEFAULT_VENUE_PATH,
            timeout_seconds: 30.0,
            longitude: None,
            latitude: None,
            open_type: None,
            token_query: self.token_query,
        })?;
        self.venues_path = out.display().to_string();
        self.load_venues()?;
        self.append_status(format!("Fetched {count} venues: {}", out.display()));
        Ok(())
    }

    fn build_config(&self) -> Result<ReplayConfig> {
        let duration_minutes = parse_f64(&self.duration_minutes, "Duration min")?;
        let track_km = parse_f64(&self.track_km, "Track km")?;
        let inset_m = parse_f64(&self.inset_m, "Inset m")?;
        let seed = self.seed.trim().parse::<u64>()?;
        if duration_minutes <= 0.0 {
            anyhow::bail!("Duration min must be greater than 0");
        }
        if track_km <= 0.0 {
            anyhow::bail!("Track km must be greater than 0");
        }
        if self.result_km.trim().is_empty() {
            anyhow::bail!("resultKm is required");
        }
        let now = Local::now()
            .naive_local()
            .with_nanosecond(0)
            .unwrap_or_else(|| Local::now().naive_local());
        Ok(ReplayConfig {
            login_name: self.login_name.trim().to_owned(),
            semester_id: self.semester_id.trim().to_owned(),
            semester_name: self.semester_name.trim().to_owned(),
            sex: self.sex.trim().to_owned(),
            run_status: self.run_status.trim().to_owned(),
            standard_pace: self.standard_pace.trim().to_owned(),
            result_km: self.result_km.trim().to_owned(),
            track_km,
            duration_minutes,
            start_time: default_run_start(now, duration_minutes, 60),
            inset_m,
            seed,
            user_agent: non_empty(&self.user_agent).map(str::to_owned),
            authorization: non_empty(&self.authorization).map(str::to_owned),
            referer: non_empty(&self.referer)
                .unwrap_or(DEFAULT_REFERER)
                .to_owned(),
            packet_seconds: 60,
        })
    }

    fn selected_venue(&mut self) -> Result<Venue> {
        if self.venues.is_empty() {
            self.load_venues()?;
        }
        self.venues
            .get(self.selected_venue)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no venue selected"))
    }

    fn generate_preview(&mut self) -> Result<()> {
        let venue = self.selected_venue()?;
        let config = self.build_config()?;
        self.packets = build_packets(&venue, &config, "")?;
        self.route = route_from_packets(&self.packets);
        self.route_key = self.current_route_key();
        self.request_log.clear();
        let packets = self.packets.clone();
        for packet in &packets {
            let body = serialize_body(&packet.body, true)?;
            let headers = rewrite_headers(
                &packet.headers,
                &body,
                false,
                non_empty(&self.authorization),
                non_empty(&self.user_agent),
            );
            self.append_request(format!(
                "\n### packet {}/{}\n{}",
                packet.index,
                packet.total,
                build_http_text(packet, &headers, &body)
            ));
        }
        self.append_status(format!(
            "Generated {} packets for {}.",
            self.packets.len(),
            venue.venue_name
        ));
        Ok(())
    }

    fn run_count(&mut self) -> Result<()> {
        let packet = build_run_count_packet(&self.build_config()?)?;
        let body = serialize_body(&packet.body, true)?;
        let headers = rewrite_headers(
            &packet.headers,
            &body,
            false,
            non_empty(&self.authorization),
            non_empty(&self.user_agent),
        );
        self.append_request(format!(
            "\n### run/count\n{}",
            build_http_text(&packet, &headers, &body)
        ));
        if self.send_enabled {
            let response = send_packet(self.base_url.trim(), &packet)?;
            self.append_status(response);
        }
        Ok(())
    }

    fn replay(&mut self) -> Result<()> {
        if self.replay_running {
            self.append_status("Replay is already running.");
            return Ok(());
        }
        let venue = self.selected_venue()?;
        let config = self.build_config()?;
        self.packets = build_packets(&venue, &config, "")?;
        self.route = route_from_packets(&self.packets);
        self.route_key = self.current_route_key();
        self.request_log.clear();
        self.append_status(format!(
            "Replay scheduled: {} packets for {}.",
            self.packets.len(),
            venue.venue_name
        ));

        let packets = self.packets.clone();
        let send_enabled = self.send_enabled;
        let base_url = self.base_url.trim().to_owned();
        let authorization = non_empty(&self.authorization).map(str::to_owned);
        let user_agent = non_empty(&self.user_agent).map(str::to_owned);
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.replay_rx = Some(rx);
        self.replay_cancel = Some(cancel.clone());
        self.replay_running = true;
        thread::spawn(move || {
            run_replay_schedule(
                packets,
                send_enabled,
                base_url,
                authorization,
                user_agent,
                cancel,
                tx,
            )
        });
        Ok(())
    }

    fn stop_replay(&mut self) {
        if let Some(cancel) = &self.replay_cancel {
            cancel.store(true, Ordering::Relaxed);
            self.append_status("Stop requested. The current send step will finish first.");
        } else {
            self.append_status("No replay is running.");
        }
    }

    fn drain_replay_events(&mut self) {
        let Some(rx) = self.replay_rx.take() else {
            return;
        };
        let mut keep_receiver = true;
        while let Ok(event) = rx.try_recv() {
            match event {
                ReplayEvent::Request(text) => self.append_request(text),
                ReplayEvent::Status(text) => self.append_status(text),
                ReplayEvent::Finished => {
                    self.replay_running = false;
                    self.replay_cancel = None;
                    keep_receiver = false;
                }
            }
        }
        if keep_receiver {
            self.replay_rx = Some(rx);
        }
    }

    fn refresh_route_if_needed(&mut self) {
        let key = self.current_route_key();
        if key == self.route_key {
            return;
        }
        self.route_key = key;
        self.map_error = None;
        let Some(venue) = self.venues.get(self.selected_venue).cloned() else {
            self.route.clear();
            return;
        };
        match self
            .build_config()
            .and_then(|config| build_packets(&venue, &config, ""))
        {
            Ok(packets) => {
                self.route = route_from_packets(&packets);
                self.map_memory = MapMemory::default();
                let display_route = carto_display_route(&self.route);
                if let Some(center) = route_center(&display_route) {
                    self.map_memory.center_at(center);
                }
                let _ = self.map_memory.set_zoom(route_zoom(&display_route));
            }
            Err(error) => {
                self.route.clear();
                self.map_error = Some(format!("{error:#}"));
            }
        }
    }

    fn current_route_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}",
            self.venues_path.trim(),
            self.selected_venue,
            self.track_km.trim(),
            self.duration_minutes.trim(),
            self.inset_m.trim(),
            self.seed.trim(),
            self.result_km.trim(),
        )
    }
}

impl eframe::App for CyberRunnerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.render(ui);
    }

    fn on_exit(&mut self) {
        self.shutdown();
    }
}

impl Drop for CyberRunnerApp {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl CyberRunnerApp {
    fn shutdown(&mut self) {
        if let Some(cancel) = &self.replay_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    fn render(&mut self, root: &mut egui::Ui) {
        self.drain_replay_events();
        self.refresh_route_if_needed();
        let outer_margin = 12;
        egui::Frame::default()
            .fill(root.visuals().window_fill())
            .inner_margin(egui::Margin::same(outer_margin))
            .show(root, |ui| {
                let spacing = 10.0;
                let available = ui.available_size();
                let top_height = ((available.y - spacing) * 0.48)
                    .clamp(280.0, 460.0)
                    .min((available.y - spacing) * 0.62);
                let bottom_height = (available.y - top_height - spacing).max(0.0);
                let left_width = ((available.x - spacing) * 0.44).clamp(390.0, 620.0);
                let right_width = (available.x - left_width - spacing).max(360.0);

                ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(left_width, top_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| panel_frame(ui, |ui| self.render_controls(ui)),
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(right_width, top_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| panel_frame(ui, |ui| self.render_map(ui)),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(left_width, bottom_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| panel_frame(ui, |ui| self.render_text_panel(ui, "Requests", true)),
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(right_width, bottom_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| panel_frame(ui, |ui| self.render_text_panel(ui, "Status", false)),
                        );
                    });
                });
            });
    }

    fn render_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("TJ CyberRunner");
            ui.add_space(8.0);
            if ui.button("Get Run Count").clicked() {
                if let Err(error) = self.run_count() {
                    self.append_status(format!("ERROR: {error:#}"));
                }
            }
            if ui.button("Preview").clicked() {
                if let Err(error) = self.generate_preview() {
                    self.append_status(format!("ERROR: {error:#}"));
                }
            }
            if ui.button("Start Replay").clicked() {
                if let Err(error) = self.replay() {
                    self.append_status(format!("ERROR: {error:#}"));
                }
            }
            if ui.button("Stop").clicked() {
                self.stop_replay();
            }
        });
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section_header(ui, "Run");
                ui.horizontal_wrapped(|ui| {
                    param_box(ui, "trackKm", &mut self.track_km, 92.0);
                    param_box(ui, "duration", &mut self.duration_minutes, 92.0);
                });
                egui::CollapsingHeader::new("Track tuning")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            param_box(ui, "seed", &mut self.seed, 116.0);
                            param_box(ui, "inset m", &mut self.inset_m, 92.0);
                        });
                    });

                section_header(ui, "Runner");
                ui.horizontal(|ui| {
                    compact_text_row(ui, "loginName", &mut self.login_name);
                    compact_password_row(ui, "Authorization", &mut self.authorization);
                });
                ui.horizontal(|ui| {
                    self.render_sex_selector(ui);
                });
                self.render_semester_selector(ui);
                egui::CollapsingHeader::new("Runner details")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            param_box(ui, "resultKm", &mut self.result_km, 92.0);
                            param_box(ui, "runStatus", &mut self.run_status, 92.0);
                            param_box(ui, "standardPace", &mut self.standard_pace, 104.0);
                        });
                        text_row(ui, "semesterName", &mut self.semester_name);
                    });

                section_header(ui, "Request");
                ui.checkbox(&mut self.send_enabled, "Send requests to server");
                egui::CollapsingHeader::new("Request settings")
                    .default_open(false)
                    .show(ui, |ui| {
                        text_row(ui, "Base URL", &mut self.base_url);
                        text_row(ui, "User-Agent", &mut self.user_agent);
                        text_row(ui, "Referer", &mut self.referer);
                        ui.checkbox(&mut self.token_query, "Token query");
                    });
            });
    }

    fn render_sex_selector(&mut self, ui: &mut egui::Ui) {
        ui.label("sex");
        if ui.selectable_label(self.sex == "0", "Male").clicked() {
            self.sex = "0".to_owned();
            self.result_km = "2.00".to_owned();
        }
        if ui.selectable_label(self.sex == "1", "Female").clicked() {
            self.sex = "1".to_owned();
            self.result_km = "1.60".to_owned();
        }
    }

    fn render_semester_selector(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("semester");
            egui::ComboBox::from_id_salt("semester-year")
                .selected_text(format!(
                    "{}-{}",
                    self.semester_start_year,
                    self.semester_start_year + 1
                ))
                .show_ui(ui, |ui| {
                    for year in 2020..=2032 {
                        changed |= ui
                            .selectable_value(
                                &mut self.semester_start_year,
                                year,
                                format!("{year}-{}", year + 1),
                            )
                            .changed();
                    }
                });
            egui::ComboBox::from_id_salt("semester-term")
                .selected_text(format!("第{}学期", self.semester_term))
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut self.semester_term, 1, "第1学期")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut self.semester_term, 2, "第2学期")
                        .changed();
                });
            if changed {
                self.semester_id =
                    semester_id_for(self.semester_start_year, self.semester_term).to_string();
                self.semester_name =
                    semester_name_for(self.semester_start_year, self.semester_term);
            }
            ui.label("semesterId");
            ui.add_sized(
                [96.0, 24.0],
                egui::TextEdit::singleline(&mut self.semester_id),
            );
        });
    }

    fn render_map(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Map");
            if let Some(error) = &self.map_error {
                ui.colored_label(egui::Color32::from_rgb(235, 95, 95), error);
            }
        });
        let display_route = carto_display_route(&self.route);
        let center = route_center(&display_route).unwrap_or_else(|| lon_lat(121.215, 31.285));
        let map_size = ui.available_size();
        let map_rect = ui.available_rect_before_wrap();
        let tiles = if ui.visuals().dark_mode {
            &mut self.tiles_dark
        } else {
            &mut self.tiles_light
        };
        ui.add_sized(
            map_size,
            Map::new(Some(tiles), &mut self.map_memory, center).with_plugin(RouteLayer {
                route: display_route,
            }),
        );

        let overlay_gap = 12.0;
        let overlay_width = (map_rect.width() - overlay_gap * 2.0).max(260.0);
        let overlay_pos = egui::pos2(map_rect.center().x, map_rect.bottom() - overlay_gap);
        egui::Area::new(egui::Id::new("venue-map-overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(overlay_pos)
            .pivot(egui::Align2::CENTER_BOTTOM)
            .show(ui.ctx(), |ui| {
                ui.set_width(overlay_width);
                overlay_frame(ui, |ui| self.render_venue_overlay(ui));
            });
    }

    fn render_venue_overlay(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Venue").strong());
        ui.horizontal(|ui| {
            ui.add_sized([72.0, 22.0], egui::Label::new("Venue JSON"));
            let input_width = (ui.available_width() - 190.0).max(120.0);
            ui.add_sized(
                [input_width, 24.0],
                egui::TextEdit::singleline(&mut self.venues_path),
            );
            if ui.button("Load Venues").clicked() {
                if let Err(error) = self.load_venues() {
                    self.append_status(format!("ERROR loading venues: {error:#}"));
                }
            }
            if ui.button("Fetch Venues").clicked() {
                if let Err(error) = self.fetch_venues() {
                    self.append_status(format!("ERROR fetching venues: {error:#}"));
                }
            }
        });
        let selected_label = self
            .venues
            .get(self.selected_venue)
            .map(venue_label)
            .unwrap_or_else(|| "No venues loaded".to_owned());
        egui::ComboBox::from_label("")
            .selected_text(selected_label)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for (index, venue) in self.venues.iter().enumerate() {
                    if ui
                        .selectable_value(&mut self.selected_venue, index, venue_label(venue))
                        .changed()
                    {
                        self.route_key.clear();
                    }
                }
            });
    }

    fn render_text_panel(&mut self, ui: &mut egui::Ui, title: &str, requests: bool) {
        ui.horizontal(|ui| {
            ui.heading(title);
            if !requests {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Clear Logs").clicked() {
                        self.request_log.clear();
                        self.status_log.clear();
                    }
                });
            }
        });
        let text = if requests {
            self.request_log.as_str()
        } else {
            self.status_log.as_str()
        };
        let size = ui.available_size();
        ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
            egui::ScrollArea::both()
                .id_salt(title)
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).monospace()).selectable(true),
                    );
                });
        });
    }
}

struct RouteLayer {
    route: Vec<(f64, f64)>,
}

impl Plugin for RouteLayer {
    fn run(
        self: Box<Self>,
        ui: &mut egui::Ui,
        response: &egui::Response,
        projector: &Projector,
        _map_memory: &MapMemory,
    ) {
        if self.route.len() < 2 {
            return;
        }
        let points: Vec<egui::Pos2> = self
            .route
            .iter()
            .map(|(lat, lon)| {
                let point = projector.project(lon_lat(*lon, *lat));
                egui::pos2(point.x, point.y)
            })
            .collect();
        ui.painter()
            .with_clip_rect(response.rect)
            .add(egui::Shape::line(
                points,
                egui::Stroke::new(3.0, egui::Color32::from_rgb(40, 220, 105)),
            ));
        response.ctx.request_repaint();
    }
}

fn route_from_packets(packets: &[GeneratedPacket]) -> Vec<(f64, f64)> {
    packets
        .iter()
        .flat_map(|packet| {
            packet
                .body
                .get("details")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
        })
        .filter_map(|detail| {
            Some((
                detail.get("latitude")?.as_f64()?,
                detail.get("longitude")?.as_f64()?,
            ))
        })
        .collect()
}

fn run_replay_schedule(
    packets: Vec<GeneratedPacket>,
    send_enabled: bool,
    base_url: String,
    authorization: Option<String>,
    user_agent: Option<String>,
    cancel: Arc<AtomicBool>,
    tx: mpsc::Sender<ReplayEvent>,
) {
    let _ = tx.send(ReplayEvent::Status(format!(
        "Replay worker started. send_enabled={send_enabled}."
    )));
    let mut uid = String::new();
    let mut completed = true;
    for mut packet in packets {
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(ReplayEvent::Status("Replay stopped by user.".to_owned()));
            let _ = tx.send(ReplayEvent::Finished);
            return;
        }
        let wait = packet
            .scheduled_at
            .signed_duration_since(Local::now().naive_local());
        if let Ok(wait) = wait.to_std() {
            if !wait.is_zero() {
                let _ = tx.send(ReplayEvent::Status(format!(
                    "Waiting {:.1}s for packet {}/{}...",
                    wait.as_secs_f64(),
                    packet.index,
                    packet.total
                )));
                if wait_with_cancel(wait, &cancel) {
                    let _ = tx.send(ReplayEvent::Status("Replay stopped by user.".to_owned()));
                    let _ = tx.send(ReplayEvent::Finished);
                    return;
                }
            }
        }

        if !uid.is_empty() {
            if let Err(error) = apply_uid_to_packet(
                &mut packet,
                &uid,
                authorization.as_deref(),
                user_agent.as_deref(),
            ) {
                let _ = tx.send(ReplayEvent::Status(format!(
                    "ERROR rewriting packet {}/{} uid/sign: {error:#}",
                    packet.index, packet.total
                )));
                completed = false;
                break;
            }
        }

        match render_packet_text(&packet, authorization.as_deref(), user_agent.as_deref()) {
            Ok(text) => {
                let _ = tx.send(ReplayEvent::Request(format!(
                    "\n### packet {}/{}\n{text}",
                    packet.index, packet.total
                )));
            }
            Err(error) => {
                let _ = tx.send(ReplayEvent::Status(format!(
                    "ERROR rendering packet {}/{}: {error:#}",
                    packet.index, packet.total
                )));
                completed = false;
                break;
            }
        }

        if send_enabled {
            match send_packet_result(&base_url, &packet) {
                Ok(response) => {
                    let _ = tx.send(ReplayEvent::Status(format!(
                        "packet {}/{}\n{}",
                        packet.index, packet.total, response.log_text
                    )));
                    if let Some(found_uid) = extract_uid_from_response(&response.body_text) {
                        uid = found_uid;
                        let _ = tx.send(ReplayEvent::Status(format!(
                            "Captured uid for following packets: {uid}"
                        )));
                    }
                }
                Err(error) => {
                    let _ = tx.send(ReplayEvent::Status(format!(
                        "ERROR sending packet {}/{}: {error:#}",
                        packet.index, packet.total
                    )));
                    completed = false;
                    break;
                }
            }
        } else {
            let _ = tx.send(ReplayEvent::Status(format!(
                "packet {}/{} emitted locally.",
                packet.index, packet.total
            )));
        }
    }
    if completed {
        let _ = tx.send(ReplayEvent::Status("Replay finished.".to_owned()));
    }
    let _ = tx.send(ReplayEvent::Finished);
}

fn wait_with_cancel(wait: Duration, cancel: &AtomicBool) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() < wait {
        if cancel.load(Ordering::Relaxed) {
            return true;
        }
        let remaining = wait.saturating_sub(started.elapsed());
        thread::sleep(remaining.min(Duration::from_millis(200)));
    }
    cancel.load(Ordering::Relaxed)
}

fn apply_uid_to_packet(
    packet: &mut GeneratedPacket,
    uid: &str,
    authorization: Option<&str>,
    user_agent: Option<&str>,
) -> Result<()> {
    packet.body["uid"] = Value::String(uid.to_owned());
    let login_name = packet
        .body
        .get("loginName")
        .and_then(Value::as_str)
        .unwrap_or("");
    let timestamp = packet
        .body
        .get("timestamp")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("packet body timestamp is missing or not an integer"))?;
    packet.body["sign"] = Value::String(calculate_sign(uid, login_name, timestamp));
    let body = serialize_body(&packet.body, false)?;
    packet.headers = rewrite_headers(&packet.headers, &body, false, authorization, user_agent);
    Ok(())
}

fn extract_uid_from_response(response_body: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(response_body).ok()?;
    find_uid(&payload)
}

fn find_uid(value: &Value) -> Option<String> {
    match value {
        Value::Object(object) => {
            if let Some(uid) = object.get("uid").and_then(Value::as_str) {
                if !uid.is_empty() {
                    return Some(uid.to_owned());
                }
            }
            object.values().find_map(find_uid)
        }
        Value::Array(items) => items.iter().find_map(find_uid),
        _ => None,
    }
}

fn render_packet_text(
    packet: &GeneratedPacket,
    authorization: Option<&str>,
    user_agent: Option<&str>,
) -> Result<String> {
    let body = serialize_body(&packet.body, true)?;
    let headers = rewrite_headers(&packet.headers, &body, false, authorization, user_agent);
    Ok(build_http_text(packet, &headers, &body))
}

fn carto_display_route(route: &[(f64, f64)]) -> Vec<(f64, f64)> {
    route
        .iter()
        .map(|(lat, lon)| gcj02_to_wgs84(*lat, *lon))
        .collect()
}

fn gcj02_to_wgs84(lat: f64, lon: f64) -> (f64, f64) {
    if out_of_china(lat, lon) {
        return (lat, lon);
    }
    let (d_lat, d_lon) = gcj02_delta(lat, lon);
    (lat - d_lat, lon - d_lon)
}

fn out_of_china(lat: f64, lon: f64) -> bool {
    !(72.004..=137.8347).contains(&lon) || !(0.8293..=55.8271).contains(&lat)
}

fn gcj02_delta(lat: f64, lon: f64) -> (f64, f64) {
    let a = 6_378_245.0;
    let ee = 0.006_693_421_622_965_943;
    let mut d_lat = transform_lat(lon - 105.0, lat - 35.0);
    let mut d_lon = transform_lon(lon - 105.0, lat - 35.0);
    let rad_lat = lat.to_radians();
    let magic = 1.0 - ee * rad_lat.sin().powi(2);
    let sqrt_magic = magic.sqrt();
    d_lat = (d_lat * 180.0) / ((a * (1.0 - ee)) / (magic * sqrt_magic) * std::f64::consts::PI);
    d_lon = (d_lon * 180.0) / (a / sqrt_magic * rad_lat.cos() * std::f64::consts::PI);
    (d_lat, d_lon)
}

fn transform_lat(x: f64, y: f64) -> f64 {
    let mut ret = -100.0 + 2.0 * x + 3.0 * y + 0.2 * y * y + 0.1 * x * y + 0.2 * x.abs().sqrt();
    ret += (20.0 * (6.0 * x * std::f64::consts::PI).sin()
        + 20.0 * (2.0 * x * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret += (20.0 * (y * std::f64::consts::PI).sin()
        + 40.0 * (y / 3.0 * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret += (160.0 * (y / 12.0 * std::f64::consts::PI).sin()
        + 320.0 * (y * std::f64::consts::PI / 30.0).sin())
        * 2.0
        / 3.0;
    ret
}

fn transform_lon(x: f64, y: f64) -> f64 {
    let mut ret = 300.0 + x + 2.0 * y + 0.1 * x * x + 0.1 * x * y + 0.1 * x.abs().sqrt();
    ret += (20.0 * (6.0 * x * std::f64::consts::PI).sin()
        + 20.0 * (2.0 * x * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret += (20.0 * (x * std::f64::consts::PI).sin()
        + 40.0 * (x / 3.0 * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret += (150.0 * (x / 12.0 * std::f64::consts::PI).sin()
        + 300.0 * (x / 30.0 * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret
}

fn route_center(route: &[(f64, f64)]) -> Option<Position> {
    if route.is_empty() {
        return None;
    }
    let lat = route.iter().map(|point| point.0).sum::<f64>() / route.len() as f64;
    let lon = route.iter().map(|point| point.1).sum::<f64>() / route.len() as f64;
    Some(lon_lat(lon, lat))
}

fn route_zoom(route: &[(f64, f64)]) -> f64 {
    if route.len() < 2 {
        return 16.0;
    }
    let min_lat = route
        .iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min);
    let max_lat = route
        .iter()
        .map(|point| point.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_lon = route
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let max_lon = route
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let span = (max_lat - min_lat).abs().max((max_lon - min_lon).abs());
    if span < 0.0015 {
        18.0
    } else if span < 0.004 {
        17.0
    } else if span < 0.01 {
        16.0
    } else {
        15.0
    }
}

fn venue_label(venue: &Venue) -> String {
    format!(
        "{}: {} / {} ({} pts)",
        venue.index,
        &venue.campus_name,
        &venue.venue_name,
        venue.polygon_lonlat.len()
    )
}

fn semester_id_for(start_year: i32, term: u8) -> i32 {
    121 + (start_year - 2025) * 2 + i32::from(term) - 2
}

fn semester_name_for(start_year: i32, term: u8) -> String {
    format!("{}-{}学年第{}学期", start_year, start_year + 1, term)
}

fn panel_frame<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let fill = ui.visuals().extreme_bg_color;
    let stroke = egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
    egui::Frame::default()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(10))
        .show(ui, add_contents)
        .inner
}

fn overlay_frame<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let fill = ui.visuals().panel_fill.linear_multiply(0.92);
    egui::Frame::default()
        .fill(fill)
        .stroke(egui::Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
        .show(ui, add_contents)
        .inner
}

fn section_header(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).strong());
    ui.separator();
}

fn param_box(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32) {
    ui.vertical(|ui| {
        ui.label(label);
        ui.add_sized([width, 24.0], egui::TextEdit::singleline(value));
    });
}

fn text_row(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.add_sized([92.0, 22.0], egui::Label::new(label));
        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
    });
}

fn compact_text_row(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.vertical(|ui| {
        ui.label(label);
        ui.add_sized([160.0, 24.0], egui::TextEdit::singleline(value));
    });
}

fn compact_password_row(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.vertical(|ui| {
        ui.label(label);
        ui.add_sized(
            [220.0, 24.0],
            egui::TextEdit::singleline(value).password(true),
        );
    });
}

fn parse_f64(value: &str, label: &str) -> Result<f64> {
    value
        .trim()
        .parse::<f64>()
        .map_err(|error| anyhow::anyhow!("{label} must be a number: {error}"))
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}
