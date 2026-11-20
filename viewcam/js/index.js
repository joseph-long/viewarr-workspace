/**
 * viewcam - Browser-based camera-frame viewer built on viewarr
 *
 * Same shape as viewarr's API: viewers are created in a container element and
 * addressed by its ID afterwards. Each viewer takes up to three images — data,
 * dark and saturation — and does the dark subtraction and saturation overlay
 * in WASM before display.
 */

// Module-level state
let wasmModule = null;
let wasmInitPromise = null;
const viewers = new Map();

/**
 * Initialize the WASM module (called automatically, idempotent)
 * @returns {Promise<void>}
 */
async function initWasm() {
  if (wasmInitPromise) {
    return wasmInitPromise;
  }

  wasmInitPromise = (async () => {
    // Dynamic import of the wasm-pack generated module
    const wasm = await import('./viewcam.js');
    await wasm.default();
    wasmModule = wasm;
  })();

  return wasmInitPromise;
}

/**
 * Look up a viewer, throwing if there is none for the container.
 * @param {string} containerId
 */
function getViewer(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  return viewer;
}

/**
 * Create a new camera viewer in the specified container.
 *
 * @param {string} containerId - The ID of the HTML element to use as the container.
 *                               This ID is also used to identify the viewer instance.
 * @returns {Promise<void>} Resolves when the viewer is ready.
 * @throws {Error} If the container is not found or initialization fails.
 */
export async function createViewer(containerId) {
  const container = document.getElementById(containerId);
  if (!container) {
    throw new Error(`Container element with id "${containerId}" not found`);
  }

  if (viewers.has(containerId)) {
    console.warn(`Viewer already exists for container "${containerId}"`);
    return;
  }

  container.innerHTML = '';
  const loadingDiv = document.createElement('div');
  loadingDiv.style.cssText = `
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    font-family: system-ui, -apple-system, sans-serif;
    color: #666;
  `;
  loadingDiv.textContent = 'Loading viewer...';
  container.appendChild(loadingDiv);

  try {
    await initWasm();

    const canvas = document.createElement('canvas');
    canvas.id = `${containerId}_canvas`;
    canvas.style.cssText = `
      width: 100%;
      height: 100%;
      display: block;
    `;

    // Prevent native browser drag behavior on the canvas
    canvas.addEventListener('dragstart', (e) => {
      e.preventDefault();
    });
    canvas.draggable = false;
    canvas.addEventListener('mousedown', (e) => {
      if (e.altKey) {
        e.preventDefault();
      }
    });

    container.innerHTML = '';
    container.appendChild(canvas);

    const handle = await wasmModule.ViewcamHandle.create(canvas);
    viewers.set(containerId, { handle, canvas, container });

    // Tear the viewer down when its container leaves the DOM (e.g. tab close)
    const mutationObserver = new MutationObserver((mutations) => {
      mutations.forEach((mutation) => {
        mutation.removedNodes.forEach((node) => {
          if (node === container || node.contains(container)) {
            destroyViewer(containerId);
            mutationObserver.disconnect();
          }
        });
      });
    });
    mutationObserver.observe(document.body, { childList: true, subtree: true });
    viewers.get(containerId).mutationObserver = mutationObserver;
  } catch (error) {
    container.innerHTML = '';
    const errorDiv = document.createElement('div');
    errorDiv.style.cssText = `
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      width: 100%;
      height: 100%;
      font-family: system-ui, -apple-system, sans-serif;
      color: #c00;
      padding: 20px;
      box-sizing: border-box;
      text-align: center;
    `;

    const title = document.createElement('div');
    title.style.fontWeight = 'bold';
    title.style.marginBottom = '10px';
    title.textContent = 'Failed to load viewer';

    const message = document.createElement('div');
    message.style.cssText = `
      font-family: monospace;
      font-size: 12px;
      white-space: pre-wrap;
      word-break: break-word;
      max-width: 100%;
    `;
    message.textContent = error.message || String(error);

    errorDiv.appendChild(title);
    errorDiv.appendChild(message);
    container.appendChild(errorDiv);

    console.error('viewcam initialization failed:', error);

    throw error;
  }
}

