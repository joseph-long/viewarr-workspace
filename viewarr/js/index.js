/**
 * viewarr - Browser-based array/image viewer
 *
 * This module provides a JavaScript API for managing multiple viewer instances,
 * each backed by a Rust/WASM/egui renderer.
 */

// Module-level state
let wasmModule = null;
let wasmInitPromise = null;
const viewers = new Map();

/**
 * Partial viewer state object for bulk state updates.
 *
 * All keys are optional; missing keys are ignored.
 *
 * @typedef {Object} ViewerStateConfig
 * @property {number=} contrast
 * @property {number=} bias
 * @property {"linear" | "log" | "symmetric"=} stretchMode
 * @property {number=} zoom
 * @property {string=} colormap
 * @property {boolean=} colormapReversed
 * @property {number=} vmin
 * @property {number=} vmax
 * @property {[number, number]=} xlim
 * @property {[number, number]=} ylim
 * @property {number=} rotation
 * @property {[number, number]=} pivot
 * @property {boolean=} showPivotMarker
 * @property {string=} overlayMessage
 * @property {[number, number][]=} markers
 */

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
    // When installed as an NPM package, pkg files are in the package root
    const wasm = await import('./viewarr.js');
    // Initialize the WASM module
    await wasm.default();
    wasmModule = wasm;
  })();

  return wasmInitPromise;
}

/**
 * Create a new viewer instance in the specified container.
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

  // Check if viewer already exists for this container
  if (viewers.has(containerId)) {
    console.warn(`Viewer already exists for container "${containerId}"`);
    return;
  }

  // Show loading indicator
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
    // Initialize WASM module (idempotent)
    await initWasm();

    // Create canvas element
    const canvas = document.createElement('canvas');
    canvas.id = `${containerId}_canvas`;
    canvas.style.cssText = `
      width: 100%;
      height: 100%;
      display: block;
    `;

    // Prevent native browser drag behavior on the canvas
    // This stops the browser from trying to drag the canvas content as an image
    canvas.addEventListener('dragstart', (e) => {
      e.preventDefault();
    });
    canvas.draggable = false;

    // Also prevent default on mousedown with alt key to avoid browser-specific behaviors
    canvas.addEventListener('mousedown', (e) => {
      if (e.altKey) {
        e.preventDefault();
      }
    });

    // Replace loading indicator with canvas
    container.innerHTML = '';
    container.appendChild(canvas);

    // Create the viewer handle using static factory method
    const handle = await wasmModule.ViewerHandle.create(canvas);

    // Store viewer state
    viewers.set(containerId, {
      handle,
      canvas,
      container
    });

    // Set up MutationObserver to detect container removal (e.g., tab close)
    const mutationObserver = new MutationObserver((mutations) => {
      mutations.forEach((mutation) => {
        mutation.removedNodes.forEach((node) => {
          if (node === container || node.contains(container)) {
            console.debug(`Cleaning up viewer ${containerId} because its DOM node went away...`);
            destroyViewer(containerId);
            console.debug(`Done cleaning up viewer ${containerId}.`);
            mutationObserver.disconnect();
          }
        });
      });
    });
    mutationObserver.observe(document.body, { childList: true, subtree: true });
    console.log("Installed mutation observer");

    // Update viewer state to include the observer
    viewers.get(containerId).mutationObserver = mutationObserver;

  } catch (error) {
    // Show error in container
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

    // Log full error to console
    console.error('viewarr initialization failed:', error);

    throw error;
  }
}

/**
 * Set image data for a viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {ArrayBuffer} buffer - The raw pixel data.
 * @param {number} width - Image width in pixels.
 * @param {number} height - Image height in pixels.
 * @param {string} dtype - Data type string (e.g., "f4", "f8", "i2", "u1").
 * @throws {Error} If the viewer is not found or data is invalid.
 */
export function setImageData(containerId, buffer, width, height, dtype) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }

  viewer.handle.setImageData(buffer, width, height, dtype);
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

  // Stop observing mutations
  if (viewer.mutationObserver) {
    viewer.mutationObserver.disconnect();
  }

  // Clear the container
  viewer.container.innerHTML = '';

  // Remove from map
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

/**
 * Get all active viewer IDs.
 *
 * @returns {string[]} Array of container IDs with active viewers.
 */
