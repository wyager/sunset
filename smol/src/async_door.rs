#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use pin_utils::pin_mut;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use std::io::Error as IoError;
use std::io::ErrorKind;

use std::sync::{Arc, Mutex, MutexGuard};

use parking_lot::lock_api::ArcMutexGuard;
use parking_lot::Mutex as ParkingLotMutex;

// TODO
use anyhow::{anyhow, Context as _, Error, Result};
use core::ops::DerefMut;

use door::{Behaviour, Runner};
use door_sshproto as door;
use door_sshproto::error::Error as DoorError;
// use door_sshproto::client::*;
use async_trait::async_trait;

use pretty_hex::PrettyHex;

pub struct Inner<'a> {
    runner: Runner<'a>,
    // TODO: perhaps behaviour can move to runner? unsure of lifetimes.
    behaviour: Behaviour<'a>,
}

pub struct AsyncDoor<'a> {
    inner: Arc<ParkingLotMutex<Inner<'a>>>,
    out_progress_fut:
        Option<Pin<Box<dyn Future<Output = Result<(), DoorError>> + 'a>>>,
}

impl Clone for AsyncDoor<'_> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), out_progress_fut: None }
    }
}

impl<'a> AsyncDoor<'a> {
    pub fn new(runner: Runner<'a>, behaviour: Behaviour<'a>) -> Self {
        let inner = Inner { runner, behaviour };
        Self { inner: Arc::new(ParkingLotMutex::new(inner)), out_progress_fut: None }
    }

    pub fn with_behaviour<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Behaviour<'a>) -> R,
    {
        f(&mut self.lock().behaviour)
    }

    fn lock(&self) -> parking_lot::MutexGuard<Inner<'a>> {
        self.inner.lock()
    }
}

impl<'a> AsyncRead for AsyncDoor<'a> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<Result<(), IoError>> {
        trace!("poll_read");

        let r = if let Some(f) = self.out_progress_fut.as_mut() {
            f.as_mut().poll(cx)
            .map_err(|e| IoError::new(ErrorKind::Other, e))
        } else {
            let mut inner = ParkingLotMutex::lock_arc(&self.inner);
            // TODO: should this be conditional on the result of the poll?
            inner.runner.set_output_waker(cx.waker().clone());
            // async move block to capture `inner`
            let mut b = Box::pin(async move {
                let inner = inner.deref_mut();
                inner.runner.out_progress(&mut inner.behaviour).await
            });
            // let mut b = Box::pin(guard_wait(inner));
            let r = b.as_mut().poll(cx);
            if let Poll::Pending = r {
                self.out_progress_fut = Some(b);
            }
            r.map_err(|e| IoError::new(ErrorKind::Other, e))
        }?;
        if let Poll::Pending = r {
            return Poll::Pending;
        } else {
            self.out_progress_fut = None
        }

        let runner = &mut self.lock().runner;

        let b = buf.initialize_unfilled();
        let r = runner.output(b).map_err(|e| IoError::new(ErrorKind::Other, e));

        trace!("runner output {r:?}");
        let r = match r {
            // sz=0 means EOF
            Ok(0) => Poll::Pending,
            Ok(sz) => {
                trace!("{:?}", (&b[..sz]).hex_dump());
                buf.advance(sz);
                Poll::Ready(Ok(()))
            }
            Err(e) => Poll::Ready(Err(e)),
        };
        info!("finish poll_read {r:?}");
        r
    }
}

impl<'a> AsyncWrite for AsyncDoor<'a> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, IoError>> {
        trace!("poll_write");
        let runner =
            &mut self.lock().runner;
        // trace!("poll_write got lock");
        // trace!("write size {}", buf.len());
        runner.set_input_waker(cx.waker().clone());
        // TODO: should runner just have poll_write/poll_read?
        // TODO: is ready_input necessary? .input() should return size=0
        // if nothing is consumed. Or .input() could return a Poll<Result<usize>>
        let r = if runner.ready_input() {
            let r = runner
                .input(buf)
                .map_err(|e| IoError::new(std::io::ErrorKind::Other, e));
            Poll::Ready(r)
        } else {
            Poll::Pending
        };
        trace!("poll_write {r:?}");
        r
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<(), IoError>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<(), IoError>> {
        todo!("poll_close")
    }
}