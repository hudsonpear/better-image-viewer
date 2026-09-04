import { svgColorsIn, replaceColorIn, readSvgSize, resizeSvgCode } from "./svgEdit.js";
import { initDraggableWindows, centerWindow } from "./dragWindows.js";

const { getCurrentWebviewWindow, WebviewWindow } = window.__TAURI__.webviewWindow;
const { invoke, convertFileSrc } = window.__TAURI__.core;
const { shell } = window.__TAURI__;
const { open, save } = window.__TAURI__.dialog;
const { listen } = window.__TAURI__.event;

const webview = getCurrentWebviewWindow();

const img = document.getElementById("imgViewer");
const btnPrev = document.getElementById("prev");
const btnNext = document.getElementById("next");
const printBtn = document.getElementById("printBtn");
const openWithBtn = document.getElementById("openWithBtn");

let icoFrames = [];
let icoIndex = 0;

// set false on every showImage() call; lets the img "error" handler retry
// once via the Rust decoder before giving up on a file the browser rejected
let imgLoadFallbackDone = false;

const icoBar = document.getElementById("icoBar");
const icoPrev = document.getElementById("icoPrev");
const icoNext = document.getElementById("icoNext");
const icoInfo = document.getElementById("icoInfo");

let images = [];
let index = 0;

const gifCanvas = document.getElementById('gifCanvas');
const gifBar = document.getElementById('gifBar');
const ctx = gifCanvas.getContext("2d");
const gifPrev = document.getElementById('gifPrev');
const gifNext = document.getElementById('gifNext');
const gifPlayPause = document.getElementById('gifPlayPause');

const slider  = document.getElementById("slider");
const info    = document.getElementById("info");

let gifReader;
let gifWidth, gifHeight, frameCount;
let currentFrame = 0;
let playing = false;
let timer = null;

let gifLoadToken = 0;

let composited;      // master RGBA buffer
let previous;        // for disposal = 3
let frameDelays = [];

let imageData;

let paused = true;   // true = user-paused
let seekTimer = null;
const SEEK_RESUME_DELAY = 120;

const KEYFRAME_INTERVAL = 10;
const keyframes = new Map();

let rotationDegrees = 0;

// -1 mirrors the image on that axis; only the SVG panel sets these today
let flipX = 1;
let flipY = 1;

// net quarter-turns (CW positive) applied to the currently displayed image
// that haven't been saved to disk yet, and the path they belong to. Committed
// (like Windows Photo Viewer) whenever the viewer moves to another image or
// the window closes.
let pendingRotationSteps = 0;
let lastDisplayedPath = null;

let scale = 1;
let baseScale = 1;
let translateX = 0;
let translateY = 0;

const MIN_SCALE = 1;     // THIS is the fitted size
const MAX_SCALE = 100;
const ZOOM_SPEED = 0.0015;

let dragging = false;
let lastX = 0;
let lastY = 0;

let isOriginalSize = false;

const imgAmount = document.getElementById('imgAmount');
const zoomValue = document.getElementById('zoomValue');
const imgSize = document.getElementById('imgSize');

const zoomLabel = document.getElementById('zoomLabel');
const imgLabel = document.getElementById('imgLabel');

const originalBtn = document.getElementById('originalSize');

const ZOOM_STEP = 1.25; // 25% per click (adjust if you want)

const zoomInBtn = document.getElementById("zoomInBtn");
const zoomOutBtn = document.getElementById("zoomOutBtn");
const openWithContextMenu = document.getElementById("openWithContextMenu");

const fileMenuContextMenu = document.getElementById('fileMenuContextMenu');

let openWithImagePath = null;

//const imageSearch = document.getElementById('imageSearch');
const loadingText = document.getElementById("loadingText");
const frame = document.getElementById("middleFrame");
const middleFrame = document.getElementById("middleFrame");

const imgViewerDiv = document.getElementById('imgViewerDiv');

// Store references to open windows keyed by label for control
const imageWindows = new Map();

let currentOpenMenu = null;

// Helper to extract filename from path
function getFileName(path) {
  if (!path) return '';
  return path.split(/[/\\]/).pop();
}

// Helper to get file extension in lowercase
function getExt(path) {
  if (!path) return 'No file selected';
  return path.split(".").pop().toLowerCase();
}

// Formats no browser engine can display, so Rust decodes them to a PNG first.
// HEIF leans on the Windows codecs (see decode_with_wic); the rest have their
// own Rust decoders.
const RUST_DECODED_EXTS = new Set([
  "tif", "tiff", "heic", "heif",
  "jxl", "psd", "psb", "exr", "hdr", "tga", "qoi",
  // camera raw, demosaiced in Rust with Windows as the fallback
  "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "dng", "orf", "rw2",
  "raf", "pef", "srw", "erf", "kdc", "dcr", "mrw", "3fr", "iiq", "mos", "rwl", "x3f"
]);

// Texture containers shown through the frame bar, one entry per mip level/face
const FRAME_LIST_EXTS = new Set(["dds", "ktx2"]);

// path -> { url, isBlob }. Holds the current image and its two neighbours so
// arrow-key browsing doesn't wait on a decode. Reserved entries have url null
// while their decode is still running.
const preloaded = new Map();

// Files past this are shown on demand instead of preloaded — see preloadImage.
const PRELOAD_MAX_BYTES = 16 * 1024 * 1024;

async function rustDecodedUrl(path) {
  const data = await invoke("load_image", { path });
  return URL.createObjectURL(new Blob([new Uint8Array(data)], { type: "image/png" }));
}

function dropPreload(path) {
  const entry = preloaded.get(path);
  if (!entry) return;
  if (entry.isBlob && entry.url) URL.revokeObjectURL(entry.url);
  preloaded.delete(path);
}

async function preloadImage(path) {
  const ext = getExt(path);

  // gif/ico/svg build their pixels through their own paths, nothing to reuse
  if (preloaded.has(path) || ext === "gif" || ext === "ico" || ext === "svg" || FRAME_LIST_EXTS.has(ext)) return;

  preloaded.set(path, { url: null, isBlob: false }); // claim it before awaiting

  try {
    // A decoded image costs width*height*4 bytes, far more than the file, so
    // holding three big ones would run into hundreds of megabytes. File size is
    // the only cheap proxy for that without decoding first.
    const { size } = await invoke("file_stat", { path });
    if (size > PRELOAD_MAX_BYTES) {
      preloaded.delete(path);
      return;
    }

    if (RUST_DECODED_EXTS.has(ext)) {
      preloaded.set(path, { url: await rustDecodedUrl(path), isBlob: true });
    } else {
      const url = freshAssetUrl(path);
      const probe = new Image();
      probe.src = url;
      await probe.decode();
      preloaded.set(path, { url, isBlob: false });
    }
  } catch {
    preloaded.delete(path); // unreadable: let showImage report it
  }
}

// Keeps the current image and its neighbours warm, releases everything else.
function preloadNeighbours() {
  if (!images.length) return;

  const keep = new Set(
    [0, 1, -1].map(offset => images[(index + offset + images.length) % images.length])
  );

  for (const path of [...preloaded.keys()]) {
    if (!keep.has(path)) dropPreload(path);
  }

  for (const path of keep) {
    if (path !== images[index]) preloadImage(path);
  }
}

// convertFileSrc() returns the same URL for the same path forever, so the
// webview's HTTP cache can serve stale bytes after an external editor
// (Paint, etc.) overwrites the file. Bust it with a timestamp on every load.
function freshAssetUrl(path) {
  return `${convertFileSrc(path)}?t=${Date.now()}`;
}

// Open image in a new window with unique label and keep reference
function openImageInNewWindow(imagePath) {
  const label = `image-window-${Date.now()}`;
  const newWindow = new WebviewWindow(label, {
    url: `index.html?image=${encodeURIComponent(imagePath)}`,
    width: 800,
    height: 600,
    title: `${getFileName(imagePath)} - Better Image Viewer`
  });

  imageWindows.set(label, newWindow);

  newWindow.once('tauri://close', () => {
    imageWindows.delete(label);
  });

  return label;
}

// Update image dynamically in an existing window by label
function updateImageInWindow(label, newImagePath) {
  const win = imageWindows.get(label);
  if (win) {
    win.emit('update-image', newImagePath);
  } 
  else {
    console.warn(`No window found with label ${label}`);
  }
}

// Get URL query parameter helper
function getQueryParam(param) {
  const urlParams = new URLSearchParams(window.location.search);
  return urlParams.get(param);
}

// On load, if image query param exists, load and display image
window.addEventListener('DOMContentLoaded', async () => {
  const imagePath = getQueryParam('image');
  if (imagePath) {
    if (img) {
      showLoading();
      img.src = freshAssetUrl(imagePath);
    }
  }
  //await webview.show();
});

// Reload the current image from disk when the window regains focus, so
// edits made in an external editor (Paint, etc.) show up like Windows'
// built-in Photo Viewer does. See #TODO.txt: "Everytime the app focus, update the current image"
// mtime of the file as it was when the current image was put on screen, so a
// refocus can tell "nothing changed" (do nothing at all) from a real edit.
let loadedMtime = { path: null, mtime: null };

async function recordMtime(path) {
  try {
    const { modified } = await invoke("file_stat", { path });
    if (images[index] === path) loadedMtime = { path, mtime: modified };
  } catch {}
}

// Reload without any visible loading state: decode the new bytes off-screen
// first and only swap them in once they're ready, so the old image stays up
// and the change lands in one frame.
async function refreshCurrentImage() {
  const path = images[index];
  const ext = getExt(path);

  dropPreload(path); // the file changed, so anything held for it is stale

  // these build their pixels elsewhere (ico frames / Rust decoders / gif canvas)
  if (ext === "ico" || ext === "gif" || FRAME_LIST_EXTS.has(ext) || RUST_DECODED_EXTS.has(ext)) {
    return showImage();
  }

  const url = freshAssetUrl(path);
  const probe = new Image();
  probe.src = url;

  try {
    await probe.decode();
  } catch {
    return; // unreadable or half-written: keep what's on screen
  }

  img.src = url; // already decoded and cached, so this paints immediately
  recordMtime(path);
}

webview.onFocusChanged(async ({ payload: focused }) => {
  if (!focused || inGridMode || !images.length) return;

  const path = images[index];
  let mtime;
  try {
    mtime = (await invoke("file_stat", { path })).modified;
  } catch {
    return; // file gone: leave the last good frame up
  }

  if (loadedMtime.path === path && loadedMtime.mtime === mtime) return;

  loadedMtime = { path, mtime };
  refreshCurrentImage();
});

// The window title and the "3/57" counter. Split out of showImage so the folder
// watcher can correct the count when files appear or vanish, without reloading
// the image that is already on screen.
async function updateImageCounter() {
  const path = images[index];
  if (!path) return;

  await webview.setTitle(
    `${getFileName(path)} (${index + 1}/${images.length}) - Better Image Viewer`
  );
  imgAmount.textContent = `${index + 1}/${images.length}`;
}

async function showImage() {
  zoomLabel.style.display = 'none';
  imgLabel.style.display = 'none';
  imgSize.textContent = '';
  zoomValue.textContent = '';

  resetGifUI();
  showLoading();

  if (!images.length) {
    lastDisplayedPath = null;
    clearViewer();
    return;
  }

  const path = images[index];
  const ext = getExt(path);

  rotationDegrees = 0;
  flipX = 1;
  flipY = 1;
  imgLoadFallbackDone = false;

  icoBar.classList.add("hidden");
  icoFrames = [];
  icoIndex = 0;

  gifCanvas.classList.add("hidden");
  gifBar.classList.add("hidden");
  img.classList.remove("hidden");

  // These formats are decoded before they ever reach <img>, so a failure here
  // throws instead of firing the img "error" handler — without this catch the
  // viewer would sit on "Loading…" forever.
  try {
    if (ext === "ico" || FRAME_LIST_EXTS.has(ext)) {
      showLoading();
      await nextImgFrame();
      // DDS carries mip levels and cubemap faces where ICO carries sizes —
      // both are just a list of frames to step through.
      icoFrames = await invoke(FRAME_LIST_EXTS.has(ext) ? "load_dds_frames" : "load_ico_frames", { path });
      icoIndex = 0;
      icoBar.classList.remove("hidden");
      showIcoFrame();
    }
    else if (RUST_DECODED_EXTS.has(ext)) {
      const ready = preloaded.get(path)?.url;
      if (ready) {
        img.src = ready;
      } else {
        showLoading();
        img.src = await rustDecodedUrl(path);
      }
    }
    else if (ext === "gif") {
      showLoading();
      img.classList.add("hidden");
      gifCanvas.classList.remove("hidden");
      icoBar.classList.add("hidden");
      gifBar.classList.remove("hidden");
      await loadGIF(path);
    }
    else {
      img.src = preloaded.get(path)?.url || freshAssetUrl(path);
    }
  } catch (err) {
    console.error(`Failed to open ${path}:`, err);
    showError(`Can't open ${getFileName(path)}`);
    lastDisplayedPath = path;
    return;
  }

  // Apply rotation (which is reset to 0 here)
  img.style.transform = `translate(0px, 0px) scale(1) rotate(${rotationDegrees}deg)`;

  await updateImageCounter();

  if (ext === "svg") {
    openSvgPanel(path);
  } else {
    closeSvgPanel();
  }

  // A different file means any pending rotation belongs to the old one
  if (imgEditPath !== path) {
    imgEditPath = path;
    imgAdjust.rotate = 0;
  }
  syncRotationUi();
  syncRotateButtons();

  // Image Controls, Convert and Compress are raster tools — a vector has the
  // SVG panel instead
  if (ext === "svg") {
    for (const [button, panel] of rasterPanels()) {
      button.classList.add("hidden");
      button.classList.remove("active");
      panel.classList.add("hidden");
    }
  } else {
    for (const [button] of rasterPanels()) button.classList.remove("hidden");

    // whichever panel is open now describes a different file
    if (!convertPanel.classList.contains("hidden")) refreshConvertTargets();
    if (!compressPanel.classList.contains("hidden")) refreshCompressOptions();
  }

  recordMtime(path);
  preloadNeighbours();

  // a different file means the old adjustments no longer apply
  if (!imgPanel.classList.contains("hidden") && imgEditPath !== path) {
    resetImgAdjustments();
    refreshImgCapabilities();
  }

  lastDisplayedPath = path;
}

// Brief message at the bottom of the viewer. Used where alert() would be too
// heavy-handed but staying silent would hide something the user should know.
let toastTimer = null;

function showToast(message, duration = 3200) {
  let toast = document.getElementById("appToast");

  if (!toast) {
    toast = document.createElement("div");
    toast.id = "appToast";
    // inside the viewer area so it sits under the topbar, not over it
    (document.getElementById("middleFrame") || document.body).appendChild(toast);
  }

  toast.textContent = message;
  toast.classList.add("visible");

  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toast.classList.remove("visible"), duration);
}

function nextImgFrame() {
  return new Promise(requestAnimationFrame);
}

// Initialize images array and index from backend on app start
(async () => {
  const path = await invoke("get_opened_image");
  if (path) await openPath(path);
})();