/**
 * Set the science frame. Shown dark-subtracted while that toggle is on and a
 * dark frame of the same shape is loaded.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {ArrayBuffer} buffer - The raw pixel data.
 * @param {number} width - Image width in pixels.
 * @param {number} height - Image height in pixels.
 * @param {string} dtype - Data type string (e.g. "u16", "f32"), as for viewarr.
 * @param {{bias: number, readNoise: number, noiseSlope: number, step: number}|null} [quantization]
 *   For a quantized frame, its noise model and step, as viewcam-proto's
 *   client hands them over: a saturation level is then compared with its
 *   rounded values as the quantizer's threshold.
 * @throws {Error} If the viewer is not found or data is invalid.
 */
export function setData(containerId, buffer, width, height, dtype, quantization = null) {
  getViewer(containerId).handle.setData(buffer, width, height, dtype, quantization);
}

/**
 * Set the dark frame. Kept until replaced or cleared, so it need only be sent
 * once for a stream of data frames.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {ArrayBuffer} buffer - The raw pixel data.
 * @param {number} width - Image width in pixels.
 * @param {number} height - Image height in pixels.
 * @param {string} dtype - Data type string (e.g. "u16", "f32").
 * @throws {Error} If the viewer is not found or data is invalid.
 */
export function setDark(containerId, buffer, width, height, dtype) {
  getViewer(containerId).handle.setDark(buffer, width, height, dtype);
}

/**
 * Remove the dark frame.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 */
export function clearDark(containerId) {
  getViewer(containerId).handle.clearDark();
}

/**
 * Set the saturation image: any nonzero pixel is painted in the saturation
 * colour over the data. Kept until replaced or cleared.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {ArrayBuffer} buffer - The raw pixel data.
 * @param {number} width - Image width in pixels.
 * @param {number} height - Image height in pixels.
 * @param {string} dtype - Data type string (e.g. "u8").
 * @throws {Error} If the viewer is not found or data is invalid.
 */
export function setSaturation(containerId, buffer, width, height, dtype) {
  getViewer(containerId).handle.setSaturation(buffer, width, height, dtype);
}

/**
 * Remove the saturation image, or the saturation level.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 */
export function clearSaturation(containerId) {
  getViewer(containerId).handle.clearSaturation();
}

/**
 * For a camera without a saturation image: paint each data frame's pixels
 * at or above this level in the saturation colour. The level is in the
 * data's units, compared before any dark is subtracted. Replaces any
 * saturation image, as setSaturation replaces the level; clearSaturation
 * removes it.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {number} level
 * @throws {Error} If the viewer is not found or level is NaN.
 */
export function setSaturationLevel(containerId, level) {
  getViewer(containerId).handle.setSaturationLevel(level);
}

/**
 * Whether dark subtraction is switched on.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {boolean}
 */
export function getDarkSubtraction(containerId) {
  return getViewer(containerId).handle.getDarkSubtraction();
}

/**
 * Switch dark subtraction on or off (the toolbar's "Subtract dark" toggle).
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {boolean} enabled
 */
export function setDarkSubtraction(containerId, enabled) {
  getViewer(containerId).handle.setDarkSubtraction(enabled);
}

/**
 * Whether the saturation overlay is switched on.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {boolean}
 */
export function getSaturationOverlay(containerId) {
  return getViewer(containerId).handle.getSaturationOverlay();
}

/**
 * Switch the saturation overlay on or off (the toolbar's "Saturation" toggle).
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {boolean} enabled
 */
export function setSaturationOverlay(containerId, enabled) {
  getViewer(containerId).handle.setSaturationOverlay(enabled);
}

/**
 * Get the saturation overlay colour as CSS `#rrggbb`.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {string}
 */
export function getSaturationColor(containerId) {
  return getViewer(containerId).handle.getSaturationColor();
}

/**
 * Set the saturation overlay colour from CSS `#rrggbb`.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {string} color
 * @throws {Error} If the colour is not `#rrggbb`.
 */
export function setSaturationColor(containerId, color) {
  getViewer(containerId).handle.setSaturationColor(color);
}

