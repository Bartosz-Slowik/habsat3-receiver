use std::io;
use std::io::Read;
use std::io::Write;
use std::{error::Error, time::Duration};

use common::RadioMsg;
use serialport::SerialPort;

fn main() -> Result<(), Box<dyn Error>> {
    // Hardcoded key for testing (use LORA_ENCRYPTION_KEY env in production)
    let key: [u8; 32] = [0u8; 32];

    let ports = serialport::available_ports().expect("Failed to list serial ports");
    let port = ports.first().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No serial ports found. Plug in a LoRa serial device (e.g. USB-UART).",
        )
    })?;

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
                Ok(msg) => {
                    if let Some((timestamp, msg)) = RadioMsg::decrypt(&msg, &key) {
                        println!("{timestamp}: {msg:?}")
                    }
                }
            }
        };
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
