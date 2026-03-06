use std::io;
use std::io::Read;
use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{error::Error, thread};

use common::RadioMsg;
use serialport::SerialPort;

#[cfg(feature = "gui")]
const MAX_GUI_MESSAGES: usize = 2000;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mock = args.iter().any(|a| a == "--mock" || a == "-m");
    let baud_rate: u32 = args
        .windows(2)
        .find(|w| w[0] == "--baud" || w[0] == "-b")
        .map(|w| w[1].parse().expect("--baud must be a valid number"))
        .unwrap_or(9600);
    let key: [u8; 32] = *b"sPvC4rYYCy6QlZxNHaB5lHjsVgz6F2NJ";

    #[cfg(feature = "gui")]
    let use_gui = args.iter().any(|a| a == "--gui" || a == "-g");

    #[cfg(feature = "gui")]
    if use_gui {
        let (tx, rx) = std::sync::mpsc::channel();
        let key_for_serial = key;
        let thread_handle = if mock {
            println!("Mock + GUI: generating fake stream in background.");
            thread::spawn(move || run_mock_stream(Some(tx)))
        } else {
            println!("GUI: opening serial and receiving in background.");
            thread::spawn(move || run_serial_loop(key_for_serial, baud_rate, Some(tx)))
        };
        let native_options = eframe::NativeOptions::default();
        let rx = std::sync::Mutex::new(rx);
        eframe::run_native(
            "LoRa listener",
            native_options,
            Box::new(move |cc| Ok(Box::new(LoraGuiApp::new(rx, cc.egui_ctx.clone())))),
        )
        .map_err(|e| format!("eframe run_native: {}", e))?;
        drop(thread_handle);
        return Ok(());
    }

    if mock {
        println!("Mock mode: generating fake RadioMsg stream (no serial). Ctrl+C to stop.");
        run_mock_stream(None);
        return Ok(());
    }

    run_serial_loop(key, baud_rate, None);
    Ok(())
}

/// Serial receive loop. When `sender` is Some, pushes (timestamp, msg) to the channel; otherwise prints via on_message.
fn run_serial_loop(key: [u8; 32], baud_rate: u32, sender: Option<std::sync::mpsc::Sender<(f64, RadioMsg)>>) {
    let ports = serialport::available_ports().expect("Failed to list serial ports");
    let port = ports.first().expect(
        "No serial ports found. Plug in a LoRa serial device (e.g. USB-UART). Use --mock for testing without hardware.",
    );

    println!("Opening {}", port.port_name);
    println!("Baud rate: {baud_rate}");
    let port = serialport::new(&port.port_name, baud_rate)
        .timeout(Duration::from_secs(1))
        .open()
        .unwrap();

    let mut tty = TtyAdapter::new(port);
    configure_lora(&mut tty);

    loop {
        let line = tty.read_line().unwrap();
        println!("{}", line.trim_end());

        if line.starts_with("+TEST: RX \"") {
            match parse_data(line.trim_end()) {
                Err(e) => println!("Failed to parse data: {e}"),
                Ok(raw) => {
                    if let Some((timestamp, msg)) = RadioMsg::decrypt(&raw, &key) {
                        on_message(timestamp, &msg);
                        if let Some(ref s) = sender {
                            let _ = s.send((timestamp, msg));
                        }
                    }
                }
            }
        }
    }
}

/// Called for each decoded (timestamp, RadioMsg) when not using GUI.
fn on_message(timestamp: f64, msg: &RadioMsg) {
    println!("{timestamp}: {msg:?}");
}

/// Generates a fake stream of RadioMsg for testing without hardware.
/// When `sender` is Some, sends (timestamp, msg) to the channel; otherwise prints via on_message.
fn run_mock_stream(sender: Option<std::sync::mpsc::Sender<(f64, RadioMsg)>>) {
    let mut lat = 52.2297;
    let mut lon = 21.0122;
    let mut course = 45.0;
    let mut speed = 5.0;
    let mut alt = 120.0;
    let mut sats = 8u64;
    let interval = Duration::from_secs(2);

    loop {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        let msg = RadioMsg {
            latitude_degrees: lat,
            longitude_degrees: lon,
            course_over_ground_degrees: course,
            speed_over_ground_meters_per_second: speed,
            altitude_meters: alt,
            satellites: sats,
        };

        if let Some(ref s) = sender {
            let _ = s.send((timestamp, msg));
        } else {
            on_message(timestamp, &msg);
        }

        lat += 0.0001;
        lon += 0.00008;
        course = (course + 2.0) % 360.0;
        speed = 4.5 + (timestamp * 0.1).sin() * 0.5;
        alt += 0.5;
        sats = 7 + (timestamp as u64 % 3);

        thread::sleep(interval);
    }
}