export function getActiveViewers() {
  return Array.from(viewers.keys());
}

/**
 * Get current zoom level for a viewer (1.0 = fit to view).
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {number} Zoom level.
 * @throws {Error} If the viewer is not found.
 */
export function getZoom(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  return viewer.handle.getZoom();
}

/**
 * Set zoom level for a viewer (1.0 = fit to view).
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {number} zoom - Zoom level.
 * @throws {Error} If the viewer is not found.
 */
export function setZoom(containerId, zoom) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.setZoom(zoom);
}

// =========================================================================
// Contrast/Bias/Stretch getters and setters
// =========================================================================

/**
 * Get current contrast value for a viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {number} Contrast value (0.0 to 10.0, default 1.0).
 * @throws {Error} If the viewer is not found.
 */
export function getContrast(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  return viewer.handle.getContrast();
}

/**
 * Set contrast value for a viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {number} contrast - Contrast value (0.0 to 10.0).
 * @throws {Error} If the viewer is not found.
 */
export function setContrast(containerId, contrast) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.setContrast(contrast);
}

/**
 * Get current bias value for a viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {number} Bias value (0.0 to 1.0, default 0.5).
 * @throws {Error} If the viewer is not found.
 */
export function getBias(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  return viewer.handle.getBias();
}

/**
 * Set bias value for a viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {number} bias - Bias value (0.0 to 1.0).
 * @throws {Error} If the viewer is not found.
 */
export function setBias(containerId, bias) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.setBias(bias);
}

/**
 * Get current stretch mode for a viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {string} Stretch mode: "linear", "log", or "symmetric".
 * @throws {Error} If the viewer is not found.
 */
export function getStretchMode(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  return viewer.handle.getStretchMode();
}

/**
 * Set stretch mode for a viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {string} mode - Stretch mode: "linear", "log", or "symmetric".
 * @throws {Error} If the viewer is not found.
 */
export function setStretchMode(containerId, mode) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.setStretchMode(mode);
}

/**
 * Get visible image bounds in pixel coordinates.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {number[]} Array [xmin, xmax, ymin, ymax] in pixel coordinates.
 * @throws {Error} If the viewer is not found.
 */
export function getViewBounds(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  // Get viewport dimensions from the container
  const rect = viewer.container.getBoundingClientRect();
  const bounds = viewer.handle.getViewBounds(rect.width, rect.height);
  return Array.from(bounds);
}

/**
 * Set view to show specific image bounds.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {number} xmin - Minimum x coordinate in pixels.
 * @param {number} xmax - Maximum x coordinate in pixels.
 * @param {number} ymin - Minimum y coordinate in pixels.
 * @param {number} ymax - Maximum y coordinate in pixels.
 * @throws {Error} If the viewer is not found.
 */
export function setViewBounds(containerId, xmin, xmax, ymin, ymax) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  // Get viewport dimensions from the container
  const rect = viewer.container.getBoundingClientRect();
  viewer.handle.setViewBounds(xmin, xmax, ymin, ymax, rect.width, rect.height);
}

/**
 * Get the colormap name for a viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {string} Colormap name (e.g., "gray", "inferno", "magma", "RdBu").
 * @throws {Error} If the viewer is not found.
 */
export function getColormap(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  return viewer.handle.getColormap();
}

/**
 * Set the colormap name for a viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {string} colormap - Colormap name (e.g., "gray", "inferno", "magma", "RdBu").
 * @throws {Error} If the viewer is not found.
 */
export function setColormap(containerId, colormap) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.setColormap(colormap);
}

/**
 * Get whether the colormap is reversed.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {boolean} True if the colormap is reversed.
 * @throws {Error} If the viewer is not found.
 */
export function getColormapReversed(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  return viewer.handle.getColormapReversed();
}

/**
 * Set whether the colormap is reversed.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {boolean} reversed - True to use reversed colormap.
 * @throws {Error} If the viewer is not found.
 */
export function setColormapReversed(containerId, reversed) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.setColormapReversed(reversed);
}

/**
 * Get the image value range (vmin, vmax).
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {number[]} Array [vmin, vmax].
 * @throws {Error} If the viewer is not found.
 */
export function getValueRange(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  const range = viewer.handle.getValueRange();
  return Array.from(range);
}