// Single instance: a second launch (Explorer double-click, "Open with") forwards
// its path here instead of starting another process, so this window has to react
// to it the same way it reacts to a drop or the file dialog.
listen("opened-image", (event) => openPath(event.payload));

// Next and previous image handlers
async function nextImage() {
  showLoading();
  if (!images.length) return;
  index = (index + 1) % images.length;
  await showImage();
}

async function prevImage() {
  showLoading();
  if (!images.length) return;
  index = (index - 1 + images.length) % images.length;
  await showImage();
}

btnNext.addEventListener("click", nextImage);
btnPrev.addEventListener("click", prevImage);

document.addEventListener("keydown", (e) => {
  if (isTypingTarget(e.target)) return;
  if (e.key === "ArrowRight") nextImage();
  if (e.key === "ArrowLeft") prevImage();
  if (e.key === "ArrowUp") nextImage();
  if (e.key === "ArrowDown") prevImage();
  if (e.key === "F5") {
    e.preventDefault();
    //reloadCurrentImage();
    location.reload(true); // force hard reload
  }
});

function reloadCurrentImage() {
  const img = document.getElementById("imgViewer");
  if (!img.src) return;

  const originalSrc = img.src.split("?")[0]; // remove previous cache buster if any
  img.src = originalSrc + "?reload=" + Date.now();

  console.log("img reloaded")
}


// Icon frames handling for ICO files
async function showIcoFrame() {
  showLoading();
  await nextImgFrame();

  const frame = icoFrames[icoIndex];

  // DDS frames arrive without pixels: a texture's mip levels are megabytes of
  // PNG in total, so each is rendered only when shown, to a cached file the
  // webview loads directly (bytes over IPC would be JSON-encoded numbers).
  if (!frame.data) {
    try {
      if (!frame.url) {
        frame.url = await invoke("load_dds_frame", { path: images[index], index: icoIndex });
      }
      img.src = convertFileSrc(frame.url);
    } catch (err) {
      console.error("Frame failed to load:", err);
      showError(`Can't show frame ${icoIndex + 1}`);
      return;
    }
  } else {
    const blob = new Blob(
      [new Uint8Array(frame.data)],
      { type: "image/png" }
    );

    img.src = URL.createObjectURL(blob);
  }

  // A texture frame names itself ("Mip 2", "+X Mip 1"), so it doesn't also need
  // the word "Frame" — but the total still belongs there, at the end.
  const count = `${icoIndex + 1}/${icoFrames.length}`;

  icoInfo.textContent = frame.label
    ? `${frame.label} — ${frame.width}×${frame.height} (${count})`
    : `Frame ${count} — ${frame.width}×${frame.height}`;
}

icoNext.addEventListener("click", async () => {
  icoIndex = (icoIndex + 1) % icoFrames.length;
  await showIcoFrame()
});

icoPrev.addEventListener("click", async () => {
  icoIndex = (icoIndex - 1 + icoFrames.length) % icoFrames.length;
  await showIcoFrame();
});

// --- Open With Context Menu ---

let openWithMenuLoading = false;

async function showOpenWithMenu(x, y) {
  // guard against re-entrant calls: without this, clicking the button again
  // while the previous invoke+icon-fetch is still pending starts a second
  // populate pass that races the first and duplicates every app entry
  if (openWithMenuLoading) return;
  openWithMenuLoading = true;

  try {
    const path = images[index];
    if (!path) {
      alert("No image loaded.");
      return;
    }
    openWithImagePath = path;
    const apps = await invoke("get_open_with_apps", { path });

    openWithContextMenu.querySelectorAll(".ctx-item.app").forEach(e => e.remove());

    for (const [exe, label, iconBytes] of apps) {
      const item = document.createElement("div");
      item.className = "ctx-item app";
      item.dataset.action = "openWithApp";
      item.dataset.app = exe;

      const iconContainer = document.createElement("div");
      iconContainer.className = "app-icon";

      if (iconBytes && iconBytes.length > 0) {
        const iconUrl = await fixIconColors(iconBytes);
        const imgIcon = document.createElement("img");
        imgIcon.src = iconUrl;
        imgIcon.className = "app-icon-img";
        iconContainer.appendChild(imgIcon);
      }
      else {
        const fallback = document.createElement("div");
        fallback.className = "app-icon-fallback";
        fallback.textContent = label.charAt(0).toUpperCase();
        iconContainer.appendChild(fallback);
      }

      const textSpan = document.createElement("span");
      textSpan.className = "app-name";
      textSpan.textContent = label;

      item.appendChild(iconContainer);
      item.appendChild(textSpan);

      openWithContextMenu.insertBefore(item, openWithSeparator);
    }

    openWithContextMenu.style.left = `${x}px`;
    openWithContextMenu.style.top = `${y}px`;
    openWithContextMenu.style.display = "block";
  } finally {
    openWithMenuLoading = false;
  }
}

async function fixIconColors(iconBytes) {
  const uint8Array = new Uint8Array(iconBytes);
  const blob = new Blob([uint8Array], { type: "image/png" });
  const url = URL.createObjectURL(blob);

  return new Promise((resolve) => {
    const imgFix = new Image();
    imgFix.onload = () => {
      const canvas = document.createElement("canvas");
      canvas.width = imgFix.width;
      canvas.height = imgFix.height;
      const ctx = canvas.getContext("2d");
      ctx.drawImage(imgFix, 0, 0);

      const imgData = ctx.getImageData(0, 0, canvas.width, canvas.height);
      const data = imgData.data;

      for (let i = 0; i < data.length; i += 4) {
        const temp = data[i];
        data[i] = data[i + 2];
        data[i + 2] = temp;
      }

      ctx.putImageData(imgData, 0, 0);

      canvas.toBlob((fixedBlob) => {
        const fixedUrl = URL.createObjectURL(fixedBlob);
        resolve(fixedUrl);
      }, "image/png");
    };
    imgFix.src = url;
  });
}

openWithContextMenu.addEventListener("click", async (e) => {
  e.stopPropagation();
  const item = e.target.closest(".ctx-item");
  if (!item) return;

  const action = item.dataset.action;
  try {
    if (action === "openWithApp") {
      await invoke("open_with_app", {
        app: item.dataset.app,
        path: openWithImagePath,
      });
    }

    if (action === "openWithDialog") {
      await invoke("open_with_dialog", {
        path: openWithImagePath,
      });
    }
  } 
  catch (err) {
    console.error("Error invoking command:", err);
  }

  openWithContextMenu.style.display = "none";
});

document.addEventListener("click", (e) => {
  if (!openWithContextMenu.contains(e.target)) {
    openWithContextMenu.style.display = "none";
  }
  if (!fileMenuContextMenu.contains(e.target)) {
    fileMenuContextMenu.style.display = "none";
  }
});

openWithBtn.addEventListener("click", (e) => {
  e.stopPropagation();

  // If already open → close it
  if (openWithContextMenu.style.display === "block") {
    openWithContextMenu.style.display = "none";
    return;
  }

  // Otherwise open it
  const rect = e.currentTarget.getBoundingClientRect();
  showOpenWithMenu(0, rect.bottom);
});

// --- Print Button ---

printBtn.addEventListener('click', async () => {
  if (!images.length) {
    alert("No image loaded.");
    return;
  }

  const filePath = images[index];

  await invoke('open_native_print_dialog', { path: filePath }).catch(console.error);
});

// --- Image Search ---

/* imageSearch.addEventListener('click', async () => {
  await shell.open('https://images.google.com/');
}); */

// --- Open current image in new window ---

/* document.getElementById("openInNewWindow").addEventListener("click", () => {
  if (!images.length) return;

  const currentImagePath = images[index];
  console.log("Opening in new window:", currentImagePath);
  openImageInNewWindow(currentImagePath);
});
 */

// --- FILE SELECTOR ---


// Every way in — the file dialog, the folder dialog, a drop on the window —
// ends up here. get_folder_images takes a file or a folder and returns the
// list plus where to start, so the supported-format list lives only in Rust.
async function openPath(path) {
  try {
    const [list, startIndex] = await invoke("get_folder_images", { currentPath: path });

    if (!list.length) {
      showError(`No images in ${getFileName(path)}`);
      return;
    }

    images = list;
    index = startIndex;

    await invoke("set_opened_image", { path: images[index] });

    // Watch whatever folder we just landed in. Failing here only costs the
    // live-refresh, so it must not stop the image from opening.
    invoke("watch_folder", { path: images[index] }).catch((err) =>
      console.error("Could not watch folder:", err)
    );

    if (inGridMode) {
      exitGridMode();
    } else {
      await showImage();
    }
  } catch (err) {
    console.error("Could not open", path, err);
    showError(`Could not open ${getFileName(path)}`);
  }
}

// The folder can change underneath the viewer: a download lands, a file is
// deleted from Explorer, a batch is renamed. Debounced because copying twenty
// files in fires an event per file.
let folderRefreshTimer = null;

listen("folder-changed", () => {
  clearTimeout(folderRefreshTimer);
  folderRefreshTimer = setTimeout(refreshFolder, 250);
});

async function refreshFolder() {
  const current = images[index];
  if (!current) return;

  let list;
  try {
    // get_folder_images only uses the path to find its folder, so this still
    // works when the file it names is the one that was just deleted.
    [list] = await invoke("get_folder_images", { currentPath: current });
  } catch (err) {
    console.error("Could not refresh folder:", err);
    return;
  }

  images = list;

  if (!images.length) {
    showToast("No images left in this folder");
    return;
  }

  // Files inserted before the current one shift its position; follow it rather
  // than letting the index point at a different image.
  const moved = images.indexOf(current);
  index = moved >= 0 ? moved : Math.min(index, images.length - 1);

  if (inGridMode) {
    populateGrid();
  } else if (moved < 0) {
    // The image on screen is gone — show whatever took its place.
    await showImage();
  } else {
    // Still on the same image, but "3 of 57" may now be "3 of 58".
    await updateImageCounter();
  }
}

async function openFileSelect() {
  const selected = await open({
    multiple: false,
    filters: [
      {
        name: "Image",
        extensions: [
          "png","jpeg","jpg","gif","bmp","ico","tif","tiff","avif",
          "webp","cur","svg","jfif","heic","heif","dds","ktx2","apng","tga","qoi",
          "jxl","psd","psb","exr","hdr",
          "crw","nrw","srf","erf","kdc","dcr","mrw","3fr","iiq","mos","rwl","x3f",
          "cr2","cr3","nef","arw","dng","orf","rw2","raf","sr2","pef","srw"
        ]
      }
    ]
  });

  if (!selected) return;

  await openPath(Array.isArray(selected) ? selected[0] : selected);
}

async function openFolderSelect() {
  const selected = await open({ multiple: false, directory: true });
  if (!selected) return;

  await openPath(Array.isArray(selected) ? selected[0] : selected);
}

// Dropping a file opens it in its folder; dropping a folder opens its first
// image. Only the first item matters — the rest of the folder comes with it.
webview.onDragDropEvent(async ({ payload }) => {
  if (payload.type !== "drop" || !payload.paths?.length) return;
  await openPath(payload.paths[0]);
});

function showLoading() {
  loadingText.textContent = "Loading…";
  loadingText.classList.add("visible");
  img.classList.add("loading");
}

// Failure card in the top-right corner instead of the spinner: the file stays
// in the list so the user can still navigate past it, unlike the img "error"
// path which drops it.
function showError(message) {
  clearViewer();
  loadingText.classList.remove("visible");
  img.classList.remove("loading");
  // longer than a normal notice: the viewer is blank, so this card is the only
  // thing telling the user what happened
  showToast(message, 7000);
}

function hideLoading() {
  loadingText.classList.remove("visible");
  img.classList.remove("loading");
}

img.addEventListener("load", hideLoading);
img.addEventListener("error", hideLoading);

// -------------------- NEW GIF SYSTEM ----------------------

gifPlayPause.onclick = () => playing ? pause() : play();

gifPrev.onclick = () => {
  pause();
  const target = (currentFrame - 1 + frameCount) % frameCount;
  resetAndDecodeTo(target);
};

gifNext.onclick = () => {
  pause();
  const target = (currentFrame + 1) % frameCount;
  resetAndDecodeTo(target);
};

function stopGifPlayback() {
  playing = false;

  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
}

function step() {
  if (!gifReader || frameCount === 0) return;
  decodeFrame(currentFrame, true); // dispose old frame
  currentFrame = (currentFrame + 1) % frameCount;
  decodeFrame(currentFrame, false); // show new frame
  draw();
  if (playing) {
    timer = setTimeout(step, frameDelays[currentFrame]);
  }
}

function play() {
  if (playing) return;
  //updateEQSliderFill(slider);
  playing = true;
  paused = false;
  gifPlayPause.textContent = "⏸";
  timer = setTimeout(step, frameDelays[currentFrame]);
}

function pause() {
  //updateEQSliderFill(slider);
  playing = false;
  paused = true;
  gifPlayPause.textContent = "▶";
  clearTimeout(timer);
}

slider.oninput = e => {
    // always stop any pending playback timer before reseeking, so step()
    // can't fire concurrently with resetAndDecodeTo and corrupt the frame buffer
    stopGifPlayback();
    resetAndDecodeTo(+e.target.value);
    if (!paused) {
      play();
    }
};

// ---- LOAD GIF -------------------------------------------------

async function loadGIF(path) {
  const token = ++gifLoadToken;
  const url = freshAssetUrl(path);
  const res = await fetch(url, { cache: "no-store" });
  if (token !== gifLoadToken) return;
  const buf = await res.arrayBuffer();
  if (token !== gifLoadToken) return;

  gifReader = new GifReader(new Uint8Array(buf));

  gifWidth = gifReader.width;
  gifHeight = gifReader.height;
  imgLabel.style.display = 'block';
  imgSize.textContent = `${gifWidth} × ${gifHeight}`;

  computeBaseScaleForGIF();

  frameCount = gifReader.numFrames();

  currentFrame = 0;
  paused = true;

  keyframes.clear();

  //gifCanvas.width = gifWidth;
  //gifCanvas.height = gifHeight;
  gifCanvas.width = middleFrame.clientWidth;
  gifCanvas.height = middleFrame.clientHeight;

  slider.min = 0;
  slider.max = frameCount - 1;
  slider.step = 1;
  slider.value = 0;

  updateEQSliderFill(slider);

  composited = new Uint8Array(gifWidth * gifHeight * 4);
  composited.fill(0);

  keyframes.set(0, composited.slice());

  imageData = ctx.createImageData(gifWidth, gifHeight);

  frameDelays = [];

  for (let i = 0; i < frameCount; i++) {
    frameDelays.push(gifReader.frameInfo(i).delay * 10 || 100);
  }

  resetAndDecodeTo(0);
  hideLoading();
  play();
}

function computeBaseScaleForGIF() {
  const frame = document.getElementById("middleFrame");

  const fw = frame.clientWidth;
  const fh = frame.clientHeight;

  if (!gifWidth || !gifHeight) return;

  // compute scale so GIF fits inside frame
  const scaleX = fw / gifWidth;
  const scaleY = fh / gifHeight;

  baseScale = Math.min(scaleX, scaleY);
  scale = baseScale;
  translateX = 0;
  translateY = 0;
  rotationDegrees = 0;
}

