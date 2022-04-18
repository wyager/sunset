#[allow(unused_imports)]
use {
    // crate::error::Error,
    log::{debug, error, info, log, trace, warn},
};
use std::error::Error;
use pretty_hex::PrettyHex;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use door_sshproto::*;

use simplelog::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {

    CombinedLogger::init(
    vec![
        TermLogger::new(LevelFilter::Trace, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
    ]
    ).unwrap();

    info!("running main");
    trace!("tracing main");

    // Connect to a peer
    let mut stream = TcpStream::connect("dropbear.nl:22").await?;

    let mut work = vec![0; 1000];
    let c = conn::Conn::new();
    let mut r = conn::Runner::new(c, work.as_mut_slice());

    let mut buf = vec![0; 100];
    loop {
        stream.readable().await?;
        let n = stream.try_read(&mut buf)?;
        let s = &buf.as_slice()[..n];
        let l = r.input(s)?;
        println!("read {l}");
    }


    // let mut d = ident::RemoteVersion::new();
    // let (taken, done) = d.consume(&buf)?;
    // println!("taken {taken} done {done}");
    // let v = d.version();
    // match v {
    //     Some(x) => {
    //         println!("v {:?}", x.hex_dump());
    //     }
    //     None => {
    //         println!("None");
    //     }
    // }
    // let (_, rest) = buf.split_at(taken + 5);
    // println!("reset {:?}", rest.hex_dump());

    // let ctx = packets::ParseContext::new();
    // let p = wireformat::packet_from_bytes(rest, &ctx)?;
    // println!("{p:#?}");

    Ok(())
}