fn configure_lora(tty: &mut TtyAdapter) {
    tty.write("AT+MODE=TEST\r\n").unwrap();
    println!("{}", tty.read_line().unwrap().trim_end());

    let frequency = 869.5;
    let spreading_factor = 11;
    let bandwidth = 250;
    let tx_preamble = 8;
    let rx_preamble = 8;
    let power = 14;
    let crc = "OFF";
    let iq = "OFF";
    let net = "OFF";

    let cmd = format!(
        "AT+TEST=RFCFG,{frequency},SF{spreading_factor},{bandwidth},{tx_preamble},{rx_preamble},{power},{crc},{iq},{net}\r\n"
    );
    tty.write(&cmd).unwrap();
    println!("{}", tty.read_line().unwrap().trim_end());

    tty.write("AT+TEST=RXLRPKT\r\n").unwrap();
    println!("{}", tty.read_line().unwrap().trim_end());
}

fn parse_data(s: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let rx = s
        .strip_prefix("+TEST: RX \"")
        .ok_or("bad RX prefix")?
        .strip_suffix('"')
        .ok_or("bad RX suffix")?;
    let mut data = Vec::new();
    for chunk in rx.as_bytes().chunks_exact(2) {
        data.push(u8::from_str_radix(std::str::from_utf8(chunk)?, 16)?);
    }
    Ok(data)
}

struct TtyAdapter {
    port: Box<dyn SerialPort>,
    len: usize,
    buf: [u8; 1000],
}

impl TtyAdapter {
    pub fn new(port: Box<dyn SerialPort>) -> Self {
        Self {
            port,
            len: 0,
            buf: [0u8; 1000],
        }
    }

    fn read_next_batch(&mut self) -> io::Result<Option<String>> {
        const CRLF: &[u8] = b"\r\n";

        if let Some(position) = memchr::memmem::find(&self.buf[..self.len], CRLF) {
            let line_length = position + CRLF.len();
            let line = String::from_utf8_lossy(&self.buf[..line_length]).to_string();

            self.buf.copy_within(line_length..self.len, 0);

            self.len -= line_length;

            Ok(Some(line))
        } else {
            let new_bytes_count = self.port.read(&mut self.buf[self.len..])?;

            self.len += new_bytes_count;
            Ok(None)
        }
    }

    pub fn read_line(&mut self) -> io::Result<String> {
        loop {
            let batch = match self.read_next_batch() {
                Ok(batch) => Ok(batch),
                Err(e) if e.kind() == io::ErrorKind::TimedOut => Ok(None),
                Err(e) => Err(e),
            };

            match batch? {
                None => continue,
                Some(line) => return Ok(line),
            }
        }
    }

    pub fn write(&mut self, command: &str) -> io::Result<()> {
        println!("Writing: {command}");
        self.port.write_all(command.as_bytes())?;
        Ok(())
    }
}

// --- GUI (feature-gated) ---

#[cfg(feature = "gui")]
use eframe::egui;

#[cfg(feature = "gui")]
struct LoraGuiApp {
    rx: std::sync::Mutex<std::sync::mpsc::Receiver<(f64, RadioMsg)>>,
    messages: Vec<(f64, RadioMsg)>,
    tiles: walkers::HttpTiles,
    map_memory: walkers::MapMemory,
}

#[cfg(feature = "gui")]
impl LoraGuiApp {
    fn new(
        rx: std::sync::Mutex<std::sync::mpsc::Receiver<(f64, RadioMsg)>>,
        egui_ctx: egui::Context,
    ) -> Self {
        Self {
            rx,
            messages: Vec::new(),
            tiles: walkers::HttpTiles::new(walkers::sources::OpenStreetMap, egui_ctx),
            map_memory: walkers::MapMemory::default(),
        }
    }
}

/// Draws the GPS path (line) and current position (dot) on the walkers map.
#[cfg(feature = "gui")]
struct GpsMarkerPlugin {
    path: Vec<(f64, f64)>,
    latest: Option<(f64, f64)>,
}

#[cfg(feature = "gui")]
impl walkers::Plugin for GpsMarkerPlugin {
    fn run(
        self: Box<Self>,
        ui: &mut egui::Ui,
        _response: &egui::Response,
        projector: &walkers::Projector,
    ) {
        let path_color = egui::Color32::from_rgb(80, 160, 240);
        let dot_color = egui::Color32::from_rgb(220, 60, 60);
        for (lon, lat) in &self.path {
            let pos = walkers::Position::from_lon_lat(*lon, *lat);
            let screen = projector.project(pos);
            ui.painter().circle_filled(egui::Pos2::new(screen.x, screen.y), 2.5, path_color);
        }
        if let Some((lon, lat)) = self.latest {
            let pos = walkers::Position::from_lon_lat(lon, lat);
            let screen = projector.project(pos);
            ui.painter().circle_filled(egui::Pos2::new(screen.x, screen.y), 10.0, dot_color);
        }
    }
}