window.addEventListener("resize", () => {
  if (!gifReader) return;
  gifCanvas.width = middleFrame.clientWidth;
  gifCanvas.height = middleFrame.clientHeight;
  draw();
});


// ---- FRAME DECODING ------------------------------------------

let offscreenCanvas = document.createElement("canvas");
let offscreenCtx = offscreenCanvas.getContext("2d");

function clearRect(info) {
  for (let y = info.y; y < info.y + info.height; y++) {
    for (let x = info.x; x < info.x + info.width; x++) {
      const idx = (y * gifWidth + x) * 4;
      composited[idx + 3] = 0;
    }
  }
}

function decodeFrame(i, applyDisposal = true) {
  const info = gifReader.frameInfo(i);

  if (info.disposal === 3) {
    previous = composited.slice();
  }

  gifReader.decodeAndBlitFrameRGBA(i, composited);

  // apply disposal ONLY if requested
  if (applyDisposal) {
    if (info.disposal === 2) {
      clearRect(info);
    } 
    else if (info.disposal === 3 && previous) {
      composited.set(previous);
    }
  }

  if (i % KEYFRAME_INTERVAL === 0) {
    keyframes.set(i, composited.slice());
  }
}

/* function draw() {
  if (currentFrame > frameCount - 1) {
    currentFrame = frameCount - 1;
  }
  imageData.data.set(composited);
  ctx.putImageData(imageData, 0, 0);
  slider.value = currentFrame;
  info.textContent = `Frame: ${currentFrame + 1}/${frameCount}`;

  updateEQSliderFill(slider);
} */
function draw() {
  if (currentFrame > frameCount - 1) {
    currentFrame = frameCount - 1;
  }

  // copy decoded pixels → imageData
  imageData.data.set(composited);

  // update offscreen buffer
  offscreenCanvas.width = gifWidth;
  offscreenCanvas.height = gifHeight;
  offscreenCtx.putImageData(imageData, 0, 0);

  // render with transforms
  renderGifFrame(offscreenCanvas);

  slider.value = currentFrame;
  info.textContent = `Frame: ${currentFrame + 1}/${frameCount}`;

  updateEQSliderFill(slider);
}

function renderGifFrame(source) {
  const canvas = gifCanvas;
  const ctx = canvas.getContext("2d");

  ctx.clearRect(0, 0, canvas.width, canvas.height);

  ctx.save();

  // Move origin to center of canvas
  ctx.translate(canvas.width / 2, canvas.height / 2);

  // Apply viewer transforms
  ctx.translate(translateX, translateY);
  ctx.scale(scale, scale);
  ctx.rotate(rotationDegrees * Math.PI / 180);

  // Draw centered
  ctx.drawImage(
    source,
    -gifWidth / 2,
    -gifHeight / 2
  );

  ctx.restore();
}


function resetAndDecodeTo(target) {
  let start = 0;
  for (const k of keyframes.keys()) {
    if (k <= target && k > start) start = k;
  }

  if (keyframes.has(start)) {
    composited.set(keyframes.get(start));
  } 
  else {
    composited.fill(0);
    keyframes.set(0, composited.slice());
  }

  previous = null;

  for (let i = start + 1; i < target; i++) {
    decodeFrame(i, true);
  }

  decodeFrame(target, false);

  currentFrame = target;
  draw();
}


function updateEQSliderFill(slider) {
  const min = Number(slider.min);
  const max = Number(slider.max);
  const val = Number(slider.value);

  if (max <= min) {
    slider.style.background = "#5e5e5eff";
    return;
  }

  const percent = ((val - min) / (max - min)) * 100;

  slider.style.background = `
    linear-gradient(
      to right,
      #ee2727ff ${percent}%,
      #5e5e5eff ${percent}%
    )
  `;
}

slider.addEventListener("input", () => {
  updateEQSliderFill(slider);
});

function resetGifUI() {
  // Stop playback completely
  stopGifPlayback();

  // Kill decoder state
  gifReader = null;
  frameCount = 0;
  currentFrame = 0;
  paused = true;

  keyframes.clear();
  frameDelays = [];

  composited = null;
  previous = null;
  imageData = null;

  // Clear canvas visually
  ctx.clearRect(0, 0, gifCanvas.width, gifCanvas.height);

  // Shrink canvas so browser drops backing store
  gifCanvas.width = 1;
  gifCanvas.height = 1;

  // Reset slider + info
  slider.min = 0;
  slider.max = 0;
  slider.value = 0;
  slider.style.background = "#5e5e5eff";
  info.textContent = "Frame: 0/0";
}

// ------------------------- ZOOM AND DRAG ------------------------

function getActiveViewer() {
  const img = document.getElementById("imgViewer");
  const canvas = document.getElementById("gifCanvas");
  //console.log("img",img.classList.contains("hidden"))
  //console.log("canvas",canvas.classList.contains("hidden"))
  // the one that is NOT hidden is the active viewer
  if (!img.classList.contains("hidden")) return img;
  if (!canvas.classList.contains("hidden")) return canvas;

  return null;
}

img.addEventListener("load", computeBaseScale);

// Mouse wheel zoom in the middleFrame

imgViewerDiv.addEventListener("wheel", e => {
  if (inGridMode) return;  // <-- Do not block wheel scroll!
  //if (!img.src) return;
  e.preventDefault();

  const rect = imgViewerDiv.getBoundingClientRect();

  // mouse position relative to frame center
  const mx = e.clientX - rect.left - rect.width / 2;
  const my = e.clientY - rect.top  - rect.height / 2;

  // image-local coords BEFORE scaling
  const ix = (mx - translateX) / scale;
  const iy = (my - translateY) / scale;

  let newScale = scale * (1 - e.deltaY * ZOOM_SPEED);
  newScale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, newScale));

  if (newScale === scale) return;

  // keep pixel under cursor stable
  translateX = mx - ix * newScale;
  translateY = my - iy * newScale;

  scale = newScale;
  updateTransform();
}, { passive: false });

// ---Click + drag pan----

img.addEventListener("mousedown", e => {
  if (e.button !== 0) return;
  if (scale === 1) return;

  dragging = true;
  lastX = e.clientX;
  lastY = e.clientY;
  img.classList.add("dragging");
});

window.addEventListener("mousemove", e => {
  if (!dragging) return;

  translateX += e.clientX - lastX;
  translateY += e.clientY - lastY;

  lastX = e.clientX;
  lastY = e.clientY;

  updateTransform();
});

window.addEventListener("mouseup", () => {
  dragging = false;
  img.classList.remove("dragging");
});

function resetView() {
  scale = 1;
  translateX = 0;
  translateY = 0;
  updateTransform();
}

img.addEventListener("load", resetView);

img.addEventListener("load", () => {
  scale = 1;
  translateX = 0;
  translateY = 0;
  isOriginalSize = false;

  originalBtn.title = "Original Size";
  originalBtn.innerHTML = originalSizeIcon;

  updateTransform();

  const w = img.naturalWidth;
  const h = img.naturalHeight;
  imgLabel.style.display = 'block';
  imgSize.textContent = `${w} × ${h}`;

});


// ------------- ORIGINAL SIZE ---------------------

const originalSizeIcon = `
<svg class="barIconSvg" viewBox="0 -960 960 960">
  <path d="M800-640v-80h-80q-17 0-28.5-11.5T680-760q0-17 11.5-28.5T720-800h80q33 0 56.5 23.5T880-720v80q0 17-11.5 28.5T840-600q-17 0-28.5-11.5T800-640Zm-720 0v-80q0-33 23.5-56.5T160-800h80q17 0 28.5 11.5T280-760q0 17-11.5 28.5T240-720h-80v80q0 17-11.5 28.5T120-600q-17 0-28.5-11.5T80-640Zm720 480h-80q-17 0-28.5-11.5T680-200q0-17 11.5-28.5T720-240h80v-80q0-17 11.5-28.5T840-360q17 0 28.5 11.5T880-320v80q0 33-23.5 56.5T800-160Zm-640 0q-33 0-56.5-23.5T80-240v-80q0-17 11.5-28.5T120-360q17 0 28.5 11.5T160-320v80h80q17 0 28.5 11.5T280-200q0 17-11.5 28.5T240-160h-80Zm80-240v-160q0-33 23.5-56.5T320-640h320q33 0 56.5 23.5T720-560v160q0 33-23.5 56.5T640-320H320q-33 0-56.5-23.5T240-400Zm80 0h320v-160H320v160Zm0 0v-160 160Z"/>
</svg> `;

const fitScreen = `
<svg class="barIconSvg" viewBox="0 -960 960 960">
  <path d="M240-240h-80q-17 0-28.5-11.5T120-280q0-17 11.5-28.5T160-320h120q17 0 28.5 11.5T320-280v120q0 17-11.5 28.5T280-120q-17 0-28.5-11.5T240-160v-80Zm480 0v80q0 17-11.5 28.5T680-120q-17 0-28.5-11.5T640-160v-120q0-17 11.5-28.5T680-320h120q17 0 28.5 11.5T840-280q0 17-11.5 28.5T800-240h-80ZM240-720v-80q0-17 11.5-28.5T280-840q17 0 28.5 11.5T320-800v120q0 17-11.5 28.5T280-640H160q-17 0-28.5-11.5T120-680q0-17 11.5-28.5T160-720h80Zm480 0h80q17 0 28.5 11.5T840-680q0 17-11.5 28.5T800-640H680q-17 0-28.5-11.5T640-680v-120q0-17 11.5-28.5T680-840q17 0 28.5 11.5T720-800v80Z"/>
</svg>`;

originalBtn.innerHTML = originalSizeIcon;

function getOriginalScale() {
  // rendered size from CSS (fit)
  const fittedWidth = img.clientWidth;
  const fittedHeight = img.clientHeight;

  // real image size
  const naturalWidth = img.naturalWidth;
  const naturalHeight = img.naturalHeight;

  // scale needed to reach 1:1 pixels
  return Math.min(
    naturalWidth / fittedWidth,
    naturalHeight / fittedHeight
  );
}
/* function getOriginalScale() {
  const imgVisible = !img.classList.contains("hidden");
  const gifVisible = !gifCanvas.classList.contains("hidden");

  if (imgVisible) {
    // normal image
    const fittedWidth = img.clientWidth;
    const fittedHeight = img.clientHeight;

    const naturalWidth = img.naturalWidth;
    const naturalHeight = img.naturalHeight;

    return Math.min(
      naturalWidth / fittedWidth,
      naturalHeight / fittedHeight
    );
  }

  if (gifVisible && gifReader) {
    // GIF: compare canvas fit size and gifWidth/gifHeight
    const fittedWidth = gifCanvas.clientWidth;
    const fittedHeight = gifCanvas.clientHeight;

    return Math.min(
      gifWidth / fittedWidth,
      gifHeight / fittedHeight
    );
  }

  return 1;
} */

originalBtn.addEventListener("click", () => {
  if (!img.src) return;

  if (!isOriginalSize) {
    // ➜ original size
    scale = getOriginalScale();
    translateX = 0;
    translateY = 0;
    isOriginalSize = true;

    originalBtn.title = "Fit to window";
    originalBtn.innerHTML = fitScreen;
  } 
  else {
    // ➜ fit
    scale = 1;
    translateX = 0;
    translateY = 0;
    isOriginalSize = false;

    originalBtn.title = "Original Size";
    originalBtn.innerHTML = originalSizeIcon;
  }

  updateTransform();
});

// ---------- ZOOM -------------

zoomInBtn.addEventListener("click", () => {
  if (!img.src) return;
  zoomRelative(ZOOM_STEP);
});

zoomOutBtn.addEventListener("click", () => {
  if (!img.src) return;
  zoomRelative(1 / ZOOM_STEP);
});


function zoomRelative(factor) {
  let newScale = scale * factor;
  newScale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, newScale));

  if (newScale === scale) return;

  const ratio = newScale / scale;

  // 🔑 preserve current view direction
  translateX *= ratio;
  translateY *= ratio;

  scale = newScale;

  // exit original-size mode if active
  if (isOriginalSize) {
    isOriginalSize = false;
    originalBtn.title = "Original Size";
    originalBtn.innerHTML = fitScreen;
  }

  updateTransform();
}

// --------------- DELETE BTN -----------------

let deleting = false;

const deleteBtn = document.getElementById("deleteBtn");

deleteBtn.addEventListener("click", async () => {
  if (!images.length) return;

  const path = images[index];
  const fileName = getFileName(path);

  const confirmed = await confirmDlg(`Send "${fileName}" to the Recycle Bin?`);
  if (!confirmed) return;

  deleting = true; // 🔑 important

  try {
    await invoke("trash_file", { path });

    images.splice(index, 1);

    if (images.length === 0) {
      clearViewer();
      return;
    }

    if (index >= images.length) {
      index = images.length - 1;
    }

    await showImage();
  } catch (err) {
    console.error("Trash failed:", err);
  } 
  finally {
    deleting = false;
  }
});

document.addEventListener("keydown", (e) => {
  if (confirmDialogOpen) return;

  const tag = e.target.tagName.toLowerCase();
  if (tag === "input" || tag === "textarea") return;

  if (e.key === "Delete") {
    e.preventDefault();
    deleteBtn.click();
  }
});



// IF NO IMAGE LEFT
function clearViewer() {
  // nothing on screen to work on, so the raster panels go away with it
  for (const [button, panel] of rasterPanels()) {
    button.classList.add("hidden");
    button.classList.remove("active");
    panel.classList.add("hidden");
  }

  img.src = "";
  img.style.transform = "translate(0px, 0px) scale(1)";
  img.classList.add("hidden");
  gifCanvas.classList.add("hidden");
  gifBar.classList.add("hidden");
}

img.addEventListener("error", async () => {
  if (deleting) return;

  const path = images[index];
  const ext = getExt(path);
  // ico and the Rust-decoded formats already came through the Rust decoder to
  // get their blob URL; retrying them here would just fail the same way again
  const alreadyRustDecoded = ext === "ico" || FRAME_LIST_EXTS.has(ext) || RUST_DECODED_EXTS.has(ext);

  if (path && !alreadyRustDecoded && !imgLoadFallbackDone) {
    imgLoadFallbackDone = true;
    try {
      const data = await invoke("load_image", { path });
      const blob = new Blob([new Uint8Array(data)], { type: "image/png" });
      img.src = URL.createObjectURL(blob);
      return; // wait for the load/error event this new src triggers
    } catch (err) {
      console.warn("Rust decode fallback also failed:", err);
    }
  }

  console.warn("Image failed to load, skipping");

  if (images.length > 0) {
    images.splice(index, 1);

    if (index >= images.length) {
      index = images.length - 1;
    }

    showImage();
  }
  else {
    clearViewer();
  }
});

// ----------- CONFIRM DLG ---------------

let confirmDialogOpen = false;

