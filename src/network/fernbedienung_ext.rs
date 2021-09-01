use std::{pin::Pin, task::{Context, Poll}};
use bytes::Bytes;
use futures::{Stream, TryFutureExt};
use tokio::sync::oneshot;

use super::fernbedienung;

// TO READ: https://carllerche.com/2021/06/17/six-ways-to-make-async-rust-easier/
// TO READ: https://rust-lang.github.io/wg-async-foundations/vision.html

#[pin_project::pin_project(PinnedDrop)]
pub struct MjpegStreamerStream<'dev, S> {
    terminate_tx: Option<oneshot::Sender<()>>,
    device: &'dev fernbedienung::Device,
    #[pin]
    stream: S
}

#[pin_project::pinned_drop]
impl<S> PinnedDrop for MjpegStreamerStream<'_, S> {
    fn drop(self: Pin<&mut Self>) {
        if let Some(terminate_tx) = self.project().terminate_tx.take() {
            terminate_tx.send(());
        }
    }
}

impl MjpegStreamerStream<'_, ()> {
    pub fn new<'dev>(
        device: &'dev fernbedienung::Device,
        camera: &str,
        width: u16,
        height: u16,
        port: u16
    ) -> impl Stream<Item = reqwest::Result<Bytes>> + 'dev {
        let mjpg_streamer = fernbedienung::Process {
            target: "mjpg_streamer".into(),
            working_dir: None,
            args: vec![
                "-i".to_owned(),
                format!("input_uvc.so -d {} -r {}x{} -n", camera, width, height),
                "-o".to_owned(),
                format!("output_http.so -p {} -l {}", port, device.addr)
            ],
        };
        let (terminate_tx, terminate_rx) = oneshot::channel::<()>();
        let mjpg_streamer = device.run(mjpg_streamer, Some(terminate_rx), None, None, None);
        let source = format!("http://{}:{}/?action=snapshot", device.addr, port);
        MjpegStreamerStream {
            device, terminate_tx: Some(terminate_tx), stream: async_stream::stream! {
                tokio::pin!(mjpg_streamer);
                loop {
                    tokio::select! {
                        
                        item = reqwest::get(&source).and_then(|response| response.bytes()) => {
                            yield item;
                        }
                        _ = &mut mjpg_streamer => break,
                    }
                }
            }
        }
    }
}

impl<S: futures::Stream> Stream for MjpegStreamerStream<'_, S>  {
    type Item = S::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().stream.poll_next(cx)
    }
}