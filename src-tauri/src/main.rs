#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ico::IconDir;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};
use std::fs::File;
use std::io::Cursor;
use std::{fs, path::Path, sync::Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
struct OpenedImage(Mutex<Option<String>>);
/// The live folder watcher. Held only to keep it alive — dropping a notify
/// watcher unregisters it — and replaced whenever a different folder is opened.
struct FolderWatcher(Mutex<Option<notify::RecommendedWatcher>>);
//use tauri::AppHandle;
//use tauri_plugin_dialog::DialogExt;
use std::path::PathBuf;

/* use serde::Serialize;
use std::time::UNIX_EPOCH; */


#[tauri::command]
fn get_opened_image(state: State<OpenedImage>) -> Option<String> {
    state.0.lock().unwrap().clone()
}

#[tauri::command]
fn set_opened_image(path: String, state: State<OpenedImage>) {
    // Every way of opening an image funnels through here, so it is also where
    // the jump list learns what was opened.
    #[cfg(target_os = "windows")]
    add_to_recent_docs(&path);

    *state.0.lock().unwrap() = Some(path);
}

// Everything the viewer will list in a folder. The raw and HEIF formats are
// decoded by Windows (see decode_with_wic), so they need the platform codec
// installed — the vendor's free Store codec for raw, HEVC for HEIC.
const RAW_EXTS: &[&str] = &[
    "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "dng", "orf", "rw2", "raf", "pef",
    "srw", "erf", "kdc", "dcr", "mrw", "3fr", "iiq", "mos", "rwl", "x3f",
];

const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "apng", "bmp", "gif", "webp", "ico", "avif", "cur", "tiff", "tif",
    "svg", "tga", "qoi", // HEIF family
    "heic", "heif", // GPU textures
    "dds", "ktx2", // decoded by their own crates
    "jxl", "psd", "psb", "exr", "hdr",
];

fn is_image_ext_str(ext: &str) -> bool {
    IMAGE_EXTS.contains(&ext) || RAW_EXTS.contains(&ext)
}

fn is_image_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| is_image_ext_str(&ext.to_lowercase()))
        .unwrap_or(false)
}

/// Lists the images beside `current_path` and says which one it is. Accepts a
/// folder too (dropped onto the window), in which case it lists that folder
/// and starts at the first image.
#[tauri::command]
fn get_folder_images(current_path: String) -> Result<(Vec<String>, usize), String> {
    let path = Path::new(&current_path);

    let dir = if path.is_dir() {
        path
    } else {
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    };

    let mut images: Vec<String> = fs::read_dir(dir)
        .map_err(|e| format!("Failed to read folder {}: {}", dir.display(), e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| is_image_ext(p))
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    #[cfg(target_os = "windows")]
    images.sort_by(|a, b| {
        let na = Path::new(a).file_name().unwrap().to_string_lossy();
        let nb = Path::new(b).file_name().unwrap().to_string_lossy();
        explorer_compare(&na, &nb)
    });

    let index = images.iter().position(|p| p == &current_path).unwrap_or(0);

    Ok((images, index))
}

/// True when an event changes *which* image files the folder holds.
///
/// A file's contents changing is deliberately ignored: the thumbnail cache is
/// keyed on mtime and size, and the viewer busts its asset URL on every load, so
/// a rewrite already shows up without rebuilding the list.
fn listing_changed(event: &notify::Event) -> bool {
    use notify::event::{CreateKind, ModifyKind, RemoveKind};
    use notify::EventKind;

    let kind_matters = matches!(
        event.kind,
        EventKind::Create(CreateKind::File | CreateKind::Any)
            | EventKind::Remove(RemoveKind::File | RemoveKind::Any)
            | EventKind::Modify(ModifyKind::Name(_))
    );

    // A rename reports both the old and the new name, so an image renamed to
    // something unsupported still counts as a change to the listing.
    kind_matters && event.paths.iter().any(|p| is_image_ext(p))
}

/// Watches the folder the open image lives in, so files appearing or vanishing
/// — a download finishing, a delete from Explorer — reach the viewer without a
/// manual refresh. Only ever one watcher: opening another folder replaces it.
#[tauri::command]
fn watch_folder(app: AppHandle, path: String, state: State<FolderWatcher>) -> Result<(), String> {
    use notify::{RecursiveMode, Watcher};

    let target = Path::new(&path);
    let dir = if target.is_dir() {
        target.to_path_buf()
    } else {
        target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };

    let handle = app.clone();

    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| listing_changed(&event)) {
            let _ = handle.emit("folder-changed", ());
        }
    })
    .map_err(|e| e.to_string())?;

    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .map_err(|e| e.to_string())?;

    *state.0.lock().unwrap() = Some(watcher);

    Ok(())
}

#[cfg(target_os = "windows")]
fn explorer_compare(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::StrCmpLogicalW;

    let wa: Vec<u16> = OsStr::new(a).encode_wide().chain(Some(0)).collect();
    let wb: Vec<u16> = OsStr::new(b).encode_wide().chain(Some(0)).collect();

    let result = unsafe { StrCmpLogicalW(wa.as_ptr(), wb.as_ptr()) };

    match result {
        x if x < 0 => Ordering::Less,
        x if x > 0 => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

// ----- WINDOWS SHELL INTEGRATION -----

/// Feeds Windows' own recent-documents list, which is what fills the app's
/// taskbar and Start-menu jump list. Building a custom jump list would mean
/// ICustomDestinationList; this gets the Recent and Pinned categories for free.
#[cfg(target_os = "windows")]
fn add_to_recent_docs(path: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{SHAddToRecentDocs, SHARD_PATHW};

    let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(Some(0)).collect();

    // Purely a nicety, and it has no failure worth reporting.
    unsafe { SHAddToRecentDocs(SHARD_PATHW as u32, wide.as_ptr().cast()) };
}

fn main() {
    //print_open_with_apps_for_test();

    tauri::Builder::default()
        // Must be registered first. A second launch (Explorer double-click,
        // "Open with") hands its argv to the process already running and exits,
        // so opening a file costs a decode instead of a WebView2 cold start.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(path) = argv.get(1) {
                *app.state::<OpenedImage>().0.lock().unwrap() = Some(path.clone());
                let _ = app.emit("opened-image", path);
            }

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(OpenedImage(Mutex::new(None)))
        .manage(FolderWatcher(Mutex::new(None)))
        .setup(|app| {
            let args: Vec<String> = std::env::args().collect();

            if args.len() > 1 {
                let path = args[1].clone();
                let state = app.state::<OpenedImage>();
                *state.0.lock().unwrap() = Some(path);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_opened_image,
            set_opened_image,
            get_folder_images,
            watch_folder,
            load_image,
            rotate_image,
            copy_image_to_clipboard,
            load_ico_frames,
            load_dds_frames,
            load_dds_frame,
            open_with,
            get_open_with_apps,
            open_with_app,
            open_with_dialog,
            open_native_print_dialog,
            open_url,
            trash_file,
            set_desktop_background,
            open_in_explorer,
            copy_file,
            load_image_metadata,
            file_stat,
            read_svg,
            save_svg,
            scan_qr_codes,
            read_text_in_image,
            get_thumbnail,
            compression_options,
            compress_estimate,
            compress_apply,
            convert_targets,
            convert_image,
            image_edit_capabilities,
            preview_adjustments,
            apply_adjustments,
            remove_metadata,
            rename_file,
            show_file_properties
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}

fn trusted_apps() -> Vec<&'static str> {
    vec![
        "mspaint.exe",
        "Fireworks.exe",
        "Photoshop.exe",
        "sdraw.exe",
        "Affinity.exe",
        "AffinityPhoto.exe",
        "AffinityDesigner.exe",
        "AffinityPublisher.exe",
        "C:\\Program Files\\Affinity\\Affinity\\Affinity.exe",
        "ImageJ.exe",
        "C:\\ImageJ\\ImageJ.exe",
        "xnview.exe",
        "FSViewer.exe",
        "i_view64.exe",
        "paintdotnet.exe",
        "gimp.exe",
        "nomacs.exe",
        "picasa.exe",
        "Honeyview.exe",
        "FastPictureViewer.exe",
        "acdsee.exe",
        "lightroom.exe",
        "voidImageViewer.exe",
        "C:\\Program Files\\voidImageViewer\\voidImageViewer.exe",
        "chrome.exe",
        "firefox.exe",
        "msedge.exe",
    ]
}

#[cfg(target_os = "windows")]
fn resolve_app_path(exe: &str) -> Option<String> {
    use std::path::PathBuf;
    use winreg::enums::*;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    let reg_path = format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{}",
        exe
    );

    for root in [hkcu, hklm] {
        if let Ok(key) = root.open_subkey(&reg_path) {
            if let Ok(p) = key.get_value::<String, _>("") {
                return Some(p);
            }
        }
    }

    // Fallback: System32
    let system32 = PathBuf::from(std::env::var("WINDIR").ok()?)
        .join("System32")
        .join(exe);

    if system32.exists() {
        return Some(system32.to_string_lossy().to_string());
    }

    None
}

#[cfg(target_os = "windows")]
fn exe_friendly_name(exe_path: &str) -> Option<String> {
    use std::path::Path;

    let exe_name = Path::new(exe_path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase());

    // --- hard overrides for known broken system apps ---
    if let Some(name) = exe_name.as_deref() {
        match name {
            "mspaint.exe" => return Some("Paint".to_string()),
            "photos.exe" => return Some("Photos".to_string()),
            _ => {}
        }
    }

    // ----- your existing code below (UNCHANGED) -----
    use std::ffi::OsStr;
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use winapi::shared::minwindef::{LPVOID, UINT};
    use winapi::um::winver::*;

    let wide: Vec<u16> = OsStr::new(exe_path).encode_wide().chain(once(0)).collect();

    unsafe {
        let mut handle = 0u32;
        let size = GetFileVersionInfoSizeW(wide.as_ptr(), &mut handle);

        if size == 0 {
            return Path::new(exe_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string());
        }

        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(wide.as_ptr(), 0, size, buf.as_mut_ptr() as LPVOID) == 0 {
            return None;
        }

        let language_codepages = vec![(0x0409, 0x04B0), (0x0409, 0x04E4), (0x0000, 0x04B0)];

        for &(lang, codepage) in &language_codepages {
            let key = format!(
                "\\StringFileInfo\\{:04x}{:04x}\\FileDescription\0",
                lang, codepage
            );

            let key_w: Vec<u16> = key.encode_utf16().collect();

            let mut ptr: LPVOID = std::ptr::null_mut();
            let mut len: UINT = 0;

            if VerQueryValueW(
                buf.as_mut_ptr() as LPVOID,
                key_w.as_ptr(),
                &mut ptr,
                &mut len,
            ) != 0
                && len > 0
            {
                let slice = std::slice::from_raw_parts(ptr as *const u16, len as usize);
                let name = String::from_utf16_lossy(slice);
                let trimmed = name.trim_end_matches('\0').to_string();

                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }

            let product_key = format!(
                "\\StringFileInfo\\{:04x}{:04x}\\ProductName\0",
                lang, codepage
            );

            let product_key_w: Vec<u16> = product_key.encode_utf16().collect();
            let mut product_ptr: LPVOID = std::ptr::null_mut();
            let mut product_len: UINT = 0;

            if VerQueryValueW(
                buf.as_mut_ptr() as LPVOID,
                product_key_w.as_ptr(),
                &mut product_ptr,
                &mut product_len,
            ) != 0
                && product_len > 0
            {
                let slice =
                    std::slice::from_raw_parts(product_ptr as *const u16, product_len as usize);
                let name = String::from_utf16_lossy(slice);
                let trimmed = name.trim_end_matches('\0').to_string();

                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
        }

        Path::new(exe_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    }
}

#[tauri::command]
fn get_open_with_apps(_path: String) -> Vec<(String, String, Option<Vec<u8>>)> {
    let mut apps = Vec::new();

    for exe in trusted_apps() {
        let is_uwp = exe.contains('!') && !exe.ends_with(".exe");

        if is_uwp {
            let label = match &exe[..] {
                "Microsoft.Windows.Photos_8wekyb3d8bbwe!App" => "Photos",
                "Microsoft.Paint_8wekyb3d8bbwe!App" => "Paint",
                _ => exe.split('!').next().unwrap_or(exe),
            };
            //println!("Adding UWP app: {} ({})", exe, label);
            apps.push((exe.to_string(), label.to_string(), None));
        } 
        else if let Some(full) = resolve_app_path(exe) {
            let label = exe_friendly_name(&full).unwrap_or_else(|| exe.replace(".exe", ""));
            //println!("Adding Win32 app: {} -> {} (path: {})", exe, label, full);
            let icon = extract_icon_fast(&full);
            apps.push((exe.to_string(), label, icon));
        } 
        else {
            //println!("App not found: {}", exe);
        }
    }

    apps
}

// The `image` crate's TIFF support is partial: old-style JPEG compression,
// libtiff-era LZW streams, CMYK and tiled layouts all fail to decode. Windows
// can read those, so fall back to WIC before giving up. Every decode in this
// file goes through here so no caller is left with the narrower support.
// Formats the `image` crate has no decoder for, each handled by its own crate.
// Tried before the generic path so their extensions never fall through to a
// "not supported" error.
fn decode_special_format(path: &str, ext: &str) -> Option<Result<DynamicImage, String>> {
    match ext {
        "jxl" => Some(decode_jxl(path)),
        "psd" | "psb" => Some(decode_psd(path)),
        // The image crate's DDS support stops at DXT1/3/5, so textures go
        // through the same block decoder the frame bar uses. Otherwise BC7
        // files would show in the viewer but fail for thumbnails and copying.
        "dds" | "ktx2" => Some(decode_texture(path)),
        _ if RAW_EXTS.contains(&ext) => Some(decode_raw(path)),
        _ => None,
    }
}

/// Largest mip of the first layer — what a texture "looks like" outside the
/// frame bar.
fn decode_texture(path: &str) -> Result<DynamicImage, String> {
    let surface = dds_surface(path)?;

    surface
        .get_image(0, 0, 0)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| "Texture has no readable image".to_string())
}

fn decode_jxl(path: &str) -> Result<DynamicImage, String> {
    let image = jxl_oxide::JxlImage::builder()
        .open(path)
        .map_err(|e| format!("JPEG XL: {e}"))?;

    let render = image.render_frame(0).map_err(|e| format!("JPEG XL: {e}"))?;
    let frame = render.image_all_channels();

    let (width, height) = (frame.width() as u32, frame.height() as u32);
    let channels = frame.channels();
    let samples = frame.buf();

    // jxl-oxide hands back normalised floats with 1..=4 channels (grey, grey+A,
    // RGB, RGBA); flatten whatever it is to RGBA8.
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);

    for px in samples.chunks_exact(channels) {
        match channels {
            1 => rgba.extend_from_slice(&[to_u8(px[0]), to_u8(px[0]), to_u8(px[0]), 255]),
            2 => rgba.extend_from_slice(&[to_u8(px[0]), to_u8(px[0]), to_u8(px[0]), to_u8(px[1])]),
            3 => rgba.extend_from_slice(&[to_u8(px[0]), to_u8(px[1]), to_u8(px[2]), 255]),
            _ => rgba.extend_from_slice(&[to_u8(px[0]), to_u8(px[1]), to_u8(px[2]), to_u8(px[3])]),
        }
    }

    RgbaImage::from_raw(width, height, rgba)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| "JPEG XL: unexpected frame size".to_string())
}

/// Maps a file read-only instead of copying it onto the heap first. Worth it for
/// the containers that have to be read whole before anything can be decoded — a
/// PSB or a KTX2 texture array runs to gigabytes, and only a fraction of those
/// bytes is usually touched.
///
/// The mapping tracks the file as it lives on disk, so another process
/// truncating it mid-decode would fault the process. That is the standing
/// trade every memory-mapping reader makes, and the file is open for a decode
/// that lasts milliseconds.
fn map_file(path: &str) -> Result<memmap2::Mmap, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;

    // SAFETY: see the note above — undefined behaviour requires another process
    // to shrink the file while this mapping is alive.
    unsafe { memmap2::Mmap::map(&file) }.map_err(|e| e.to_string())
}