function confirmDlg(message) {
  return new Promise((resolve) => {
    confirmDialogOpen = true;

    const dlg = document.getElementById('confirmDlg');
    const text = document.getElementById('confirmText');
    const ok = document.getElementById('confirmOk');
    const cancel = document.getElementById('confirmCancel');
    const closeBtn = document.getElementById('confirmDlgCloseBtn');

    text.textContent = message;
    dlg.style.display = 'block';

    // 🔑 focus OK button
    requestAnimationFrame(() => ok.focus());

    let finished = false;

    const cleanup = (result) => {
      if (finished) return;
      finished = true;

      confirmDialogOpen = false;
      dlg.style.display = 'none';

      ok.onclick = null;
      cancel.onclick = null;
      closeBtn.onclick = null;

      document.removeEventListener('keydown', keyHandler);

      resolve(result);
    };

    ok.onclick = () => cleanup(true);
    cancel.onclick = () => cleanup(false);
    closeBtn.onclick = () => cleanup(false);

    const keyHandler = (e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        cleanup(false);
      }

      if (e.key === 'Enter') {
        e.preventDefault();
        cleanup(true);
      }
      
    };

    document.addEventListener('keydown', keyHandler);
  });
}

// ------------ ROTATE IMAGE -------------------




function updateTransform() {
  clampTranslation();
  img.style.transform = `
    translate(${translateX}px, ${translateY}px)
    scale(${scale})
    rotate(${rotationDegrees}deg)
    scale(${flipX}, ${flipY})
  `;
  let z = getTrueZoomPercent();
  if (z) {
    zoomLabel.style.display = 'block';
    zoomValue.textContent = z + "%";
  }
}

function getTrueZoomPercent() {
  const fittedScale = getOriginalScale();
  return Math.round((scale / fittedScale) * 100);
}

function clampTranslation() {
  const fw = frame.clientWidth;
  const fh = frame.clientHeight;

  const iw = img.clientWidth * scale;
  const ih = img.clientHeight * scale;

  const maxX = Math.max(0, (iw - fw) / 2);
  const maxY = Math.max(0, (ih - fh) / 2);

  translateX = Math.min(maxX, Math.max(-maxX, translateX));
  translateY = Math.min(maxY, Math.max(-maxY, translateY));
}

function computeBaseScale() {
  const cw = frame.clientWidth;
  const ch = frame.clientHeight;

  // on cold start the window (esp. with "maximized": true) can still be
  // laying out when the first image's "load" fires, so the frame reads as
  // 0x0 here; committing to scale=0 makes the image invisible until
  // something else recomputes it. Retry next frame instead.
  if (!cw || !ch) {
    requestAnimationFrame(computeBaseScale);
    return;
  }

  // img's own CSS box (width:100%/height:100% inside a shrink-wrapped,
  // max-width/max-height-constrained parent) is already aspect-ratio-fitted
  // to the frame in CSS pixels via object-fit:contain. transform:scale()
  // scales that box, not the raw bitmap, so the ratio must be taken against
  // img.clientWidth/clientHeight (CSS pixels) — mixing in naturalWidth/Height
  // (raw bitmap pixels) here produced a near-arbitrary scale after rotating.
  let iw = img.clientWidth;
  let ih = img.clientHeight;

  if (!iw || !ih) {
    requestAnimationFrame(computeBaseScale);
    return;
  }

  if (rotationDegrees % 180 !== 0) {
    [iw, ih] = [ih, iw];
  }

  // never upscale: a small image that both frame dimensions have slack for
  // should stay at its native size (matching the un-rotated view, which the
  // "load" handlers pin to scale=1), not get stretched to fill the frame
  baseScale = Math.min(1, cw / iw, ch / ih);

  scale = baseScale;
  translateX = 0;
  translateY = 0;

  updateTransform();
}


// ----------- ROTATE -------------
//
// The bar buttons and the Image Controls panel drive the same value, so a turn
// made in one shows up in the other. Nothing is written until "Save rotation"
// is pressed — in the bar or in the panel.

const rotateLeftBtn = document.getElementById("rotateLeft");
const rotateRightBtn = document.getElementById("rotateRight");

// Keeps the bar and the panel showing the same thing
function syncRotationUi() {
  const rotated = imgAdjust.rotate !== 0;

  const readout = document.getElementById("imgRotateValue");
  if (readout) readout.textContent = rotated ? `${imgAdjust.rotate}°` : "";

  const panelSave = document.getElementById("imgRotateSave");
  if (panelSave) panelSave.classList.toggle("hidden", !rotated);
}

// Formats this build can write back. Rotating anything else would either be
// thrown away or, worse, fail mid-write — so those aren't rotated at all.
const WRITABLE_EXTS = [
  "png", "jpg", "jpeg", "jfif", "bmp", "tif", "tiff", "tga", "qoi", "webp", "gif", "ico"
];

// Why the current image can't be rotated, or null when it can be.
function rotationBlockedReason() {
  const path = images[index];
  if (!path) return "No image open";

  const ext = getExt(path);
  if (ext === "svg") return "SVG is rotated in the SVG panel, not here";
  if (!WRITABLE_EXTS.includes(ext)) {
    return `${ext.toUpperCase()} can't be rotated — this app can't write that format`;
  }

  return null;
}

// Buttons stay visible either way, just greyed out when rotation is impossible.
function syncRotateButtons() {
  const blocked = rotationBlockedReason() !== null;
  rotateLeftBtn.classList.toggle("disabled", blocked);
  rotateRightBtn.classList.toggle("disabled", blocked);
}

function rotateBy(degrees, autoSave = false) {
  if (!img.src || !images[index]) return;

  const blocked = rotationBlockedReason();
  if (blocked) {
    showToast(blocked);
    return;
  }

  imgAdjust.rotate = degrees === 0
    ? 0
    : (((imgAdjust.rotate + degrees) % 360) + 360) % 360;

  syncRotationUi();
  queueImgPreview();

  if (autoSave) queueRotationAutoSave();
}

// The bar buttons write the rotation by themselves. Waiting a moment first
// means clicking twice quickly is one turn of 180° and one file write, not two.
let rotationSaveTimer = null;

function queueRotationAutoSave() {
  clearTimeout(rotationSaveTimer);
  rotationSaveTimer = setTimeout(() => saveRotationOnly({ confirm: false }), 700);
}

rotateLeftBtn.addEventListener("click", () => rotateBy(-90, true));
rotateRightBtn.addEventListener("click", () => rotateBy(90, true));

// ----------- FILE MENU -------------

document.getElementById("fileMenuContextMenu").addEventListener("click", async (e) => {
  e.stopPropagation(); // ⬅️ THIS IS THE KEY

  const item = e.target.closest(".ctx-item");
  if (!item) return;

  const action = item.dataset.action;
  const currentFilePath = images[index];
  const fileName = getFileName(currentFilePath);

  try {
      switch (action) {
        case "openFile":
          openFileSelect();
          break;

        case "openFolder":
          openFolderSelect();
          break;

        case "save":
          //await invoke("save_file", { path: currentFilePath });
          break;

        case "saveAs":
          //await invoke("save_file_as", { path: currentFilePath });
          break;

        case "saveCopy":

          break;

        case "copyImage":
          await copyCurrentImage();
          break;

        case "scanQr":
          closeAllMenus();
          await scanQrCodes();
          break;

        case "readText":
          closeAllMenus();
          await extractImageText();
          break;

        case "removeMetadata": {
          if (!currentFilePath) {
            alert("No image loaded.");
            return;
          }
          closeAllMenus();

          // Stripping EXIF also drops the rotation tag, so a photo that was
          // only rotated by metadata will come back in its original direction.
          const stripOk = await confirmDlg(
            `Remove all metadata from "${fileName}"?\n\n` +
            `EXIF, GPS location and camera info are deleted from the file. ` +
            `If the photo was rotated by its EXIF tag it will show unrotated. ` +
            `This cannot be undone.`
          );
          if (!stripOk) return;

          try {
            await invoke("remove_metadata", { path: currentFilePath });
            dropPreload(currentFilePath);
            await showImage();
          } catch (err) {
            alert(`Failed to remove metadata: ${err}`);
          }
          break;
        }

        case "setWallpaper":
          if (!currentFilePath) {
            alert("No image loaded.");
            return;
          }
          closeAllMenus();
          const confirmed = await confirmDlg(`Set "${fileName}" as Desktop Background Image?`);
          if (!confirmed) return;
          try {
            await invoke("set_desktop_background", { path: currentFilePath });
          } catch (wallpaperErr) {
            alert(`Failed to set desktop background: ${wallpaperErr}`);
          }
          break;

        case "openExplorer":
          if (!currentFilePath) {
            alert("No image loaded.");
            return;
          }
          await invoke("open_in_explorer", { path: currentFilePath });
          break;

        case "renameFile":
          if (!currentFilePath) {
            alert("No image loaded.");
            return;
          }
          renameTargetPath = currentFilePath;
          inputDlgInput.value = fileName.replace(/\.[^.]+$/, "");
          inputDlg.style.display = 'block';
          break;

        case "properties":
          if (!currentFilePath) {
            alert("No image loaded.");
            return;
          }
          await invoke('show_file_properties', { path: currentFilePath });
          break;

        case "imageInfo":
          if (!currentFilePath) {
            alert("No image loaded.");
            return;
          }
          await fillImageInfo(currentFilePath);
          break;
      }
    } 
    catch (err) {
      console.error(err);
    }

  closeAllMenus(); // ✅ use the centralized closer
});

function closeAllMenus() {
  document.querySelectorAll(".context-menu2").forEach(menu => {
    menu.style.display = "none";
  });
  currentOpenMenu = null;
}
const fileBtn = document.getElementById("fileMenuBtn");
const fileMenu = document.getElementById("fileMenuContextMenu");

fileBtn.addEventListener("click", (e) => {
  e.stopPropagation();
  if (fileMenu.style.display === 'none' || fileMenu.style.display === '') {
    const rect = fileBtn.getBoundingClientRect();
    fileMenu.style.left = `${rect.left}px`;
    fileMenu.style.top = `${rect.bottom + 4}px`;
    fileMenu.style.display = "block";
  }
  else {
    fileMenu.style.display = 'none';
  }
});

// --------- INPUT DLG ----------

const inputDlg = document.getElementById('inputDlg');
const inputOk = document.getElementById('inputOk');
const inputCancel = document.getElementById('inputCancel');
const inputDlgInput = document.getElementById('inputDlgInput');
const inputDlgCloseBtn = document.getElementById('inputDlgCloseBtn');

let renameTargetPath = null;

inputOk.addEventListener('click', async function (event) {
  inputDlg.style.display = 'none';
  const name = inputDlgInput.value;
  if (name && renameTargetPath) {
    try {
      const newPath = await invoke("rename_file", { path: renameTargetPath, newName: name });
      const renamedIndex = images.indexOf(renameTargetPath);
      if (renamedIndex !== -1) {
        images[renamedIndex] = newPath;
        if (renamedIndex === index) {
          await showImage();
        }
      }
    } catch (err) {
      console.error("Rename failed:", err);
      alert(`Rename failed: ${err}`);
    }
  }
  renameTargetPath = null;
});

inputDlgCloseBtn.addEventListener('click', function (event) {
  inputDlg.style.display = 'none';
  renameTargetPath = null;
});

inputCancel.addEventListener('click', function (event) {
  inputDlg.style.display = 'none';
  renameTargetPath = null;
});

// -------------- GRID VIEW -----------

const thumbCache = new Map(); // key: filePath, value: <div class="thumbWrapper">

const gridViewBtn = document.getElementById("gridViewBtn");
const gridView = document.getElementById("gridView");
const gridContainer = document.getElementById("gridContainer");

const THUMB_SIZE = 256; // longest edge, matches the grid's CSS cell size

// get_thumbnail is an async command, so each call in flight gets its own thread
// and decoding really does scale with cores. Capped because the queue is only
// worth as many decodes as the machine can actually run at once.
const THUMB_WORKERS = Math.min(navigator.hardwareConcurrency || 4, 16);

// Cells waiting for a thumbnail, newest first: whatever the user just scrolled
// to should be decoded before the rows they scrolled past.
const thumbQueue = [];
let thumbWorkers = 0;

// Only cells near the viewport ask for a thumbnail, so opening a folder of a
// thousand images costs one screenful of decodes, not a thousand.
const thumbObserver = new IntersectionObserver((entries) => {
  for (const entry of entries) {
    if (!entry.isIntersecting) continue;
    thumbObserver.unobserve(entry.target);
    thumbQueue.push(entry.target);
  }
  runThumbWorkers();
}, { root: gridView, rootMargin: "300px" });

function runThumbWorkers() {
  while (thumbWorkers < THUMB_WORKERS && thumbQueue.length) {
    thumbWorkers++;
    fillThumb(thumbQueue.pop()).finally(() => {
      thumbWorkers--;
      runThumbWorkers();
    });
  }
}

async function fillThumb(wrap) {
  const path = wrap.dataset.path;
  const thumb = wrap.firstElementChild;

  if (!path || wrap.dataset.filled) return;
  wrap.dataset.filled = "1";

  if (getExt(path) === "svg") {
    thumb.src = convertFileSrc(path); // vector, already tiny
    return;
  }

  try {
    const data = await invoke("get_thumbnail", { path, size: THUMB_SIZE });
    const blobURL = URL.createObjectURL(new Blob([new Uint8Array(data)]));
    thumb.src = blobURL;
    wrap.dataset.blob = blobURL; // revoked when the entry leaves the cache
  } catch (err) {
    console.error("Thumbnail failed:", path, err);
    delete wrap.dataset.filled; // let a later pass retry it
  }
}

let inGridMode = false;

gridViewBtn.addEventListener("click", () => {
  if (!images.length) return;

  if (inGridMode) {
    exitGridMode();
  } 
  else {
    enterGridMode();
  }
});

function enterGridMode() {
  inGridMode = true;
  // hide single view UI
  img.classList.add("hidden");
  gifCanvas.classList.add("hidden");
  gifBar.classList.add("hidden");
  icoBar.classList.add("hidden");
  loadingText.classList.remove("visible");

  gridView.classList.remove("hidden");

  populateGrid();
}

function populateGrid() {
  gridContainer.innerHTML = "";

  for (let i = 0; i < images.length; i++) {
    const path = images[i];

    // REUSE FROM CACHE IF EXISTS
    if (thumbCache.has(path)) {
      const cached = thumbCache.get(path);
      cached.dataset.index = i; // keep index updated
      gridContainer.appendChild(cached);
      if (!cached.dataset.filled) thumbObserver.observe(cached);
      continue;
    }

    // CREATE NEW WRAPPER. Nothing is decoded here — the grid has to appear
    // immediately, so thumbnails are filled in afterwards as cells scroll
    // into view.
    const wrap = document.createElement("div");
    wrap.className = "thumbWrapper";
    wrap.dataset.index = i;
    wrap.dataset.path = path;

    const thumb = document.createElement("img");
    thumb.className = "gridThumb";
    wrap.appendChild(thumb);

    // format badge, sits in the corner of the tile
    const badge = document.createElement("span");
    badge.className = "thumbFormat";
    badge.textContent = getExt(path).toUpperCase();
    wrap.appendChild(badge);

    // SET TITLE (hover tooltip)
    const fileName = path.split(/[/\\]/).pop();
    wrap.title = fileName;

    // CLICK HANDLER
    wrap.addEventListener("click", () => {
      index = Number(wrap.dataset.index);
      exitGridMode();
    });

    // STORE IN CACHE + APPEND
    thumbCache.set(path, wrap);
    gridContainer.appendChild(wrap);
    thumbObserver.observe(wrap);
  }

  // OPTIONAL: clean orphaned cache entries  
  for (const cachedPath of thumbCache.keys()) {
    if (!images.includes(cachedPath)) {
      // release blob URLs if TIFF cached
      const oldWrap = thumbCache.get(cachedPath);
      if (oldWrap.dataset.blob) {
        URL.revokeObjectURL(oldWrap.dataset.blob);
      }
      thumbCache.delete(cachedPath);
    }
  }
}

