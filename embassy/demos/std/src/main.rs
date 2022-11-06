#![feature(type_alias_impl_trait)]

#[allow(unused_imports)]
use {
    log::{debug, error, info, log, trace, warn},
};

use core::future::Future;

use core::todo;
use embassy_executor::{Spawner, Executor};
use embassy_sync::mutex::Mutex;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Stack, StackResources, ConfigStrategy};
use embassy_futures::join::join3;
use embedded_io::asynch::{Read, Write};
use static_cell::StaticCell;

use core::str::FromStr;
use core::cell::RefCell;
use core::num::NonZeroU32;
use futures::{task::noop_waker_ref,pending};
use core::task::{Context,Poll,Waker,RawWaker,RawWakerVTable};

use rand::rngs::OsRng;
use rand::RngCore;

use sunset::*;
use sunset_embassy::SSHServer;

use crate::tuntap::TunTapDevice;

mod tuntap;

const NUM_LISTENERS: usize = 4;

macro_rules! singleton {
    ($val:expr) => {{
        type T = impl Sized;
        static STATIC_CELL: StaticCell<T> = StaticCell::new();
        STATIC_CELL.init_with(move || $val)
    }};
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<TunTapDevice>) -> ! {
    stack.run().await
}

#[embassy_executor::task]
async fn main_task(spawner: Spawner) {
    info!("Hello World!");

    // TODO config
    let opt_tap0 = "tap0";
    let config = ConfigStrategy::Dhcp;

    // Init network device
    let device = TunTapDevice::new(opt_tap0).unwrap();

    let seed = OsRng.next_u64();

    // Init network stack
    let stack = &*singleton!(Stack::new(
        device,
        config,
        singleton!(StackResources::<1, 10, 8>::new()),
        seed
    ));

    // Launch network task
    spawner.spawn(net_task(stack)).unwrap();

    for _ in 0..NUM_LISTENERS {
        spawner.spawn(listener(stack)).unwrap();
    }
}

// TODO: pool_size should be NUM_LISTENERS but needs a literal
#[embassy_executor::task(pool_size = 4)]
async fn listener(stack: &'static Stack<TunTapDevice>) -> ! {
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(embassy_net::SmolDuration::from_secs(10)));

        info!("Listening on TCP:22...");
        if let Err(e) = socket.accept(22).await {
            warn!("accept error: {:?}", e);
            continue;
        }

        let r = session(&mut socket).await;
        if let Err(_e) = r {
            // warn!("Ended with error: {:?}", e);
            warn!("Ended with error");
        }
    }
}

struct DemoServer {
    keys: [SignKey; 1],

    sess: Option<u32>,
    want_shell: bool,
    shell_started: bool,
}

impl DemoServer {
    fn new() -> Result<Self> {

        let keys = [SignKey::generate(KeyType::Ed25519)?];

        Ok(Self {
            sess: None,
            keys,
            want_shell: false,
            shell_started: false,
        })
    }
}

impl ServBehaviour for DemoServer {
    fn hostkeys(&mut self) -> BhResult<&[SignKey]> {
        Ok(&self.keys)
    }


    fn have_auth_password(&self, user: TextString) -> bool {
        true
    }

    fn have_auth_pubkey(&self, user: TextString) -> bool {
        false
    }

    fn auth_unchallenged(&mut self, username: TextString) -> bool {
        info!("Allowing auth for user {:?}", username.as_str());
        true
    }

    fn auth_password(&mut self, user: TextString, password: TextString) -> bool {
        user.as_str().unwrap_or("") == "matt" && password.as_str().unwrap_or("") == "pw"
    }

    // fn auth_pubkey(&mut self, user: TextString, pubkey: &PubKey) -> bool {
    //     if user.as_str().unwrap_or("") != "matt" {
    //         return false
    //     }

    //     // key is tested1
    //     pubkey.matches_openssh("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMkNdReJERy1rPGqdfTN73TnayPR+lTNhdZvOgkAOs5x")
    //     .unwrap_or_else(|e| {
    //         warn!("Failed loading openssh key: {e}");
    //         false
    //     })
    // }

    fn open_session(&mut self, chan: u32) -> ChanOpened {
        if self.sess.is_some() {
            ChanOpened::Failure(ChanFail::SSH_OPEN_ADMINISTRATIVELY_PROHIBITED)
        } else {
            self.sess = Some(chan);
            ChanOpened::Success
        }
    }

    fn sess_shell(&mut self, chan: u32) -> bool {
        let r = !self.want_shell && self.sess == Some(chan);
        self.want_shell = true;
        trace!("req want shell");
        r
    }

    fn sess_pty(&mut self, chan: u32, _pty: &Pty) -> bool {
        self.sess == Some(chan)
    }
}

async fn session(socket: &mut TcpSocket<'_>) -> sunset::Result<()> {
    let mut app = DemoServer::new()?;

    let mut ssh_rxbuf = [0; 2000];
    let mut ssh_txbuf = [0; 2000];
    let serv = SSHServer::new(&mut ssh_rxbuf, &mut ssh_txbuf, &mut app)?;
    let serv = &serv;

    let app = Mutex::<NoopRawMutex, _>::new(app);

    serv.run(socket, &app).await
}

static EXECUTOR: StaticCell<Executor> = StaticCell::new();

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Trace)
        .filter_module("async_io", log::LevelFilter::Info)
        .filter_module("polling", log::LevelFilter::Info)
        .format_timestamp_nanos()
        .init();

    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(main_task(spawner)).unwrap();
    });
}