fn decode_psd(path: &str) -> Result<DynamicImage, String> {
    let bytes = map_file(path)?;
    let psd = psd::Psd::from_bytes(&bytes).map_err(|e| format!("PSD: {e}"))?;

    // The flattened composite Photoshop saves with the file — the same thing
    // Explorer's preview shows. Individual layers are listed separately.
    RgbaImage::from_raw(psd.width(), psd.height(), psd.rgba())
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| "PSD: composite image is malformed".to_string())
}

/// Camera raw, decoded in Rust so no vendor codec has to be installed.
///
/// Three rungs, because no single decoder covers every camera:
///   1. imagepipe — full demosaic with its own colour pipeline.
///   2. rawler — knows newer formats imagepipe's rawloader doesn't (CR3).
///   3. rawler's embedded full-size preview — the JPEG the camera wrote, which
///      is what most viewers fall back to for a raw they can't develop.
/// decode_image tries WIC after all three.
fn decode_raw(path: &str) -> Result<DynamicImage, String> {
    use rawler::analyze::{extract_full_pixels, raw_to_srgb};
    use rawler::decoders::RawDecodeParams;

    let pipeline_err = match imagepipe::Pipeline::new_from_file(path) {
        Ok(mut pipeline) => match pipeline.output_8bit(None) {
            Ok(decoded) => {
                return image::RgbImage::from_raw(
                    decoded.width as u32,
                    decoded.height as u32,
                    decoded.data,
                )
                .map(DynamicImage::ImageRgb8)
                .ok_or_else(|| "RAW: unexpected image size".to_string())
            }
            Err(e) => e,
        },
        Err(e) => e,
    };

    // rawler is built against image 0.24 while this crate is on 0.25, so its
    // result is rebuilt from raw pixels rather than passed through (the two
    // DynamicImage types are unrelated as far as the compiler is concerned).
    fn rebuild(width: u32, height: u32, rgb: Vec<u8>) -> Result<DynamicImage, String> {
        image::RgbImage::from_raw(width, height, rgb)
            .map(DynamicImage::ImageRgb8)
            .ok_or_else(|| "RAW: unexpected image size".to_string())
    }

    if let Ok(img) = raw_to_srgb(path, RawDecodeParams::default()) {
        let rgb = img.to_rgb8();
        let (w, h) = (rgb.width(), rgb.height());
        return rebuild(w, h, rgb.into_raw());
    }

    if let Ok(img) = extract_full_pixels(path, RawDecodeParams::default()) {
        let rgb = img.to_rgb8();
        let (w, h) = (rgb.width(), rgb.height());
        return rebuild(w, h, rgb.into_raw());
    }

    Err(format!("RAW: {pipeline_err}"))
}

// PNG has no 32-bit-float colour type and the encoder refuses one outright,
// which is exactly what EXR and HDR decode to — so they reached the viewer as
// an encode error while their thumbnails worked, those being flattened by the
// resizer on the way through. Everything downstream is 8-bit anyway.
// ponytail: straight clamp, no tone mapping. Add Reinhard if HDR looks blown out.
fn flatten_float(img: DynamicImage) -> DynamicImage {
    if matches!(
        img,
        DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_)
    ) {
        return DynamicImage::ImageRgba8(img.to_rgba8());
    }

    img
}

fn decode_image(path: &str) -> Result<DynamicImage, String> {
    decode_image_raw(path).map(flatten_float)
}

fn decode_image_raw(path: &str) -> Result<DynamicImage, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if let Some(result) = decode_special_format(path, &ext) {
        match result {
            Ok(img) => return Ok(apply_exif_orientation(img, path)),
            // A dedicated decoder refusing the file isn't the end: Windows may
            // still know the format, so try that before giving up.
            Err(err) => {
                return decode_with_wic(path)
                    .map(|img| apply_exif_orientation(img, path))
                    .map_err(|wic_err| format!("{err} (Windows decoder also failed: {wic_err})"));
            }
        }
    }

    let direct = ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .into_decoder()
        .and_then(|mut decoder| {
            // Cameras store the rotation in EXIF instead of rotating the pixels.
            // Browsers apply it on their own, so anything decoded here has to as
            // well or the Rust path (clipboard, thumbnails, TIFF/raw display)
            // would show sideways photos the <img> tag shows upright.
            let orientation = decoder.orientation()?;
            let mut img = DynamicImage::from_decoder(decoder)?;
            img.apply_orientation(orientation);
            Ok(img)
        });

    match direct {
        Ok(img) => Ok(img),
        Err(err) => {
            let img = decode_with_wic(path)
                .map_err(|wic_err| format!("{err} (Windows decoder also failed: {wic_err})"))?;
            Ok(apply_exif_orientation(img, path))
        }
    }
}