/**
 * Set the value range (vmin, vmax) for display scaling.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {number} vmin - The minimum display value.
 * @param {number} vmax - The maximum display value.
 * @throws {Error} If the viewer is not found.
 */
export function setValueRange(containerId, vmin, vmax) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.setValueRange(vmin, vmax);
}

// =========================================================================
// Rotation getters and setters
// =========================================================================

/**
 * Get current rotation angle in degrees (counter-clockwise).
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {number} Rotation angle in degrees.
 * @throws {Error} If the viewer is not found.
 */
export function getRotation(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  return viewer.handle.getRotation();
}

/**
 * Set rotation angle in degrees (counter-clockwise).
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {number} degrees - Rotation angle in degrees.
 * @throws {Error} If the viewer is not found.
 */
export function setRotation(containerId, degrees) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.setRotation(degrees);
}

/**
 * Get pivot point in image coordinates.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {number[]} Array [x, y] in image coordinates.
 * @throws {Error} If the viewer is not found.
 */
export function getPivotPoint(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  const pivot = viewer.handle.getPivotPoint();
  return Array.from(pivot);
}

/**
 * Set pivot point in image coordinates.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {number} x - X coordinate in image pixels.
 * @param {number} y - Y coordinate in image pixels.
 * @throws {Error} If the viewer is not found.
 */
export function setPivotPoint(containerId, x, y) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.setPivotPoint(x, y);
}

/**
 * Get whether the pivot marker is visible.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {boolean} True if the pivot marker is visible.
 * @throws {Error} If the viewer is not found.
 */
export function getShowPivotMarker(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  return viewer.handle.getShowPivotMarker();
}

/**
 * Set whether to show the pivot marker.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {boolean} show - True to show the pivot marker.
 * @throws {Error} If the viewer is not found.
 */
export function setShowPivotMarker(containerId, show) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.setShowPivotMarker(show);
}

/**
 * Get the viewer overlay message.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {string} Overlay message.
 * @throws {Error} If the viewer is not found.
 */
export function getOverlayMessage(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  return viewer.handle.getOverlayMessage();
}

/**
 * Set the viewer overlay message.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {string} message - Overlay message (empty string hides the overlay).
 * @throws {Error} If the viewer is not found.
 */
export function setOverlayMessage(containerId, message) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.setOverlayMessage(message);
}

/**
 * Get point markers from the viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @returns {[number, number][]} Marker points in image coordinates.
 * @throws {Error} If the viewer is not found.
 */
export function getMarkers(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  const flat = Array.from(viewer.handle.getMarkers());
  const markers = [];
  for (let i = 0; i + 1 < flat.length; i += 2) {
    markers.push([flat[i], flat[i + 1]]);
  }
  return markers;
}

/**
 * Set point markers in the viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {[number, number][]} markers - Marker points in image coordinates.
 * @throws {Error} If the viewer is not found.
 */
export function setMarkers(containerId, markers) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  if (!Array.isArray(markers)) {
    viewer.handle.setMarkers([]);
    return;
  }
  const flat = [];
  for (const point of markers) {
    if (!Array.isArray(point) || point.length !== 2) continue;
    const x = Number(point[0]);
    const y = Number(point[1]);
    if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
    flat.push(x, y);
  }
  viewer.handle.setMarkers(flat);
}

/**
 * Apply viewer state from a partial configuration object.
 *
 * Missing keys are ignored. Unknown keys are ignored.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {ViewerStateConfig} state - Partial state to apply.
 * @throws {Error} If the viewer is not found.
 */
