use std::sync::{Arc, Mutex};

use wayland_client::{
    globals::GlobalList,
    protocol::{wl_buffer::WlBuffer, wl_output, wl_shm},
    Dispatch, QueueHandle,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{Event, Flags, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use crate::globals::GlobalData;

// zwlr_screencopy_manager_v1

pub trait WlrScreencopyHandler: Sized {
    fn wlr_screencopy_state(&mut self) -> &mut WlrScreencopyState;
}

#[derive(Debug)]
pub struct WlrScreencopyState {
    manager: ZwlrScreencopyManagerV1,
    frames: Vec<Arc<Mutex<WlrScreencopyFrameInner>>>,
}

impl WlrScreencopyState {
    pub fn new<
        D: Dispatch<ZwlrScreencopyManagerV1, GlobalData> + WlrScreencopyHandler + 'static,
    >(
        global_list: &GlobalList,
        qh: &QueueHandle<D>,
    ) -> Self {
        let manager =
            global_list.bind::<ZwlrScreencopyManagerV1, _, _>(qh, 1..=3, GlobalData).unwrap();
        WlrScreencopyState { manager, frames: vec![] }
    }

    pub fn capture_output<D>(
        &mut self,
        output: &wl_output::WlOutput,
        qh: &QueueHandle<D>,
    ) -> WlrScreencopyFrame
    where
        D: Dispatch<ZwlrScreencopyFrameV1, GlobalData> + WlrScreencopyHandler + 'static,
    {
        let frame = self.manager.capture_output(0, output, qh, GlobalData);
        let inner = Arc::new(Mutex::new(WlrScreencopyFrameInner {
            frame,
            buffers: vec![],
            buffers_done: false,
            flags: None,
            status: FrameStatus::NotReady,
        }));
        self.frames.push(inner.clone());
        WlrScreencopyFrame { inner }
    }
}

impl<D> Dispatch<ZwlrScreencopyManagerV1, GlobalData, D> for WlrScreencopyState
where
    D: Dispatch<ZwlrScreencopyManagerV1, GlobalData> + WlrScreencopyHandler + 'static,
{
    fn event(
        _state: &mut D,
        _proxy: &ZwlrScreencopyManagerV1,
        _event: <ZwlrScreencopyManagerV1 as wayland_client::Proxy>::Event,
        _data: &GlobalData,
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<D>,
    ) {
        unreachable!("zwlr_screencopy_manager_v1 has no events");
    }
}

#[macro_export]
macro_rules! delegate_wlr_screencopy {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        $crate::reexports::client::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty:
            [
                $crate::reexports::protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1: $crate::globals::GlobalData
            ] => $crate::wlr_screencopy::WlrScreencopyState);
        $crate::reexports::client::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty:
            [
                $crate::reexports::protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1: $crate::globals::GlobalData
            ] => $crate::wlr_screencopy::WlrScreencopyState);
    };
}

// zwlr_screencopy_frame_v1

impl<D> Dispatch<ZwlrScreencopyFrameV1, GlobalData, D> for WlrScreencopyState
where
    D: Dispatch<ZwlrScreencopyFrameV1, GlobalData, D> + WlrScreencopyHandler + 'static,
{
    fn event(
        state: &mut D,
        proxy: &ZwlrScreencopyFrameV1,
        event: <ZwlrScreencopyFrameV1 as wayland_client::Proxy>::Event,
        _data: &GlobalData,
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<D>,
    ) {
        let mut inner = state
            .wlr_screencopy_state()
            .frames
            .iter_mut()
            .find(|f| {
                let f = f.lock().unwrap();
                &f.frame == proxy
            })
            .unwrap()
            .lock()
            .unwrap();

        match event {
            Event::Buffer { format, width, height, stride } => {
                let buffer = BufferType::WlShm {
                    format: format.into_result().unwrap(),
                    width,
                    height,
                    stride,
                };
                inner.buffers.push(buffer);
            }
            Event::LinuxDmabuf { format, width, height } => {
                let buffer = BufferType::LinuxDmabuf { format, width, height };
                inner.buffers.push(buffer);
            }
            Event::BufferDone => inner.buffers_done = true,
            Event::Flags { flags } => {
                inner.flags = Some(flags.into_result().unwrap());
            }
            Event::Damage { .. } => todo!(),
            Event::Ready { tv_sec_hi, tv_sec_lo, tv_nsec } => {
                inner.status = FrameStatus::Ready((tv_sec_hi, tv_sec_lo, tv_nsec));
            }
            Event::Failed => inner.status = FrameStatus::Failed,
            _ => (),
        }
    }
}

#[derive(Debug)]
pub struct WlrScreencopyFrame {
    inner: Arc<Mutex<WlrScreencopyFrameInner>>,
}

impl WlrScreencopyFrame {
    pub fn copy(&self, buffer: &WlBuffer) {
        let inner = self.inner.lock().unwrap();
        inner.frame.copy(buffer);
    }

    pub fn buffer_types(&self) -> Vec<BufferType> {
        let inner = self.inner.lock().unwrap();
        if !inner.buffers_done {
            return vec![];
        }
        inner.buffers.clone()
    }

    pub fn status(&self) -> FrameStatus {
        let inner = self.inner.lock().unwrap();
        inner.status.clone()
    }
}

#[derive(Debug, Clone)]
pub enum FrameStatus {
    NotReady,
    Failed,
    Ready((u32, u32, u32)),
}

#[derive(Debug, Clone)]
pub enum BufferType {
    WlShm { format: wl_shm::Format, width: u32, height: u32, stride: u32 },
    LinuxDmabuf { format: u32, width: u32, height: u32 },
}

#[derive(Debug)]
struct WlrScreencopyFrameInner {
    frame: ZwlrScreencopyFrameV1,
    buffers: Vec<BufferType>,
    buffers_done: bool,
    flags: Option<Flags>,
    status: FrameStatus,
}