// ponytail: rexif reads JPEG/TIFF EXIF, which covers TIFF-based raw files
// (CR2/NEF/ARW) but not HEIC. Read WIC's own metadata reader if HEIC photos
// ever show up rotated.
fn apply_exif_orientation(img: DynamicImage, path: &str) -> DynamicImage {
    let Ok(exif) = rexif::parse_file(path) else {
        return img;
    };

    let orientation = exif
        .entries
        .iter()
        .find(|e| e.tag == rexif::ExifTag::Orientation)
        .and_then(|e| match &e.value {
            rexif::TagValue::U16(v) => v.first().copied(),
            _ => None,
        })
        .unwrap_or(1);

    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

#[cfg(windows)]
fn decode_with_wic(path: &str) -> Result<DynamicImage, String> {
    use windows::core::HSTRING;
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory,
        WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };

    unsafe {
        // Already-initialized (or differently-initialized) threads are fine;
        // we only need COM to be live for this call.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.message())?;

        let decoder = factory
            .CreateDecoderFromFilename(
                &HSTRING::from(path),
                None,
                windows::Win32::Foundation::GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(|e| e.message())?;

        let frame = decoder.GetFrame(0).map_err(|e| e.message())?;

        // Normalise whatever exotic pixel format the file uses to plain RGBA.
        let converter = factory.CreateFormatConverter().map_err(|e| e.message())?;
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(|e| e.message())?;

        let (mut width, mut height) = (0u32, 0u32);
        converter
            .GetSize(&mut width, &mut height)
            .map_err(|e| e.message())?;

        let stride = width
            .checked_mul(4)
            .ok_or_else(|| "Image too large".to_string())?;
        let len = stride
            .checked_mul(height)
            .ok_or_else(|| "Image too large".to_string())? as usize;

        let mut buf = vec![0u8; len];
        converter
            .CopyPixels(std::ptr::null(), stride, &mut buf)
            .map_err(|e| e.message())?;

        RgbaImage::from_raw(width, height, buf)
            .map(DynamicImage::ImageRgba8)
            .ok_or_else(|| "WIC returned an unexpected buffer size".to_string())
    }
}

#[cfg(not(windows))]
fn decode_with_wic(_path: &str) -> Result<DynamicImage, String> {
    Err("no platform decoder available".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Renders a QR code to a PNG the scanner can read back. Scaled up with a
    // quiet zone, like any real-world QR, so detection has something to lock on.
    fn write_qr_png(data: &str, path: &Path) {
        use qrcode::{Color, QrCode};

        const SCALE: u32 = 6;
        const QUIET: u32 = 4;

        let code = QrCode::new(data.as_bytes()).expect("encode QR");
        let modules = code.to_colors();
        let side = code.width() as u32;
        let pixels = (side + QUIET * 2) * SCALE;

        let mut img = image::GrayImage::from_pixel(pixels, pixels, image::Luma([255u8]));

        for y in 0..side {
            for x in 0..side {
                if modules[(y * side + x) as usize] != Color::Dark {
                    continue;
                }
                for dy in 0..SCALE {
                    for dx in 0..SCALE {
                        img.put_pixel(
                            (x + QUIET) * SCALE + dx,
                            (y + QUIET) * SCALE + dy,
                            image::Luma([0u8]),
                        );
                    }
                }
            }
        }

        img.save(path).expect("write QR png");
    }

    #[test]
    fn reads_back_a_generated_qr_code() {
        let path = std::env::temp_dir().join("image_viewer_qr_test.png");
        write_qr_png("https://example.com/hello", &path);

        let found = scan_qr_codes(path.to_string_lossy().to_string()).expect("scan");
        let _ = fs::remove_file(&path);

        assert_eq!(found, vec!["https://example.com/hello".to_string()]);
    }

    // A camera writes the rotation into EXIF and leaves the pixels alone, so a
    // 40x20 image tagged "rotate 90" has to come back 20x40.
    #[test]
    fn applies_exif_orientation_when_decoding() {
        use little_exif::exif_tag::ExifTag;
        use little_exif::metadata::Metadata;

        let path = std::env::temp_dir().join("image_viewer_orientation_test.jpg");
        image::RgbImage::from_pixel(40, 20, image::Rgb([10u8, 20, 30]))
            .save(&path)
            .expect("write jpeg");

        let before = decode_image(&path.to_string_lossy()).expect("decode untagged");
        assert_eq!((before.width(), before.height()), (40, 20));

        let mut metadata = Metadata::new();
        metadata.set_tag(ExifTag::Orientation(vec![6]));
        metadata.write_to_file(&path).expect("write exif");

        let after = decode_image(&path.to_string_lossy()).expect("decode tagged");
        let _ = fs::remove_file(&path);

        assert_eq!(
            (after.width(), after.height()),
            (20, 40),
            "orientation 6 should rotate the image a quarter turn"
        );
    }

    #[test]
    fn lists_images_when_given_a_folder() {
        let dir = std::env::temp_dir().join("image_viewer_folder_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("make dir");

        for name in ["b.png", "a.png", "notes.txt"] {
            let file = dir.join(name);
            if name.ends_with(".png") {
                image::RgbImage::from_pixel(4, 4, image::Rgb([0u8, 0, 0]))
                    .save(&file)
                    .expect("write png");
            } else {
                fs::write(&file, "ignore me").expect("write txt");
            }
        }

        let (images, index) =
            get_folder_images(dir.to_string_lossy().to_string()).expect("list folder");
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(images.len(), 2, "only images are listed: {images:?}");
        assert_eq!(index, 0, "a dropped folder starts at the first image");
        assert!(images[0].ends_with("a.png"), "sorted by name: {images:?}");
    }

    // Needs a real image with text plus an installed OCR language, so it only
    // runs when pointed at one:
    //   $env:IMAGE_VIEWER_OCR_FILE="C:\shot.png"; cargo test -- --nocapture
    #[test]
    fn reads_text_from_env_image() {
        let Some(path) = std::env::var("IMAGE_VIEWER_OCR_FILE").ok().filter(|p| !p.trim().is_empty()) else {
            return;
        };

        let lines = read_text_in_image(path.clone()).expect("ocr failed");
        println!("{path}: {lines:?}");

        assert!(!lines.is_empty(), "expected to read some text from {path}");
    }

    // IMAGE_VIEWER_QR_FILE=<path> cargo test qr_from_env -- --nocapture
    #[test]
    fn scans_qr_from_env_image() {
        let Some(path) = std::env::var("IMAGE_VIEWER_QR_FILE").ok().filter(|p| !p.trim().is_empty()) else {
            return;
        };

        let found = scan_qr_codes(path.clone()).expect("scan");
        println!("{path}: found {} code(s)", found.len());
        assert!(!found.is_empty(), "no QR code detected in {path}");
    }

    #[test]
    fn times_a_big_dds() {
        use image_dds::{ImageFormat as DdsFormat, Mipmaps, Quality};

        // noisy, like a real texture: a flat colour would compress to nothing
        // and hide the actual encoding cost
        let mut seed = 0x1234_5678u32;
        let source = image::RgbaImage::from_fn(2048, 2048, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let b = seed.to_le_bytes();
            image::Rgba([b[0], b[1], b[2], 255])
        });

        let dds = image_dds::dds_from_image(
            &source,
            DdsFormat::BC1RgbaUnorm,
            Quality::Fast,
            Mipmaps::GeneratedAutomatic,
        )
        .expect("encode dds");

        let path = std::env::temp_dir().join("image_viewer_dds_big.dds");
        let mut file = File::create(&path).expect("create dds");
        dds.write(&mut file).expect("write dds");
        drop(file);

        let started = std::time::Instant::now();
        let frames = load_dds_frames(path.to_string_lossy().to_string()).expect("load");
        let listing = started.elapsed();

        let _ = fs::remove_file(&path);

        println!(
            "2048x2048 BC1: {} frames listed in {:?}",
            frames.len(),
            listing
        );
    }

    // A DDS with mipmaps should come back as one frame per level, each half
    // the size of the last.
    #[test]
    fn reads_dds_mipmaps_as_frames() {
        use image_dds::{ImageFormat as DdsFormat, Mipmaps, Quality};

        let source = image::RgbaImage::from_pixel(64, 64, image::Rgba([200u8, 40, 90, 255]));
        let dds = image_dds::dds_from_image(
            &source,
            DdsFormat::BC1RgbaUnorm,
            Quality::Fast,
            Mipmaps::GeneratedAutomatic,
        )
        .expect("encode dds");

        let path = std::env::temp_dir().join("image_viewer_dds_test.dds");
        let mut file = File::create(&path).expect("create dds");
        dds.write(&mut file).expect("write dds");
        drop(file);

        let frames = load_dds_frames(path.to_string_lossy().to_string()).expect("load frames");
        let _ = fs::remove_file(&path);

        assert_eq!(frames.len(), 7, "64x64 has 7 mip levels down to 1x1");
        assert_eq!((frames[0].width, frames[0].height), (64, 64));
        assert_eq!((frames[1].width, frames[1].height), (32, 32));
        assert_eq!(frames[1].label, "Mip 2");
    }

    // The one new format there's an encoder for locally; JXL/PSD/RAW/KTX2 have
    // decoders only, so point IMAGE_VIEWER_TEST_FILES at real samples for those.
    #[test]
    fn decodes_openexr() {
        let path = std::env::temp_dir().join("image_viewer_exr_test.exr");

        image::Rgb32FImage::from_pixel(32, 16, image::Rgb([0.5f32, 0.25, 1.0]))
            .save(&path)
            .expect("write exr");

        let img = decode_image(&path.to_string_lossy()).expect("decode exr");
        let _ = fs::remove_file(&path);

        assert_eq!((img.width(), img.height()), (32, 16));
    }

    #[test]
    fn adjustments_change_pixels_as_described() {
        let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            8,
            4,
            image::Rgba([200u8, 80, 40, 255]),
        ));

        let identity = apply_adjustments_to(source.clone(), &Adjustments::default());
        assert_eq!(
            identity.to_rgba8().into_raw(),
            source.to_rgba8().into_raw(),
            "no adjustments must leave the pixels untouched"
        );

        let inverted = apply_adjustments_to(
            source.clone(),
            &Adjustments {
                invert: true,
                ..Default::default()
            },
        );
        assert_eq!(inverted.to_rgba8().get_pixel(0, 0)[0], 55, "255 - 200");

        let grey = apply_adjustments_to(
            source.clone(),
            &Adjustments {
                grayscale: true,
                ..Default::default()
            },
        );
        let px = *grey.to_rgba8().get_pixel(0, 0);
        assert!(px[0] == px[1] && px[1] == px[2], "greyscale equalises channels");

        let dark = apply_adjustments_to(
            source.clone(),
            &Adjustments {
                exposure: -1.0,
                ..Default::default()
            },
        );
        assert!(
            dark.to_rgba8().get_pixel(0, 0)[0] < 200,
            "negative exposure darkens"
        );

        let no_red = apply_adjustments_to(
            source.clone(),
            &Adjustments {
                red: -100.0,
                ..Default::default()
            },
        );
        let px = *no_red.to_rgba8().get_pixel(0, 0);
        assert_eq!(px[0], 0, "-100 red removes the channel");
        assert_eq!(px[1], 80, "other channels untouched");

        let more_blue = apply_adjustments_to(
            source.clone(),
            &Adjustments {
                blue: 100.0,
                ..Default::default()
            },
        );
        assert!(
            more_blue.to_rgba8().get_pixel(0, 0)[2] > 40,
            "+100 blue brightens that channel"
        );

        let turned = apply_adjustments_to(
            source.clone(),
            &Adjustments {
                rotate: 90,
                ..Default::default()
            },
        );
        assert_eq!(
            (turned.width(), turned.height()),
            (4, 8),
            "a quarter turn swaps width and height"
        );

        assert!(
            Adjustments { rotate: 90, ..Default::default() }.is_rotation_only(),
            "rotation alone takes the lossless JPEG path"
        );
        assert!(
            !Adjustments { rotate: 90, invert: true, ..Default::default() }.is_rotation_only(),
            "rotation plus anything else must be re-encoded"
        );

        let resized = apply_adjustments_to(
            source.clone(),
            &Adjustments {
                resize: Some((16, 8)),
                ..Default::default()
            },
        );
        assert_eq!((resized.width(), resized.height()), (16, 8));

        let thresholded = apply_adjustments_to(
            source.clone(),
            &Adjustments {
                threshold: Some(128),
                ..Default::default()
            },
        );
        let px = *thresholded.to_rgba8().get_pixel(0, 0);
        assert!(px[0] == 0 || px[0] == 255, "threshold produces pure black or white");

        // alpha must survive everything above
        assert_eq!(inverted.to_rgba8().get_pixel(0, 0)[3], 255);
    }

    #[test]
    fn resize_to_fit_keeps_aspect_and_leaves_small_images_alone() {
        // 800x400 into a 256 box: the long edge lands on 256, the short one
        // follows it down rather than being squashed to a square.
        let wide = DynamicImage::ImageRgb8(image::RgbImage::new(800, 400));
        let shrunk = resize_to_fit(wide, 256).unwrap();
        assert_eq!((shrunk.width(), shrunk.height()), (256, 128));

        // Tall images pick the same longest edge.
        let tall = DynamicImage::ImageRgb8(image::RgbImage::new(400, 800));
        let shrunk = resize_to_fit(tall, 256).unwrap();
        assert_eq!((shrunk.width(), shrunk.height()), (128, 256));

        // Already smaller than the box: returned untouched, no upscaling.
        let small = DynamicImage::ImageRgba8(RgbaImage::new(64, 32));
        let same = resize_to_fit(small, 256).unwrap();
        assert_eq!((same.width(), same.height()), (64, 32));

        // An image with alpha keeps it, so transparent thumbnails still cache
        // as PNG instead of silently turning into black JPEGs.
        let transparent = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            600,
            600,
            image::Rgba([255, 0, 0, 0]),
        ));
        let shrunk = resize_to_fit(transparent, 128).unwrap();
        assert!(shrunk.color().has_alpha(), "alpha channel survives the resize");
        assert_eq!(shrunk.to_rgba8().get_pixel(0, 0)[3], 0);

        // A degenerate image is an error, not a panic in the resizer.
        let empty = DynamicImage::ImageRgb8(image::RgbImage::new(0, 0));
        assert!(resize_to_fit(empty, 256).is_err());
    }

    #[test]
    fn float_images_flatten_so_the_viewer_can_encode_them() {
        // What an EXR or HDR decodes to, including a value past the 1.0 range.
        let float = DynamicImage::ImageRgb32F(image::Rgb32FImage::from_pixel(
            2,
            2,
            image::Rgb([2.0, 0.5, 0.0]),
        ));

        let flat = flatten_float(float);
        assert_eq!(
            *flat.to_rgba8().get_pixel(0, 0),
            image::Rgba([255, 128, 0, 255]),
            "over-range clips to white, mid-range survives"
        );

        let mut buf = Vec::new();
        flat.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .expect("PNG encoding is what fails on an unflattened float image");

        // Everything else passes through untouched — no needless 16-bit loss.
        let sixteen_bit = DynamicImage::ImageRgb16(image::ImageBuffer::new(1, 1));
        assert!(matches!(
            flatten_float(sixteen_bit),
            DynamicImage::ImageRgb16(_)
        ));
    }

    #[test]
    fn folder_watcher_only_reacts_to_the_listing_changing() {
        use notify::event::{CreateKind, DataChange, ModifyKind, RemoveKind, RenameMode};
        use notify::{Event, EventKind};

        let with = |kind, path: &str| Event {
            kind,
            paths: vec![PathBuf::from(path)],
            attrs: Default::default(),
        };

        // A new image, or one deleted, is exactly what the list has to follow.
        assert!(listing_changed(&with(
            EventKind::Create(CreateKind::File),
            "C:\\pics\\new.png"
        )));
        assert!(listing_changed(&with(
            EventKind::Remove(RemoveKind::File),
            "C:\\pics\\gone.jpg"
        )));
        assert!(listing_changed(&with(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            "C:\\pics\\renamed.cr2"
        )));

        // Contents changing does not: the thumbnail cache key and the viewer's
        // asset-URL buster already handle a file being rewritten in place.
        assert!(!listing_changed(&with(
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            "C:\\pics\\edited.png"
        )));

        // Non-images in the same folder are noise — a folder of photos next to
        // a .txt shouldn't rebuild the list every time the text file is saved.
        assert!(!listing_changed(&with(
            EventKind::Create(CreateKind::File),
            "C:\\pics\\notes.txt"
        )));
    }

    #[test]
    fn auto_levels_stretches_a_flat_range() {
        // everything squeezed into 100..150 should end up spanning 0..255
        let mut img = RgbaImage::from_fn(64, 1, |x, _| {
            image::Rgba([(100 + x * 50 / 63) as u8, 128, 128, 255])
        });

        auto_levels(&mut img);

        let first = img.get_pixel(0, 0)[0];
        let last = img.get_pixel(63, 0)[0];
        assert!(first < 20, "dark end pushed to black, got {first}");
        assert!(last > 235, "bright end pushed to white, got {last}");
    }

    #[test]
    fn compression_reports_what_each_format_can_do() {
        let jpg = compression_options("photo.jpg".to_string()).expect("jpg");
        assert!(jpg.can_compress && jpg.has_quality, "JPEG has a quality dial");

        let png = compression_options("shot.png".to_string()).expect("png");
        assert!(png.can_compress, "PNG can be rewritten");
        assert!(!png.has_quality, "PNG is lossless, no quality dial");
        assert!(png.has_palette, "PNG offers colour reduction");

        let raw = compression_options("shot.cr3".to_string()).expect("cr3");
        assert!(!raw.can_compress, "no raw encoder, so no in-place compression");
        assert!(raw.note.contains("cannot be written"), "and it says why");
    }

    #[test]
    fn compressing_shrinks_a_jpeg_and_never_grows_the_original() {
        let dir = std::env::temp_dir().join("image_viewer_compress_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("make dir");

        // noisy, so quality actually matters
        let mut seed = 0xC0FFEEu32;
        let noisy = image::RgbImage::from_fn(320, 240, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let b = seed.to_le_bytes();
            image::Rgb([b[0], b[1], b[2]])
        });

        let path = dir.join("noise.jpg");
        image::DynamicImage::ImageRgb8(noisy)
            .save_with_format(&path, ImageFormat::Jpeg)
            .expect("write jpeg");

        let source = path.to_string_lossy().to_string();
        let original = fs::metadata(&path).unwrap().len();

        let low = compress_estimate(source.clone(), 20, 100, false).expect("estimate");
        assert!(
            low.bytes < original,
            "quality 20 should be smaller than the default encode ({} vs {original})",
            low.bytes
        );

        let half = compress_estimate(source.clone(), 80, 50, false).expect("estimate");
        assert_eq!((half.width, half.height), (160, 120), "50% scale");

        // estimating must not have touched the file
        assert_eq!(fs::metadata(&path).unwrap().len(), original);

        // refuses to write a result that is bigger than what is already there
        let grew = compress_apply(source.clone(), 100, 100, false, None, false);
        assert!(grew.is_err(), "should refuse to make the file bigger");
        assert_eq!(fs::metadata(&path).unwrap().len(), original, "file untouched");

        let applied = compress_apply(source.clone(), 20, 100, false, None, false).expect("apply");
        assert!(fs::metadata(&path).unwrap().len() < original, "file shrank");
        assert_eq!(applied.bytes, fs::metadata(&path).unwrap().len());

        let _ = fs::remove_dir_all(&dir);
    }

    // A DDS rotated in place used to be destroyed: image::save() truncates the
    // file, then fails because it can't encode DDS.
    #[test]
    fn saving_to_an_unwritable_format_leaves_the_file_alone() {
        use image_dds::{ImageFormat as DdsFormat, Mipmaps, Quality};

        let dir = std::env::temp_dir().join("image_viewer_unwritable_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("make dir");

        let source = RgbaImage::from_pixel(32, 32, image::Rgba([90u8, 140, 200, 255]));
        let dds = image_dds::dds_from_image(
            &source,
            DdsFormat::BC1RgbaUnorm,
            Quality::Fast,
            Mipmaps::Disabled,
        )
        .expect("encode dds");

        let path = dir.join("tex.dds");
        let mut file = File::create(&path).expect("create dds");
        dds.write(&mut file).expect("write dds");
        drop(file);

        let before = fs::metadata(&path).unwrap().len();

        let result = apply_adjustments(
            path.to_string_lossy().to_string(),
            Adjustments {
                rotate: 90,
                ..Default::default()
            },
            None,
        );

        assert!(result.is_err(), "must refuse rather than try");
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            before,
            "the original must be byte-for-byte untouched"
        );
        assert!(
            decode_image(&path.to_string_lossy()).is_ok(),
            "and must still open afterwards"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn convert_targets_exclude_the_current_format() {
        let from_png = convert_targets("photo.png".to_string());
        assert!(!from_png.iter().any(|t| t.ext == "png"), "no PNG -> PNG");
        assert!(from_png.iter().any(|t| t.ext == "jpg"));

        // .jpeg and .jpg are the same format under different names
        let from_jpeg = convert_targets("photo.jpeg".to_string());
        assert!(!from_jpeg.iter().any(|t| t.ext == "jpg"), "no JPEG -> JPEG");

        let from_tif = convert_targets("scan.tif".to_string());
        assert!(!from_tif.iter().any(|t| t.ext == "tiff"), "no TIFF -> TIFF");

        // a format with no encoder here can still be converted away from
        let from_cr3 = convert_targets("shot.cr3".to_string());
        assert!(from_cr3.iter().any(|t| t.ext == "png"));
    }

    #[test]
    fn converting_writes_a_new_file_without_clobbering() {
        let dir = std::env::temp_dir().join("image_viewer_convert_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("make dir");

        let source = dir.join("shot.png");
        RgbaImage::from_pixel(8, 8, image::Rgba([10u8, 200, 40, 255]))
            .save(&source)
            .expect("write source");

        // something already sitting on the obvious output name
        fs::write(dir.join("shot.jpg"), b"not an image").expect("write blocker");

        let written = convert_image(
            source.to_string_lossy().to_string(),
            "jpg".to_string(),
            Some(85),
            None,
            false,
        )
        .expect("convert");

        assert!(written.ends_with("shot (1).jpg"), "picked a free name: {written}");

        // an icon is fitted into the chosen box, not stretched to a square
        let big = dir.join("big.png");
        RgbaImage::from_pixel(400, 200, image::Rgba([0u8, 0, 0, 255]))
            .save(&big)
            .expect("write big png");

        let icon = convert_image(
            big.to_string_lossy().to_string(),
            "ico".to_string(),
            None,
            Some(32),
            false,
        )
        .expect("convert to ico");

        let (w, h) = decode_image(&icon).expect("decode icon").dimensions();
        assert_eq!((w, h), (32, 16), "fitted into 32px, aspect kept");
        assert!(source.exists(), "original kept when not asked to delete it");
        assert_eq!(
            decode_image(&written).expect("decode result").dimensions(),
            (8, 8)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn capabilities_depend_on_the_format() {
        let png = image_edit_capabilities("photo.png".to_string());
        assert!(png.can_edit && png.can_save_in_place);

        let raw = image_edit_capabilities("shot.cr3".to_string());
        assert!(raw.can_edit, "raw can still be adjusted");
        assert!(!raw.can_save_in_place, "but not written back as raw");

        let svg = image_edit_capabilities("logo.svg".to_string());
        assert!(!svg.can_edit, "svg has its own panel");

        let dds = image_edit_capabilities("tex.dds".to_string());
        assert!(dds.can_edit && !dds.can_save_in_place);
    }

    #[test]
    fn unknown_extensions_are_not_listed_as_images() {
        assert!(is_image_ext_str("jxl"));
        assert!(is_image_ext_str("exr") && is_image_ext_str("hdr"));
        assert!(is_image_ext_str("tga"), "tga decodes via the image crate");
        assert!(is_image_ext_str("qoi"));
        assert!(is_image_ext_str("cr3"), "raw extensions count as images");
        assert!(!is_image_ext_str("txt"));
        assert!(!is_image_ext_str("exe"));
    }

    #[test]
    fn reports_no_codes_for_a_plain_image() {
        let path = std::env::temp_dir().join("image_viewer_blank_test.png");
        image::GrayImage::from_pixel(120, 120, image::Luma([200u8]))
            .save(&path)
            .expect("write blank png");

        let found = scan_qr_codes(path.to_string_lossy().to_string()).expect("scan");
        let _ = fs::remove_file(&path);

        assert!(found.is_empty(), "expected nothing, got {found:?}");
    }

    // Point this at real files to check they decode, e.g.
    //   $env:IMAGE_VIEWER_TEST_FILES="C:\a.tiff;C:\b.tif"; cargo test
    // Skips silently when unset so the suite stays portable.
    #[test]
    fn decodes_listed_files() {
        let Some(list) = std::env::var("IMAGE_VIEWER_TEST_FILES").ok().filter(|p| !p.trim().is_empty()) else {
            return;
        };

        for path in list.split(';').filter(|p| !p.trim().is_empty()) {
            let started = std::time::Instant::now();
            match decode_image(path) {
                Ok(img) => {
                    // `cargo test --release -- --nocapture` to see the timings
                    println!(
                        "{path}: {}x{} in {:?}",
                        img.width(),
                        img.height(),
                        started.elapsed()
                    );
                    assert!(img.width() > 0 && img.height() > 0, "{path}: empty image");
                }
                Err(err) => panic!("{path}: {err}"),
            }
        }
    }
}

// Off the IPC thread too — decoding a 53MP photo here would otherwise hold the
// window hostage for the whole decode.
#[tauri::command(async)]
fn load_image(path: String) -> Result<Vec<u8>, String> {
    let img = decode_image(&path)?;

    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .map_err(|e| e.to_string())?;

    Ok(buf)
}

#[tauri::command]
fn copy_image_to_clipboard(path: String, app: AppHandle) -> Result<(), String> {
    let img = decode_image(&path)?;

    let rgba = img.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    let rgba_bytes = rgba.into_raw();

    // Windows clipboard APIs are thread-affine: arboard's Clipboard handle is
    // bound to whichever thread first created it (the app's main thread), but
    // this command runs on a worker thread, so writing directly here fails
    // with ERROR_CLIPBOARD_NOT_OPEN (os error 1418). Hop to the main thread.
    let (tx, rx) = std::sync::mpsc::channel();

    app.clone().run_on_main_thread(move || {
        let clipboard_image = tauri::image::Image::new_owned(rgba_bytes, width, height);
        let result = app
            .clipboard()
            .write_image(&clipboard_image)
            .map_err(|e| e.to_string());
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;

    rx.recv()
        .map_err(|_| "Clipboard write task did not respond".to_string())?
}

// ----- ROTATE + SAVE -----

// Composition table for a single 90°-clockwise step, covering all 8 standard
// EXIF orientation values (the plain-rotation cycle 1/6/3/8 and the
// mirrored-rotation cycle 2/7/4/5). Unknown/0 values are left unchanged.
fn rotate_orientation_cw(current: u16) -> u16 {
    match current {
        1 => 6,
        6 => 3,
        3 => 8,
        8 => 1,
        2 => 7,
        7 => 4,
        4 => 5,
        5 => 2,
        other => other,
    }
}

fn rotate_jpeg_exif(path: &str, steps: u32) -> Result<(), String> {
    use little_exif::exif_tag::ExifTag;
    use little_exif::metadata::Metadata;

    let path = Path::new(path);
    // Files with no EXIF segment at all (common for screenshots, web-saved
    // JPEGs, etc.) make new_from_path error out rather than hand back an
    // empty Metadata, so start fresh in that case instead of failing.
    let mut metadata = Metadata::new_from_path(path).unwrap_or_else(|_| Metadata::new());

    let mut orientation = metadata
        .get_tag(&ExifTag::Orientation(vec![]))
        .find_map(|tag| match tag {
            ExifTag::Orientation(v) => v.first().copied(),
            _ => None,
        })
        .unwrap_or(1);

    for _ in 0..steps {
        orientation = rotate_orientation_cw(orientation);
    }

    metadata.set_tag(ExifTag::Orientation(vec![orientation]));
    metadata.write_to_file(path).map_err(|e| e.to_string())
}

fn rotate_pixels(path: &str, steps: u32) -> Result<(), String> {
    let img = decode_image(path)?;

    let rotated = match steps {
        1 => img.rotate90(),
        2 => img.rotate180(),
        3 => img.rotate270(),
        _ => return Ok(()),
    };

    rotated.save(path).map_err(|e| e.to_string())
}

/// Rotates the image on disk by `quarter_turns` * 90° clockwise (negative
/// values rotate counter-clockwise). JPEGs are rotated losslessly by only
/// rewriting the EXIF Orientation tag (matches Windows Photo Viewer); other
/// supported formats have no such convention, so their pixels are actually
/// transposed and the file re-encoded.
#[tauri::command]
fn rotate_image(path: String, quarter_turns: i32) -> Result<(), String> {
    let steps = quarter_turns.rem_euclid(4) as u32;
    if steps == 0 {
        return Ok(());
    }

    let ext = Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "jpg" | "jpeg" | "jfif" => rotate_jpeg_exif(&path, steps),
        "png" | "bmp" | "tif" | "tiff" | "tga" | "qoi" => rotate_pixels(&path, steps),
        "webp" => {
            // Only the first frame survives a re-encode, so an animation would
            // be silently destroyed by rotating it.
            if webp_is_animated(&path) {
                Err("Animated WebP can't be rotated on disk without losing the animation"
                    .to_string())
            } else {
                rotate_pixels(&path, steps)
            }
        }
        "gif" | "apng" => Err(format!(
            "Rotating a .{ext} on disk would flatten it to a single frame"
        )),
        "ico" | "cur" | "dds" | "ktx2" => Err(format!(
            "A .{ext} holds several images — rotation stays on screen only"
        )),
        _ => Err(format!(
            "This app can't write .{ext} files, so the rotation stays on screen only"
        )),
    }
}

/// True for a WebP holding more than one frame. Errors count as "not animated"
/// so an unreadable file falls through to the normal failure path.
fn webp_is_animated(path: &str) -> bool {
    File::open(path)
        .ok()
        .and_then(|file| image_webp::WebPDecoder::new(std::io::BufReader::new(file)).ok())
        .map(|decoder| decoder.is_animated())
        .unwrap_or(false)
}

// ----- ICO ------

#[derive(serde::Serialize)]
struct IcoFrame {
    width: u32,
    height: u32,
    has_smooth_alpha: bool,
    is_png: bool,
    // What to call this frame in the bar; ICO frames have nothing to add to
    // their size, DDS frames name their mip level or cubemap face.
    label: String,
    data: Vec<u8>,
}

fn has_smooth_alpha(rgba: &[u8]) -> bool {
    // true if any pixel has partial transparency
    rgba.iter()
        .skip(3)
        .step_by(4)
        .any(|&a| a != 0 && a != 255)
}

#[tauri::command]
fn load_ico_frames(path: String) -> Result<Vec<IcoFrame>, String> {
    let file = File::open(&path).map_err(|e| e.to_string())?;
    let icon_dir = IconDir::read(file).map_err(|e| e.to_string())?;

    let mut frames = Vec::new();

    for entry in icon_dir.entries() {
        let is_png = entry.is_png();

        // Some ICO files bundle PNG frames in encodings the `ico`/`png` crates
        // can't decode (e.g. 4-bit or indexed color). Skip just that frame
        // instead of failing the whole file — other frames are usually fine.
        let icon_image = match entry.decode() {
            Ok(img) => img,
            Err(_) => continue,
        };

        let width = icon_image.width();
        let height = icon_image.height();
        let rgba_data = icon_image.rgba_data().to_vec();

        let has_smooth_alpha = has_smooth_alpha(&rgba_data);

        let rgba = match RgbaImage::from_raw(width, height, rgba_data) {
            Some(rgba) => rgba,
            None => continue,
        };

        let mut buf = Vec::new();
        if DynamicImage::ImageRgba8(rgba)
            .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .is_err()
        {
            continue;
        }

        frames.push(IcoFrame {
            width,
            height,
            has_smooth_alpha,
            is_png,
            label: String::new(), // size alone is enough for an icon frame
            data: buf,
        });
    }

    if frames.is_empty() {
        return Err("No decodable frames found in ICO file".to_string());
    }

    // ⭐ Windows-style quality sort
    frames.sort_by(|a, b| {
        (
            b.has_smooth_alpha,          // real alpha first
            b.is_png,                    // then PNG
            b.width * b.height,          // then resolution
        )
        .cmp(&(
            a.has_smooth_alpha,
            a.is_png,
            a.width * a.height,
        ))
    });

    Ok(frames)
}


// ----- DDS ------

/// One entry in the frame bar. Deliberately carries no pixels: a 2048² texture
/// with mipmaps is ~15MB of PNG across its levels, and Tauri sends bytes over
/// IPC as a JSON number array, so returning them all up front cost seconds
/// before the first frame appeared. The viewer asks for a frame's pixels with
/// load_dds_frame when it actually shows it.
#[derive(serde::Serialize)]
struct DdsFrameInfo {
    width: u32,
    height: u32,
    label: String,
}

fn dds_surface(path: &str) -> Result<image_dds::SurfaceRgba8<Vec<u8>>, String> {
    use image_dds::ddsfile::Dds;
    use image_dds::SurfaceRgba8;

    if Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ktx2"))
    {
        return ktx2_surface(path);
    }

    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let dds = Dds::read(&mut file).map_err(|e| e.to_string())?;

    SurfaceRgba8::decode_dds(&dds).map_err(|e| e.to_string())
}

/// KTX2 is a different container around the same block-compressed data, so it
/// decodes into the same surface the DDS frame bar already understands.
///
/// Only the BC formats and plain RGBA are handled. Basis Universal payloads
/// (UASTC/ETC1S) and mobile formats (ASTC/ETC/PVRTC) need a transcoder that
/// only exists as a C++ library, so those report an unsupported format instead.
fn ktx2_surface(path: &str) -> Result<image_dds::SurfaceRgba8<Vec<u8>>, String> {
    use image_dds::{ImageFormat, Surface};
    use ktx2::Format;

    let bytes = map_file(path)?;
    let reader = ktx2::Reader::new(&bytes).map_err(|e| format!("KTX2: {e:?}"))?;
    let header = reader.header();

    let format = match header.format {
        Some(Format::BC1_RGB_UNORM_BLOCK) | Some(Format::BC1_RGBA_UNORM_BLOCK) => {
            ImageFormat::BC1RgbaUnorm
        }
        Some(Format::BC1_RGB_SRGB_BLOCK) | Some(Format::BC1_RGBA_SRGB_BLOCK) => {
            ImageFormat::BC1RgbaUnormSrgb
        }
        Some(Format::BC2_UNORM_BLOCK) => ImageFormat::BC2RgbaUnorm,
        Some(Format::BC2_SRGB_BLOCK) => ImageFormat::BC2RgbaUnormSrgb,
        Some(Format::BC3_UNORM_BLOCK) => ImageFormat::BC3RgbaUnorm,
        Some(Format::BC3_SRGB_BLOCK) => ImageFormat::BC3RgbaUnormSrgb,
        Some(Format::BC4_UNORM_BLOCK) => ImageFormat::BC4RUnorm,
        Some(Format::BC4_SNORM_BLOCK) => ImageFormat::BC4RSnorm,
        Some(Format::BC5_UNORM_BLOCK) => ImageFormat::BC5RgUnorm,
        Some(Format::BC5_SNORM_BLOCK) => ImageFormat::BC5RgSnorm,
        Some(Format::BC6H_UFLOAT_BLOCK) => ImageFormat::BC6hRgbUfloat,
        Some(Format::BC6H_SFLOAT_BLOCK) => ImageFormat::BC6hRgbSfloat,
        Some(Format::BC7_UNORM_BLOCK) => ImageFormat::BC7RgbaUnorm,
        Some(Format::BC7_SRGB_BLOCK) => ImageFormat::BC7RgbaUnormSrgb,
        // UINT/SINT differ only in how a shader reads them; the bytes on disk
        // are laid out the same, so they display as the normalised form.
        Some(Format::R8G8B8A8_UNORM)
        | Some(Format::R8G8B8A8_UINT)
        | Some(Format::R8G8B8A8_SINT) => ImageFormat::Rgba8Unorm,
        Some(Format::R8G8B8A8_SNORM) => ImageFormat::Rgba8Snorm,
        Some(Format::R8G8B8A8_SRGB) => ImageFormat::Rgba8UnormSrgb,
        Some(Format::B8G8R8A8_UNORM) | Some(Format::B8G8R8A8_UINT) => ImageFormat::Bgra8Unorm,
        Some(Format::B8G8R8A8_SRGB) => ImageFormat::Bgra8UnormSrgb,
        Some(Format::R8_UNORM) | Some(Format::R8_UINT) => ImageFormat::R8Unorm,
        Some(Format::R8_SNORM) => ImageFormat::R8Snorm,
        Some(Format::R8G8_UNORM) | Some(Format::R8G8_UINT) => ImageFormat::Rg8Unorm,
        Some(Format::R8G8_SNORM) => ImageFormat::Rg8Snorm,
        Some(Format::R16G16B16A16_SFLOAT) => ImageFormat::Rgba16Float,
        Some(Format::R32G32B32A32_SFLOAT) => ImageFormat::Rgba32Float,
        Some(other) => return Err(format!("KTX2: unsupported texture format {other:?}")),
        None => {
            return Err(
                "KTX2: Basis Universal textures (UASTC/ETC1S) need a transcoder this build \
                 doesn't include"
                    .to_string(),
            )
        }
    };

    // Levels are stored largest-first here, which is the order the surface wants.
    let mut data = Vec::new();
    for level in reader.levels() {
        data.extend_from_slice(level.data);
    }

    let surface = Surface {
        width: header.pixel_width,
        height: header.pixel_height.max(1),
        depth: header.pixel_depth.max(1),
        layers: header.face_count.max(1) * header.layer_count.max(1),
        mipmaps: header.level_count.max(1),
        image_format: format,
        data,
    };

    surface
        .decode_rgba8()
        .map_err(|e| format!("KTX2: {e}"))
}

/// Every (layer, mipmap) pair the texture stores, in bar order, with a label.
fn dds_frame_list(surface: &image_dds::SurfaceRgba8<Vec<u8>>) -> Vec<(u32, u32, String)> {
    const CUBEMAP_FACES: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];
    let is_cubemap = surface.layers == 6;

    let mut frames = Vec::new();

    for layer in 0..surface.layers {
        for mipmap in 0..surface.mipmaps {
            let mut label = String::new();

            if is_cubemap {
                label.push_str(&format!("{} ", CUBEMAP_FACES[layer as usize]));
            } else if surface.layers > 1 {
                label.push_str(&format!("Layer {} ", layer + 1));
            }
            if surface.mipmaps > 1 {
                label.push_str(&format!("Mip {}", mipmap + 1));
            }

            frames.push((layer, mipmap, label.trim().to_string()));
        }
    }

    frames
}

/// Lists a DDS texture's mipmap levels (and layers, for cubemaps and texture
/// arrays) so the frame bar can step through them like ICO frames.
#[tauri::command]
fn load_dds_frames(path: String) -> Result<Vec<DdsFrameInfo>, String> {
    let surface = dds_surface(&path)?;

    let frames: Vec<DdsFrameInfo> = dds_frame_list(&surface)
        .into_iter()
        .filter_map(|(layer, mipmap, label)| {
            let image = surface.get_image(layer, 0, mipmap)?;
            Some(DdsFrameInfo {
                width: image.width(),
                height: image.height(),
                label,
            })
        })
        .collect();

    if frames.is_empty() {
        return Err("No decodable images found in DDS file".to_string());
    }

    Ok(frames)
}

/// Renders one frame of a DDS to a cached PNG and returns its path.
///
/// A path rather than the bytes: Tauri sends a byte array over IPC as JSON
/// numbers, so a full-size mip level (12MB of PNG for a 2048² texture) becomes
/// tens of megabytes of text. The webview loads the file directly instead.
#[tauri::command]
fn load_dds_frame(app: AppHandle, path: String, index: usize) -> Result<String, String> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};

    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("dds-frames");

    let cached = dir.join(format!("{}-{index}.png", thumb_cache_key(&path, 0)?));
    if cached.exists() {
        return Ok(cached.to_string_lossy().to_string());
    }

    let surface = dds_surface(&path)?;
    let frames = dds_frame_list(&surface);

    let &(layer, mipmap, _) = frames.get(index).ok_or("No such frame in DDS file")?;
    let image = surface
        .get_image(layer, 0, mipmap)
        .ok_or("That frame is not stored in the file")?;

    // Fast compression: this PNG is a scratch file for the webview, so time
    // spent shrinking it would only delay the frame appearing.
    let mut buf = Vec::new();
    image
        .write_with_encoder(PngEncoder::new_with_quality(
            &mut Cursor::new(&mut buf),
            CompressionType::Fast,
            FilterType::Adaptive,
        ))
        .map_err(|e| e.to_string())?;

    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(&cached, &buf).map_err(|e| e.to_string())?;

    Ok(cached.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn open_with(path: String) -> Result<(), String> {
    show_openas_dialog(&path)
}

#[tauri::command]
fn open_with_app(app: String, path: String) -> Result<(), String> {
    use std::process::Command;

    #[cfg(windows)]
    use std::os::windows::process::CommandExt;

    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    if app.contains('!') {
        // ── UWP app (AppUserModelID)
        Command::new("explorer")
            .arg(format!("shell:AppsFolder\\{}", app))
            .spawn()
            .map_err(|e| e.to_string())?;

        Ok(())
    } 
    else {
        // ── Win32 app (.exe)
        let exe_path =
            resolve_app_path(&app).ok_or_else(|| format!("Executable not found: {}", app))?;

        let mut cmd = Command::new(exe_path);
        cmd.arg(path);

        #[cfg(windows)]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.spawn().map_err(|e| e.to_string())?;
        Ok(())
    }
}


#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    use std::process::Command;

    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(&["/C", "start", "", &url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn show_openas_dialog(path: &str) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHOpenWithDialog, OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OPENASINFO,
    };

    // ShellExecuteW verb="openas" (and the older rundll32 shell32.dll,OpenAs_RunDLL
    // trick) both fail with SE_ERR_NOASSOC when the extension's registered ProgID
    // has no "open" verb at all (a broken/dangling association). SHOpenWithDialog
    // is the actual API behind Explorer's "Open with" picker and doesn't need any
    // existing association to be valid.
    const HRESULT_CANCELLED: i32 = 0x800704C7u32 as i32;

    let wide_path: Vec<u16> = OsStr::new(path).encode_wide().chain(Some(0)).collect();

    let info = OPENASINFO {
        pcszFile: wide_path.as_ptr(),
        pcszClass: std::ptr::null(),
        oaifInFlags: OAIF_EXEC | OAIF_ALLOW_REGISTRATION,
    };

    let hr = unsafe { SHOpenWithDialog(0, &info) };

    if hr < 0 && hr != HRESULT_CANCELLED {
        return Err(format!(
            "Failed to open 'Open With' dialog. HRESULT: {:#x}",
            hr
        ));
    }

    Ok(())
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn open_with_dialog(path: String) -> Result<(), String> {
    show_openas_dialog(&path)
}

#[cfg(target_os = "windows")]
fn extract_icon_fast(exe_path: &str) -> Option<Vec<u8>> {
    use image::{ImageFormat, RgbaImage};
    use std::io::Cursor;
    use std::mem::zeroed;
    use std::ptr::null_mut;
    use winapi::um::shellapi::*;
    use winapi::um::wingdi::*;
    use winapi::um::winuser::*;
    let mut shinfo: SHFILEINFOW = unsafe { std::mem::zeroed() };

    let wide: Vec<u16> = exe_path.encode_utf16().chain(Some(0)).collect();

    let res = unsafe {
        SHGetFileInfoW(
            wide.as_ptr(),
            0,
            &mut shinfo,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };

    if res == 0 || shinfo.hIcon.is_null() {
        return None;
    }

    let hicon = shinfo.hIcon;

    unsafe {
        let mut info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: null_mut(),
            hbmColor: null_mut(),
        };

        GetIconInfo(hicon, &mut info);

        let mut bmp = BITMAP {
            bmType: 0,
            bmWidth: 0,
            bmHeight: 0,
            bmWidthBytes: 0,
            bmPlanes: 0,
            bmBitsPixel: 0,
            bmBits: null_mut(),
        };

        GetObjectW(
            info.hbmColor as _,
            std::mem::size_of::<BITMAP>() as i32,
            &mut bmp as *mut _ as _,
        );

        let width = bmp.bmWidth as u32;
        let height = bmp.bmHeight as u32;

        let mut buffer = vec![0u8; (width * height * 4) as usize];

        let hdc = GetDC(null_mut());

        //let mut bmi = BITMAPINFO::default();
        //let mut bmi: BITMAPINFO = unsafe { zeroed() };
        let mut bmi: BITMAPINFO = zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width as i32;
        bmi.bmiHeader.biHeight = -(height as i32);
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;

        GetDIBits(
            hdc,
            info.hbmColor,
            0,
            height,
            buffer.as_mut_ptr() as _,
            &mut bmi,
            DIB_RGB_COLORS,
        );

        ReleaseDC(null_mut(), hdc);
        DestroyIcon(hicon);
        DeleteObject(info.hbmColor as _);
        DeleteObject(info.hbmMask as _);

        let image = RgbaImage::from_raw(width, height, buffer)?;
        let mut out = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .ok()?;
        Some(out)
    }
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn open_native_print_dialog(path: String) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::shellapi::ShellExecuteW;
    use winapi::um::winuser::SW_SHOW;

    let operation: Vec<u16> = OsStr::new("print").encode_wide().chain(Some(0)).collect();
    let file: Vec<u16> = OsStr::new(&path).encode_wide().chain(Some(0)).collect();

    unsafe {
        let result = ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOW,
        );

        if (result as usize) <= 32 {
            return Err(format!(
                "Failed to open print dialog. Error code: {:?}",
                result
            ));
        }
    }

    Ok(())
}

#[tauri::command]
async fn trash_file(path: String) -> Result<(), String> {
    let path = Path::new(&path);
    
    // Validate path exists
    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }
    
    trash::delete(path)
        .map_err(|e| format!("Failed to move to trash: {}", e))
}


#[tauri::command]
fn set_desktop_background(path: String) -> Result<(), String> {
  wallpaper::set_from_path(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_in_explorer(path: String) {
  let _ = std::process::Command::new("explorer")
    .args(["/select,", &path])
    .spawn();
}

/* #[tauri::command]
fn save_file_copy(app: AppHandle, path: String) -> Result<(), String> {
    let old_path = Path::new(&path);
    let parent = old_path.parent().ok_or("Invalid source path")?;

    if let Some(dest) = app.dialog().file().set_directory(parent).save_file() {
        std::fs::copy(&path, &dest).map_err(|e| e.to_string())?;
    }
    Ok(())
} */

/* 
#[tauri::command]
fn save_file_as(app: AppHandle, path: String) -> Result<(), String> {
    let old_path = Path::new(&path);
    let parent = old_path.parent().ok_or("Invalid source path")?;

    if let Some(dest) = app.dialog().file().set_directory(parent).save_file() {
        std::fs::copy(&path, &dest).map_err(|e| e.to_string())?;
    }
    Ok(())
} */

#[tauri::command]
fn copy_file(src: String, dest: String) -> Result<(), String> {
    std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;
    Ok(())
}

/* #[tauri::command]
fn save_file(path: String) {
  println!("Save {}", path);
} */

/* 
#[tauri::command]
fn rename_file(app: AppHandle, path: String) -> Result<(), String> {
    let old_path = PathBuf::from(path);

    let parent = old_path
        .parent()
        .ok_or("Invalid path")?
        .to_path_buf();

    app.dialog()
        .file()
        .set_directory(parent)
        .save_file(move |new_path| {
            if let Some(file_path) = new_path {
                if let tauri_plugin_dialog::FilePath::Path(dest) = file_path {
                    if let Err(err) = std::fs::rename(&old_path, dest) {
                        eprintln!("Rename failed: {err}");
                    }
                }
            }
        });

    Ok(())
}
 */

#[tauri::command]
fn rename_file(path: String, new_name: String) -> Result<String, String> {
    let old_path = PathBuf::from(&path);
    let parent = old_path.parent().ok_or("Invalid path")?;
    let ext = old_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let mut new_path = parent.join(&new_name);
    if !ext.is_empty() {
        new_path.set_extension(ext);
    }

    std::fs::rename(&old_path, &new_path).map_err(|e| e.to_string())?;

    Ok(new_path.to_string_lossy().to_string())
}

// ---------------- IMAGE INFO ----------------

use serde::Serialize;
use rexif::parse_file;
use image::GenericImageView;

use std::time::{SystemTime, UNIX_EPOCH};

// ----- OCR -----

// The engine rejects very large bitmaps and gains nothing from them; this also
// keeps a 50MP photo from being copied around as raw BGRA.
const OCR_MAX_EDGE: u32 = 4000;

/// Reads the text in the image with the OCR engine that ships with Windows,
/// one entry per recognised line. Empty when the image has no text.
#[tauri::command]
fn read_text_in_image(path: String) -> Result<Vec<String>, String> {
    let img = decode_image(&path)?;

    let img = if img.width().max(img.height()) > OCR_MAX_EDGE {
        img.resize(
            OCR_MAX_EDGE,
            OCR_MAX_EDGE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    };

    ocr_bgra(img.to_rgba8())
}

#[cfg(windows)]
fn ocr_bgra(rgba: RgbaImage) -> Result<Vec<String>, String> {
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    let (width, height) = (rgba.width() as i32, rgba.height() as i32);

    let mut bgra = rgba.into_raw();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2); // RGBA -> BGRA, what SoftwareBitmap expects
    }

    // Own thread so the blocking wait below always happens in an MTA. The
    // command's own thread may already be an STA from a WIC decode, where
    // blocking on a WinRT async operation can deadlock.
    std::thread::spawn(move || -> Result<Vec<String>, String> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let writer = DataWriter::new().map_err(|e| e.message())?;
        writer.WriteBytes(&bgra).map_err(|e| e.message())?;
        let buffer = writer.DetachBuffer().map_err(|e| e.message())?;

        let bitmap =
            SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, width, height)
                .map_err(|e| e.message())?;

        // Follows the languages the user has installed; there's nothing to
        // recognise with if none of them ship an OCR model.
        let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
            "Windows has no OCR language installed. Add a language in \
             Settings > Time & language > Language & region."
                .to_string()
        })?;

        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(|e| e.message())?
            .get()
            .map_err(|e| e.message())?;

        let mut lines = Vec::new();
        for line in result.Lines().map_err(|e| e.message())? {
            let text = line.Text().map_err(|e| e.message())?.to_string();
            if !text.trim().is_empty() {
                lines.push(text);
            }
        }

        Ok(lines)
    })
    .join()
    .map_err(|_| "OCR thread panicked".to_string())?
}

