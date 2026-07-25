const FRAME_WIDTH = 1000;
const FRAME_HEIGHT = 600;
const TILE_SIZE = 256;
const MIN_ZOOM = 2;
const MAX_ZOOM = 17;
const ZOOM_STEP = 0.5;
const WHEEL_ZOOM_SENSITIVITY = 0.0025;
const MAX_LATITUDE = 85.05112878;
const FALLBACK_VIEWPORT = { latitude: 9.9325, longitude: -84.08, zoom: 12 };

let drag;
const wheelStates = new WeakMap();

function clamp(value, minimum, maximum) {
  return Math.max(minimum, Math.min(maximum, value));
}

function wrapLongitude(longitude) {
  return ((((longitude + 180) % 360) + 360) % 360) - 180;
}

function normalizeViewport(viewport) {
  return {
    latitude: clamp(viewport.latitude, -MAX_LATITUDE, MAX_LATITUDE),
    longitude: wrapLongitude(viewport.longitude),
    zoom: clamp(viewport.zoom, MIN_ZOOM, MAX_ZOOM),
  };
}

function parseViewport(value) {
  const [latitude, longitude, zoom, extra] = String(value ?? "")
    .split(",")
    .map(Number);
  if (
    extra !== undefined ||
    !Number.isFinite(latitude) ||
    !Number.isFinite(longitude) ||
    !Number.isFinite(zoom)
  ) {
    return { ...FALLBACK_VIEWPORT };
  }
  return normalizeViewport({ latitude, longitude, zoom });
}

function serializeViewport(viewport) {
  const normalized = normalizeViewport(viewport);
  return [
    normalized.latitude.toFixed(5),
    normalized.longitude.toFixed(5),
    normalized.zoom.toFixed(3),
  ].join(",");
}

function cityViewports(root) {
  try {
    const cities = JSON.parse(root.dataset.mapCities ?? "[]");
    if (Array.isArray(cities) && cities.length > 1) return cities;
  } catch {
    // Fall through to the built-in recovery location.
  }
  return [FALLBACK_VIEWPORT];
}

function randomCityViewport(root, viewport) {
  const cities = cityViewports(root);
  const candidates = cities.filter(
    (city) =>
      Math.abs(city.latitude - viewport.latitude) > 0.1 ||
      Math.abs(city.longitude - viewport.longitude) > 0.1
  );
  const pool = candidates.length > 0 ? candidates : cities;
  return {
    ...pool[Math.floor(Math.random() * pool.length)],
  };
}

function rootViewport(root) {
  return parseViewport(root.dataset.mapViewport);
}

function worldSize(zoom) {
  return TILE_SIZE * 2 ** zoom;
}

function longitudeToX(longitude, zoom) {
  return ((longitude + 180) / 360) * worldSize(zoom);
}

function latitudeToY(latitude, zoom) {
  const radians =
    (clamp(latitude, -MAX_LATITUDE, MAX_LATITUDE) * Math.PI) / 180;
  return (
    (0.5 -
      Math.log((1 + Math.sin(radians)) / (1 - Math.sin(radians))) /
        (4 * Math.PI)) *
    worldSize(zoom)
  );
}

function xToLongitude(x, zoom) {
  return (x / worldSize(zoom)) * 360 - 180;
}

function yToLatitude(y, zoom) {
  const mercator = Math.PI * (1 - (2 * y) / worldSize(zoom));
  return (Math.atan(Math.sinh(mercator)) * 180) / Math.PI;
}

function dispatchRenderStart(root) {
  root.classList.add("is-rendering");
  root.dispatchEvent(new Event("maprenderstart", { bubbles: true }));
}

