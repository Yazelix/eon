use crate::Result;
use orbit_protocol::session::{
    self, ClientMessage, KeyAction, KeyEvent, Modifiers, PhysicalKey, ServerMessage,
};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
};

pub(super) fn run(socket: &Path) -> Result<i32> {
    let stream = UnixStream::connect(socket)?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    write_message(&mut writer, &ClientMessage::Hello)?;
    loop {
        let message = read_message(&mut reader)?.ok_or("server closed during attachment")?;
        print_message(&message);
        match message {
            ServerMessage::Attached => break,
            ServerMessage::Busy => return Ok(2),
            ServerMessage::Failure(_) => return Ok(4),
            _ => {}
        }
    }

    for line in io::stdin().lock().lines() {
        write_message(&mut writer, &ClientMessage::Paste(line?.into_bytes()))?;
        write_message(
            &mut writer,
            &ClientMessage::Key(KeyEvent {
                action: KeyAction::Press,
                key: PhysicalKey::ENTER,
                modifiers: Modifiers::empty(),
                consumed_modifiers: Modifiers::empty(),
                composing: false,
                text: None,
                unshifted_codepoint: None,
            }),
        )?;
        let mut accepted = 0;
        while accepted < 2 {
            let Some(message) = read_message(&mut reader)? else {
                return Ok(0);
            };
            print_message(&message);
            match message {
                ServerMessage::Accepted => accepted += 1,
                ServerMessage::Failure(_) => return Ok(4),
                ServerMessage::Exited { code } => return Ok(code),
                _ => {}
            }
        }
    }
    Ok(0)
}

pub(crate) fn write_message(writer: &mut impl Write, message: &ClientMessage) -> Result {
    writer.write_all(&session::encode_client_message(message)?)?;
    Ok(())
}

pub(crate) fn read_message(reader: &mut impl Read) -> Result<Option<ServerMessage>> {
    let mut header = [0; session::HEADER_BYTES];
    if reader.read(&mut header[..1])? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut header[1..])?;
    let length =
        session::server_message_len(&header)?.expect("complete header has a server-message length");
    let mut message = Vec::with_capacity(length);
    message.extend_from_slice(&header);
    message.resize(length, 0);
    reader.read_exact(&mut message[session::HEADER_BYTES..])?;
    Ok(Some(session::decode_server_message(&message)?))
}

fn print_message(message: &ServerMessage) {
    match message {
        ServerMessage::Frame(frame) => println!(
            "FRAME revision={} size={}x{} title={:?}",
            frame.revision, frame.dimensions.cols, frame.dimensions.rows, frame.title
        ),
        message => println!("{message:?}"),
    }
}