#[cfg(not(windows))]
fn ocr_bgra(_rgba: RgbaImage) -> Result<Vec<String>, String> {
    Err("OCR is only available on Windows".to_string())
}

// ----- IMAGE CONTROLS -----

/// Everything the Image Controls panel can ask for, in one value. The preview
/// and the saved file run the exact same pipeline, so what you see is what
/// lands on disk.
#[derive(serde::Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
struct Adjustments {
    exposure: f32,    // stops, -3..3
    brightness: f32,  // -100..100
    contrast: f32,    // -100..100
    saturation: f32,  // -100..100
    hue: f32,         // degrees, -180..180
    red: f32,         // per-channel gain, -100..100 (-100 removes the channel)
    green: f32,
    blue: f32,
    blur: f32,        // gaussian sigma, 0..20
    sharpen: f32,     // unsharp amount, 0..100
    rotate: u32,      // clockwise degrees: 0, 90, 180 or 270
    threshold: Option<u8>,
    grayscale: bool,
    invert: bool,
    sepia: bool,
    auto_levels: bool,
    flip_h: bool,
    flip_v: bool,
    resize: Option<(u32, u32)>,
}

impl Adjustments {
    /// Only a quarter-turn, nothing else — the case JPEG can save losslessly.
    fn is_rotation_only(&self) -> bool {
        if self.rotate == 0 {
            return false;
        }

        let without_rotation = Adjustments {
            rotate: 0,
            ..self.clone()
        };

        without_rotation.is_identity()
    }

