// S0b spike (#366): raw frames over a Channel, same tauri as Irori (2.12.5).
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};

pub struct RawFrame(pub Vec<u8>);
impl IpcResponse for RawFrame {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

fn now_us() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as f64
}

/// Frame layout: u32 seq, f64 send time (unix µs), then filler.
#[tauri::command]
async fn spike_frames(frames: Channel<RawFrame>, count: u32, size: u32, interval_ms: u32) -> Result<f64, String> {
    let t0 = Instant::now();
    for i in 0..count {
        let mut buf = vec![0x41u8; (size as usize).max(12)];
        buf[..4].copy_from_slice(&i.to_le_bytes());
        buf[4..12].copy_from_slice(&now_us().to_le_bytes());
        frames.send(RawFrame(buf)).map_err(|e| e.to_string())?;
        if interval_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(interval_ms as u64));
        }
    }
    Ok(t0.elapsed().as_micros() as f64)
}

#[tauri::command]
fn spike_report(app: tauri::AppHandle, text: String, done: bool) {
    println!("{text}");
    if done {
        app.exit(0);
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![spike_frames, spike_report])
        .run(tauri::generate_context!())
        .unwrap();
}
