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
    let key: [u8; 32] = [0u8; 32];

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
            thread::spawn(move || run_serial_loop(key_for_serial, Some(tx)))
        };
        let native_options = eframe::NativeOptions::default();
        let rx = std::sync::Mutex::new(rx);
        eframe::run_native(
            "LoRa listener",
            native_options,
            Box::new(move |_cc| Ok(Box::new(LoraGuiApp::new(rx)))),
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

    run_serial_loop(key, None);
    Ok(())
}

/// Serial receive loop. When `sender` is Some, pushes (timestamp, msg) to the channel; otherwise prints via on_message.
fn run_serial_loop(key: [u8; 32], sender: Option<std::sync::mpsc::Sender<(f64, RadioMsg)>>) {
    let ports = serialport::available_ports().expect("Failed to list serial ports");
    let port = ports.first().expect(
        "No serial ports found. Plug in a LoRa serial device (e.g. USB-UART). Use --mock for testing without hardware.",
    );

    println!("Opening {}", port.port_name);
    let port = serialport::new(&port.port_name, 9600)
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
                        if let Some(ref s) = sender {
                            let _ = s.send((timestamp, msg));
                        } else {
                            on_message(timestamp, &msg);
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

    let frequency = 868;
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
}

#[cfg(feature = "gui")]
impl LoraGuiApp {
    fn new(rx: std::sync::Mutex<std::sync::mpsc::Receiver<(f64, RadioMsg)>>) -> Self {
        Self {
            rx,
            messages: Vec::new(),
        }
    }
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

        egui::TopBottomPanel::top("telemetry").show(ctx, |ui| {
            ui.heading("LoRa receiver");
            if let Some((ts, msg)) = self.messages.last() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Lat: {:.6}°", msg.latitude_degrees));
                    ui.label(format!("Lon: {:.6}°", msg.longitude_degrees));
                    ui.label(format!("Course: {:.1}°", msg.course_over_ground_degrees));
                    ui.label(format!("Speed: {:.2} m/s", msg.speed_over_ground_meters_per_second));
                    ui.label(format!("Alt: {:.1} m", msg.altitude_meters));
                    ui.label(format!("Sats: {}", msg.satellites));
                    ui.label(format!("Time: {:.1}", ts));
                });
            } else {
                ui.label("Waiting for first message…");
            }
            ui.label(format!("Messages: {}", self.messages.len()));
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.messages.is_empty() {
                ui.label("No track data yet.");
                return;
            }

            let points: Vec<[f64; 2]> = self
                .messages
                .iter()
                .map(|(_, m)| [m.longitude_degrees, m.latitude_degrees])
                .collect();

            let plot = egui_plot::Plot::new("track")
                .view_aspect(1.0)
                .label_formatter(|_name, value| {
                    format!("Lon: {:.4}° Lat: {:.4}°", value.x, value.y)
                })
                .include_x(points.iter().map(|p| p[0]).min_by(f64::total_cmp).unwrap_or(0.0) - 0.001)
                .include_x(points.iter().map(|p| p[0]).max_by(f64::total_cmp).unwrap_or(0.0) + 0.001)
                .include_y(points.iter().map(|p| p[1]).min_by(f64::total_cmp).unwrap_or(0.0) - 0.001)
                .include_y(points.iter().map(|p| p[1]).max_by(f64::total_cmp).unwrap_or(0.0) + 0.001);

            plot.show(ui, |plot_ui| {
                plot_ui.points(
                    egui_plot::Points::new(points)
                        .radius(2.0)
                        .color(egui::Color32::from_rgb(80, 180, 240)),
                );
            });
        });

        ctx.request_repaint();
    }
}