    fn is_identity(&self) -> bool {
        self.exposure == 0.0
            && self.brightness == 0.0
            && self.contrast == 0.0
            && self.saturation == 0.0
            && self.hue == 0.0
            && self.red == 0.0
            && self.green == 0.0
            && self.blue == 0.0
            && self.blur == 0.0
            && self.sharpen == 0.0
            && self.rotate == 0
            && self.threshold.is_none()
            && !self.grayscale
            && !self.invert
            && !self.sepia
            && !self.auto_levels
            && !self.flip_h
            && !self.flip_v
            && self.resize.is_none()
    }
}

/// Stretches each channel so the darkest and brightest 0.5% of pixels hit the
/// ends of the range — the usual "auto levels".
fn auto_levels(img: &mut RgbaImage) {
    const CLIP: f32 = 0.005;

    let pixel_count = (img.width() * img.height()) as f32;
    if pixel_count == 0.0 {
        return;
    }

    for channel in 0..3 {
        let mut histogram = [0u32; 256];
        for px in img.pixels() {
            histogram[px[channel] as usize] += 1;
        }

        let cutoff = (pixel_count * CLIP) as u32;

        let mut seen = 0;
        let low = histogram
            .iter()
            .position(|&count| {
                seen += count;
                seen > cutoff
            })
            .unwrap_or(0) as f32;

        seen = 0;
        let high = histogram
            .iter()
            .rposition(|&count| {
                seen += count;
                seen > cutoff
            })
            .unwrap_or(255) as f32;

        if high - low < 1.0 {
            continue; // flat channel, nothing to stretch
        }

        let scale = 255.0 / (high - low);
        for px in img.pixels_mut() {
            px[channel] = (((px[channel] as f32 - low) * scale).clamp(0.0, 255.0)) as u8;
        }
    }
}