/**
 * Name the stream a feed comes from, putting it in the status bar below the
 * image as connecting; null takes it out. The status bar shows only what it
 * is told, so whatever receives the feed reports to it with
 * setFeedConnection, noteFeedFrame, noteFeedEmpty and noteReceived.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {'data'|'dark'|'saturation'} role
 * @param {string|null} name
 */
export function setFeed(containerId, role, name) {
  getViewer(containerId).handle.setFeed(role, name);
}

/**
 * A feed's connection changed. Its frame rate starts over.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {'data'|'dark'|'saturation'} role
 * @param {'connecting'|'open'|'closed'} connection
 */
export function setFeedConnection(containerId, role, connection) {
  getViewer(containerId).handle.setFeedConnection(role, connection);
}

/**
 * A feed's frame was shown: the status bar lists its shape, dtype and seq,
 * and times it for the frame rate. If no newer frame comes for longer than
 * that rate would have it, the status bar warns, with how long ago the
 * frame shown was acquired (or, without `acquiredMs`, since it arrived).
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {'data'|'dark'|'saturation'} role
 * @param {bigint} seq
 * @param {number} width
 * @param {number} height
 * @param {string} dtype
 * @param {number|null} [acquiredMs] - When the image was acquired, in
 *   milliseconds since the Unix epoch (as `Date.now()`), if known.
 */
export function noteFeedFrame(containerId, role, seq, width, height, dtype, acquiredMs = null) {
  getViewer(containerId).handle.noteFeedFrame(role, BigInt(seq), width, height, dtype, acquiredMs ?? undefined);
}

/**
 * A feed said it has no image.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {'data'|'dark'|'saturation'} role
 */
export function noteFeedEmpty(containerId, role) {
  getViewer(containerId).handle.noteFeedEmpty(role);
}

/**
 * A message of this many bytes arrived on any feed, shown or not: the
 * status bar's bandwidth counts them all.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {number} bytes
 */
export function noteReceived(containerId, bytes) {
  getViewer(containerId).handle.noteReceived(bytes);
}

/**
 * Whether the status bar's turbo toggle is on. What turbo does is up to
 * whatever receives the feeds; onStateChange says when it changes.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {boolean}
 */
export function getTurbo(containerId) {
  return getViewer(containerId).handle.getTurbo();
}

/**
 * Switch turbo on or off, as the status bar's toggle does.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {boolean} turbo
 */
export function setTurbo(containerId, turbo) {
  getViewer(containerId).handle.setTurbo(turbo);
}

/**
 * Register a callback for toolbar setting and turbo changes, whether made in
 * the UI or through this API. It receives
 * { darkSubtraction, saturationOverlay, saturationColor, turbo }.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {Function} callback
 */
export function onStateChange(containerId, callback) {
  getViewer(containerId).handle.onStateChange(callback);
}

/**
 * Clear all registered callbacks for a viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 */
export function clearCallbacks(containerId) {
  getViewer(containerId).handle.clearCallbacks();
}

/**
 * Destroy a viewer instance and clean up resources.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 */
export function destroyViewer(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    return; // Already destroyed or never created
  }
  viewer.handle.destroy();
  if (viewer.mutationObserver) {
    viewer.mutationObserver.disconnect();
  }
  viewer.container.innerHTML = '';
  viewers.delete(containerId);
}

/**
 * Check if a viewer exists for a given container.
 *
 * @param {string} containerId - The ID of the container.
 * @returns {boolean} True if a viewer exists.
 */
export function hasViewer(containerId) {
  return viewers.has(containerId);
}

const api = {
  createViewer,
  setData,
  setDark,
  clearDark,
  setSaturation,
  clearSaturation,
  setSaturationLevel,
  getDarkSubtraction,
  setDarkSubtraction,
  getSaturationOverlay,
  setSaturationOverlay,
  getSaturationColor,
  setSaturationColor,
  setFeed,
  setFeedConnection,
  noteFeedFrame,
  noteFeedEmpty,
  noteReceived,
  getTurbo,
  setTurbo,
  onStateChange,
  clearCallbacks,
  destroyViewer,
  hasViewer
};

window.viewcam = api;

export default api;