function exitGridMode() {
  inGridMode = false;
  gridView.classList.add("hidden");
  img.classList.remove("hidden");

  // Drop queued thumbnail work so it stops competing with the image decode.
  // In-flight calls finish on their own; the cells keep what they got.
  thumbQueue.length = 0;

  showImage();
}

document.addEventListener("keydown", e => {
  if (isTypingTarget(e.target)) return;
  if (e.key.toLowerCase() === "g") {
    gridViewBtn.click();
  }
});

document.getElementById("closeImgInfo").onclick =
document.getElementById("imgInfoCloseBtn").onclick = () => {
  document.getElementById("imgInfoDlg").style.display = "none";
};

async function fillImageInfo(path) {
  const info = await invoke("load_image_metadata", { path });

  centerWindow(document.getElementById("imgInfoDlg"));
  document.getElementById("imgInfoDlg").style.display = "block";

  // BASIC INFO
  document.getElementById("imgInfoFileName").textContent = info.file_name;
  document.getElementById("imgInfoFormat").textContent = info.format;
  document.getElementById("imgInfoDimensions").textContent = `${info.width} × ${info.height}`;
  document.getElementById("imgInfoFileSize").textContent = formatBytes(info.file_size);

  // FIXED FIELD NAMES
  document.getElementById("imgInfoColorMode").textContent = info.color_mode;
  document.getElementById("imgInfoBitDepth").textContent = info.bit_depth;
  document.getElementById("imgInfoAlpha").textContent = info.alpha ? "Yes" : "No";

  const ratio = (info.width / info.height).toFixed(3);
  document.getElementById("imgInfoAspect").textContent = ratio;

  document.getElementById("imgInfoOrientation").textContent =
    info.width >= info.height ? "Landscape" : "Portrait";

  // FILE SYSTEM METADATA
  document.getElementById("imgInfoFullPath").textContent = info.full_path;
  document.getElementById("imgInfoCreated").textContent = formatUnix(info.created);
  document.getElementById("imgInfoModified").textContent = formatUnix(info.modified);
  /* document.getElementById("imgInfoReadOnly").textContent = info.read_only ? "Yes" : "No"; */

  // UI-ONLY  
  /* document.getElementById("imgInfoZoom").textContent = currentZoom + "%";
  document.getElementById("imgInfoDisplayedRes").textContent = `${displayWidth} × ${displayHeight}`;
  document.getElementById("imgInfoScaling").textContent = scalingType; */

  // EXIF DATA
  const exifBlock = document.getElementById("imgExifBlock");

  const hasExif =
    info.date_taken ||
    info.camera ||
    info.aperture ||
    info.shutter ||
    info.iso ||
    info.focal ||
    info.color_profile;

  if (hasExif) {
    exifBlock.style.display = "block";

    document.getElementById("imgInfoDateTaken").textContent = info.date_taken || "-";
    document.getElementById("imgInfoCamera").textContent = info.camera || "-";
    document.getElementById("imgInfoAperture").textContent = info.aperture || "-";
    document.getElementById("imgInfoShutter").textContent = info.shutter || "-";
    document.getElementById("imgInfoISO").textContent = info.iso || "-";
    document.getElementById("imgInfoFocal").textContent = info.focal || "-";
    document.getElementById("imgInfoFlash").textContent = info.flash || "-";
    document.getElementById("imgInfoColorProfile").textContent = info.color_profile || "-";
    /* document.getElementById("imgInfoDPI").textContent = "N/A"; */
  } 
  else {
    exifBlock.style.display = "none";
  }
}

function formatBytes(bytes, decimals = 2) {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + " " + sizes[i];
}

function formatUnix(sec) {
  if (!sec || sec === 0) return "-";
  return new Date(sec * 1000).toLocaleString();
}

//DRAG WINDOW SYSTEM------------------------------------

initDraggableWindows(['dragImgInfo', 'dragQrDlg', 'dragOcrDlg']);
// ------------ SLIDESHOW -----------------

let slideshowInterval = null;
let slideshowDelay = 4000; // 3 seconds per slide
let slideshowActive = false;

async function startSlideshow() {
  if (slideshowActive) return;
  slideshowActive = true;

  document.getElementById("imgViewerDiv").requestFullscreen();

  const btn = document.getElementById("slideShow");
  btn.classList.add("active");
  btn.title = "Stop slideshow";

  slideshowInterval = setInterval(() => {
    nextImage();
  }, slideshowDelay);

  document.addEventListener("keydown", exitFromInput);
}

function exitFromInput(e) {
  if (!slideshowActive) return;

  // Ignore arrow keys during slideshow
  if (
    e.key === "ArrowLeft" ||
    e.key === "ArrowRight" ||
    e.key === "ArrowUp" ||
    e.key === "ArrowDown"
  ) {
    return;
  }

  stopSlideshow();
}

function stopSlideshow() {
  slideshowActive = false;

  clearInterval(slideshowInterval);
  slideshowInterval = null;

  const btn = document.getElementById("slideShow");
  btn.classList.remove("active");
  btn.title = "Slideshow";

  if (document.fullscreenElement) {
    document.exitFullscreen();
  }

  document.removeEventListener("keydown", exitFromInput);
}

function toggleSlideshow() {
  if (!slideshowActive) {
    startSlideshow();
  } 
  else {
    stopSlideshow();
  }
}

document.getElementById("slideShow").addEventListener("click", toggleSlideshow);

document.addEventListener("fullscreenchange", () => {
  if (!document.fullscreenElement && slideshowActive) {
    stopSlideshow();
  }
});

document.addEventListener("keydown", (e) => {
  if (e.key === "F11") {
    e.preventDefault();
    toggleFullscreen();
  }
});

// ---- COPY IMAGE (menu + Ctrl+C) ----

async function copyCurrentImage() {
  const path = images[index];
  if (!path) {
    alert("No image loaded.");
    return;
  }

  // Fast path (what the webview's own "Copy image" does): re-encode the bytes
  // the webview already has instead of making Rust re-read and re-decode the
  // file from disk. ClipboardItem takes a promise, so the write is claimed
  // immediately and the PNG encode finishes in the background.
  if (img.src && gifCanvas.classList.contains("hidden")) {
    try {
      const png = (async () => {
        const blob = await (await fetch(img.src)).blob();
        if (blob.type === "image/png") return blob;

        const bitmap = await createImageBitmap(blob);
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        canvas.getContext("2d").drawImage(bitmap, 0, 0);
        bitmap.close();
        return canvas.convertToBlob({ type: "image/png" });
      })();

      await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
      return;
    } catch (err) {
      // e.g. SVG, which createImageBitmap refuses — let the backend handle it
      console.warn("Fast copy failed, falling back to backend:", err);
    }
  }

  try {
    await invoke("copy_image_to_clipboard", { path });
  } catch (err) {
    console.error("Failed to copy image:", err);
    alert(`Failed to copy image: ${err}`);
  }
}

document.addEventListener("keydown", (e) => {
  if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== "c") return;

  // let inputs and real text selections copy normally
  const t = e.target;
  if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
  if (!document.getSelection().isCollapsed) return;

  e.preventDefault();
  copyCurrentImage();
});

function toggleFullscreen() {
  const viewer = document.getElementById("imgViewerDiv");
  if (!document.fullscreenElement) {
    viewer.requestFullscreen();
  } 
  else {
    document.exitFullscreen();
  }
}

document.addEventListener("keydown", (e) => {
  if (!document.fullscreenElement) return;

  // ignore arrow keys and shortcut modifiers (e.g. Ctrl+C copy)
  if (e.ctrlKey || e.metaKey) return;
  if (e.key === "ArrowLeft" || e.key === "ArrowRight" || e.key === "ArrowUp" || e.key === "ArrowDown") {
    return;
  }

  // exit fullscreen for any other key
  document.exitFullscreen();
});

// ---- ABOUT ----

const helpGithub = document.getElementById('helpGithub');
helpGithub.onclick = async () => {
  await shell.open('https://github.com/hudsonpear/better-image-viewer');
};
const aboutBtn = document.getElementById('aboutBtn');
const aboutWindow = document.getElementById('aboutWindow');
const aboutCloseBtn = document.getElementById('aboutCloseBtn');
const copyIcon = document.getElementById("copyIcon");
const theEmail = document.getElementById("theEmail");

copyIcon.onclick = function() {
  const textToCopy = "coolnewtabpage@gmail.com";
  copyToClipboard(textToCopy);
}
theEmail.onclick = function() {
  const textToCopy = "coolnewtabpage@gmail.com";
  copyToClipboard(textToCopy);
}
aboutBtn.onclick = function() {
  aboutWindow.classList.toggle('hidden');
}
async function copyToClipboard(text) {
  try {
    await navigator.clipboard.writeText(text);
  } 
  catch (err) {}
}

aboutCloseBtn.addEventListener('click', () => {
  aboutWindow.classList.add('hidden');
});

// ---- QR CODE SCANNER ----

const qrDlg = document.getElementById("qrDlg");
const qrContent = document.getElementById("qrContent");

document.getElementById("qrCloseX").onclick =
document.getElementById("qrCloseBtn").onclick = () => {
  qrDlg.style.display = "none";
};

async function scanQrCodes() {
  const path = images[index];
  if (!path) {
    alert("No image loaded.");
    return;
  }

  qrContent.replaceChildren(qrLine("Scanning…"));
  centerWindow(qrDlg);
  qrDlg.style.display = "flex";

  let results;
  try {
    results = await invoke("scan_qr_codes", { path });
  } catch (err) {
    qrContent.replaceChildren(qrLine(`Could not scan this image: ${err}`));
    return;
  }

  showQrResults(results);
}

function qrLine(text) {
  const line = document.createElement("div");
  line.className = "qrCount";
  line.textContent = text;
  return line;
}

// Only these schemes are offered as a clickable link. A QR code is untrusted
// input, so anything else (file:, custom app schemes, ...) is shown as plain
// text the user can read and copy, never as something one click can launch.
const QR_SAFE_LINK = /^(https?|mailto):/i;

function showQrResults(results) {
  if (!results.length) {
    qrContent.replaceChildren(qrLine("No QR code found in this image."));
    return;
  }

  const rows = [qrLine(`${results.length} code${results.length === 1 ? "" : "s"} found`)];

  for (const text of results) {
    const row = document.createElement("div");
    row.className = "qrRow";

    const value = document.createElement("span");
    value.className = "qrText";
    value.textContent = text; // never innerHTML: this came from the image

    if (QR_SAFE_LINK.test(text)) {
      value.classList.add("qrLink");
      value.title = "Open in your browser";
      value.onclick = () => shell.open(text);
    }

    const copy = document.createElement("button");
    copy.className = "buttonStyle qrCopy";
    copy.type = "button";
    copy.textContent = "Copy";
    copy.onclick = async () => {
      await copyToClipboard(text);
      copy.textContent = "Copied";
      setTimeout(() => { copy.textContent = "Copy"; }, 1200);
    };

    row.append(value, copy);
    rows.push(row);
  }

  qrContent.replaceChildren(...rows);
}

// ---- OCR / TEXT IN IMAGE ----

const ocrDlg = document.getElementById("ocrDlg");
const ocrText = document.getElementById("ocrText");
const ocrStatus = document.getElementById("ocrStatus");
const ocrCopyAll = document.getElementById("ocrCopyAll");

document.getElementById("ocrCloseX").onclick =
document.getElementById("ocrCloseBtn").onclick = () => {
  ocrDlg.style.display = "none";
};

async function extractImageText() {
  const path = images[index];
  if (!path) {
    alert("No image loaded.");
    return;
  }

  ocrText.value = "";
  ocrStatus.textContent = "Reading…";
  ocrCopyAll.disabled = true;
  centerWindow(ocrDlg);
  ocrDlg.style.display = "flex";

  let lines;
  try {
    lines = await invoke("read_text_in_image", { path });
  } catch (err) {
    ocrStatus.textContent = `${err}`;
    return;
  }

  if (!lines.length) {
    ocrStatus.textContent = "No text found in this image.";
    return;
  }

  ocrText.value = lines.join("\n");
  ocrStatus.textContent = `${lines.length} line${lines.length === 1 ? "" : "s"} of text`;
  ocrCopyAll.disabled = false;
}

ocrCopyAll.onclick = async () => {
  if (!ocrText.value) return;

  await copyToClipboard(ocrText.value);
  ocrCopyAll.textContent = "Copied";
  setTimeout(() => { ocrCopyAll.textContent = "Copy all text"; }, 1200);
};

// ---- COMPRESS PANEL ----

const compressPanel = document.getElementById("compressPanel");
const compressPanelBtn = document.getElementById("compressPanelBtn");
const compressPanelBody = document.getElementById("compressPanelBody");

// The panels that only make sense for a raster image: shown together, hidden
// together, and never open at the same time as each other. A function, not a
// const, because the panel elements are looked up further down this file.
function rasterPanels() {
  return [
    [imgPanelBtn, imgPanel],
    [convertPanelBtn, convertPanel],
    [compressPanelBtn, compressPanel]
  ];
}

let compressBuilt = false;
let compressOptions = null;
let compressQuality = 80;
let compressScale = 100;
let compressReduceColors = false;
let compressEstimateTimer = null;
let compressBusy = false;

function setCompressStatus(text, isError = false) {
  const status = document.getElementById("compressStatus");
  if (!status) return;
  status.textContent = text;
  status.classList.toggle("error", isError);
}

