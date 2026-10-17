// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Every Lua allocation in the engines goes through the global allocator (mlua
// hands LuaJIT a Rust-backed allocator); the system heap is slower for that
// and slow to give memory back after a scan.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The AppImage carries Ubuntu 22.04's libwayland ahead of the host's on the
/// library path, and a newer Mesa refuses to create an EGL display against it
/// (tauri-apps/tauri#15976). Until the bundler can leave those libraries out,
/// restart once with the host's copies preloaded. POB_REDUX_NO_PRELOAD=1 skips
/// this.
#[cfg(target_os = "linux")]
fn preload_host_wayland() {
    use std::os::unix::process::CommandExt;
    use std::path::Path;

    if std::env::var_os("APPIMAGE").is_none()
        || std::env::var_os("POB_REDUX_PRELOADED").is_some()
        || std::env::var_os("POB_REDUX_NO_PRELOAD").is_some()
    {
        return;
    }
    const DIRS: [&str; 6] = [
        "/usr/lib64",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib",
        "/lib64",
        "/lib/x86_64-linux-gnu",
    ];
    const LIBS: [&str; 4] = [
        "libwayland-client.so.0",
        "libwayland-cursor.so.0",
        "libwayland-egl.so.1",
        "libwayland-server.so.0",
    ];
    let mut found: Vec<String> = Vec::new();
    for lib in LIBS {
        if let Some(path) = DIRS.iter().map(|d| Path::new(d).join(lib)).find(|p| p.is_file()) {
            found.push(path.to_string_lossy().into_owned());
        }
    }
    if found.is_empty() {
        return;
    }
    let mut preload = found.join(":");
    if let Some(existing) = std::env::var_os("LD_PRELOAD") {
        let existing = existing.to_string_lossy();
        if !existing.is_empty() {
            preload = format!("{preload}:{existing}");
        }
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let err = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env("LD_PRELOAD", preload)
        .env("POB_REDUX_PRELOADED", "1")
        .exec();
    eprintln!("could not restart with the host's wayland libraries preloaded: {err}");
}

fn main() {
    #[cfg(target_os = "linux")]
    {
        preload_host_wayland();
        // WebKitGTK's DMA-BUF path paints a black window from the AppImage's bundled WebKit and on NVIDIA's driver.
        let dmabuf_breaks = std::env::var_os("APPIMAGE").is_some() || std::path::Path::new("/sys/module/nvidia").exists();
        if dmabuf_breaks
            && std::env::var_os("POB_REDUX_GPU").is_none()
            && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
        {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
    pob_redux_lib::run()
}
