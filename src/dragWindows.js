// Draggable in-page windows (Image Info, QR Codes, Text in Image). Each is
// centred by CSS and remembers where the user drags it, between sessions.

// These windows are centred with transform: translate(-50%, -50%). Setting
// left/top while that transform is live moves them by half their own size, so
// the centring is baked into real coordinates once, at the first drag.
function pinToCurrentPosition(form) {
  if (getComputedStyle(form).transform === "none") return;

  const rect = form.getBoundingClientRect();
  form.style.transform = "none";
  form.style.left = rect.left + "px";
  form.style.top = rect.top + "px";
}

/// Drops whatever the last drag left behind so CSS centres the window again.
/// Called every time a window is shown.
export function centerWindow(form) {
  form.style.removeProperty("left");
  form.style.removeProperty("top");
  form.style.removeProperty("transform");
}

function makeDraggable(dragHandle) {
  const form = dragHandle.parentElement;

  let isDragging = false;
  let offsetX = 0;
  let offsetY = 0;

  // Pointer events, not mouse events: setPointerCapture keeps delivering moves
  // and the release even when the cursor leaves the app window. With a
  // document-level mouseup, letting go outside the window (dragging toward a
  // screen edge, or over another app) never ended the drag, so the position
  // was never written down.
  dragHandle.addEventListener("pointerdown", start);
  dragHandle.addEventListener("pointermove", move);
  dragHandle.addEventListener("pointerup", stop);
  dragHandle.addEventListener("pointercancel", stop);
  dragHandle.addEventListener("lostpointercapture", stop);

  function start(e) {
    if (e.button !== 0) return;
    if (e.target.closest(".closeBtn2, .closeBtn")) return; // let the X be clicked

    e.preventDefault(); // don't select the title text while dragging
    pinToCurrentPosition(form);

    isDragging = true;
    offsetX = e.clientX - form.offsetLeft;
    offsetY = e.clientY - form.offsetTop;

    dragHandle.setPointerCapture(e.pointerId);
  }

  function move(e) {
    if (!isDragging) return;

    // Keep a grabbable strip on screen so a window can't be lost off-edge
    const maxX = window.innerWidth - 80;
    const maxY = window.innerHeight - 40;

    form.style.left =
      Math.min(maxX, Math.max(80 - form.offsetWidth, e.clientX - offsetX)) + "px";
    form.style.top = Math.min(maxY, Math.max(0, e.clientY - offsetY)) + "px";
  }

  function stop() {
    isDragging = false;
  }
}

/// Makes each window draggable by the handle with the given id.
export function initDraggableWindows(handleIds) {
  for (const id of handleIds) {
    const handle = document.getElementById(id);
    if (handle) makeDraggable(handle);
  }
}
