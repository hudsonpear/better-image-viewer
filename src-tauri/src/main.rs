#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ico::IconDir;
use image::{DynamicImage, ImageFormat, ImageReader, RgbaImage};
use std::fs::File;
use std::io::Cursor;
use std::{fs, path::Path, sync::Mutex};
use tauri::{Manager, State};
struct OpenedImage(Mutex<Option<String>>);
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
    *state.0.lock().unwrap() = Some(path);
}

#[tauri::command]
fn get_folder_images(current_path: String) -> (Vec<String>, usize) {
    let path = Path::new(&current_path);
    let dir = path.parent().unwrap();

    let mut images: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|ext| {
                    matches!(
                        ext.to_lowercase().as_str(),
                        "jpg"
                            | "jpeg"
                            | "png"
                            | "bmp"
                            | "gif"
                            | "webp"
                            | "ico"
                            | "avif"
                            | "cur"
                            | "tiff"
                            | "tif"
                            | "svg"
                            | "jfif"
                    )
                })
                .unwrap_or(false)
        })
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    #[cfg(target_os = "windows")]
    images.sort_by(|a, b| {
        let na = Path::new(a).file_name().unwrap().to_string_lossy();
        let nb = Path::new(b).file_name().unwrap().to_string_lossy();
        explorer_compare(&na, &nb)
    });

    let index = images.iter().position(|p| p == &current_path).unwrap_or(0);

    (images, index)
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