function buildCompressPanel() {
  if (compressBuilt) return;
  compressBuilt = true;

  const note = document.createElement("div");
  note.className = "imgNote";
  note.id = "compressNote";
  compressPanelBody.appendChild(note);

  // before / after, kept at the top where it's read
  const sizes = document.createElement("div");
  sizes.className = "compressSize";

  const now = document.createElement("span");
  now.id = "compressNow";
  const after = document.createElement("span");
  after.id = "compressAfter";

  sizes.append(now, after);
  compressPanelBody.appendChild(sizes);

  const dims = document.createElement("div");
  dims.className = "compressDims";
  dims.id = "compressDims";
  compressPanelBody.appendChild(dims);

  const controls = document.createElement("div");
  controls.className = "svgSection";
  controls.id = "compressControls";

  // quality (lossy formats only)
  const qualityRow = document.createElement("div");
  qualityRow.className = "imgSliderRow";
  qualityRow.id = "compressQualityRow";

  const qualityLabel = document.createElement("label");
  qualityLabel.textContent = "Quality";

  const qualitySlider = document.createElement("input");
  qualitySlider.type = "range";
  qualitySlider.min = 1;
  qualitySlider.max = 100;
  qualitySlider.value = compressQuality;

  const qualityValue = document.createElement("span");
  qualityValue.className = "imgSliderValue";
  qualityValue.textContent = compressQuality;

  qualitySlider.addEventListener("input", () => {
    compressQuality = Number(qualitySlider.value);
    qualityValue.textContent = qualitySlider.value;
    queueCompressEstimate();
  });

  qualityRow.append(qualityLabel, qualitySlider, qualityValue);
  controls.appendChild(qualityRow);

  // scale — the one lever that works for every writable format
  const scaleRow = document.createElement("div");
  scaleRow.className = "imgSliderRow";

  const scaleLabel = document.createElement("label");
  scaleLabel.textContent = "Size";

  const scaleSlider = document.createElement("input");
  scaleSlider.type = "range";
  scaleSlider.min = 5;
  scaleSlider.max = 100;
  scaleSlider.step = 5;
  scaleSlider.value = compressScale;

  const scaleValue = document.createElement("span");
  scaleValue.className = "imgSliderValue";
  scaleValue.textContent = "100%";

  scaleSlider.addEventListener("input", () => {
    compressScale = Number(scaleSlider.value);
    scaleValue.textContent = `${scaleSlider.value}%`;
    queueCompressEstimate();
  });

  scaleRow.append(scaleLabel, scaleSlider, scaleValue);
  controls.appendChild(scaleRow);

  // colour reduction (PNG)
  const colorsRow = document.createElement("div");
  colorsRow.className = "svgRow";
  colorsRow.id = "compressColorsRow";

  const colorsToggle = document.createElement("button");
  colorsToggle.type = "button";
  colorsToggle.className = "buttonStyle imgBtn svgWideBtn";
  colorsToggle.textContent = "Reduce colours";
  colorsToggle.onclick = () => {
    compressReduceColors = !compressReduceColors;
    colorsToggle.classList.toggle("active", compressReduceColors);
    queueCompressEstimate();
  };

  colorsRow.appendChild(colorsToggle);
  controls.appendChild(colorsRow);

  compressPanelBody.appendChild(controls);

  // actions
  const actions = document.createElement("div");
  actions.className = "imgActions";

  const saveAsNew = document.createElement("button");
  saveAsNew.type = "button";
  saveAsNew.className = "buttonStyle imgBtn";
  saveAsNew.textContent = "Save as new file…";
  saveAsNew.onclick = () => applyCompression(true);

  const saveOver = document.createElement("button");
  saveOver.type = "button";
  saveOver.className = "buttonStyle imgBtn";
  saveOver.textContent = "Compress this file";
  saveOver.onclick = () => applyCompression(false);

  const status = document.createElement("span");
  status.className = "imgStatus";
  status.id = "compressStatus";

  actions.append(saveAsNew, saveOver, status);
  compressPanelBody.appendChild(actions);
}

async function refreshCompressOptions() {
  if (!compressBuilt) return;

  const path = images[index];
  if (!path) return;

  try {
    compressOptions = await invoke("compression_options", { path });
  } catch (err) {
    setCompressStatus(`${err}`, true);
    return;
  }

  document.getElementById("compressNote").textContent = compressOptions.note;
  document.getElementById("compressNow").innerHTML =
    `Now: <b>${formatBytes(compressOptions.current_bytes)}</b>`;
  document.getElementById("compressAfter").textContent = "";
  document.getElementById("compressDims").textContent =
    compressOptions.width ? `${compressOptions.width}×${compressOptions.height}` : "";

  // A format this build can't write has nothing to offer but the explanation
  document.getElementById("compressControls")
    .classList.toggle("hidden", !compressOptions.can_compress);
  compressPanelBody.querySelectorAll(".imgActions button")
    .forEach(b => { b.disabled = !compressOptions.can_compress; });

  document.getElementById("compressQualityRow")
    .classList.toggle("hidden", !compressOptions.has_quality);
  document.getElementById("compressColorsRow")
    .classList.toggle("hidden", !compressOptions.has_palette);

  setCompressStatus("");
  if (compressOptions.can_compress) queueCompressEstimate();
}

// Encoding to measure the result is real work, so it's debounced and never
// runs twice at once.
function queueCompressEstimate() {
  clearTimeout(compressEstimateTimer);
  compressEstimateTimer = setTimeout(runCompressEstimate, 250);
}

async function runCompressEstimate() {
  const path = images[index];
  if (!path || !compressOptions?.can_compress) return;

  if (compressBusy) {
    queueCompressEstimate();
    return;
  }

  compressBusy = true;
  const after = document.getElementById("compressAfter");
  after.textContent = "measuring…";

  try {
    const result = await invoke("compress_estimate", {
      path,
      quality: compressQuality,
      scalePercent: compressScale,
      reduceColors: compressReduceColors
    });

    const before = compressOptions.current_bytes;
    const delta = before > 0 ? Math.round((1 - result.bytes / before) * 100) : 0;

    after.innerHTML =
      `After: <b>${formatBytes(result.bytes)}</b> ` +
      `<span class="${delta > 0 ? "compressSaving" : "compressGrow"}">` +
      `${delta > 0 ? `−${delta}%` : `+${Math.abs(delta)}%`}</span>`;

    const dims = document.getElementById("compressDims");
    dims.textContent = compressScale < 100
      ? `${compressOptions.width}×${compressOptions.height} → ${result.width}×${result.height}`
      : `${result.width}×${result.height}`;
  } catch (err) {
    after.textContent = "";
    document.getElementById("compressDims").textContent = "";
    setCompressStatus(`${err}`, true);
  } finally {
    compressBusy = false;
  }
}

async function applyCompression(asNewFile) {
  const path = images[index];
  if (!path || !compressOptions?.can_compress) return;

  let target = null;

  if (asNewFile) {
    const suggested = path.replace(/(\.[^.\\/]+)$/, "-small$1");
    target = await save({ defaultPath: suggested });
    if (!target) return;
  } else {
    const ok = await confirmDlg(
      `Compress "${getFileName(path)}" in place?\n\n` +
      `The original file is replaced and cannot be recovered.`
    );
    if (!ok) return;
  }

  setCompressStatus("Writing…");

  try {
    const result = await invoke("compress_apply", {
      path,
      quality: compressQuality,
      scalePercent: compressScale,
      reduceColors: compressReduceColors,
      saveAs: target,
      allowLarger: false
    });

    dropPreload(path);
    await showImage();
    await refreshCompressOptions();
    setCompressStatus(`Saved — ${formatBytes(result.bytes)}`);
  } catch (err) {
    setCompressStatus(`${err}`, true);
  }
}

function toggleCompressPanel(show) {
  const open = show ?? compressPanel.classList.contains("hidden");

  if (open) {
    toggleImgPanel(false);
    toggleConvertPanel(false);
    buildCompressPanel();
    compressPanel.classList.remove("hidden");
    refreshCompressOptions();
  } else {
    compressPanel.classList.add("hidden");
  }

  compressPanelBtn.classList.toggle("active", open);
}

compressPanelBtn.onclick = () => toggleCompressPanel();
document.getElementById("compressPanelClose").onclick = () => toggleCompressPanel(false);

// ---- CONVERTER PANEL ----

const convertPanel = document.getElementById("convertPanel");
const convertPanelBtn = document.getElementById("convertPanelBtn");
const convertPanelBody = document.getElementById("convertPanelBody");

const ICO_SIZES = [16, 24, 32, 48, 64, 128, 256];

let convertBuilt = false;
let convertTarget = null;   // chosen output format
let convertQuality = 90;
let convertIcoSize = 256;

function setConvertStatus(text, isError = false) {
  const status = document.getElementById("convertStatus");
  if (!status) return;
  status.textContent = text;
  status.classList.toggle("error", isError);
}

function buildConvertPanel() {
  if (convertBuilt) return;
  convertBuilt = true;

  const heading = document.createElement("span");
  heading.className = "svgSectionTitle";
  heading.textContent = "Convert to";
  convertPanelBody.appendChild(heading);

  const formats = document.createElement("div");
  formats.className = "convertFormats";
  formats.id = "convertFormats";
  convertPanelBody.appendChild(formats);

  // quality, shown only while a lossy format is selected
  const qualitySection = document.createElement("div");
  qualitySection.className = "svgSection hidden";
  qualitySection.id = "convertQualitySection";

  const qualityRow = document.createElement("div");
  qualityRow.className = "imgSliderRow";

  const qualityLabel = document.createElement("label");
  qualityLabel.textContent = "Quality";

  const qualitySlider = document.createElement("input");
  qualitySlider.type = "range";
  qualitySlider.min = 1;
  qualitySlider.max = 100;
  qualitySlider.value = convertQuality;

  const qualityValue = document.createElement("span");
  qualityValue.className = "imgSliderValue";
  qualityValue.textContent = convertQuality;

  qualitySlider.addEventListener("input", () => {
    convertQuality = Number(qualitySlider.value);
    qualityValue.textContent = qualitySlider.value;
  });

  qualityRow.append(qualityLabel, qualitySlider, qualityValue);
  qualitySection.appendChild(qualityRow);
  convertPanelBody.appendChild(qualitySection);

  // ICO only: the standard icon sizes Windows uses
  const icoSection = document.createElement("div");
  icoSection.className = "svgSection hidden";
  icoSection.id = "convertIcoSection";

  const icoHeading = document.createElement("span");
  icoHeading.className = "svgSectionTitle";
  icoHeading.textContent = "Icon size";

  const icoGrid = document.createElement("div");
  icoGrid.className = "convertFormats";

  for (const size of ICO_SIZES) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "buttonStyle imgBtn";
    button.textContent = `${size}×${size}`;
    button.classList.toggle("active", size === convertIcoSize);
    button.onclick = () => {
      convertIcoSize = size;
      icoGrid.querySelectorAll("button")
        .forEach(b => b.classList.toggle("active", b === button));
    };
    icoGrid.appendChild(button);
  }

  icoSection.append(icoHeading, icoGrid);
  convertPanelBody.appendChild(icoSection);

  // actions
  const actions = document.createElement("div");
  actions.className = "imgActions";

  const asNew = document.createElement("button");
  asNew.type = "button";
  asNew.className = "buttonStyle imgBtn";
  asNew.textContent = "Convert as new image";
  asNew.onclick = () => runConvert(false);

  const replace = document.createElement("button");
  replace.type = "button";
  replace.className = "buttonStyle imgBtn";
  replace.textContent = "Convert and delete old";
  replace.onclick = () => runConvert(true);

  const status = document.createElement("span");
  status.className = "imgStatus";
  status.id = "convertStatus";

  actions.append(asNew, replace, status);
  convertPanelBody.appendChild(actions);
}

// The list depends on the open file: its own format is left out, and only
// formats this build can actually encode are offered.
async function refreshConvertTargets() {
  if (!convertBuilt) return;

  const path = images[index];
  const formats = document.getElementById("convertFormats");
  formats.replaceChildren();
  convertTarget = null;
  setConvertStatus("");

  // nothing is selected any more, so the per-format options go with it
  document.getElementById("convertQualitySection").classList.add("hidden");
  document.getElementById("convertIcoSection").classList.add("hidden");

  if (!path) return;

  let targets;
  try {
    targets = await invoke("convert_targets", { path });
  } catch (err) {
    setConvertStatus(`${err}`, true);
    return;
  }

  for (const target of targets) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "buttonStyle convertFormat";

    const name = document.createElement("span");
    name.textContent = target.label;

    const note = document.createElement("small");
    note.textContent = target.note;

    button.append(name, note);
    button.onclick = () => {
      convertTarget = target;
      formats.querySelectorAll(".convertFormat")
        .forEach(b => b.classList.toggle("active", b === button));

      document.getElementById("convertQualitySection")
        .classList.toggle("hidden", !target.lossy);
      document.getElementById("convertIcoSection")
        .classList.toggle("hidden", target.ext !== "ico");

      // warn before transparency is silently flattened
      const losesAlpha = !target.keeps_alpha && imageHasAlpha();
      setConvertStatus(
        losesAlpha ? `${target.label} has no transparency — it will be flattened.` : ""
      );
    };

    formats.appendChild(button);
  }
}

// Best-effort: the displayed image is the source of truth for transparency
function imageHasAlpha() {
  const ext = getExt(images[index]);
  return ["png", "webp", "gif", "ico", "cur", "tiff", "tif", "apng", "psd", "dds", "ktx2"]
    .includes(ext);
}

async function runConvert(deleteOriginal) {
  const path = images[index];
  if (!path) return;

  if (!convertTarget) {
    setConvertStatus("Pick a format first.", true);
    return;
  }

  if (deleteOriginal) {
    const ok = await confirmDlg(
      `Convert "${getFileName(path)}" to ${convertTarget.label} and remove the original?\n\n` +
      `The original goes to the Recycle Bin, and only after the new file is written.`
    );
    if (!ok) return;
  }

  setConvertStatus("Converting…");

  try {
    const written = await invoke("convert_image", {
      path,
      targetExt: convertTarget.ext,
      quality: convertTarget.lossy ? convertQuality : null,
      icoSize: convertTarget.ext === "ico" ? convertIcoSize : null,
      deleteOriginal
    });

    setConvertStatus(`Saved ${getFileName(written)}`);

    // rebuild the folder list so the new file is there and the old one isn't
    const [list, startIndex] = await invoke("get_folder_images", { currentPath: written });
    images = list;
    index = startIndex;
    dropPreload(path);
    await showImage();
  } catch (err) {
    setConvertStatus(`${err}`, true);
  }
}

function toggleConvertPanel(show) {
  const open = show ?? convertPanel.classList.contains("hidden");

  if (open) {
    toggleImgPanel(false); // only one side panel at a time
    toggleCompressPanel(false);
    buildConvertPanel();
    convertPanel.classList.remove("hidden");
    refreshConvertTargets();
  } else {
    convertPanel.classList.add("hidden");
  }

  convertPanelBtn.classList.toggle("active", open);
}

convertPanelBtn.onclick = () => toggleConvertPanel();
document.getElementById("convertPanelClose").onclick = () => toggleConvertPanel(false);

// ---- IMAGE CONTROLS PANEL ----

const imgPanel = document.getElementById("imgPanel");
const imgPanelBtn = document.getElementById("imgPanelBtn");
const imgPanelBody = document.getElementById("imgPanelBody");

// Sliders: [key, label, min, max, step, suffix]
const IMG_SLIDERS = [
  ["exposure", "Exposure", -3, 3, 0.1, " EV"],
  ["brightness", "Brightness", -100, 100, 1, ""],
  ["contrast", "Contrast", -100, 100, 1, ""],
  ["saturation", "Saturation", -100, 100, 1, ""],
  ["hue", "Hue", -180, 180, 1, "°"],
  ["blur", "Blur", 0, 20, 0.5, ""],
  ["sharpen", "Sharpen", 0, 100, 1, ""]
];

// Per-channel gain, its own section so it reads as colour work not tone work
const IMG_RGB_SLIDERS = [
  ["red", "Red", -100, 100, 1, ""],
  ["green", "Green", -100, 100, 1, ""],
  ["blue", "Blue", -100, 100, 1, ""]
];

