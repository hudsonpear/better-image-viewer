<p align="center">
  <img src="https://i.imgur.com/GKl4fNh.png" width="180">
</p>

# <p align="center">Better Image Viewer</p>

<p align="center"> A fast, modern, image viewer built with Tauri. </p>

<p align="center">
  <b>Supported Formats</b><br>
  jpg, jpeg, jfif, png, apng, bmp, gif, webp, avif, ico, cur, svg, tiff, tif, tga, qoi<br>
  jxl, psd, psb, exr, hdr, heic, heif, dds, ktx2<br>
  <b>Camera RAW:</b> cr2, cr3, crw, nef, nrw, arw, srf, sr2, dng, orf, rw2, raf, pef, srw, erf, kdc, dcr, mrw, 3fr, iiq, mos, rwl, x3f
</p>

## Features

📁 **Folder Grid View**

Browse images from a folder using a clean, responsive grid layout. Thumbnails load only as you scroll, are cached on disk so reopening a folder is instant, and each square shows the image format in the corner.

🖼️ **Image Viewer**

Zoom in / out, navigate between images, slideshow. Next and previous images decode ahead of time, so changing image is instant. The folder keeps itself up to date — files added, deleted or renamed show up without reopening it.

🎚️ **Image Controls**

Exposure, brightness, contrast, saturation and hue, per-channel RGB, auto adjust, threshold, blur, sharpen, invert, greyscale, vintage, flip and resize. Brightness, saturation and hue are computed in Oklab, so colors don't go muddy. The preview is produced by the same code that saves, so what you see is what you get.

🔄 **Converter**

Convert to PNG, JPEG, WebP, BMP, TIFF, GIF, TGA, QOI or ICO — only the formats the open image can actually become are offered. Quality slider for JPEG, icon size for ICO, and a warning before converting to a format without transparency. Never overwrites: a taken name becomes `name (1)`.

🗜️ **Compress**

Shows the current file size and what it will become, with the percentage saved, before writing anything. Quality slider for JPEG, color reduction for PNG, size slider for every writable format. Refuses to compress in place if the result would be larger than the original.

🎞️ **GIF Player**

Play / pause, scrub and select the exact frame you want, frame counter, smooth playback, and zoom.

🧩 **ICO & Texture Viewer**

View every icon size inside a `.ico`, and browse mip levels and cube faces of `.dds` / `.ktx2` textures on the same frame bar.

✏️ **SVG Controller**

Rotate, flip, resize, recolor any fill or stroke individually or all at once, and edit the SVG source with preview / save / revert.

🔍 **QR Scanner & OCR**

Read every QR code in an image (links are clickable), and extract text with the Windows OCR engine.

🧹 **Metadata**

Remove EXIF, GPS and camera info without recompressing the image.

⚡ **Fast & Lightweight**

Native performance powered by Tauri + Rust. Decoding runs off the UI thread across every core, thumbnails resize with SIMD, and huge PSD/PSB and KTX2 files are read straight from disk instead of being loaded whole into memory. Opening a file from Explorer reuses the window already running instead of a cold start.

Also: drag and drop a file or folder onto the window, `Ctrl+C` to copy the image to the clipboard, EXIF rotation respected everywhere, and Windows jump-list integration.

## Download

Available for Windows

[Download Latest Release](https://github.com/hudsonpear/better-image-viewer/releases)

## Screenshots

![screenshot1](https://i.imgur.com/TNsBtIc.png)
![screenshot2](https://i.imgur.com/JcObQlU.png)
![screenshot3](https://i.imgur.com/tkyphP9.png)
![screenshot4](https://i.imgur.com/OQzfXng.png)

## Notes

- HEIC / HEIF need the Windows HEVC codec installed.
- Camera RAW is decoded by the app itself; the Windows codec is only a fallback for files it can't read.
- Basis Universal (UASTC / ETC1S) inside KTX2 is not supported and says so instead of failing oddly.

## How to Build

<b>Requirements:</b> Node.js (LTS), Rust, Tauri prerequisites for your OS

Install dependencies with:

```
npm install
```

then run with:

```
npm run tauri dev
```

to compile:

```
npx tauri build
```