/// Tone and colour, done per pixel. Exposure and contrast work in linear light
/// (the physically correct place for them), while brightness, saturation and
/// hue go through Oklab so they stay perceptually even instead of the muddy
/// results of scaling sRGB directly.
fn adjust_colors(img: &mut RgbaImage, adj: &Adjustments) {
    use palette::{FromColor, IntoColor, Okhsl, Srgb};

    let exposure_gain = 2f32.powf(adj.exposure);
    let contrast = 1.0 + adj.contrast / 100.0;
    let brightness = adj.brightness / 100.0;
    let saturation = 1.0 + adj.saturation / 100.0;
    let hue_shift = adj.hue;

    // Per-channel gain: 0 leaves the channel alone, -100 removes it, +100
    // doubles it. Applied in linear light with the rest of the tone work.
    let red_gain = 1.0 + adj.red / 100.0;
    let green_gain = 1.0 + adj.green / 100.0;
    let blue_gain = 1.0 + adj.blue / 100.0;

    let touches_color = brightness != 0.0 || saturation != 1.0 || hue_shift != 0.0;
    let touches_channels = red_gain != 1.0 || green_gain != 1.0 || blue_gain != 1.0;
    let touches_tone = adj.exposure != 0.0 || adj.contrast != 0.0 || touches_channels;

    if !touches_color && !touches_tone {
        return;
    }

    for px in img.pixels_mut() {
        let mut rgb: Srgb<f32> = Srgb::new(
            px[0] as f32 / 255.0,
            px[1] as f32 / 255.0,
            px[2] as f32 / 255.0,
        );

        if touches_tone {
            let mut linear = rgb.into_linear();
            linear.red = (linear.red * exposure_gain * red_gain - 0.5) * contrast + 0.5;
            linear.green = (linear.green * exposure_gain * green_gain - 0.5) * contrast + 0.5;
            linear.blue = (linear.blue * exposure_gain * blue_gain - 0.5) * contrast + 0.5;
            rgb = Srgb::from_linear(palette::LinSrgb::new(
                linear.red.clamp(0.0, 1.0),
                linear.green.clamp(0.0, 1.0),
                linear.blue.clamp(0.0, 1.0),
            ));
        }

        if touches_color {
            let mut hsl = Okhsl::from_color(rgb);
            hsl.lightness = (hsl.lightness + brightness).clamp(0.0, 1.0);
            hsl.saturation = (hsl.saturation * saturation).clamp(0.0, 1.0);
            hsl.hue = hsl.hue + hue_shift;
            rgb = hsl.into_color();
        }

        px[0] = (rgb.red.clamp(0.0, 1.0) * 255.0).round() as u8;
        px[1] = (rgb.green.clamp(0.0, 1.0) * 255.0).round() as u8;
        px[2] = (rgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

fn apply_sepia(img: &mut RgbaImage) {
    for px in img.pixels_mut() {
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        px[0] = (0.393 * r + 0.769 * g + 0.189 * b).min(255.0) as u8;
        px[1] = (0.349 * r + 0.686 * g + 0.168 * b).min(255.0) as u8;
        px[2] = (0.272 * r + 0.534 * g + 0.131 * b).min(255.0) as u8;
    }
}

/// Runs the whole pipeline. Order matters and follows what an editor does:
/// geometry, then tone, then colour, then effects that read neighbouring
/// pixels, then anything that flattens the image.
fn apply_adjustments_to(image: DynamicImage, adj: &Adjustments) -> DynamicImage {
    let mut image = image;

    match adj.rotate % 360 {
        90 => image = image.rotate90(),
        180 => image = image.rotate180(),
        270 => image = image.rotate270(),
        _ => {}
    }

    if adj.flip_h {
        image = image.fliph();
    }
    if adj.flip_v {
        image = image.flipv();
    }
    if let Some((w, h)) = adj.resize {
        if w > 0 && h > 0 {
            image = image.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
        }
    }

    let mut rgba = image.to_rgba8();

    if adj.auto_levels {
        auto_levels(&mut rgba);
    }

    adjust_colors(&mut rgba, adj);

    if adj.sepia {
        apply_sepia(&mut rgba);
    }
    if adj.grayscale {
        for px in rgba.pixels_mut() {
            let luma = (0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32)
                .round() as u8;
            px[0] = luma;
            px[1] = luma;
            px[2] = luma;
        }
    }
    if adj.invert {
        for px in rgba.pixels_mut() {
            px[0] = 255 - px[0];
            px[1] = 255 - px[1];
            px[2] = 255 - px[2];
        }
    }

    let mut image = DynamicImage::ImageRgba8(rgba);

    if adj.blur > 0.0 {
        image = image.blur(adj.blur);
    }
    if adj.sharpen > 0.0 {
        // unsharp masking: threshold 0 so it sharpens everywhere, amount scaled
        // to the 0..100 the slider uses
        image = image.unsharpen(1.5, (100 - (adj.sharpen as i32).clamp(0, 100)) as i32 / 4);
    }

    if let Some(level) = adj.threshold {
        let mut rgba = image.to_rgba8();
        for px in rgba.pixels_mut() {
            let luma =
                (0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32) as u8;
            let value = if luma >= level { 255 } else { 0 };
            px[0] = value;
            px[1] = value;
            px[2] = value;
        }
        image = DynamicImage::ImageRgba8(rgba);
    }

    image
}

/// What the panel is allowed to offer for this file. Formats differ: an
/// animation or a texture can't be written back as itself, and a raw file or
/// a HEIC has no encoder here at all.
#[derive(serde::Serialize)]
struct EditCapabilities {
    can_edit: bool,
    can_save_in_place: bool,
    format: String,
    note: String,
}

#[tauri::command]
fn image_edit_capabilities(path: String) -> EditCapabilities {
    let ext = Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    // Formats this build can both decode and re-encode with the same extension
    let writable = matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "jfif" | "bmp" | "tif" | "tiff");

    let (can_edit, note) = match ext.as_str() {
        "svg" => (false, "SVG is vector — use the SVG panel".to_string()),
        "gif" | "apng" => (
            true,
            "Animation: edits apply to the frame on screen, save as a new file".to_string(),
        ),
        "ico" | "cur" | "dds" | "ktx2" => (
            true,
            "Multi-frame file: edits apply to the frame on screen, save as a new file".to_string(),
        ),
        _ if RAW_EXTS.contains(&ext.as_str()) => (
            true,
            "RAW is read-only here — save the result as a new file".to_string(),
        ),
        "heic" | "heif" | "jxl" | "psd" | "psb" | "exr" | "hdr" | "avif" | "webp" => (
            true,
            format!("No {} encoder in this build — save as a new file", ext.to_uppercase()),
        ),
        _ => (true, String::new()),
    };

    EditCapabilities {
        can_edit,
        can_save_in_place: can_edit && writable,
        format: ext.to_uppercase(),
        note,
    }
}

/// Renders the adjustments onto a preview-sized copy and returns its path.
/// Preview-sized so dragging a slider stays responsive on a 50MP photo; the
/// same pipeline runs at full resolution when saving.
#[tauri::command(async)]
fn preview_adjustments(
    app: AppHandle,
    path: String,
    adjustments: Adjustments,
    max_size: u32,
    revision: u64,
) -> Result<String, String> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};

    let max_size = max_size.clamp(256, 4096);

    let image = resize_to_fit(decode_image(&path)?, max_size)?;
    let image = apply_adjustments_to(image, &adjustments);

    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("previews");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // revision in the name so the webview can't serve a stale cached preview
    let target = dir.join(format!("preview-{revision}.png"));

    let mut buf = Vec::new();
    image
        .to_rgba8()
        .write_with_encoder(PngEncoder::new_with_quality(
            &mut Cursor::new(&mut buf),
            CompressionType::Fast,
            FilterType::Adaptive,
        ))
        .map_err(|e| e.to_string())?;

    fs::write(&target, &buf).map_err(|e| e.to_string())?;

    // keep the cache from growing forever: drop everything but this render
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.path() != target {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    Ok(target.to_string_lossy().to_string())
}

/// Formats this build can encode. Anything else must never be handed to
/// `image::save()`, which truncates the destination before finding out it
/// can't write that format.
fn can_encode_ext(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "jfif" | "bmp" | "tif" | "tiff" | "tga" | "qoi" | "webp" | "gif"
            | "ico"
    )
}

