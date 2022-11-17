use embassy_sync::mutex::Mutex;
use embassy_sync::blocking_mutex::raw::{NoopRawMutex, RawMutex};
use embassy_net::tcp::TcpSocket;

use sunset::*;

use crate::*;
use embassy_sunset::EmbassySunset;

pub struct SSHServer<'a> {
    sunset: EmbassySunset<'a>,
}

impl<'a> SSHServer<'a> {
    pub fn new(inbuf: &'a mut [u8], outbuf: &'a mut [u8],
        b: &mut (dyn ServBehaviour + Send),
        ) -> Result<Self> {
        let runner = Runner::new_server(inbuf, outbuf, b)?;
        let sunset = EmbassySunset::new(runner);
        Ok(Self { sunset })
    }

    pub async fn run<M, B: ?Sized>(&self, socket: &mut TcpSocket<'_>,
        b: &Mutex<M, B>) -> Result<()>
        where
            M: RawMutex,
            for<'f> Behaviour<'f>: From<&'f mut B>
    {
        self.sunset.run(socket, b).await
    }

    pub async fn read_channel(&self, ch: u32, ext: Option<u32>, buf: &mut [u8]) -> Result<usize> {
        self.sunset.read_channel(ch, ext, buf).await
    }

    pub async fn write_channel(&self, ch: u32, ext: Option<u32>, buf: &[u8]) -> Result<usize> {
        self.sunset.write_channel(ch, ext, buf).await
    }
}