const IMG_TOGGLES = [
  ["invert", "Invert Colors"],
  ["grayscale", "Greyscale"],
  ["sepia", "Vintage"],
  ["autoLevels", "Auto Adjust"],
  ["flipH", "Flip H"],
  ["flipV", "Flip V"]
];

function blankAdjustments() {
  return {
    exposure: 0, brightness: 0, contrast: 0, saturation: 0, hue: 0,
    red: 0, green: 0, blue: 0,
    blur: 0, sharpen: 0, rotate: 0, threshold: null,
    invert: false, grayscale: false, sepia: false, autoLevels: false,
    flipH: false, flipV: false, resize: null
  };
}

let imgAdjust = blankAdjustments();
let imgPanelBuilt = false;
let imgPreviewRevision = 0;
let imgPreviewTimer = null;
let imgPreviewBusy = false;
let imgEditPath = null;   // file the current adjustments belong to

function imgIsIdentity(a) {
  return a.exposure === 0 && a.brightness === 0 && a.contrast === 0 &&
    a.saturation === 0 && a.hue === 0 &&
    a.red === 0 && a.green === 0 && a.blue === 0 &&
    a.blur === 0 && a.sharpen === 0 && a.rotate === 0 &&
    a.threshold === null && !a.invert && !a.grayscale && !a.sepia &&
    !a.autoLevels && !a.flipH && !a.flipV && !a.resize;
}

// Everything below is created on first open only — the panel costs nothing
// while it's closed.
function buildImgPanel() {
  if (imgPanelBuilt) return;
  imgPanelBuilt = true;

  const section = (title) => {
    const wrap = document.createElement("div");
    wrap.className = "svgSection";
    const heading = document.createElement("span");
    heading.className = "svgSectionTitle";
    heading.textContent = title;
    wrap.appendChild(heading);
    return wrap;
  };

  const note = document.createElement("div");
  note.className = "imgNote hidden";
  note.id = "imgPanelNote";
  imgPanelBody.appendChild(note);

  const sliderRow = ([key, label, min, max, step, suffix]) => {
    const row = document.createElement("div");
    row.className = "imgSliderRow";

    const name = document.createElement("label");
    name.textContent = label;
    name.htmlFor = `imgSlider_${key}`;

    const slider = document.createElement("input");
    slider.type = "range";
    slider.id = `imgSlider_${key}`;
    slider.min = min;
    slider.max = max;
    slider.step = step;
    slider.value = 0;

    const readout = document.createElement("span");
    readout.className = "imgSliderValue";
    readout.textContent = `0${suffix}`;

    slider.addEventListener("input", () => {
      imgAdjust[key] = Number(slider.value);
      readout.textContent = `${slider.value}${suffix}`;
      queueImgPreview();
    });

    row.append(name, slider, readout);
    return row;
  };

  // --- rotate ---
  const rotateSection = section("Rotate");
  const rotateGrid = document.createElement("div");
  rotateGrid.className = "imgToggleGrid";

  const rotateReadout = document.createElement("span");
  rotateReadout.className = "imgStatus";
  rotateReadout.id = "imgRotateValue";

  const turn = (degrees, label) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "buttonStyle imgBtn";
    button.textContent = label;
    button.onclick = () => rotateBy(degrees); // same path as the bar buttons
    return button;
  };

  // Appears once something is rotated, goes away again after saving
  const rotateSave = document.createElement("button");
  rotateSave.type = "button";
  rotateSave.className = "buttonStyle imgBtn svgWideBtn hidden";
  rotateSave.id = "imgRotateSave";
  rotateSave.textContent = "Save rotation";
  rotateSave.onclick = () => saveRotationOnly(); // not the click event as options

  rotateGrid.append(turn(-90, "⟲ 90°"), turn(90, "⟳ 90°"), turn(180, "180°"), turn(0, "Reset"));
  rotateSection.append(rotateGrid, rotateReadout, rotateSave);
  imgPanelBody.appendChild(rotateSection);

  // --- tone ---
  const adjustSection = section("Adjust");
  for (const spec of IMG_SLIDERS) adjustSection.appendChild(sliderRow(spec));
  imgPanelBody.appendChild(adjustSection);

  // --- per-channel RGB ---
  const rgbSection = section("RGB channels");
  for (const spec of IMG_RGB_SLIDERS) rgbSection.appendChild(sliderRow(spec));

  const rgbReset = document.createElement("button");
  rgbReset.type = "button";
  rgbReset.className = "buttonStyle imgBtn";
  rgbReset.textContent = "Reset channels";
  rgbReset.onclick = () => {
    for (const [key, , , , , suffix] of IMG_RGB_SLIDERS) {
      imgAdjust[key] = 0;
      const slider = document.getElementById(`imgSlider_${key}`);
      if (slider) {
        slider.value = 0;
        slider.nextElementSibling.textContent = `0${suffix}`;
      }
    }
    queueImgPreview();
  };
  rgbSection.appendChild(rgbReset);
  imgPanelBody.appendChild(rgbSection);

  // --- threshold, off unless enabled ---
  const thresholdSection = section("Threshold");
  const thresholdRow = document.createElement("div");
  thresholdRow.className = "imgSliderRow";

  const thresholdToggle = document.createElement("input");
  thresholdToggle.type = "checkbox";
  thresholdToggle.id = "imgThresholdOn";

  const thresholdSlider = document.createElement("input");
  thresholdSlider.type = "range";
  thresholdSlider.min = 0;
  thresholdSlider.max = 255;
  thresholdSlider.value = 128;
  thresholdSlider.disabled = true;

  const thresholdValue = document.createElement("span");
  thresholdValue.className = "imgSliderValue";
  thresholdValue.textContent = "128";

  const syncThreshold = () => {
    thresholdSlider.disabled = !thresholdToggle.checked;
    imgAdjust.threshold = thresholdToggle.checked ? Number(thresholdSlider.value) : null;
    thresholdValue.textContent = thresholdSlider.value;
    queueImgPreview();
  };

  thresholdToggle.addEventListener("change", syncThreshold);
  thresholdSlider.addEventListener("input", syncThreshold);

  const thresholdLabel = document.createElement("label");
  thresholdLabel.append(thresholdToggle, document.createTextNode(" On"));
  thresholdLabel.style.flex = "0 0 auto";

  thresholdRow.append(thresholdLabel, thresholdSlider, thresholdValue);
  thresholdSection.appendChild(thresholdRow);
  imgPanelBody.appendChild(thresholdSection);

  // --- toggles ---
  const filterSection = section("Filters");
  const grid = document.createElement("div");
  grid.className = "imgToggleGrid";

  for (const [key, label] of IMG_TOGGLES) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "buttonStyle imgBtn";
    button.textContent = label;
    button.onclick = () => {
      imgAdjust[key] = !imgAdjust[key];
      button.classList.toggle("active", imgAdjust[key]);
      queueImgPreview();
    };
    button.dataset.key = key;
    grid.appendChild(button);
  }

  filterSection.appendChild(grid);
  imgPanelBody.appendChild(filterSection);

  // --- resize ---
  const sizeSection = section("Resize");
  const sizeRow = document.createElement("div");
  sizeRow.className = "svgRow";

  const widthInput = document.createElement("input");
  widthInput.type = "number";
  widthInput.className = "svgNumber";
  widthInput.id = "imgResizeW";
  widthInput.min = 1;

  const heightInput = document.createElement("input");
  heightInput.type = "number";
  heightInput.className = "svgNumber";
  heightInput.id = "imgResizeH";
  heightInput.min = 1;

  const wLabel = document.createElement("span");
  wLabel.className = "svgFieldLabel";
  wLabel.textContent = "W";
  const hLabel = document.createElement("span");
  hLabel.className = "svgFieldLabel";
  hLabel.textContent = "H";

  sizeRow.append(wLabel, widthInput, hLabel, heightInput);

  const sizeButtons = document.createElement("div");
  sizeButtons.className = "svgRow";

  const applySize = document.createElement("button");
  applySize.type = "button";
  applySize.className = "buttonStyle imgBtn";
  applySize.textContent = "Set size";
  applySize.onclick = () => {
    const w = Number(widthInput.value);
    const h = Number(heightInput.value);
    if (!(w > 0) || !(h > 0)) {
      setImgStatus("Width and height must be above zero.", true);
      return;
    }
    imgAdjust.resize = [w, h];
    queueImgPreview();
  };

  const clearSize = document.createElement("button");
  clearSize.type = "button";
  clearSize.className = "buttonStyle imgBtn";
  clearSize.textContent = "Original size";
  clearSize.onclick = () => {
    imgAdjust.resize = null;
    widthInput.value = img.naturalWidth || "";
    heightInput.value = img.naturalHeight || "";
    queueImgPreview();
  };

  sizeButtons.append(applySize, clearSize);
  sizeSection.append(sizeRow, sizeButtons);
  imgPanelBody.appendChild(sizeSection);

  // --- actions ---
  const actions = document.createElement("div");
  actions.className = "imgActions";

  const resetBtn = document.createElement("button");
  resetBtn.type = "button";
  resetBtn.className = "buttonStyle imgBtn";
  resetBtn.textContent = "Reset all";
  resetBtn.onclick = resetImgAdjustments;

  const saveAsBtn = document.createElement("button");
  saveAsBtn.type = "button";
  saveAsBtn.className = "buttonStyle imgBtn";
  saveAsBtn.textContent = "Save as new file…";
  saveAsBtn.onclick = () => saveImgAdjustments(true);

  const saveBtn = document.createElement("button");
  saveBtn.type = "button";
  saveBtn.className = "buttonStyle imgBtn";
  saveBtn.id = "imgSaveInPlace";
  saveBtn.textContent = "Save over original";
  saveBtn.onclick = () => saveImgAdjustments(false);

  const status = document.createElement("span");
  status.className = "imgStatus";
  status.id = "imgPanelStatus";

  actions.append(resetBtn, saveAsBtn, saveBtn, status);
  imgPanelBody.appendChild(actions);
}

function setImgStatus(text, isError = false) {
  const status = document.getElementById("imgPanelStatus");
  if (!status) return;
  status.textContent = text;
  status.classList.toggle("error", isError);
}

function resetImgAdjustments() {
  imgAdjust = blankAdjustments();

  for (const [key, , , , , suffix] of [...IMG_SLIDERS, ...IMG_RGB_SLIDERS]) {
    const slider = document.getElementById(`imgSlider_${key}`);
    if (!slider) continue;
    slider.value = 0;
    slider.nextElementSibling.textContent = `0${suffix}`;
  }

  const thresholdToggle = document.getElementById("imgThresholdOn");
  if (thresholdToggle) thresholdToggle.checked = false;

  const rotateValue = document.getElementById("imgRotateValue");
  if (rotateValue) rotateValue.textContent = "";

  const rotateSave = document.getElementById("imgRotateSave");
  if (rotateSave) rotateSave.classList.add("hidden");

  imgPanelBody.querySelectorAll(".imgToggleGrid .imgBtn")
    .forEach(button => button.classList.remove("active"));

  showImage(); // back to the untouched file
  setImgStatus("");
}

// Renders through Rust, debounced, and never more than one render in flight —
// dragging a slider on a 50MP photo would otherwise queue dozens of decodes.
function queueImgPreview() {
  clearTimeout(imgPreviewTimer);
  imgPreviewTimer = setTimeout(runImgPreview, 90);
}

async function runImgPreview() {
  if (!imgEditPath) return;

  if (imgPreviewBusy) {
    queueImgPreview(); // try again once the current render lands
    return;
  }

  if (imgIsIdentity(imgAdjust)) {
    showImage();
    setImgStatus("");
    return;
  }

  imgPreviewBusy = true;
  imgPanel.classList.add("imgBusy");

  try {
    const url = await invoke("preview_adjustments", {
      path: imgEditPath,
      adjustments: imgAdjust,
      // a preview only has to fill the window, not the whole sensor
      maxSize: Math.max(900, Math.round(window.innerWidth * window.devicePixelRatio)),
      revision: ++imgPreviewRevision
    });

    img.src = convertFileSrc(url) + `?r=${imgPreviewRevision}`;
    setImgStatus("Preview — not saved yet.");
  } catch (err) {
    setImgStatus(`${err}`, true);
  } finally {
    imgPreviewBusy = false;
    imgPanel.classList.remove("imgBusy");
  }
}

/// Writes the rotation and nothing else, leaving any other pending adjustments
/// alone. Rotation alone is also what lets a JPEG be saved losslessly.
async function saveRotationOnly({ confirm = true } = {}) {
  if (!imgEditPath || !imgAdjust.rotate) return;

  // The bar buttons save on their own and don't ask — rotating is easy to undo
  // by rotating back. The panel button still confirms, since other adjustments
  // may be pending there.
  if (confirm) {
    const ok = await confirmDlg(
      `Rotate "${getFileName(imgEditPath)}" by ${imgAdjust.rotate}° and save it?\n\n` +
      `Only the rotation is saved — other adjustments stay unsaved.`
    );
    if (!ok) return;
  }

  setImgStatus("Saving rotation…");

  try {
    await invoke("apply_adjustments", {
      path: imgEditPath,
      adjustments: { ...blankAdjustments(), rotate: imgAdjust.rotate },
      saveAs: null
    });

    // the file now carries the rotation, so drop it from the pending set and
    // keep whatever else the user was still working on
    imgAdjust.rotate = 0;
    syncRotationUi();

    dropPreload(imgEditPath);
    await showImage();

    if (!imgIsIdentity(imgAdjust)) queueImgPreview();
    setImgStatus("Rotation saved.");
  } catch (err) {
    setImgStatus(`Save failed: ${err}`, true);
    showToast(`${err}`); // the panel may be closed when saving from the bar
  }
}

async function saveImgAdjustments(asNewFile) {
  if (!imgEditPath) return;

  if (imgIsIdentity(imgAdjust)) {
    setImgStatus("Nothing to save — no adjustments made.");
    return;
  }

  let target = null;

  if (asNewFile) {
    const suggested = imgEditPath.replace(/(\.[^.\\/]+)$/, "-edited$1");
    target = await save({
      defaultPath: suggested,
      filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "bmp", "tif", "tiff"] }]
    });
    if (!target) return;
  } else {
    const ok = await confirmDlg(
      `Overwrite "${getFileName(imgEditPath)}" with the adjusted image?\n\n` +
      `The original cannot be recovered.`
    );
    if (!ok) return;
  }

  setImgStatus("Saving…");

  try {
    const written = await invoke("apply_adjustments", {
      path: imgEditPath,
      adjustments: imgAdjust,
      saveAs: target
    });

    imgAdjust = blankAdjustments();
    resetImgAdjustments();

    if (asNewFile) {
      setImgStatus(`Saved to ${getFileName(written)}`);
    } else {
      dropPreload(imgEditPath);
      await showImage();
      setImgStatus("Saved.");
    }
  } catch (err) {
    setImgStatus(`Save failed: ${err}`, true);
  }
}