fn main() {
    //print_open_with_apps_for_test();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(OpenedImage(Mutex::new(None)))
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
            load_image,
            load_ico_frames,
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

#[cfg(target_os = "linux")]
fn resolve_app_path(exe: &str) -> Option<String> {
    use std::process::Command;
    
    // Check if it's already an absolute path that exists
    let path = std::path::Path::new(exe);
    if path.is_absolute() && path.exists() {
        return Some(exe.to_string());
    }
    
    // Use `which` to find the executable in PATH
    let output = Command::new("which")
        .arg(exe)
        .output()
        .ok()?;
    
    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_string();
        if !path_str.is_empty() {
            return Some(path_str);
        }
    }
    
    // Check common Linux application directories
    let common_paths = [
        "/usr/bin",
        "/usr/local/bin",
        "/opt",
        "/snap/bin",
        "/var/lib/flatpak/exports/bin",
    ];
    
    for dir in common_paths {
        let full_path = format!("{}/{}", dir, exe);
        if std::path::Path::new(&full_path).exists() {
            return Some(full_path);
        }
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

#[cfg(target_os = "linux")]
fn exe_friendly_name(exe_path: &str) -> Option<String> {
    use std::path::Path;
    
    // For Linux, just return the file stem (filename without extension)
    Path::new(exe_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| {
            // Capitalize first letter for display
            let mut chars = s.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
}

#[cfg(target_os = "linux")]
fn extract_icon_fast(_exe_path: &str) -> Option<Vec<u8>> {
    // Icon extraction is not implemented for Linux
    // This would require reading .desktop files and icon themes
    None
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

#[tauri::command]
fn load_image(path: String) -> Result<Vec<u8>, String> {
    let img = ImageReader::open(&path)
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;

    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .map_err(|e| e.to_string())?;

    Ok(buf)
}

// ----- ICO ------

#[derive(serde::Serialize)]
struct IcoFrame {
    width: u32,
    height: u32,
    has_smooth_alpha: bool,
    is_png: bool,
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

        let icon_image = entry.decode().map_err(|e| e.to_string())?;

        let width = icon_image.width();
        let height = icon_image.height();
        let rgba_data = icon_image.rgba_data().to_vec();

        let has_smooth_alpha = has_smooth_alpha(&rgba_data);

        let rgba = RgbaImage::from_raw(width, height, rgba_data)
            .ok_or("Invalid ICO image data")?;

        let mut buf = Vec::new();
        DynamicImage::ImageRgba8(rgba)
            .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .map_err(|e| e.to_string())?;

        frames.push(IcoFrame {
            width,
            height,
            has_smooth_alpha,
            is_png,
            data: buf,
        });
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


#[cfg(target_os = "windows")]
#[tauri::command]
fn open_with(path: String) -> Result<(), String> {
    use std::process::Command;

    Command::new("rundll32.exe")
        .args(["shell32.dll,OpenAs_RunDLL", &path])
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(target_os = "linux")]
#[tauri::command]
fn open_with(path: String) -> Result<(), String> {
    use std::process::Command;

    // On Linux, use xdg-open which opens with the default application
    // or shows an application chooser if configured
    Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
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
#[tauri::command]
fn open_with_dialog(path: String) -> Result<(), String> {
    std::process::Command::new("rundll32.exe")
        .args(["shell32.dll,OpenAs_RunDLL", &path])
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(target_os = "linux")]
#[tauri::command]
fn open_with_dialog(path: String) -> Result<(), String> {
    use std::process::Command;

    // Try to open with a file manager that supports "Open With" dialogs
    // First try xdg-open, which may show an app chooser on some desktop environments
    Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
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

#[cfg(target_os = "linux")]
#[tauri::command]
fn open_native_print_dialog(path: String) -> Result<(), String> {
    use std::process::Command;

    // On Linux, we can use lp (CUPS) or xdg-open with a print scheme
    // lp is the standard printing command on Linux
    let result = Command::new("lp")
        .arg(&path)
        .spawn();

    match result {
        Ok(_) => Ok(()),
        Err(_) => {
            // Fallback: try to open with the default image viewer
            // which usually has a print option
            Command::new("xdg-open")
                .arg(&path)
                .spawn()
                .map_err(|e| format!("Failed to open print dialog: {}", e))?;
            Ok(())
        }
    }
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
fn set_desktop_background(path: String) {
  wallpaper::set_from_path(&path).ok();
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn open_in_explorer(path: String) {
    let _ = std::process::Command::new("explorer")
        .args(["/select,", &path])
        .spawn();
}

#[cfg(target_os = "linux")]
#[tauri::command]
fn open_in_explorer(path: String) {
    use std::path::Path;
    use std::process::Command;

    // Get the parent directory of the file
    let parent = Path::new(&path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());

    // Try different file managers in order of preference
    // First try to select the file if the file manager supports it
    let file_managers = [
        ("nautilus", vec!["--select", &path]),      // GNOME Files
        ("dolphin", vec!["--select", &path]),       // KDE Dolphin
        ("nemo", vec![&path]),                       // Cinnamon Nemo
        ("thunar", vec![&parent]),                   // XFCE Thunar
        ("pcmanfm", vec![&parent]),                  // LXDE PCManFM
        ("xdg-open", vec![&parent]),                 // Fallback to xdg-open
    ];

    for (cmd, args) in file_managers {
        if Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            let _ = Command::new(cmd).args(&args).spawn();
            return;
        }
    }
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
fn rename_file(path: String, new_name: String) -> Result<(), String> {
    let old_path = PathBuf::from(&path);
    let parent = old_path.parent().ok_or("Invalid path")?;
    let ext = old_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let mut new_path = parent.join(&new_name);
    if !ext.is_empty() {
        new_path.set_extension(ext);
    }

    std::fs::rename(&old_path, &new_path).map_err(|e| e.to_string())?;

    Ok(())
}

// ---------------- IMAGE INFO ----------------

use serde::Serialize;
use rexif::parse_file;
use image::GenericImageView;

use std::time::{SystemTime, UNIX_EPOCH};

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

    let img = reader.decode()
        .map_err(|e| format!("Decode failed: {}", e))?;

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

#[cfg(target_os = "linux")]
#[tauri::command]
fn show_file_properties(path: String) -> Result<(), String> {
    use std::process::Command;

    // Try different file managers to show properties
    // Most file managers support a properties dialog via D-Bus or command line
    
    // Try nautilus (GNOME Files) with properties action
    if Command::new("which")
        .arg("nautilus")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        // nautilus doesn't have a direct properties flag, open the parent and let user right-click
        let _parent = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        
        Command::new("nautilus")
            .arg("--select")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    
    // Try dolphin (KDE) with properties
    if Command::new("which")
        .arg("dolphin")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        Command::new("dolphin")
            .arg("--select")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    
    // Fallback: use gio info to display file information
    // This doesn't show a dialog but returns file info
    if Command::new("which")
        .arg("gio")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        // Just open the file manager with selection as fallback
        Command::new("xdg-open")
            .arg(std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new(&path)))
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    
    Err("No supported file manager found to show properties".to_string())
}