function commitViewport(root, viewport) {
  const next = serializeViewport(viewport);
  if (next === root.dataset.mapViewport) return;

  root.dataset.mapViewport = next;
  dispatchRenderStart(root);

  const input = root.querySelector("[data-map-state]");
  if (!input) return;
  input.value = next;
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function panBy(root, deltaX, deltaY, units = "display") {
  const viewport = rootViewport(root);
  const bounds = root.getBoundingClientRect();
  const scaleX = units === "display" ? FRAME_WIDTH / bounds.width : 1;
  const scaleY = units === "display" ? FRAME_HEIGHT / bounds.height : 1;
  const centerX = longitudeToX(viewport.longitude, viewport.zoom);
  const centerY = latitudeToY(viewport.latitude, viewport.zoom);

  commitViewport(root, {
    latitude: yToLatitude(centerY - deltaY * scaleY, viewport.zoom),
    longitude: xToLongitude(centerX - deltaX * scaleX, viewport.zoom),
    zoom: viewport.zoom,
  });
}

function zoomViewportAt(root, viewport, zoom, clientX, clientY) {
  if (clientX === undefined || clientY === undefined) {
    return { ...viewport, zoom };
  }

  const bounds = root.getBoundingClientRect();
  const offsetX =
    (clientX - bounds.left - bounds.width / 2) * (FRAME_WIDTH / bounds.width);
  const offsetY =
    (clientY - bounds.top - bounds.height / 2) * (FRAME_HEIGHT / bounds.height);
  const anchorLongitude = xToLongitude(
    longitudeToX(viewport.longitude, viewport.zoom) + offsetX,
    viewport.zoom
  );
  const anchorLatitude = yToLatitude(
    latitudeToY(viewport.latitude, viewport.zoom) + offsetY,
    viewport.zoom
  );
  const centerX = longitudeToX(anchorLongitude, zoom) - offsetX;
  const centerY = latitudeToY(anchorLatitude, zoom) - offsetY;

  return normalizeViewport({
    latitude: yToLatitude(centerY, zoom),
    longitude: xToLongitude(centerX, zoom),
    zoom,
  });
}

function previewZoom(root, zoom, clientX, clientY) {
  const frame = root.querySelector("[data-map-frame-image]");
  if (!frame) return;

  if (clientX !== undefined && clientY !== undefined) {
    const bounds = root.getBoundingClientRect();
    const originX = clamp(clientX - bounds.left, 0, bounds.width);
    const originY = clamp(clientY - bounds.top, 0, bounds.height);
    frame.style.transformOrigin = `${originX}px ${originY}px`;
  } else {
    frame.style.removeProperty("transform-origin");
  }

  frame.style.transform = `scale(${2 ** (zoom - rootViewport(root).zoom)})`;
}

function zoomBy(root, amount, clientX, clientY) {
  const viewport = rootViewport(root);
  const zoom = clamp(viewport.zoom + amount * ZOOM_STEP, MIN_ZOOM, MAX_ZOOM);
  if (zoom === viewport.zoom) return;

  previewZoom(root, zoom, clientX, clientY);
  commitViewport(root, zoomViewportAt(root, viewport, zoom, clientX, clientY));
}

function resetFrame(root) {
  const frame = root.querySelector("[data-map-frame-image]");
  frame?.style.removeProperty("transform");
  frame?.style.removeProperty("transform-origin");
}

function finishDrag(event, cancelled = false) {
  if (!drag || event.pointerId !== drag.pointerId) return;

  const { root, frame, startX, startY, pointerId } = drag;
  const deltaX = event.clientX - startX;
  const deltaY = event.clientY - startY;
  drag = undefined;
  root.classList.remove("is-dragging");

  if (root.hasPointerCapture(pointerId)) root.releasePointerCapture(pointerId);

  if (cancelled || Math.hypot(deltaX, deltaY) < 3) {
    frame?.style.removeProperty("transform");
    return;
  }

  panBy(root, deltaX, deltaY);
}

document.addEventListener("pointerdown", (event) => {
  const root = event.target.closest?.("[data-map-root]");
  if (
    !root ||
    event.button !== 0 ||
    event.target.closest?.("[data-map-control], button, a, input")
  ) {
    return;
  }

  const frame = root.querySelector("[data-map-frame-image]");
  drag = {
    root,
    frame,
    pointerId: event.pointerId,
    startX: event.clientX,
    startY: event.clientY,
  };
  root.setPointerCapture(event.pointerId);
  root.classList.add("is-dragging");
});

document.addEventListener("pointermove", (event) => {
  if (!drag || event.pointerId !== drag.pointerId) return;
  const deltaX = event.clientX - drag.startX;
  const deltaY = event.clientY - drag.startY;
  if (drag.frame) {
    drag.frame.style.transform = `translate3d(${deltaX}px, ${deltaY}px, 0)`;
  }
});

document.addEventListener("pointerup", (event) => finishDrag(event));
document.addEventListener("pointercancel", (event) => finishDrag(event, true));

document.addEventListener("click", (event) => {
  const control = event.target.closest?.("[data-map-action]");
  if (!control) return;
  const root =
    control.closest("[data-map-root]") ??
    control.closest("main")?.querySelector("[data-map-root]");
  if (!root) return;

  switch (control.dataset.mapAction) {
    case "zoom-in":
      zoomBy(root, 1);
      break;
    case "zoom-out":
      zoomBy(root, -1);
      break;
    case "random-city":
      resetFrame(root);
      commitViewport(root, randomCityViewport(root, rootViewport(root)));
      break;
    case "download": {
      const frame = root.querySelector("[data-map-frame-image]");
      if (!frame?.src) break;
      const viewport = rootViewport(root);
      const link = document.createElement("a");
      link.href = frame.src;
      const style = root.dataset.mapTheme ?? "warm";
      link.download =
        `topcoat-map-${style}-z${viewport.zoom}-${viewport.latitude.toFixed(
          4
        )}-` + `${viewport.longitude.toFixed(4)}.jpg`;
      link.click();
      break;
    }
  }
});

document.addEventListener("keydown", (event) => {
  const root = event.target.closest?.("[data-map-root]");
  if (!root) return;

  const step = event.shiftKey ? 240 : 120;
  switch (event.key) {
    case "ArrowLeft":
      event.preventDefault();
      panBy(root, step, 0, "image");
      break;
    case "ArrowRight":
      event.preventDefault();
      panBy(root, -step, 0, "image");
      break;
    case "ArrowUp":
      event.preventDefault();
      panBy(root, 0, step, "image");
      break;
    case "ArrowDown":
      event.preventDefault();
      panBy(root, 0, -step, "image");
      break;
    case "+":
    case "=":
      event.preventDefault();
      zoomBy(root, 1);
      break;
    case "-":
    case "_":
      event.preventDefault();
      zoomBy(root, -1);
      break;
  }
});

document.addEventListener(
  "wheel",
  (event) => {
    const root = event.target.closest?.("[data-map-root]");
    if (!root) return;
    event.preventDefault();

    let state = wheelStates.get(root);
    if (!state) {
      state = {
        clientX: event.clientX,
        clientY: event.clientY,
        viewport: rootViewport(root),
        timer: undefined,
      };
      wheelStates.set(root, state);
    }
    const zoom = clamp(
      state.viewport.zoom - event.deltaY * WHEEL_ZOOM_SENSITIVITY,
      MIN_ZOOM,
      MAX_ZOOM
    );
    state.viewport = zoomViewportAt(
      root,
      state.viewport,
      zoom,
      state.clientX,
      state.clientY
    );
    previewZoom(root, zoom, state.clientX, state.clientY);

    clearTimeout(state.timer);
    state.timer = setTimeout(() => {
      wheelStates.delete(root);
      commitViewport(root, state.viewport);
    }, 120);
  },
  { passive: false }
);

document.addEventListener("dblclick", (event) => {
  const root = event.target.closest?.("[data-map-root]");
  if (!root || event.target.closest?.("[data-map-control]")) return;
  event.preventDefault();
  zoomBy(root, 1, event.clientX, event.clientY);
});

document.addEventListener("maprendercomplete", (event) => {
  const root = event.target.closest?.("[data-map-root]");
  if (!root) return;
  root.classList.remove("is-rendering", "is-dragging");
  resetFrame(root);
});