export function setViewerState(containerId, state) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  if (!state || typeof state !== 'object') {
    return;
  }

  if ('contrast' in state && state.contrast !== undefined) {
    viewer.handle.setContrast(state.contrast);
  }
  if ('bias' in state && state.bias !== undefined) {
    viewer.handle.setBias(state.bias);
  }
  if ('stretchMode' in state && state.stretchMode !== undefined) {
    viewer.handle.setStretchMode(state.stretchMode);
  }
  if ('colormap' in state && state.colormap !== undefined) {
    viewer.handle.setColormap(state.colormap);
  }
  if ('colormapReversed' in state && state.colormapReversed !== undefined) {
    viewer.handle.setColormapReversed(state.colormapReversed);
  }
  if ('vmin' in state && 'vmax' in state && state.vmin !== undefined && state.vmax !== undefined) {
    viewer.handle.setValueRange(state.vmin, state.vmax);
  }
  if (
    'xlim' in state &&
    'ylim' in state &&
    state.xlim !== undefined &&
    state.ylim !== undefined &&
    Array.isArray(state.xlim) &&
    Array.isArray(state.ylim) &&
    state.xlim.length === 2 &&
    state.ylim.length === 2
  ) {
    const rect = viewer.container.getBoundingClientRect();
    viewer.handle.setViewBounds(
      state.xlim[0],
      state.xlim[1],
      state.ylim[0],
      state.ylim[1],
      rect.width,
      rect.height
    );
  }
  if ('rotation' in state && state.rotation !== undefined) {
    viewer.handle.setRotation(state.rotation);
  }
  if (
    'pivot' in state &&
    state.pivot !== undefined &&
    Array.isArray(state.pivot) &&
    state.pivot.length === 2
  ) {
    viewer.handle.setPivotPoint(state.pivot[0], state.pivot[1]);
  }
  if ('showPivotMarker' in state && state.showPivotMarker !== undefined) {
    viewer.handle.setShowPivotMarker(state.showPivotMarker);
  }
  if ('overlayMessage' in state && state.overlayMessage !== undefined) {
    viewer.handle.setOverlayMessage(state.overlayMessage);
  }
  if ('markers' in state && state.markers !== undefined) {
    setMarkers(containerId, state.markers);
  }
  // Apply zoom last so explicit zoom takes precedence over bounds-derived zoom.
  if ('zoom' in state && state.zoom !== undefined) {
    viewer.handle.setZoom(state.zoom);
  }
}

/**
 * Register a callback to be called when the viewer state changes.
 *
 * The callback receives an object with the current state:
 * { contrast, bias, stretchMode, zoom, colormap, colormapReversed, vmin, vmax, xlim, ylim }
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {Function} callback - Callback function to receive state updates.
 * @throws {Error} If the viewer is not found.
 */
export function onStateChange(containerId, callback) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.onStateChange(callback);
}

/**
 * Register a callback to be called when the user shift-clicks in the viewer.
 *
 * The callback receives the click coordinates in data space: { x, y }
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @param {Function} callback - Callback function to receive click events.
 * @throws {Error} If the viewer is not found.
 */
export function onClick(containerId, callback) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.onClick(callback);
}

/**
 * Clear all registered callbacks for a viewer.
 *
 * @param {string} containerId - The ID of the container (viewer instance).
 * @throws {Error} If the viewer is not found.
 */
export function clearCallbacks(containerId) {
  const viewer = viewers.get(containerId);
  if (!viewer) {
    throw new Error(`No viewer found for container "${containerId}"`);
  }
  viewer.handle.clearCallbacks();
}

window.viewarr = {
  createViewer,
  setImageData,
  destroyViewer,
  hasViewer,
  getActiveViewers,
  getZoom,
  setZoom,
  getContrast,
  setContrast,
  getBias,
  setBias,
  getStretchMode,
  setStretchMode,
  getViewBounds,
  setViewBounds,
  getColormap,
  setColormap,
  getColormapReversed,
  setColormapReversed,
  getValueRange,
  setValueRange,
  getRotation,
  setRotation,
  getPivotPoint,
  setPivotPoint,
  getShowPivotMarker,
  setShowPivotMarker,
  getOverlayMessage,
  setOverlayMessage,
  getMarkers,
  setMarkers,
  setViewerState,
  onStateChange,
  onClick,
  clearCallbacks
};

// Default export for convenience
export default {
  createViewer,
  setImageData,
  destroyViewer,
  hasViewer,
  getActiveViewers,
  getZoom,
  setZoom,
  getContrast,
  setContrast,
  getBias,
  setBias,
  getStretchMode,
  setStretchMode,
  getViewBounds,
  setViewBounds,
  getColormap,
  setColormap,
  getColormapReversed,
  setColormapReversed,
  getValueRange,
  setValueRange,
  getRotation,
  setRotation,
  getPivotPoint,
  setPivotPoint,
  getShowPivotMarker,
  setShowPivotMarker,
  getOverlayMessage,
  setOverlayMessage,
  getMarkers,
  setMarkers,
  setViewerState,
  onStateChange,
  onClick,
  clearCallbacks
};