// Format-dependent: an animation or texture edits its current frame, a RAW or
// HEIC can be adjusted but not written back, SVG has its own panel.
async function refreshImgCapabilities() {
  const path = images[index];
  imgEditPath = path || null;
  if (!path || !imgPanelBuilt) return;

  const note = document.getElementById("imgPanelNote");

  try {
    const caps = await invoke("image_edit_capabilities", { path });

    note.textContent = caps.note;
    note.classList.toggle("hidden", !caps.note);

    imgPanelBody.querySelectorAll("input, button").forEach(el => {
      if (el.id !== "imgPanelClose") el.disabled = !caps.can_edit;
    });

    // "Save rotation" writes over the original, so it follows the same rule
    for (const id of ["imgSaveInPlace", "imgRotateSave"]) {
      const button = document.getElementById(id);
      if (!button) continue;

      button.disabled = !caps.can_save_in_place;
      button.title = caps.can_save_in_place
        ? "Replace the original file"
        : `Cannot write ${caps.format} — use Save as new file`;
    }

    const width = document.getElementById("imgResizeW");
    const height = document.getElementById("imgResizeH");
    if (width && !width.value) width.value = img.naturalWidth || "";
    if (height && !height.value) height.value = img.naturalHeight || "";
  } catch (err) {
    console.error("Could not read edit capabilities:", err);
  }
}

function toggleImgPanel(show) {
  const open = show ?? imgPanel.classList.contains("hidden");

  if (open) {
    toggleConvertPanel(false); // only one side panel at a time
    toggleCompressPanel(false);
    buildImgPanel(); // first open pays for the DOM, later ones don't
    imgPanel.classList.remove("hidden");
    refreshImgCapabilities();
  } else {
    imgPanel.classList.add("hidden");
  }

  imgPanelBtn.classList.toggle("active", open);
}

imgPanelBtn.onclick = () => toggleImgPanel();
document.getElementById("imgPanelClose").onclick = () => toggleImgPanel(false);

// ---- SVG CONTROL PANEL ----

const svgPanel = document.getElementById("svgPanel");
const svgCode = document.getElementById("svgCode");
const svgStatus = document.getElementById("svgStatus");
const svgRotSlider = document.getElementById("svgRotSlider");
const svgRotValue = document.getElementById("svgRotValue");
const svgFlipH = document.getElementById("svgFlipH");
const svgFlipV = document.getElementById("svgFlipV");

const svgPanelBtn = document.getElementById("svgPanelBtn");

let svgPath = null;
let svgDiskCode = "";      // markup as it is on disk right now
let svgLoadedFor = null;   // path whose code is currently in the textarea
let svgPreviewUrl = null;  // object URL of the last unsaved preview

// Whether the panel opens by itself for every SVG. Toggled by the topbar SVG
// button and remembered between sessions.
let svgPanelAuto = localStorage.getItem("svgPanelAuto") !== "false";

// true while a text field has focus, so global one-key shortcuts (arrows, "g")
// don't fire while the user is typing in the code editor or the rename box
function isTypingTarget(target) {
  return !!target &&
    (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable);
}

function setSvgStatus(text, isError = false) {
  svgStatus.textContent = text;
  svgStatus.classList.toggle("error", isError);
}

function dropSvgPreview() {
  if (svgPreviewUrl) {
    URL.revokeObjectURL(svgPreviewUrl);
    svgPreviewUrl = null;
  }
}

// Called for every SVG that comes on screen: the topbar button appears, and
// the panel itself follows whatever the user last chose.
function openSvgPanel(path) {
  svgPath = path;
  svgPanelBtn.classList.remove("hidden");
  svgPanelBtn.classList.toggle("active", svgPanelAuto);
  svgPanel.classList.toggle("hidden", !svgPanelAuto);
  setSvgStatus("");
  syncSvgControls();

  if (svgPanelAuto) loadSvgCode(path);
}

// Reading a multi-megabyte SVG isn't free, so only do it once the panel is
// actually on screen for this file.
async function loadSvgCode(path) {
  if (svgLoadedFor === path) return;
  svgLoadedFor = path;

  try {
    svgDiskCode = await invoke("read_svg", { path });
    svgCode.value = svgDiskCode;
  } catch (err) {
    svgDiskCode = "";
    svgCode.value = "";
    svgLoadedFor = null;
    setSvgStatus(`Could not read file: ${err}`, true);
  }

  renderSvgColors();
  showSvgSize();
}

function closeSvgPanel() {
  svgPanel.classList.add("hidden");
  svgPanelBtn.classList.add("hidden");
  dropSvgPreview();
  svgPath = null;
  svgLoadedFor = null;
}

// The topbar button is the only way back once the panel is dismissed, so
// dismissing it also stops it opening on its own for the next SVG.
function setSvgPanelAuto(on) {
  svgPanelAuto = on;
  localStorage.setItem("svgPanelAuto", String(on));
  svgPanelBtn.classList.toggle("active", on);
  svgPanel.classList.toggle("hidden", !on);
  if (on && svgPath) loadSvgCode(svgPath);
}

svgPanelBtn.onclick = () => setSvgPanelAuto(svgPanel.classList.contains("hidden"));

function syncSvgControls() {
  svgRotSlider.value = rotationDegrees;
  svgRotValue.textContent = `${rotationDegrees}°`;
  svgFlipH.classList.toggle("active", flipX === -1);
  svgFlipV.classList.toggle("active", flipY === -1);
}

// ponytail: computeBaseScale() only swaps width/height for multiples of 90, so
// the fit for a free angle is approximate. Good enough to eyeball; swap in a
// real rotated-bbox fit if it ever looks wrong.
function setSvgRotation(deg) {
  rotationDegrees = ((Math.round(deg) % 360) + 360) % 360;
  translateX = 0;
  translateY = 0;
  computeBaseScale();
  syncSvgControls();
}

document.getElementById("svgPanelClose").onclick = () => setSvgPanelAuto(false);
document.getElementById("svgRotL").onclick = () => setSvgRotation(rotationDegrees - 90);
document.getElementById("svgRotR").onclick = () => setSvgRotation(rotationDegrees + 90);
document.getElementById("svgRotReset").onclick = () => {
  flipX = 1;
  flipY = 1;
  setSvgRotation(0);
};
svgRotSlider.addEventListener("input", () => setSvgRotation(Number(svgRotSlider.value)));

svgFlipH.onclick = () => {
  flipX = -flipX;
  updateTransform();
  syncSvgControls();
};
svgFlipV.onclick = () => {
  flipY = -flipY;
  updateTransform();
  syncSvgControls();
};

// ---- code editing ----

function previewSvg(message = "Previewing unsaved code.") {
  dropSvgPreview();
  svgPreviewUrl = URL.createObjectURL(new Blob([svgCode.value], { type: "image/svg+xml" }));
  img.src = svgPreviewUrl;
  setSvgStatus(message);
}

document.getElementById("svgPreview").onclick = () => previewSvg();

document.getElementById("svgRevert").onclick = () => {
  svgCode.value = svgDiskCode;
  dropSvgPreview();
  img.src = freshAssetUrl(svgPath);
  renderSvgColors();
  showSvgSize();
  setSvgStatus("Reverted to the file on disk.");
};

document.getElementById("svgSave").onclick = () => writeSvg(svgCode.value, "Saved.");

async function writeSvg(text, okMsg) {
  if (!svgPath) return;
  try {
    await invoke("save_svg", { path: svgPath, contents: text });
    svgDiskCode = text;
    svgCode.value = text;
    renderSvgColors();
  showSvgSize();
    dropSvgPreview();
    img.src = freshAssetUrl(svgPath);
    recordMtime(svgPath);
    setSvgStatus(okMsg);
  } catch (err) {
    setSvgStatus(`Save failed: ${err}`, true);
  }
}

// ---- size ----

const svgWidth = document.getElementById("svgWidth");
const svgHeight = document.getElementById("svgHeight");
const svgKeepRatio = document.getElementById("svgKeepRatio");

let svgAspect = 1; // width / height of the size currently in the file

function showSvgSize() {
  const size = readSvgSize(svgCode.value);

  if (!size) {
    svgWidth.value = "";
    svgHeight.value = "";
    return;
  }

  svgAspect = size.width / size.height;
  svgWidth.value = Math.round(size.width);
  svgHeight.value = Math.round(size.height);
}

// Rounded to whole pixels: the viewBox keeps the real proportions, so this only
// affects the box the drawing is scaled into.
function linkSvgSize(changed) {
  if (!svgKeepRatio.checked || !svgAspect) return;

  if (changed === svgWidth) {
    const w = Number(svgWidth.value);
    if (w > 0) svgHeight.value = Math.max(1, Math.round(w / svgAspect));
  } else {
    const h = Number(svgHeight.value);
    if (h > 0) svgWidth.value = Math.max(1, Math.round(h * svgAspect));
  }
}

svgWidth.addEventListener("input", () => linkSvgSize(svgWidth));
svgHeight.addEventListener("input", () => linkSvgSize(svgHeight));

function scaleSvgSize(factor) {
  const w = Number(svgWidth.value);
  const h = Number(svgHeight.value);
  if (!(w > 0) || !(h > 0)) return;

  svgWidth.value = Math.max(1, Math.round(w * factor));
  svgHeight.value = Math.max(1, Math.round(h * factor));
}

document.getElementById("svgSizeHalf").onclick = () => scaleSvgSize(0.5);
document.getElementById("svgSizeDouble").onclick = () => scaleSvgSize(2);

document.getElementById("svgApplySize").onclick = () => {
  const width = Number(svgWidth.value);
  const height = Number(svgHeight.value);

  const resized = resizeSvgCode(svgCode.value, width, height);
  if (!resized) {
    setSvgStatus("Need a width and height above zero, and a readable <svg> tag.", true);
    return;
  }

  svgCode.value = resized;
  showSvgSize();
  previewSvg(`Resized to ${width} × ${height} — not saved yet.`);
};

// ---- colors ----

const svgColorList = document.getElementById("svgColorList");
const svgColorAll = document.getElementById("svgColorAll");

// Canvas normalises anything CSS understands ("red", "rgb(1,2,3)", "#abc") to
// #rrggbb. Invalid values leave fillStyle untouched, so probing from two
// different starting colors tells a real color from a rejected one.
function toHexColor(value) {
  const probe = (start) => {
    const ctx = document.createElement("canvas").getContext("2d");
    ctx.fillStyle = start;
    ctx.fillStyle = value;
    return ctx.fillStyle;
  };

  const fromBlack = probe("#000000");
  return fromBlack === probe("#ffffff") && fromBlack.startsWith("#") ? fromBlack : null;
}

function replaceSvgColor(from, to) {
  svgCode.value = replaceColorIn(svgCode.value, from, to);
}

function renderSvgColors() {
  const counts = svgColorsIn(svgCode.value);
  svgColorList.innerHTML = "";

  if (!counts.size) {
    const note = document.createElement("span");
    note.className = "svgEmptyNote";
    note.textContent = "No fill or stroke colors found.";
    svgColorList.appendChild(note);
    return;
  }

  for (const [raw, count] of counts) {
    const hex = toHexColor(raw);

    const row = document.createElement("div");
    row.className = "svgColorRow";

    const picker = document.createElement("input");
    picker.type = "color";
    picker.value = hex || "#000000";
    picker.disabled = !hex; // e.g. a gradient reference we can't show as one swatch

    const label = document.createElement("span");
    label.textContent = raw;

    const times = document.createElement("span");
    times.className = "svgColorCount";
    times.textContent = `×${count}`;

    // the token changes as the user drags, so follow it instead of `raw`
    let current = raw;
    picker.addEventListener("input", () => {
      replaceSvgColor(current, picker.value);
      current = picker.value;
      label.textContent = current;
      previewSvg("Color changed — not saved yet.");
    });

    row.append(picker, label, times);
    svgColorList.appendChild(row);
  }
}

document.getElementById("svgColorRescan").onclick = () => {
  renderSvgColors();
  showSvgSize();
};

document.getElementById("svgColorAllBtn").onclick = () => {
  const counts = svgColorsIn(svgCode.value);
  if (!counts.size) {
    setSvgStatus("No fill or stroke colors to change.");
    return;
  }

  for (const raw of counts.keys()) replaceSvgColor(raw, svgColorAll.value);
  renderSvgColors();
  showSvgSize();
  previewSvg(`Recolored ${counts.size} color${counts.size === 1 ? "" : "s"} — not saved yet.`);
};

// ---- bake the on-screen rotation/flip into the markup ----

// Width/height attributes can be percentages or carry units, so the viewBox is
// the more reliable source; naturalWidth/Height is the last resort.
function svgIntrinsicSize(root) {
  const plain = value => {
    if (!value || /%\s*$/.test(value)) return null;
    const n = parseFloat(value);
    return Number.isFinite(n) && n > 0 ? n : null;
  };

  const box = (root.getAttribute("viewBox") || "").split(/[\s,]+/).map(Number);
  const fromBox = box.length === 4 && box[2] > 0 && box[3] > 0 ? [box[2], box[3]] : null;

  const w = plain(root.getAttribute("width")) || (fromBox && fromBox[0]) || img.naturalWidth;
  const h = plain(root.getAttribute("height")) || (fromBox && fromBox[1]) || img.naturalHeight;

  return w > 0 && h > 0 ? [w, h] : null;
}

// Wraps the whole document in a new outer <svg> sized to the rotated bounding
// box, with the original nested inside a transformed <g>. Nesting keeps every
// id, style and defs of the original intact instead of rewriting its contents.
function wrapSvgTransform(code, deg, fx, fy) {
  const doc = new DOMParser().parseFromString(code, "image/svg+xml");
  const root = doc.documentElement;

  if (!root || root.nodeName.toLowerCase() !== "svg" || doc.querySelector("parsererror")) return null;

  const size = svgIntrinsicSize(root);
  if (!size) return null;
  const [w, h] = size;

  const rad = deg * Math.PI / 180;
  const outerW = Math.abs(w * Math.cos(rad)) + Math.abs(h * Math.sin(rad));
  const outerH = Math.abs(w * Math.sin(rad)) + Math.abs(h * Math.cos(rad));

  // pin the nested copy's size, otherwise its default 100%/100% would resolve
  // against the new outer viewport instead of its own
  root.setAttribute("width", w);
  root.setAttribute("height", h);
  root.removeAttribute("x");
  root.removeAttribute("y");

  const r = n => Math.round(n * 1000) / 1000;
  const inner = new XMLSerializer().serializeToString(root);

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${r(outerW)}" height="${r(outerH)}" viewBox="0 0 ${r(outerW)} ${r(outerH)}">
<g transform="translate(${r(outerW / 2)} ${r(outerH / 2)}) rotate(${deg}) scale(${fx} ${fy}) translate(${r(-w / 2)} ${r(-h / 2)})">
${inner}
</g>
</svg>
`;
}

document.getElementById("svgBake").onclick = async () => {
  if (!svgPath) return;

  if (rotationDegrees === 0 && flipX === 1 && flipY === 1) {
    setSvgStatus("No rotation or flip to save.");
    return;
  }

  const wrapped = wrapSvgTransform(svgCode.value || svgDiskCode, rotationDegrees, flipX, flipY);
  if (!wrapped) {
    setSvgStatus("Could not parse this SVG.", true);
    return;
  }

  await writeSvg(wrapped, "Rotation saved into the file.");

  // the file now carries the rotation, so drop the on-screen one
  flipX = 1;
  flipY = 1;
  setSvgRotation(0);
};