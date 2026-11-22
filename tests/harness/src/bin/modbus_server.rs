//! A Modbus TCP server for trials by hand of `examples/modbus_tcp` (docs/io.md, sections 6 and 10), with no Modbus crate:
//! `cargo run -p labcraft-harness --bin modbus_server [ADDRESS]` listens on ADDRESS, `127.0.0.1:5020` by default (502 needs
//! privileges under Linux), and answers function 3, read holding registers, for any unit. Its registers are those of the
//! simulated server of io.md 6: 230 at 0, 25 at 1, then 0 up to 99. Other functions give exception 1 (illegal function), a
//! register past 99 exception 2 (illegal data address), a count of 0 or above 125 exception 3 (illegal data value), as the
//! Modbus application protocol says. Each connection is served on a thread of its own, until the client closes it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::ExitCode;

const REGISTERS: usize = 100;

fn register(address: usize) -> u16 {
    match address {
        0 => 230,
        1 => 25,
        _ => 0,
    }
}

/// The response to the PDU of one request (its function code and what follows), without the MBAP header.
fn answer(pdu: &[u8]) -> Vec<u8> {
    let exception = |function: u8, code: u8| vec![function | 0x80, code];
    let Some(&function) = pdu.first() else {
        return exception(0, 1);
    };
    if function != 3 {
        return exception(function, 1);
    }
    let [_, first_high, first_low, count_high, count_low] = pdu else {
        return exception(function, 3);
    };
    let first = usize::from(u16::from_be_bytes([*first_high, *first_low]));
    let count = usize::from(u16::from_be_bytes([*count_high, *count_low]));
    if count == 0 || count > 125 {
        return exception(function, 3);
    }
    if first + count > REGISTERS {
        return exception(function, 2);
    }
    let mut response = vec![function, (count * 2) as u8];
    for address in first..first + count {
        response.extend_from_slice(&register(address).to_be_bytes());
    }
    response
}

/// The whole response to one request: the MBAP header of the request (transaction, protocol, unit) with the length of
/// the response, then its PDU.
fn respond(header: [u8; 7], pdu: &[u8]) -> Vec<u8> {
    let body = answer(pdu);
    let length = u16::try_from(body.len() + 1).unwrap_or(u16::MAX).to_be_bytes();
    [&header[..4], &length[..], &header[6..], &body[..]].concat()
}

/// Serves one connection: a header of 7 bytes, then the rest of the request by its length, until the client closes.
fn serve(mut stream: TcpStream) -> std::io::Result<()> {
    let peer = stream.peer_addr()?;
    loop {
        let mut header = [0u8; 7];
        if let Err(error) = stream.read_exact(&mut header) {
            if error.kind() != std::io::ErrorKind::UnexpectedEof {
                return Err(error);
            }
            eprintln!("{peer}: closed");
            return Ok(());
        }
        let length = usize::from(u16::from_be_bytes([header[4], header[5]]));
        let mut pdu = vec![0u8; length.saturating_sub(1)];
        stream.read_exact(&mut pdu)?;
        let response = respond(header, &pdu);
        eprintln!("{peer}: {}{} -> {}", hex(&header), hex(&pdu), hex(&response));
        stream.write_all(&response)?;
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn main() -> ExitCode {
    let address = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:5020".to_owned());
    let listener = match TcpListener::bind(&address) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("modbus_server: {address}: {error}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("modbus_server: listening on {address}; Ctrl-C stops it");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                std::thread::spawn(move || {
                    if let Err(error) = serve(stream) {
                        eprintln!("modbus_server: {error}");
                    }
                });
            }
            Err(error) => eprintln!("modbus_server: {error}"),
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::respond;

    fn bytes(hex: &str) -> Vec<u8> {
        let digits: Vec<u8> = hex.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect();
        digits.chunks(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII"), 16).expect("hexadecimal")).collect()
    }

    fn reply(request: &str) -> Vec<u8> {
        let request = bytes(request);
        let mut header = [0u8; 7];
        header.copy_from_slice(&request[..7]);
        respond(header, &request[7..])
    }

    /// The exchange of io.md 6, which the simulation file of `examples/modbus_tcp` holds too.
    #[test]
    fn it_answers_the_exchange_of_the_simulated_server() {
        assert_eq!(reply("0001 0000 0006 01 03 0000 0002"), bytes("0001 0000 0007 01 03 04 00e6 0019"));
    }

    /// By hand, from the Modbus application protocol: function code with its high bit set, then the exception code;
    /// the transaction and the unit of the request come back.
    #[test]
    fn it_gives_the_exceptions_of_function_3() {
        assert_eq!(reply("0007 0000 0006 11 03 0063 0002"), bytes("0007 0000 0003 11 83 02"), "registers 99 and 100");
        assert_eq!(reply("0001 0000 0006 01 03 0000 0000"), bytes("0001 0000 0003 01 83 03"), "a count of 0");
        assert_eq!(reply("0001 0000 0006 01 03 0000 007e"), bytes("0001 0000 0003 01 83 03"), "a count of 126");
        assert_eq!(reply("0001 0000 0006 01 04 0000 0002"), bytes("0001 0000 0003 01 84 01"), "function 4");
        assert_eq!(reply("0001 0000 0004 01 03 0000"), bytes("0001 0000 0003 01 83 03"), "a request too short");
    }

    #[test]
    fn it_reads_the_last_register() {
        assert_eq!(reply("0001 0000 0006 01 03 0063 0001"), bytes("0001 0000 0005 01 03 02 0000"));
    }
}