/// Applies the adjustments at full resolution and writes the result.
/// `save_as` picks the destination; without it the original file is replaced.
#[tauri::command]
fn apply_adjustments(
    path: String,
    adjustments: Adjustments,
    save_as: Option<String>,
) -> Result<String, String> {
    if adjustments.is_identity() {
        return Err("Nothing to apply".to_string());
    }

    // A quarter-turn on a JPEG with nothing else changed is written as an EXIF
    // orientation tag instead: the pixels stay bit-for-bit identical rather
    // than being re-encoded and losing quality, the same trick Windows Photo
    // Viewer uses.
    if adjustments.is_rotation_only() && save_as.is_none() {
        let ext = Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if matches!(ext.as_str(), "jpg" | "jpeg" | "jfif") {
            rotate_jpeg_exif(&path, adjustments.rotate / 90)?;
            return Ok(path);
        }
    }

    let target = save_as.unwrap_or_else(|| path.clone());

    let target_ext = Path::new(&target)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    // Checked before anything is decoded or opened: image::save() creates (and
    // truncates) the file before it discovers it can't encode that format,
    // which would destroy the original.
    if !can_encode_ext(&target_ext) {
        return Err(format!(
            "This app can't write .{target_ext} files — save as another format instead"
        ));
    }

    let image = apply_adjustments_to(decode_image(&path)?, &adjustments);

    // JPEG has no alpha channel; saving RGBA to it fails rather than silently
    // dropping the channel, so drop it here on purpose.
    let image = if matches!(target_ext.as_str(), "jpg" | "jpeg" | "jfif") {
        DynamicImage::ImageRgb8(image.to_rgb8())
    } else {
        image
    };

    // Encode beside the target, then swap it in. A failure part-way through
    // leaves the original untouched instead of half-written.
    let scratch = PathBuf::from(format!("{target}.tmp-write"));

    let write_result = image
        .save_with_format(
            &scratch,
            ImageFormat::from_extension(&target_ext)
                .ok_or_else(|| format!("Unknown format .{target_ext}"))?,
        )
        .map_err(|e| e.to_string());

    if let Err(err) = write_result {
        let _ = fs::remove_file(&scratch);
        return Err(err);
    }

    fs::rename(&scratch, &target).map_err(|e| {
        let _ = fs::remove_file(&scratch);
        e.to_string()
    })?;

    Ok(target)
}

// ----- COMPRESS -----

/// What compression this file's own format supports. A format with no encoder
/// here can't be rewritten at all, and the panel says so rather than pretending.
#[derive(serde::Serialize)]
struct CompressionOptions {
    can_compress: bool,
    has_quality: bool,
    has_palette: bool,
    format: String,
    note: String,
    current_bytes: u64,
    width: u32,
    height: u32,
}

fn compressible_format(ext: &str) -> Option<&'static str> {
    match ext {
        "jpg" | "jpeg" | "jfif" => Some("jpg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        "tiff" | "tif" => Some("tiff"),
        "bmp" => Some("bmp"),
        "tga" => Some("tga"),
        "qoi" => Some("qoi"),
        "gif" => Some("gif"),
        _ => None,
    }
}

#[tauri::command]
fn compression_options(path: String) -> Result<CompressionOptions, String> {
    let ext = Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    let current_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let format = ext.to_uppercase();

    let Some(kind) = compressible_format(&ext) else {
        return Ok(CompressionOptions {
            can_compress: false,
            has_quality: false,
            has_palette: false,
            format: format.clone(),
            note: format!(
                "{format} cannot be written by this app, so it can't be compressed in place. \
                 Use Convert to make a PNG or JPEG instead."
            ),
            current_bytes,
            width: 0,
            height: 0,
        });
    };

    // dimensions come from a real decode so scaling shows true numbers
    let (width, height) = decode_image(&path)
        .map(|img| (img.width(), img.height()))
        .unwrap_or((0, 0));

    let note = match kind {
        "jpg" => "Lossy: lower quality means a smaller file.".to_string(),
        "png" => "Lossless: shrink by reducing colours or size.".to_string(),
        "webp" => "This build writes lossless WebP — shrink by reducing size.".to_string(),
        "gif" => "Already palette-based — shrink by reducing size.".to_string(),
        "bmp" | "tga" => {
            format!("{format} is uncompressed — only a smaller size reduces the file.")
        }
        _ => "Shrink by reducing size.".to_string(),
    };

    Ok(CompressionOptions {
        can_compress: true,
        has_quality: kind == "jpg",
        has_palette: matches!(kind, "png"),
        format,
        note,
        current_bytes,
        width,
        height,
    })
}

/// Encodes with the requested settings and hands back the bytes, so the panel
/// can show the resulting size before anything is written to disk.
fn compress_to_bytes(
    path: &str,
    quality: u8,
    scale_percent: u32,
    reduce_colors: bool,
) -> Result<(Vec<u8>, u32, u32), String> {
    use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};

    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    let kind = compressible_format(&ext).ok_or("This format cannot be written by this app")?;

    let mut image = decode_image(path)?;

    let scale = scale_percent.clamp(5, 100);
    if scale < 100 {
        let width = (image.width() * scale / 100).max(1);
        let height = (image.height() * scale / 100).max(1);
        image = image.resize(width, height, image::imageops::FilterType::Lanczos3);
    }

    let (width, height) = (image.width(), image.height());
    let mut buf = Vec::new();

    match kind {
        "jpg" => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut Cursor::new(&mut buf),
                quality.clamp(1, 100),
            )
            .encode_image(&image.to_rgb8())
            .map_err(|e| e.to_string())?;
        }
        "png" => {
            let rgba = image.to_rgba8();

            // Fewer distinct colours compress far better, and the encoder
            // alone can't do much on a photo. Posterising to 4 bits a channel
            // keeps the picture recognisable while collapsing near-identical
            // shades that PNG would otherwise store one by one.
            let source = if reduce_colors {
                let mut reduced = rgba;
                for px in reduced.pixels_mut() {
                    for channel in 0..3 {
                        px[channel] = px[channel] & 0xF0 | (px[channel] & 0xF0) >> 4;
                    }
                }
                reduced
            } else {
                rgba
            };

            source
                .write_with_encoder(PngEncoder::new_with_quality(
                    &mut Cursor::new(&mut buf),
                    CompressionType::Best,
                    PngFilter::Adaptive,
                ))
                .map_err(|e| e.to_string())?;
        }
        _ => {
            let format = ImageFormat::from_extension(&ext)
                .ok_or_else(|| format!("Unknown format .{ext}"))?;
            image
                .write_to(&mut Cursor::new(&mut buf), format)
                .map_err(|e| e.to_string())?;
        }
    }

    Ok((buf, width, height))
}

#[derive(serde::Serialize)]
struct CompressionResult {
    bytes: u64,
    width: u32,
    height: u32,
}

/// Size the file would be with these settings, without touching the original.
#[tauri::command]
fn compress_estimate(
    path: String,
    quality: u8,
    scale_percent: u32,
    reduce_colors: bool,
) -> Result<CompressionResult, String> {
    let (buf, width, height) = compress_to_bytes(&path, quality, scale_percent, reduce_colors)?;

    Ok(CompressionResult {
        bytes: buf.len() as u64,
        width,
        height,
    })
}

