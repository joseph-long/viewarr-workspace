/**
 * TypeScript type definitions for viewarr
 */

/**
 * JavaScript TypedArray type names supported by viewarr.
 */
export type ArrayType =
  | 'Int8Array'
  | 'Uint8Array'
  | 'Int16Array'
  | 'Uint16Array'
  | 'Int32Array'
  | 'Uint32Array'
  | 'BigInt64Array'
  | 'BigUint64Array'
  | 'Float32Array'
  | 'Float64Array';

export type StretchMode = 'linear' | 'log' | 'symmetric';

export interface ViewerStateConfig {
  contrast?: number;
  bias?: number;
  stretchMode?: StretchMode;
  zoom?: number;
  colormap?: string;
  colormapReversed?: boolean;
  vmin?: number;
  vmax?: number;
  xlim?: [number, number];
  ylim?: [number, number];
  rotation?: number;
  pivot?: [number, number];
  showPivotMarker?: boolean;
  overlayMessage?: string;
  markers?: [number, number][];
}

/**
 * Create a new viewer instance in the specified container.
 *
 * @param containerId - The ID of the HTML element to use as the container.
 *                      This ID is also used to identify the viewer instance.
 * @returns Resolves when the viewer is ready.
 * @throws If the container is not found or initialization fails.
 */
export function createViewer(containerId: string): Promise<void>;

/**
 * Set image data for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param buffer - The raw pixel data.
 * @param width - Image width in pixels.
 * @param height - Image height in pixels.
 * @param arrayType - JavaScript TypedArray type name for interpreting the buffer.
 * @throws If the viewer is not found or data is invalid.
 */
export function setImageData(
  containerId: string,
  buffer: ArrayBuffer,
  width: number,
  height: number,
  arrayType: ArrayType
): void;

/**
 * Declare the sliceable leading axes of an N-D cube.
 *
 * The widget renders a slider + play control per axis and requests slices via
 * the `onSliceRequest` callback. Pass an empty array for a plain 2D image.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param dims - Lengths of the leading (sliceable) axes, outer→inner order.
 */
export function setCube(containerId: string, dims: number[]): void;

/**
 * Set image data for a specific cube slice.
 *
 * Like {@link setImageData} but tagged with the slice indices it represents, so
 * the widget can sync slider positions and correlate play-mode prefetches.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param buffer - The raw pixel data.
 * @param width - Image width in pixels.
 * @param height - Image height in pixels.
 * @param arrayType - JavaScript TypedArray type name for interpreting the buffer.
 * @param indices - Slice indices this image corresponds to (may be empty).
 */
export function setSliceData(
  containerId: string,
  buffer: ArrayBuffer,
  width: number,
  height: number,
  arrayType: ArrayType,
  indices: number[]
): void;

/**
 * Destroy a viewer instance and clean up resources.
 *
 * @param containerId - The ID of the container (viewer instance).
 */
export function destroyViewer(containerId: string): void;

/**
 * Check if a viewer exists for a given container.
 *
 * @param containerId - The ID of the container.
 * @returns True if a viewer exists.
 */
export function hasViewer(containerId: string): boolean;

/**
 * Get all active viewer IDs.
 *
 * @returns Array of container IDs with active viewers.
 */
export function getActiveViewers(): string[];

/**
 * Get current zoom level for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns Zoom level (1.0 means fit-to-view).
 */
export function getZoom(containerId: string): number;

/**
 * Set zoom level for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param zoom - Zoom level (1.0 means fit-to-view).
 */
export function setZoom(containerId: string, zoom: number): void;

/**
 * Get current contrast value for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns Contrast value (0.0 to 10.0, default 1.0).
 */
export function getContrast(containerId: string): number;

/**
 * Set contrast value for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param contrast - Contrast value (0.0 to 10.0).
 */
export function setContrast(containerId: string, contrast: number): void;

/**
 * Get current bias value for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns Bias value (0.0 to 1.0, default 0.5).
 */
export function getBias(containerId: string): number;

/**
 * Set bias value for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param bias - Bias value (0.0 to 1.0).
 */
export function setBias(containerId: string, bias: number): void;

/**
 * Get current stretch mode for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns Stretch mode: "linear", "log", or "symmetric".
 */
export function getStretchMode(containerId: string): StretchMode;

/**
 * Set stretch mode for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param mode - Stretch mode: "linear", "log", or "symmetric".
 */
export function setStretchMode(containerId: string, mode: StretchMode): void;

/**
 * Get visible image bounds in pixel coordinates.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns Array [xmin, xmax, ymin, ymax] in pixel coordinates.
 */
export function getViewBounds(containerId: string): [number, number, number, number];

/**
 * Set view to show specific image bounds.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param xmin - Minimum x coordinate in pixels.
 * @param xmax - Maximum x coordinate in pixels.
 * @param ymin - Minimum y coordinate in pixels.
 * @param ymax - Maximum y coordinate in pixels.
 */
export function setViewBounds(
  containerId: string,
  xmin: number,
  xmax: number,
  ymin: number,
  ymax: number
): void;

/**
 * Get the colormap name for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns Colormap name (e.g., "gray", "inferno", "magma", "RdBu").
 */
export function getColormap(containerId: string): string;

/**
 * Set the colormap name for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param colormap - Colormap name (e.g., "gray", "inferno", "magma", "RdBu").
 */
export function setColormap(containerId: string, colormap: string): void;