#[cfg(feature = "gui")]
fn tile_plot(
    ui: &mut egui::Ui,
    title: &str,
    side: f32,
    x: &[f64],
    y: Vec<f64>,
    color: egui::Color32,
) {
    ui.vertical(|ui| {
        ui.set_max_width(side);
        ui.label(egui::RichText::new(title).strong());
        if x.len() != y.len() || x.is_empty() {
            ui.add_space(side - ui.text_style_height(&egui::TextStyle::Body));
            ui.label("—");
            return;
        }
        let points: Vec<[f64; 2]> = x.iter().copied().zip(y).map(|(a, b)| [a, b]).collect();
        let x_min = x.iter().copied().fold(f64::NAN, f64::min);
        let x_max = x.iter().copied().fold(f64::NAN, f64::max);
        let y_min = points.iter().map(|p| p[1]).fold(f64::NAN, f64::min);
        let y_max = points.iter().map(|p| p[1]).fold(f64::NAN, f64::max);
        let x_range = (x_max - x_min).abs();
        let y_range = (y_max - y_min).abs();
        let x_margin = if x_range > 1e-9 { x_range * 0.08 } else { 0.5 };
        let y_margin = if y_range > 1e-9 { y_range * 0.08 } else { 0.5 };
        let plot = egui_plot::Plot::new(egui::Id::new(title))
            .height(side)
            .label_formatter(move |_name, value| format!("{:.3}", value.y))
            .include_x(x_min - x_margin)
            .include_x(x_max + x_margin)
            .include_y(y_min - y_margin)
            .include_y(y_max + y_margin)
            .show_axes([true, true]);
        plot.show(ui, |plot_ui| {
            plot_ui.line(
                egui_plot::Line::new(egui_plot::PlotPoints::new(points))
                    .color(color)
                    .width(2.0),
            );
        });
    });
}

#[cfg(feature = "gui")]
impl eframe::App for LoraGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let rx = self.rx.lock().unwrap();
        while let Ok(pair) = rx.try_recv() {
            self.messages.push(pair);
            if self.messages.len() > MAX_GUI_MESSAGES {
                self.messages.remove(0);
            }
        }
        drop(rx);

        egui::TopBottomPanel::bottom("telemetry").show(ctx, |ui| {
            if let Some((ts, msg)) = self.messages.last() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Lat: {:.6}°", msg.latitude_degrees));
                    ui.label(format!("Lon: {:.6}°", msg.longitude_degrees));
                    ui.label(format!("Course: {:.1}°", msg.course_over_ground_degrees));
                    ui.label(format!("Speed: {:.2} m/s", msg.speed_over_ground_meters_per_second));
                    ui.label(format!("Alt: {:.1} m", msg.altitude_meters));
                    ui.label(format!("Sats: {}", msg.satellites));
                    let dt = chrono::DateTime::from_timestamp(*ts as i64, (ts.fract() * 1e9) as u32)
                        .map(|utc| utc.with_timezone(&chrono::Local));
                    if let Some(dt) = dt {
                        ui.label(format!("Time: {}", dt.format("%H:%M:%S")));
                    }
                    ui.label(format!("Msgs: {}", self.messages.len()));
                });
            } else {
                ui.label("Waiting for first message…");
            }
        });

        let ts: Vec<f64> = self.messages.iter().map(|(t, _)| *t).collect();

        egui::SidePanel::right("graphs")
            .resizable(true)
            .default_width(220.0)
            .show(ctx, |ui| {
                let side = ui.available_width();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    tile_plot(
                        ui,
                        "Course (°)",
                        side,
                        &ts,
                        self.messages.iter().map(|(_, m)| m.course_over_ground_degrees).collect::<Vec<_>>(),
                        egui::Color32::from_rgb(220, 160, 80),
                    );
                    tile_plot(
                        ui,
                        "Speed (m/s)",
                        side,
                        &ts,
                        self.messages.iter().map(|(_, m)| m.speed_over_ground_meters_per_second).collect::<Vec<_>>(),
                        egui::Color32::from_rgb(200, 100, 140),
                    );
                    tile_plot(
                        ui,
                        "Altitude (m)",
                        side,
                        &ts,
                        self.messages.iter().map(|(_, m)| m.altitude_meters).collect::<Vec<_>>(),
                        egui::Color32::from_rgb(140, 100, 200),
                    );
                    tile_plot(
                        ui,
                        "Satellites",
                        side,
                        &ts,
                        self.messages.iter().map(|(_, m)| m.satellites as f64).collect::<Vec<_>>(),
                        egui::Color32::from_rgb(100, 200, 200),
                    );
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let path: Vec<(f64, f64)> = self
                .messages
                .iter()
                .map(|(_, m)| (m.longitude_degrees, m.latitude_degrees))
                .collect();
            let latest = self.messages.last().map(|(_, m)| (m.longitude_degrees, m.latitude_degrees));
            let (center_lon, center_lat) = latest.unwrap_or((21.0122, 52.2297));
            let plugin = GpsMarkerPlugin {
                path,
                latest,
            };
            ui.add_sized(
                ui.available_size(),
                walkers::Map::new(
                    Some(&mut self.tiles),
                    &mut self.map_memory,
                    walkers::Position::from_lon_lat(center_lon, center_lat),
                )
                .with_plugin(plugin),
            );
        });

        ctx.request_repaint();
    }
}