/// Writes the compressed image. Without `save_as` it replaces the original —
/// but only after the new bytes exist, and never if they came out larger.
#[tauri::command]
fn compress_apply(
    path: String,
    quality: u8,
    scale_percent: u32,
    reduce_colors: bool,
    save_as: Option<String>,
    allow_larger: bool,
) -> Result<CompressionResult, String> {
    let (buf, width, height) = compress_to_bytes(&path, quality, scale_percent, reduce_colors)?;

    let original_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if !allow_larger && save_as.is_none() && buf.len() as u64 >= original_bytes && original_bytes > 0
    {
        return Err(format!(
            "That would make the file bigger ({} vs {}), so nothing was written",
            format_bytes(buf.len() as u64),
            format_bytes(original_bytes)
        ));
    }

    let target = match save_as {
        Some(target) => PathBuf::from(target),
        None => PathBuf::from(&path),
    };

    fs::write(&target, &buf).map_err(|e| e.to_string())?;

    Ok(CompressionResult {
        bytes: buf.len() as u64,
        width,
        height,
    })
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

// ----- CONVERT -----

/// A format this build can write. `lossy` drives whether the quality slider is
/// offered, `keeps_alpha` warns before transparency is thrown away.
#[derive(serde::Serialize)]
struct ConvertTarget {
    ext: String,
    label: String,
    lossy: bool,
    keeps_alpha: bool,
    note: String,
}

fn convert_target_list() -> Vec<ConvertTarget> {
    let target = |ext: &str, label: &str, lossy: bool, keeps_alpha: bool, note: &str| {
        ConvertTarget {
            ext: ext.to_string(),
            label: label.to_string(),
            lossy,
            keeps_alpha,
            note: note.to_string(),
        }
    };

    vec![
        target("png", "PNG", false, true, "Lossless, keeps transparency"),
        target("jpg", "JPEG", true, false, "Lossy, no transparency"),
        target("webp", "WebP", false, true, "Lossless in this build"),
        target("bmp", "BMP", false, false, "Uncompressed"),
        target("tiff", "TIFF", false, true, "Lossless"),
        target("gif", "GIF", false, true, "256 colours, single frame"),
        target("tga", "TGA", false, true, "Lossless"),
        target("qoi", "QOI", false, true, "Fast lossless"),
        target("ico", "ICO", false, true, "Icon, 256×256 max"),
    ]
}

/// Formats the file on screen can be turned into — everything this build can
/// encode, minus the format it already is.
#[tauri::command]
fn convert_targets(path: String) -> Vec<ConvertTarget> {
    let current = Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    let same_as_current = |ext: &str| match current.as_str() {
        "jpg" | "jpeg" | "jfif" => ext == "jpg",
        "tif" | "tiff" => ext == "tiff",
        other => other == ext,
    };

    convert_target_list()
        .into_iter()
        .filter(|t| !same_as_current(&t.ext))
        .collect()
}

/// Never silently replaces a file: "photo.png" becomes "photo (1).png" if
/// something is already there.
fn free_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }

    for n in 1..1000 {
        let candidate = dir.join(format!("{stem} ({n}).{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }

    dir.join(format!("{stem}-{}.{ext}", to_unix(Some(SystemTime::now()))))
}

/// Converts the image to `target_ext` beside the original.
///
/// With `delete_original`, the source goes to the Recycle Bin once the new file
/// is safely written — never deleted outright, and never before.
#[tauri::command]
fn convert_image(
    path: String,
    target_ext: String,
    quality: Option<u8>,
    ico_size: Option<u32>,
    delete_original: bool,
) -> Result<String, String> {
    let source = Path::new(&path);
    let dir = source.parent().unwrap_or_else(|| Path::new("."));
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");

    let target_ext = target_ext.to_lowercase();
    if !convert_target_list().iter().any(|t| t.ext == target_ext) {
        return Err(format!("Cannot write .{target_ext} files"));
    }

    let image = decode_image(&path)?;
    let target = free_path(dir, stem, &target_ext);

    match target_ext.as_str() {
        "jpg" => {
            // JPEG has no alpha; flatten rather than fail
            let rgb = image.to_rgb8();
            let file = File::create(&target).map_err(|e| e.to_string())?;
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                std::io::BufWriter::new(file),
                quality.unwrap_or(90).clamp(1, 100),
            );
            encoder.encode_image(&rgb).map_err(|e| e.to_string())?;
        }
        "ico" => {
            // ICO tops out at 256 in each direction. The chosen size is a box
            // the image is fitted into, so a non-square picture keeps its shape
            // instead of being squashed to a square.
            let size = ico_size.unwrap_or(256).clamp(8, 256);
            let icon = if image.width() > size || image.height() > size {
                image.resize(size, size, image::imageops::FilterType::Lanczos3)
            } else {
                image
            };
            icon.save(&target).map_err(|e| e.to_string())?;
        }
        _ => image.save(&target).map_err(|e| e.to_string())?,
    }

    if delete_original {
        // Only now that the new file exists, and to the Recycle Bin so it can
        // be undone.
        trash::delete(source).map_err(|e| {
            format!(
                "Converted to {}, but could not remove the original: {e}",
                target.display()
            )
        })?;
    }

    Ok(target.to_string_lossy().to_string())
}

/// Downscales to fit inside `max` on the longest edge — the same rule
/// `DynamicImage::thumbnail` follows, but with the convolution done in SIMD.
///
/// fast_image_resize's defaults are already what we want: Lanczos3, and alpha
/// premultiplied for the duration of the resize so transparent edges don't pick
/// up a halo from whatever colour was hiding in the fully-transparent pixels.
fn resize_to_fit(src: DynamicImage, max: u32) -> Result<DynamicImage, String> {
    use fast_image_resize::images::Image;
    use fast_image_resize::{PixelType, ResizeOptions, Resizer};

    let (w, h) = (src.width(), src.height());
    let longest = w.max(h);

    if longest == 0 {
        return Err("image has no pixels".to_string());
    }

    // Nothing to do — and no copy, since the caller handed over ownership.
    if longest <= max {
        return Ok(src);
    }

    let scale = f64::from(max) / f64::from(longest);
    let dst_w = ((f64::from(w) * scale).round() as u32).max(1);
    let dst_h = ((f64::from(h) * scale).round() as u32).max(1);

    // Only carry an alpha channel when the image actually has one, matching what
    // the thumbnail encoder decides below.
    let has_alpha = src.color().has_alpha();
    let (pixel_type, buffer) = if has_alpha {
        (PixelType::U8x4, src.into_rgba8().into_raw())
    } else {
        (PixelType::U8x3, src.into_rgb8().into_raw())
    };

    let src_image = Image::from_vec_u8(w, h, buffer, pixel_type).map_err(|e| e.to_string())?;
    let mut dst_image = Image::new(dst_w, dst_h, pixel_type);

    Resizer::new()
        .resize(&src_image, &mut dst_image, &ResizeOptions::new())
        .map_err(|e| e.to_string())?;

    let raw = dst_image.into_vec();

    if has_alpha {
        RgbaImage::from_raw(dst_w, dst_h, raw).map(DynamicImage::ImageRgba8)
    } else {
        image::RgbImage::from_raw(dst_w, dst_h, raw).map(DynamicImage::ImageRgb8)
    }
    .ok_or_else(|| "resize produced an unexpected buffer size".to_string())
}

// ----- THUMBNAILS -----

// Cache entries are keyed by path + mtime + file size, so an edited or replaced
// file gets a fresh thumbnail without any invalidation bookkeeping.
fn thumb_cache_key(path: &str, size: u32) -> Result<String, String> {
    use std::hash::{Hash, Hasher};

    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    let modified = to_unix(meta.modified().ok());

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.to_lowercase().hash(&mut hasher);
    modified.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    size.hash(&mut hasher);

    Ok(format!("{:016x}", hasher.finish()))
}

/// Returns a downscaled preview of the image, cached on disk between runs so
/// the grid doesn't re-decode whole photos every time it opens.
///
/// `(async)` puts this on the runtime's thread pool. Without it the command runs
/// inline on the IPC thread, so the grid's parallel workers queue up behind each
/// other and every decode freezes the window.
#[tauri::command(async)]
fn get_thumbnail(app: AppHandle, path: String, size: u32) -> Result<Vec<u8>, String> {
    let size = size.clamp(32, 1024);

    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("thumbnails");

    let key = thumb_cache_key(&path, size)?;
    let as_jpeg = dir.join(format!("{key}.jpg"));
    let as_png = dir.join(format!("{key}.png"));

    for cached in [&as_jpeg, &as_png] {
        if let Ok(bytes) = fs::read(cached) {
            return Ok(bytes);
        }
    }

    let thumb = resize_to_fit(decode_image(&path)?, size)?;

    // Keep transparency when the image has it, otherwise JPEG — a folder of
    // photos would otherwise cache tens of megabytes of PNG.
    let (bytes, target) = if thumb.color().has_alpha() {
        let mut buf = Vec::new();
        thumb
            .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        (buf, as_png)
    } else {
        let mut buf = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80)
            .encode_image(&thumb.to_rgb8())
            .map_err(|e| e.to_string())?;
        (buf, as_jpeg)
    };

    // Now that this command runs on a thread pool, two callers can be building
    // the same thumbnail at once. Write to a private file and rename it into
    // place so a third caller can only ever read a complete one — on Windows the
    // rename fails when the destination already exists, which just means the
    // other thread finished first with byte-identical content.
    //
    // A cache write failing (read-only disk, full disk) shouldn't stop the
    // caller getting its thumbnail.
    if fs::create_dir_all(&dir).is_ok() {
        // Unique per call, not just per process — the racing callers are threads
        // inside this one.
        static STAGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let ticket = STAGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let staged = dir.join(format!("{key}.{}-{ticket}.part", std::process::id()));

        if fs::write(&staged, &bytes).is_ok() && fs::rename(&staged, &target).is_err() {
            let _ = fs::remove_file(&staged);
        }
    }

    Ok(bytes)
}

// ----- METADATA -----

/// Strips EXIF/XMP/etc. from the file in place, without re-encoding the pixels.
#[tauri::command]
fn remove_metadata(path: String) -> Result<(), String> {
    use little_exif::metadata::Metadata;

    Metadata::file_clear_metadata(Path::new(&path)).map_err(|e| e.to_string())
}

// ----- QR CODES -----

use std::collections::HashSet;

// Reads every QR code in the image. Goes through decode_image, so anything the
// viewer can display can be scanned. Duplicates (the same code detected twice)
// are collapsed, order of first appearance kept.
#[tauri::command]
fn scan_qr_codes(path: String) -> Result<Vec<String>, String> {
    let gray = decode_image(&path)?.to_luma8();
    let (width, height) = (gray.width(), gray.height());

    // ZXing rather than a lighter decoder: printed codes in the wild are small,
    // colour-printed, scanned and noisy, and simpler detectors find nothing in
    // them at any preprocessing. Restricted to QR so the page's text and rules
    // can't be mistaken for a 1D barcode.
    let mut hints = rxing::DecodeHints::default();
    hints.PossibleFormats = Some(HashSet::from([rxing::BarcodeFormat::QR_CODE]));
    hints.TryHarder = Some(true);

    let results =
        rxing::helpers::detect_multiple_in_luma_with_hints(gray.into_raw(), width, height, &mut hints);

    // "nothing here" comes back as an error; only real failures should surface
    let Ok(results) = results else {
        return Ok(Vec::new());
    };

    let mut found: Vec<String> = Vec::new();
    for result in results {
        let content = result.getText().to_string();
        if !content.is_empty() && !found.contains(&content) {
            found.push(content);
        }
    }

    Ok(found)
}

// ----- SVG CONTROL PANEL -----

// The panel edits raw markup, so both commands refuse anything that isn't a
// .svg: a bad path from the frontend can't overwrite an unrelated file.
fn ensure_svg(path: &str) -> Result<(), String> {
    let is_svg = Path::new(path)
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("svg"))
        .unwrap_or(false);

    if is_svg {
        Ok(())
    } else {
        Err("Not an SVG file".to_string())
    }
}

#[tauri::command]
fn read_svg(path: String) -> Result<String, String> {
    ensure_svg(&path)?;
    fs::read_to_string(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_svg(path: String, contents: String) -> Result<(), String> {
    ensure_svg(&path)?;
    fs::write(&path, contents).map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
struct FileStat {
    /// Milliseconds since the epoch — fine enough that an edit saved in the
    /// same second as the load isn't missed.
    modified: u64,
    size: u64,
}

// Cheap "did the file change / how big is it?" probe: metadata only, no decode
// (unlike load_image_metadata). Drives the on-focus refresh and tells the
// frontend when a file is too large to preload.
#[tauri::command]
fn file_stat(path: String) -> Result<FileStat, String> {
    let meta = fs::metadata(&path).map_err(|e| e.to_string())?;

    let modified = meta
        .modified()
        .map_err(|e| e.to_string())?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .map_err(|e| e.to_string())?;

    Ok(FileStat {
        modified,
        size: meta.len(),
    })
}

fn to_unix(t: Option<SystemTime>) -> u64 {
    t.and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}


#[derive(Serialize)]
pub struct ImageInfoBackend {
    file_name: String,
    format: String,
    width: u32,
    height: u32,
    file_size: u64,

    color_mode: String,
    bit_depth: u8,
    alpha: bool,

    date_taken: Option<String>,
    camera: Option<String>,
    aperture: Option<String>,
    shutter: Option<String>,
    iso: Option<String>,
    focal: Option<String>,
    flash: Option<String>,
    color_profile: Option<String>,

    full_path: String,
    created: u64,
    modified: u64
}

#[tauri::command]
fn load_image_metadata(path: String) -> Result<ImageInfoBackend, String> {

    let reader = ImageReader::open(&path)
        .map_err(|e| format!("Failed to open: {}", e))?;

    let format = reader.format().unwrap_or(ImageFormat::Png);
    drop(reader);

    let img = decode_image(&path).map_err(|e| format!("Decode failed: {}", e))?;

    let (width, height) = img.dimensions();
    let color = img.color();

    let bit_depth = color.bits_per_pixel() as u8;
    let color_mode = format!("{:?}", color);
    let alpha = color.has_alpha();

    let meta = fs::metadata(&path).map_err(|_| "meta fail".to_string())?;

    let created = to_unix(meta.created().ok());
    let modified = to_unix(meta.modified().ok());

    let file_name = Path::new(&path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();

    // EXIF fields
    let mut date_taken = None;
    let mut camera = None;
    let mut aperture = None;
    let mut shutter = None;
    let mut iso = None;
    let mut focal = None;
    let mut flash = None;
    let mut color_profile = None;

    if let Ok(exif) = parse_file(&path) {
        for entry in exif.entries {
            let v = entry.value_more_readable.to_string();

            match entry.tag.to_string().as_str() {
                "DateTimeOriginal" => date_taken = Some(v),
                "Model"            => camera = Some(v),
                "FNumber"          => aperture = Some(v),
                "ExposureTime"     => shutter = Some(v),
                "ISOSpeedRatings"  => iso = Some(v),
                "FocalLength"      => focal = Some(v),
                "Flash"            => flash = Some(v),
                "ColorSpace"       => color_profile = Some(v),
                _ => {}
            }
        }
    }

    Ok(ImageInfoBackend {
        file_name,
        format: format!("{:?}", format),
        width,
        height,
        file_size: meta.len(),

        color_mode,
        bit_depth,
        alpha,

        date_taken,
        camera,
        aperture,
        shutter,
        iso,
        focal,
        flash,
        color_profile,

        full_path: path,
        created,
        modified
    })
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn show_file_properties(path: String) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use winapi::um::shellapi::{ShellExecuteExW, SHELLEXECUTEINFOW};
    use winapi::um::winuser::SW_SHOW;
    
    let verb: Vec<u16> = OsStr::new("properties")
        .encode_wide()
        .chain(Some(0))
        .collect();
    
    let file: Vec<u16> = OsStr::new(&path)
        .encode_wide()
        .chain(Some(0))
        .collect();
    
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = 0x0000000C; // SEE_MASK_INVOKEIDLIST
        info.hwnd = null_mut();
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = null_mut();
        info.lpDirectory = null_mut();
        info.nShow = SW_SHOW;
        
        let result = ShellExecuteExW(&mut info);
        
        if result == 0 {
            return Err("Failed to open file properties".to_string());
        }
    }
    
    Ok(())
}