/**
 * Get whether the colormap is reversed.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns True if the colormap is reversed.
 */
export function getColormapReversed(containerId: string): boolean;

/**
 * Set whether the colormap is reversed.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param reversed - True if the colormap should be reversed.
 */
export function setColormapReversed(containerId: string, reversed: boolean): void;

/**
 * Get the image value range (vmin, vmax).
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns Array [vmin, vmax].
 */
export function getValueRange(containerId: string): [number, number];

/**
 * Get current rotation angle in degrees (counter-clockwise).
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns Rotation angle in degrees.
 */
export function getRotation(containerId: string): number;

/**
 * Set rotation angle in degrees (counter-clockwise).
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param degrees - Rotation angle in degrees.
 */
export function setRotation(containerId: string, degrees: number): void;

/**
 * Get pivot point in image coordinates.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns Array [x, y] in image coordinates.
 */
export function getPivotPoint(containerId: string): [number, number];

/**
 * Set pivot point in image coordinates.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param x - X coordinate in image pixels.
 * @param y - Y coordinate in image pixels.
 */
export function setPivotPoint(containerId: string, x: number, y: number): void;

/**
 * Get whether the pivot marker is visible.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns True if the pivot marker is visible.
 */
export function getShowPivotMarker(containerId: string): boolean;

/**
 * Set whether to show the pivot marker.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param show - True to show the pivot marker.
 */
export function setShowPivotMarker(containerId: string, show: boolean): void;

/**
 * Get the viewer overlay message.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns Overlay message string.
 */
export function getOverlayMessage(containerId: string): string;

/**
 * Set the viewer overlay message.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param message - Overlay message (empty string hides the overlay).
 */
export function setOverlayMessage(containerId: string, message: string): void;

/**
 * Get point markers from the viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @returns Marker points in image coordinates.
 */
export function getMarkers(containerId: string): [number, number][];

/**
 * Set point markers in the viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param markers - Marker points in image coordinates.
 */
export function setMarkers(containerId: string, markers: [number, number][]): void;

/**
 * Apply viewer state from a partial object.
 *
 * Missing keys are ignored.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param state - Partial viewer state object.
 */
export function setViewerState(containerId: string, state: ViewerStateConfig): void;

/**
 * State object passed to state change callbacks.
 */
export interface ViewerState {
  contrast: number;
  bias: number;
  stretchMode: StretchMode;
  zoom: number;
  colormap: string;
  colormapReversed: boolean;
  vmin: number;
  vmax: number;
  xlim?: [number, number];
  ylim?: [number, number];
  rotation: number;
  pivot: [number, number];
  showPivotMarker: boolean;
}

/**
 * Click event object passed to click callbacks.
 */
export interface ClickEvent {
  x: number;
  y: number;
}

/**
 * Register a callback to be called when the viewer state changes.
 *
 * The callback receives an object with the current state.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param callback - Callback function to receive state updates.
 */
export function onStateChange(
  containerId: string,
  callback: (state: ViewerState) => void
): void;

/**
 * Register a callback to be called when the user shift-clicks in the viewer.
 *
 * The callback receives the click coordinates in data space.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param callback - Callback function to receive click events.
 */
export function onClick(
  containerId: string,
  callback: (event: ClickEvent) => void
): void;

/**
 * Register a callback invoked when the widget needs a cube slice fetched
 * (slider drag or play loop). The callback receives the requested slice indices;
 * the host should fetch that slice and deliver it via {@link setSliceData}.
 *
 * @param containerId - The ID of the container (viewer instance).
 * @param callback - Callback function to receive requested slice indices.
 */
export function onSliceRequest(
  containerId: string,
  callback: (indices: number[]) => void
): void;

/**
 * Clear all registered callbacks for a viewer.
 *
 * @param containerId - The ID of the container (viewer instance).
 */
export function clearCallbacks(containerId: string): void;

declare const viewarr: {
  createViewer: typeof createViewer;
  setImageData: typeof setImageData;
  setCube: typeof setCube;
  setSliceData: typeof setSliceData;
  destroyViewer: typeof destroyViewer;
  hasViewer: typeof hasViewer;
  getActiveViewers: typeof getActiveViewers;
  getContrast: typeof getContrast;
  setContrast: typeof setContrast;
  getBias: typeof getBias;
  setBias: typeof setBias;
  getStretchMode: typeof getStretchMode;
  setStretchMode: typeof setStretchMode;
  getViewBounds: typeof getViewBounds;
  setViewBounds: typeof setViewBounds;
  getColormap: typeof getColormap;
  getColormapReversed: typeof getColormapReversed;
  getValueRange: typeof getValueRange;
  getRotation: typeof getRotation;
  setRotation: typeof setRotation;
  getPivotPoint: typeof getPivotPoint;
  setPivotPoint: typeof setPivotPoint;
  getShowPivotMarker: typeof getShowPivotMarker;
  setShowPivotMarker: typeof setShowPivotMarker;
  getOverlayMessage: typeof getOverlayMessage;
  setOverlayMessage: typeof setOverlayMessage;
  getMarkers: typeof getMarkers;
  setMarkers: typeof setMarkers;
  onStateChange: typeof onStateChange;
  onClick: typeof onClick;
  onSliceRequest: typeof onSliceRequest;
  clearCallbacks: typeof clearCallbacks;
};

export default viewarr;
